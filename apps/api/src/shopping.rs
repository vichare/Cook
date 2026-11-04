use std::collections::{BTreeMap, HashMap, HashSet};

use axum::{
    extract::{Path, State},
    http::{header::ETAG, HeaderMap, HeaderValue, StatusCode},
    Extension, Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    ingredients::{bounded_levenshtein, clean_display_name, normalize_ingredient},
    models::{
        PantryItem, PantryPurchaseCandidate, PantryStatus, ShoppingChoiceAlternative, ShoppingItem,
        ShoppingItemInput, ShoppingItemSource, ShoppingPurchaseOptions,
    },
    routes::CurrentUser,
    AppState,
};

const PURCHASE_REPLAY_SECONDS: i64 = 7 * 24 * 60 * 60;
const PURCHASE_CLEANUP_BATCH: i64 = 100;
const PURCHASE_ACTION_CAP: i64 = 200;
const PURCHASE_RATE_LIMIT: i64 = 60;
const MAX_PANTRY_ITEMS_PER_OWNER: i64 = 500;
const MAX_SHOPPING_ITEMS_PER_OWNER: i64 = 500;
const MAX_SHOPPING_SOURCES_PER_OWNER: i64 = 10_000;
const MAX_INGREDIENT_ALIASES_PER_OWNER: i64 = 2_000;
const PURCHASE_RATE_WINDOW_SECONDS: i64 = 60;
const DELETE_ACTION_CAP: i64 = 200;
const DELETE_RATE_LIMIT: i64 = 60;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecipeShoppingInput {
    #[serde(default)]
    name: String,
    #[serde(default)]
    canonical_ingredient_id: Option<String>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    alternative_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PurchaseInput {
    disposition: PurchaseDisposition,
    shopping_expected_version: i64,
    #[serde(default)]
    pantry_item_id: Option<String>,
    #[serde(default)]
    create_using_current_name: bool,
    #[serde(default)]
    pantry_expected_version: Option<i64>,
    #[serde(default)]
    expires_on: Option<String>,
    #[serde(default)]
    quantity_note: String,
    #[serde(default)]
    auto_add_to_shopping_list_when_out: bool,
    #[serde(default)]
    selected_alternative_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum PurchaseDisposition {
    Remove,
    Keep,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteInput {
    expected_version: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DeleteOutcome {
    Removed,
    AutomaticRemains,
    AutomaticOnly,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResponse {
    outcome: DeleteOutcome,
    requires_disable_auto_restock: bool,
    shopping_items: Vec<ShoppingItem>,
    undo_available: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseResponse {
    pantry: PantryItem,
    shopping_items: Vec<ShoppingItem>,
    purchase_action_id: String,
    undo_available: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PantrySnapshot {
    id: String,
    name: String,
    normalized_name: String,
    status: String,
    quantity_note: String,
    confirmed_at: Option<String>,
    expires_on: Option<String>,
    canonical_ingredient_id: Option<String>,
    auto_add_to_shopping_list_when_out: bool,
    version: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShoppingSnapshot {
    id: String,
    name: String,
    normalized_name: String,
    canonical_ingredient_id: Option<String>,
    #[serde(default)]
    choice_key: Option<String>,
    #[serde(default)]
    choice_alternatives: Vec<ShoppingChoiceAlternative>,
    version: i64,
    sources: Vec<SourceSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SourceSnapshot {
    id: String,
    source_type: String,
    recipe_id: Option<String>,
    label: String,
    created_at: i64,
}

#[derive(Debug)]
struct StoredItem {
    id: String,
    name: String,
    normalized_name: String,
    canonical_ingredient_id: Option<String>,
    choice_alternatives: Vec<ShoppingChoiceAlternative>,
    sources: Vec<ShoppingItemSource>,
    version: i64,
}

struct PantryCandidateRow {
    id: String,
    name: String,
    normalized_name: String,
    status: PantryStatus,
    quantity_note: String,
    expires_on: Option<String>,
    canonical_ingredient_id: Option<String>,
    auto_add_to_shopping_list_when_out: bool,
    version: i64,
    canonical_name: Option<String>,
}

struct PantryCandidateContext {
    rows: Vec<PantryCandidateRow>,
    aliases: HashMap<String, HashSet<String>>,
}

pub async fn list_shopping_items(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<(HeaderMap, Json<Vec<ShoppingItem>>)> {
    let items = load_shopping_items(&state, &user.id).await?;
    let mut headers = HeaderMap::new();
    headers.insert(ETAG, state_etag(&items)?);
    Ok((headers, Json(items)))
}

pub async fn create_manual_item(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(input): Json<ShoppingItemInput>,
) -> AppResult<(StatusCode, Json<Vec<ShoppingItem>>)> {
    let mut transaction = state.pool.begin().await?;
    let (item_id, _) = upsert_stored_item(
        &mut transaction,
        &user.id,
        &input.name,
        input.canonical_ingredient_id.as_deref(),
    )
    .await?;
    insert_source(&mut transaction, &item_id, "manual", None, "").await?;
    transaction.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(load_shopping_items(&state, &user.id).await?),
    ))
}

pub async fn update_manual_item(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(item_id): Path<String>,
    Json(input): Json<ShoppingItemInput>,
) -> AppResult<Json<Vec<ShoppingItem>>> {
    let mut transaction = state.pool.begin().await?;
    require_manual_source(&mut transaction, &user.id, &item_id).await?;
    let (name, normalized) = validate_name(&input.name)?;
    validate_selected_canonical(&mut transaction, input.canonical_ingredient_id.as_deref()).await?;
    let result = sqlx::query(
        "UPDATE shopping_items
         SET name = ?, normalized_name = ?, canonical_ingredient_id = ?,
             completed_at = NULL, version = version + 1, updated_at = ?
         WHERE id = ? AND owner_id = ?",
    )
    .bind(name)
    .bind(normalized)
    .bind(input.canonical_ingredient_id)
    .bind(Utc::now().timestamp())
    .bind(&item_id)
    .bind(&user.id)
    .execute(&mut *transaction)
    .await
    .map_err(map_unique_conflict)?;
    if result.rows_affected() != 1 {
        return Err(AppError::NotFound);
    }
    transaction.commit().await?;
    Ok(Json(load_shopping_items(&state, &user.id).await?))
}

pub async fn delete_item(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    Path(item_id): Path<String>,
    Json(input): Json<DeleteInput>,
) -> AppResult<Json<DeleteResponse>> {
    if input.expected_version < 1 {
        return Err(AppError::Validation(
            "expectedVersion must be at least 1".into(),
        ));
    }
    let now = Utc::now().timestamp();
    let key = idempotency_key(&headers, now)?;
    let fingerprint: [u8; 32] =
        Sha256::digest(serde_json::to_vec(&(&item_id, &input)).unwrap()).into();
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    cleanup_delete_actions(&mut transaction, now).await?;
    if let Some(response) = replay_delete(&mut transaction, &user.id, &key, &fingerprint).await? {
        transaction.commit().await?;
        return Ok(Json(response));
    }
    enforce_delete_rate_limit(&mut transaction, &user.id, now).await?;
    enforce_delete_capacity(&mut transaction, &user.id, now).await?;

    let response = if let Some(snapshot) =
        load_shopping_snapshot(&mut transaction, &user.id, &item_id).await?
    {
        if snapshot.version != input.expected_version {
            return Err(AppError::Conflict(
                "shopping item changed; refresh and retry".into(),
            ));
        }
        let automatic_remains =
            has_automatic_identity(&mut transaction, &user.id, &snapshot).await?;
        let deleted =
            sqlx::query("DELETE FROM shopping_items WHERE id = ? AND owner_id = ? AND version = ?")
                .bind(&item_id)
                .bind(&user.id)
                .bind(input.expected_version)
                .execute(&mut *transaction)
                .await?;
        if deleted.rows_affected() != 1 {
            return Err(AppError::Conflict(
                "shopping item changed; refresh and retry".into(),
            ));
        }
        DeleteResponse {
            outcome: if automatic_remains {
                DeleteOutcome::AutomaticRemains
            } else {
                DeleteOutcome::Removed
            },
            requires_disable_auto_restock: automatic_remains,
            shopping_items: load_shopping_items_tx(&mut transaction, &user.id).await?,
            undo_available: false,
        }
    } else if let Some(pantry_id) = item_id.strip_prefix("auto:") {
        let pantry = load_pantry_snapshot(&mut transaction, &user.id, pantry_id)
            .await?
            .filter(|item| item.status == "out" && item.auto_add_to_shopping_list_when_out)
            .ok_or(AppError::NotFound)?;
        if pantry.version != input.expected_version {
            return Err(AppError::Conflict(
                "shopping item changed; refresh and retry".into(),
            ));
        }
        DeleteResponse {
            outcome: DeleteOutcome::AutomaticOnly,
            requires_disable_auto_restock: true,
            shopping_items: load_shopping_items_tx(&mut transaction, &user.id).await?,
            undo_available: false,
        }
    } else {
        return Err(AppError::NotFound);
    };
    sqlx::query(
        "INSERT INTO shopping_delete_actions
         (id, owner_id, shopping_item_id, idempotency_key, request_fingerprint,
          response_json, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&user.id)
    .bind(&item_id)
    .bind(&key)
    .bind(fingerprint.as_slice())
    .bind(serde_json::to_string(&response).map_err(|error| AppError::Data(error.to_string()))?)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Json(response))
}

pub async fn add_recipe_ingredient(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(recipe_id): Path<String>,
    Json(input): Json<RecipeShoppingInput>,
) -> AppResult<(StatusCode, Json<Vec<ShoppingItem>>)> {
    let mut transaction = state.pool.begin().await?;
    if input.group_id.is_none() && input.alternative_id.is_some() {
        return Err(AppError::Validation(
            "alternativeId requires groupId".into(),
        ));
    }
    let recipe_title: String = sqlx::query_scalar(
        "SELECT title FROM recipes
         WHERE id = ? AND (owner_id = ? OR visibility = 'public')",
    )
    .bind(&recipe_id)
    .bind(&user.id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::NotFound)?;
    let item_id = if let Some(group_id) = input.group_id.as_deref() {
        let rows = sqlx::query(
            "SELECT a.external_id AS id, a.name, a.canonical_ingredient_id
             FROM recipe_ingredient_groups g
             JOIN recipe_ingredient_alternatives a ON a.group_id = g.id
             WHERE g.external_id = ? AND g.recipe_id = ?
             ORDER BY a.sort_order",
        )
        .bind(group_id)
        .bind(&recipe_id)
        .fetch_all(&mut *transaction)
        .await?;
        if rows.is_empty() {
            return Err(AppError::NotFound);
        }
        let alternatives = rows
            .into_iter()
            .map(|row| ShoppingChoiceAlternative {
                id: row.get("id"),
                name: row.get("name"),
                canonical_ingredient_id: row.get("canonical_ingredient_id"),
                purchase_options: None,
            })
            .collect::<Vec<_>>();
        if input.alternative_id.is_some() || alternatives.len() == 1 {
            let alternative = if let Some(alternative_id) = input.alternative_id.as_deref() {
                alternatives
                    .iter()
                    .find(|alternative| alternative.id == alternative_id)
                    .ok_or_else(|| {
                        AppError::Validation(
                            "selected alternative does not belong to the ingredient group".into(),
                        )
                    })?
            } else {
                &alternatives[0]
            };
            upsert_stored_item(
                &mut transaction,
                &user.id,
                &alternative.name,
                alternative.canonical_ingredient_id.as_deref(),
            )
            .await?
            .0
        } else {
            upsert_choice_item(&mut transaction, &user.id, &alternatives)
                .await?
                .0
        }
    } else {
        let normalized = normalize_ingredient(&input.name);
        let ingredient = sqlx::query(
            "SELECT ri.name, ri.canonical_ingredient_id
             FROM recipe_ingredients ri
             WHERE ri.recipe_id = ? AND ((? IS NOT NULL AND ri.canonical_ingredient_id = ?)
               OR (? IS NULL AND ri.normalized_name = ?))
             ORDER BY ri.sort_order LIMIT 1",
        )
        .bind(&recipe_id)
        .bind(&input.canonical_ingredient_id)
        .bind(&input.canonical_ingredient_id)
        .bind(&input.canonical_ingredient_id)
        .bind(&normalized)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(AppError::NotFound)?;
        upsert_stored_item(
            &mut transaction,
            &user.id,
            &ingredient.get::<String, _>("name"),
            ingredient
                .get::<Option<String>, _>("canonical_ingredient_id")
                .as_deref(),
        )
        .await?
        .0
    };
    insert_source(
        &mut transaction,
        &item_id,
        "recipe",
        Some(&recipe_id),
        &recipe_title,
    )
    .await?;
    transaction.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(load_shopping_items(&state, &user.id).await?),
    ))
}

pub async fn purchase_item(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    Path(item_id): Path<String>,
    Json(input): Json<PurchaseInput>,
) -> AppResult<Json<PurchaseResponse>> {
    validate_purchase_input(&input)?;
    let now = Utc::now().timestamp();
    let key = idempotency_key(&headers, now)?;
    let fingerprint: [u8; 32] =
        Sha256::digest(serde_json::to_vec(&(&item_id, &input)).unwrap()).into();
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    cleanup_purchase_actions(&mut transaction, now).await?;
    if let Some(response) = replay_purchase(&mut transaction, &user.id, &key, &fingerprint).await? {
        transaction.commit().await?;
        return Ok(Json(response));
    }
    enforce_purchase_rate_limit(&mut transaction, &user.id, now).await?;
    enforce_purchase_capacity(&mut transaction, &user.id, now).await?;

    let shopping_before = load_shopping_snapshot(&mut transaction, &user.id, &item_id).await?;
    let automatic_pantry_id = item_id.strip_prefix("auto:").map(str::to_string);
    let (name, normalized_name, canonical_id, shopping_version) =
        if let Some(snapshot) = &shopping_before {
            let selected_choice = if snapshot.choice_alternatives.is_empty() {
                if input.selected_alternative_id.is_some() {
                    return Err(AppError::Validation(
                        "selectedAlternativeId is only valid for a shopping choice".into(),
                    ));
                }
                None
            } else {
                let selected_id = input.selected_alternative_id.as_deref().ok_or_else(|| {
                    AppError::Validation(
                        "selectedAlternativeId is required for a shopping choice".into(),
                    )
                })?;
                Some(
                    snapshot
                        .choice_alternatives
                        .iter()
                        .find(|alternative| alternative.id == selected_id)
                        .ok_or_else(|| {
                            AppError::Validation(
                                "selectedAlternativeId is not in this shopping choice".into(),
                            )
                        })?,
                )
            };
            (
                selected_choice
                    .map(|alternative| alternative.name.clone())
                    .unwrap_or_else(|| snapshot.name.clone()),
                selected_choice
                    .map(|alternative| normalize_ingredient(&alternative.name))
                    .unwrap_or_else(|| snapshot.normalized_name.clone()),
                selected_choice
                    .and_then(|alternative| alternative.canonical_ingredient_id.clone())
                    .or_else(|| snapshot.canonical_ingredient_id.clone()),
                snapshot.version,
            )
        } else if let Some(pantry_id) = &automatic_pantry_id {
            let snapshot = load_pantry_snapshot(&mut transaction, &user.id, pantry_id)
                .await?
                .ok_or(AppError::NotFound)?;
            if snapshot.status != "out" || !snapshot.auto_add_to_shopping_list_when_out {
                return Err(AppError::NotFound);
            }
            (
                snapshot.name,
                snapshot.normalized_name,
                snapshot.canonical_ingredient_id,
                snapshot.version,
            )
        } else {
            return Err(AppError::NotFound);
        };
    if shopping_version != input.shopping_expected_version {
        return Err(AppError::Conflict("shopping item changed".into()));
    }

    let candidates = pantry_candidates_tx(
        &mut transaction,
        &user.id,
        &normalized_name,
        canonical_id.as_deref(),
    )
    .await?;
    let automatic_match =
        deterministic_reuse(&candidates, canonical_id.as_deref(), &normalized_name);
    if input.create_using_current_name && automatic_match.is_some() {
        return Err(AppError::Conflict(
            "an exact pantry item already exists".into(),
        ));
    }
    let selected_id = if input.create_using_current_name {
        None
    } else {
        input.pantry_item_id.clone().or(automatic_match)
    };
    if selected_id.is_none() && !input.create_using_current_name && !candidates.is_empty() {
        return Err(AppError::Conflict("pantry match choice is required".into()));
    }
    let pantry_before = if let Some(id) = &selected_id {
        let candidate_allowed = candidates.iter().any(|candidate| candidate.id == *id)
            || automatic_pantry_id.as_deref() == Some(id.as_str());
        if !candidate_allowed {
            return Err(AppError::Conflict(
                "selected pantry item is not a current match".into(),
            ));
        }
        let snapshot = load_pantry_snapshot(&mut transaction, &user.id, id)
            .await?
            .ok_or(AppError::NotFound)?;
        if input.pantry_expected_version != Some(snapshot.version) {
            return Err(AppError::Conflict("pantry item changed".into()));
        }
        Some(snapshot)
    } else {
        let pantry_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pantry_items WHERE owner_id = ?")
                .bind(&user.id)
                .fetch_one(&mut *transaction)
                .await?;
        if pantry_count >= MAX_PANTRY_ITEMS_PER_OWNER {
            return Err(AppError::Validation("pantry item limit exceeded".into()));
        }
        None
    };
    let pantry_id = selected_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let pantry_canonical_id = pantry_before
        .as_ref()
        .and_then(|item| item.canonical_ingredient_id.clone())
        .or(canonical_id.clone());
    let pantry_name = pantry_before
        .as_ref()
        .map(|item| item.name.clone())
        .unwrap_or(name);
    let pantry_normalized = normalize_ingredient(&pantry_name);
    let pantry_version = pantry_before.as_ref().map_or(1, |item| item.version + 1);
    sqlx::query(
        "INSERT INTO pantry_items
         (id, owner_id, name, normalized_name, status, quantity_note, confirmed_at, expires_on,
          canonical_ingredient_id, auto_add_to_shopping_list_when_out, version)
         VALUES (?, ?, ?, ?, 'have', ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET status = 'have', quantity_note = excluded.quantity_note,
           confirmed_at = excluded.confirmed_at, expires_on = excluded.expires_on,
           auto_add_to_shopping_list_when_out = excluded.auto_add_to_shopping_list_when_out,
           version = excluded.version, updated_at = CURRENT_TIMESTAMP",
    )
    .bind(&pantry_id)
    .bind(&user.id)
    .bind(&pantry_name)
    .bind(&pantry_normalized)
    .bind(input.quantity_note.trim())
    .bind(Utc::now().to_rfc3339())
    .bind(input.expires_on.as_deref())
    .bind(&pantry_canonical_id)
    .bind(input.auto_add_to_shopping_list_when_out)
    .bind(pantry_version)
    .execute(&mut *transaction)
    .await
    .map_err(map_unique_conflict)?;

    let mut action_shopping_item_id = item_id.clone();
    let mut action_shopping_before = shopping_before.clone();
    let result_shopping_version = match (input.disposition, &shopping_before) {
        (PurchaseDisposition::Remove, Some(snapshot)) => {
            sqlx::query("DELETE FROM shopping_items WHERE id = ? AND owner_id = ? AND version = ?")
                .bind(&snapshot.id)
                .bind(&user.id)
                .bind(snapshot.version)
                .execute(&mut *transaction)
                .await?;
            None
        }
        (PurchaseDisposition::Keep, Some(snapshot)) => {
            let version = snapshot.version + 1;
            sqlx::query("UPDATE shopping_items SET version = ?, updated_at = ? WHERE id = ?")
                .bind(version)
                .bind(now)
                .bind(&snapshot.id)
                .execute(&mut *transaction)
                .await?;
            Some(version)
        }
        (PurchaseDisposition::Keep, None) => {
            let existing = load_shopping_snapshot_by_identity(
                &mut transaction,
                &user.id,
                pantry_canonical_id.as_deref(),
                &normalize_ingredient(&pantry_name),
            )
            .await?;
            let (id, version) = upsert_stored_item(
                &mut transaction,
                &user.id,
                &pantry_name,
                pantry_canonical_id.as_deref(),
            )
            .await?;
            insert_source(&mut transaction, &id, "manual", None, "").await?;
            action_shopping_item_id = id;
            action_shopping_before = existing;
            Some(version)
        }
        _ => None,
    };
    let action_id = Uuid::new_v4().to_string();
    let mut pantry = pantry_from_snapshot(
        load_pantry_snapshot(&mut transaction, &user.id, &pantry_id)
            .await?
            .ok_or(AppError::NotFound)?,
    );
    pantry.aliases = load_aliases_tx(
        &mut transaction,
        &user.id,
        pantry.canonical_ingredient_id.as_deref(),
    )
    .await?;
    let shopping_items = load_shopping_items_tx(&mut transaction, &user.id).await?;
    let response = PurchaseResponse {
        pantry,
        shopping_items,
        purchase_action_id: action_id.clone(),
        undo_available: true,
    };
    sqlx::query(
        "INSERT INTO shopping_purchase_actions
         (id, owner_id, shopping_item_id, pantry_item_id, idempotency_key, request_fingerprint,
          disposition, pantry_before_json, shopping_before_json, result_pantry_version,
          result_shopping_version, response_json, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&action_id)
    .bind(&user.id)
    .bind(&action_shopping_item_id)
    .bind(&pantry_id)
    .bind(&key)
    .bind(fingerprint.as_slice())
    .bind(match input.disposition {
        PurchaseDisposition::Remove => "remove",
        PurchaseDisposition::Keep => "keep",
    })
    .bind(
        pantry_before
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?,
    )
    .bind(
        action_shopping_before
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?,
    )
    .bind(pantry_version)
    .bind(result_shopping_version)
    .bind(serde_json::to_string(&response).map_err(|error| AppError::Data(error.to_string()))?)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Json(response))
}

pub async fn undo_purchase(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(action_id): Path<String>,
) -> AppResult<Json<Vec<ShoppingItem>>> {
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let row = sqlx::query(
        "SELECT shopping_item_id, pantry_item_id, disposition, pantry_before_json, shopping_before_json,
                result_pantry_version, result_shopping_version, undone_at
         FROM shopping_purchase_actions WHERE id = ? AND owner_id = ?",
    )
    .bind(&action_id)
    .bind(&user.id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::NotFound)?;
    let undone_at: Option<i64> = row.get("undone_at");
    if undone_at.is_some() {
        return Err(AppError::Conflict("purchase was already undone".into()));
    }
    let pantry_id: String = row.get("pantry_item_id");
    let result_pantry_version: i64 = row.get("result_pantry_version");
    let current_pantry_version: Option<i64> =
        sqlx::query_scalar("SELECT version FROM pantry_items WHERE id = ? AND owner_id = ?")
            .bind(&pantry_id)
            .bind(&user.id)
            .fetch_optional(&mut *transaction)
            .await?;
    if current_pantry_version != Some(result_pantry_version) {
        return Err(AppError::Conflict("pantry changed after purchase".into()));
    }
    let shopping_before: Option<ShoppingSnapshot> = row
        .get::<Option<String>, _>("shopping_before_json")
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(|error| AppError::Data(error.to_string()))?;
    let disposition: String = row.get("disposition");
    let result_shopping_item_id: String = row.get("shopping_item_id");
    if let Some(snapshot) = &shopping_before {
        let current_version: Option<i64> =
            sqlx::query_scalar("SELECT version FROM shopping_items WHERE id = ? AND owner_id = ?")
                .bind(&snapshot.id)
                .bind(&user.id)
                .fetch_optional(&mut *transaction)
                .await?;
        let expected = if disposition == "remove" {
            None
        } else {
            row.get("result_shopping_version")
        };
        if current_version != expected {
            return Err(AppError::Conflict(
                "shopping item changed after purchase".into(),
            ));
        }
    } else if disposition == "keep" {
        let current_version: Option<i64> =
            sqlx::query_scalar("SELECT version FROM shopping_items WHERE id = ? AND owner_id = ?")
                .bind(&result_shopping_item_id)
                .bind(&user.id)
                .fetch_optional(&mut *transaction)
                .await?;
        if current_version != row.get("result_shopping_version") {
            return Err(AppError::Conflict(
                "shopping item changed after purchase".into(),
            ));
        }
    }
    let pantry_before: Option<PantrySnapshot> = row
        .get::<Option<String>, _>("pantry_before_json")
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(|error| AppError::Data(error.to_string()))?;
    if let Some(snapshot) = pantry_before {
        restore_pantry(&mut transaction, &user.id, &snapshot).await?;
    } else {
        sqlx::query("DELETE FROM pantry_items WHERE id = ? AND owner_id = ? AND version = ?")
            .bind(&pantry_id)
            .bind(&user.id)
            .bind(result_pantry_version)
            .execute(&mut *transaction)
            .await?;
    }
    if disposition == "remove" {
        if let Some(snapshot) = shopping_before {
            let identity_conflict: Option<String> = if let Some(choice_key) = &snapshot.choice_key {
                sqlx::query_scalar(
                    "SELECT id FROM shopping_items
                         WHERE owner_id = ? AND choice_key = ? AND completed_at IS NULL",
                )
                .bind(&user.id)
                .bind(choice_key)
                .fetch_optional(&mut *transaction)
                .await?
            } else if let Some(canonical_id) = &snapshot.canonical_ingredient_id {
                sqlx::query_scalar(
                    "SELECT id FROM shopping_items
                     WHERE owner_id = ? AND canonical_ingredient_id = ?
                       AND completed_at IS NULL",
                )
                .bind(&user.id)
                .bind(canonical_id)
                .fetch_optional(&mut *transaction)
                .await?
            } else {
                sqlx::query_scalar(
                    "SELECT id FROM shopping_items
                     WHERE owner_id = ? AND canonical_ingredient_id IS NULL
                       AND normalized_name = ? AND completed_at IS NULL",
                )
                .bind(&user.id)
                .bind(&snapshot.normalized_name)
                .fetch_optional(&mut *transaction)
                .await?
            };
            if identity_conflict.is_some() {
                return Err(AppError::Conflict(
                    "shopping ingredient was re-added after purchase".into(),
                ));
            }
            restore_shopping(&mut transaction, &user.id, &snapshot).await?;
        }
    } else if let Some(snapshot) = shopping_before {
        sqlx::query(
            "DELETE FROM shopping_items
             WHERE id = ? AND owner_id = ? AND version = ?",
        )
        .bind(&snapshot.id)
        .bind(&user.id)
        .bind(row.get::<i64, _>("result_shopping_version"))
        .execute(&mut *transaction)
        .await?;
        restore_shopping(&mut transaction, &user.id, &snapshot).await?;
    } else {
        sqlx::query(
            "DELETE FROM shopping_items
             WHERE id = ? AND owner_id = ? AND version = ?",
        )
        .bind(&result_shopping_item_id)
        .bind(&user.id)
        .bind(row.get::<i64, _>("result_shopping_version"))
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query("UPDATE shopping_purchase_actions SET undone_at = ? WHERE id = ?")
        .bind(Utc::now().timestamp())
        .bind(&action_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(Json(load_shopping_items(&state, &user.id).await?))
}

async fn load_shopping_items(state: &AppState, owner_id: &str) -> AppResult<Vec<ShoppingItem>> {
    let mut transaction = state.pool.begin().await?;
    let items = load_shopping_items_tx(&mut transaction, owner_id).await?;
    transaction.commit().await?;
    Ok(items)
}

#[derive(Default)]
pub(crate) struct ActiveShoppingCoverage {
    pub concrete_identity_keys: HashSet<String>,
    pub exact_choice_keys: HashSet<String>,
}

pub(crate) async fn active_coverage_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
) -> AppResult<ActiveShoppingCoverage> {
    let persistent: Vec<(Option<String>, String, Option<String>)> = sqlx::query_as(
        "SELECT canonical_ingredient_id, normalized_name, choice_key
         FROM shopping_items
         WHERE owner_id = ? AND completed_at IS NULL",
    )
    .bind(owner_id)
    .fetch_all(&mut **transaction)
    .await?;
    let automatic: Vec<(Option<String>, String)> = sqlx::query_as(
        "SELECT canonical_ingredient_id, normalized_name
         FROM pantry_items
         WHERE owner_id = ? AND status = 'out'
           AND auto_add_to_shopping_list_when_out = 1",
    )
    .bind(owner_id)
    .fetch_all(&mut **transaction)
    .await?;
    let mut coverage = ActiveShoppingCoverage {
        concrete_identity_keys: automatic
            .into_iter()
            .map(|(canonical_id, normalized_name)| {
                dedupe_key(canonical_id.as_deref(), &normalized_name)
            })
            .collect::<HashSet<_>>(),
        ..Default::default()
    };
    for (canonical_id, normalized_name, choice_key) in persistent {
        if let Some(choice_key) = choice_key {
            coverage.exact_choice_keys.insert(choice_key);
        } else {
            coverage
                .concrete_identity_keys
                .insert(dedupe_key(canonical_id.as_deref(), &normalized_name));
        }
    }
    Ok(coverage)
}

pub(crate) fn exact_choice_key<I>(identities: I) -> String
where
    I: IntoIterator<Item = String>,
{
    let mut identities = identities.into_iter().collect::<Vec<_>>();
    identities.sort();
    identities.dedup();
    let digest = Sha256::digest(identities.join("\n").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn load_shopping_items_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
) -> AppResult<Vec<ShoppingItem>> {
    let rows = sqlx::query(
        "SELECT id, name, normalized_name, canonical_ingredient_id, choice_key, choice_json, version
         FROM shopping_items WHERE owner_id = ? AND completed_at IS NULL
         ORDER BY normalized_name, id LIMIT ?",
    )
    .bind(owner_id)
    .bind(MAX_SHOPPING_ITEMS_PER_OWNER + 1)
    .fetch_all(&mut **transaction)
    .await?;
    if rows.len() as i64 > MAX_SHOPPING_ITEMS_PER_OWNER {
        return Err(AppError::Validation(
            "shopping list item limit exceeded".into(),
        ));
    }
    let source_rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT s.item_id, s.source_type, s.label, s.recipe_id
         FROM shopping_item_sources s
         JOIN shopping_items i ON i.id = s.item_id
         WHERE i.owner_id = ? AND i.completed_at IS NULL
         ORDER BY s.item_id, s.source_type, s.label LIMIT ?",
    )
    .bind(owner_id)
    .bind(MAX_SHOPPING_SOURCES_PER_OWNER + 1)
    .fetch_all(&mut **transaction)
    .await?;
    if source_rows.len() as i64 > MAX_SHOPPING_SOURCES_PER_OWNER {
        return Err(AppError::Validation(
            "shopping list source limit exceeded".into(),
        ));
    }
    let mut sources_by_item = HashMap::<String, Vec<ShoppingItemSource>>::new();
    for (item_id, kind, label, recipe_id) in source_rows {
        sources_by_item
            .entry(item_id)
            .or_default()
            .push(ShoppingItemSource {
                kind,
                label,
                recipe_id,
            });
    }
    let mut items = BTreeMap::<String, StoredItem>::new();
    for row in rows {
        let canonical_id: Option<String> = row.get("canonical_ingredient_id");
        let normalized_name: String = row.get("normalized_name");
        let choice_key: Option<String> = row.get("choice_key");
        let choice_alternatives = row
            .get::<Option<String>, _>("choice_json")
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(|error| AppError::Data(format!("invalid shopping choice: {error}")))?
            .unwrap_or_default();
        let id: String = row.get("id");
        let sources = sources_by_item.remove(&id).unwrap_or_default();
        items.insert(
            choice_key
                .as_deref()
                .map(|key| format!("choice:{key}"))
                .unwrap_or_else(|| dedupe_key(canonical_id.as_deref(), &normalized_name)),
            StoredItem {
                id,
                name: row.get("name"),
                normalized_name,
                canonical_ingredient_id: canonical_id,
                choice_alternatives,
                sources,
                version: row.get("version"),
            },
        );
    }
    let auto_rows = sqlx::query(
        "SELECT id, name, normalized_name, canonical_ingredient_id, version
         FROM pantry_items WHERE owner_id = ? AND status = 'out'
           AND auto_add_to_shopping_list_when_out = 1 ORDER BY normalized_name, id",
    )
    .bind(owner_id)
    .fetch_all(&mut **transaction)
    .await?;
    let mut automatic_ids = BTreeMap::<String, String>::new();
    for row in auto_rows {
        let pantry_id: String = row.get("id");
        let canonical_id: Option<String> = row.get("canonical_ingredient_id");
        let normalized_name: String = row.get("normalized_name");
        let key = dedupe_key(canonical_id.as_deref(), &normalized_name);
        automatic_ids.insert(key.clone(), pantry_id.clone());
        let item = items.entry(key).or_insert_with(|| StoredItem {
            id: format!("auto:{pantry_id}"),
            name: row.get("name"),
            normalized_name,
            canonical_ingredient_id: canonical_id,
            choice_alternatives: Vec::new(),
            sources: Vec::new(),
            version: row.get("version"),
        });
        if !item.sources.iter().any(|source| source.kind == "automatic") {
            item.sources.push(ShoppingItemSource {
                kind: "automatic".into(),
                label: String::new(),
                recipe_id: None,
            });
        }
    }
    let pantry_context = load_pantry_candidate_context_tx(transaction, owner_id).await?;
    let mut result = Vec::new();
    for (key, mut item) in items {
        for alternative in &mut item.choice_alternatives {
            let normalized = normalize_ingredient(&alternative.name);
            let candidates = pantry_candidates_from_context(
                &pantry_context,
                &normalized,
                alternative.canonical_ingredient_id.as_deref(),
            );
            let resolution = if deterministic_reuse(
                &candidates,
                alternative.canonical_ingredient_id.as_deref(),
                &normalized,
            )
            .is_some()
            {
                "reuse"
            } else if candidates.is_empty() {
                "create"
            } else {
                "choice"
            };
            alternative.purchase_options = Some(ShoppingPurchaseOptions {
                resolution: resolution.into(),
                candidates,
            });
        }
        let candidates = if item.choice_alternatives.is_empty() {
            pantry_candidates_from_context(
                &pantry_context,
                &item.normalized_name,
                item.canonical_ingredient_id.as_deref(),
            )
        } else {
            Vec::new()
        };
        let automatic = automatic_ids.contains_key(&key);
        let resolution = if !item.choice_alternatives.is_empty() {
            "selectAlternative"
        } else if deterministic_reuse(
            &candidates,
            item.canonical_ingredient_id.as_deref(),
            &item.normalized_name,
        )
        .is_some()
        {
            "reuse"
        } else if candidates.is_empty() {
            "create"
        } else {
            "choice"
        };
        result.push(ShoppingItem {
            id: item.id,
            name: item.name,
            canonical_ingredient_id: item.canonical_ingredient_id,
            choice_alternatives: item.choice_alternatives,
            automatic,
            pantry_item_id: automatic_ids.get(&key).cloned(),
            sources: item.sources,
            version: item.version,
            purchase_options: ShoppingPurchaseOptions {
                resolution: resolution.into(),
                candidates,
            },
        });
    }
    Ok(result)
}

async fn pantry_candidates_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    normalized_name: &str,
    canonical_id: Option<&str>,
) -> AppResult<Vec<PantryPurchaseCandidate>> {
    let context = load_pantry_candidate_context_tx(transaction, owner_id).await?;
    Ok(pantry_candidates_from_context(
        &context,
        normalized_name,
        canonical_id,
    ))
}

async fn load_pantry_candidate_context_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
) -> AppResult<PantryCandidateContext> {
    let rows = sqlx::query(
        "SELECT p.id, p.name, p.normalized_name, p.status, p.quantity_note, p.expires_on,
                p.canonical_ingredient_id, p.auto_add_to_shopping_list_when_out, p.version,
                c.normalized_name AS canonical_name
         FROM pantry_items p
         LEFT JOIN ingredient_catalog c ON c.id = p.canonical_ingredient_id
         WHERE p.owner_id = ? ORDER BY p.normalized_name, p.id LIMIT ?",
    )
    .bind(owner_id)
    .bind(MAX_PANTRY_ITEMS_PER_OWNER + 1)
    .fetch_all(&mut **transaction)
    .await?;
    if rows.len() as i64 > MAX_PANTRY_ITEMS_PER_OWNER {
        return Err(AppError::Validation("pantry item limit exceeded".into()));
    }
    let alias_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT alias_normalized, canonical_ingredient_id FROM ingredient_aliases
         WHERE owner_id = ? ORDER BY alias_normalized LIMIT ?",
    )
    .bind(owner_id)
    .bind(MAX_INGREDIENT_ALIASES_PER_OWNER + 1)
    .fetch_all(&mut **transaction)
    .await?;
    if alias_rows.len() as i64 > MAX_INGREDIENT_ALIASES_PER_OWNER {
        return Err(AppError::Validation(
            "ingredient alias limit exceeded".into(),
        ));
    }
    let mut aliases = HashMap::<String, HashSet<String>>::new();
    for (alias_normalized, canonical_id) in alias_rows {
        aliases
            .entry(alias_normalized)
            .or_default()
            .insert(canonical_id);
    }
    Ok(PantryCandidateContext {
        rows: rows
            .into_iter()
            .map(|row| PantryCandidateRow {
                id: row.get("id"),
                name: row.get("name"),
                normalized_name: row.get("normalized_name"),
                status: PantryStatus::parse(&row.get::<String, _>("status")),
                quantity_note: row.get("quantity_note"),
                expires_on: row.get("expires_on"),
                canonical_ingredient_id: row.get("canonical_ingredient_id"),
                auto_add_to_shopping_list_when_out: row.get("auto_add_to_shopping_list_when_out"),
                version: row.get("version"),
                canonical_name: row.get("canonical_name"),
            })
            .collect(),
        aliases,
    })
}

fn pantry_candidates_from_context(
    context: &PantryCandidateContext,
    normalized_name: &str,
    canonical_id: Option<&str>,
) -> Vec<PantryPurchaseCandidate> {
    let alias_ids = context.aliases.get(normalized_name);
    let mut candidates = Vec::new();
    for row in &context.rows {
        let fuzzy = row.canonical_name.as_deref().is_some_and(|candidate| {
            let max = if normalized_name.chars().count() <= 5 {
                1
            } else {
                2
            };
            bounded_levenshtein(normalized_name, candidate, max).is_some()
        });
        let matches = row.normalized_name == normalized_name
            || canonical_id.is_some() && row.canonical_ingredient_id.as_deref() == canonical_id
            || row
                .canonical_ingredient_id
                .as_ref()
                .is_some_and(|id| alias_ids.is_some_and(|aliases| aliases.contains(id)))
            || fuzzy;
        if matches {
            candidates.push(PantryPurchaseCandidate {
                id: row.id.clone(),
                name: row.name.clone(),
                status: row.status,
                quantity_note: row.quantity_note.clone(),
                expires_on: row.expires_on.clone(),
                canonical_ingredient_id: row.canonical_ingredient_id.clone(),
                auto_add_to_shopping_list_when_out: row.auto_add_to_shopping_list_when_out,
                version: row.version,
            });
        }
    }
    candidates
}

fn deterministic_reuse(
    candidates: &[PantryPurchaseCandidate],
    canonical_id: Option<&str>,
    normalized_name: &str,
) -> Option<String> {
    let exact: Vec<_> = candidates
        .iter()
        .filter(|candidate| match canonical_id {
            Some(id) => candidate.canonical_ingredient_id.as_deref() == Some(id),
            None => {
                candidate.canonical_ingredient_id.is_none()
                    && normalize_ingredient(&candidate.name) == normalized_name
            }
        })
        .collect();
    (exact.len() == 1).then(|| exact[0].id.clone())
}

async fn load_shopping_snapshot(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    item_id: &str,
) -> AppResult<Option<ShoppingSnapshot>> {
    let row = sqlx::query(
        "SELECT id, name, normalized_name, canonical_ingredient_id, choice_key, choice_json, version
         FROM shopping_items WHERE id = ? AND owner_id = ? AND completed_at IS NULL",
    )
    .bind(item_id)
    .bind(owner_id)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let sources = sqlx::query(
        "SELECT id, source_type, recipe_id, label, created_at
         FROM shopping_item_sources WHERE item_id = ? ORDER BY id",
    )
    .bind(item_id)
    .fetch_all(&mut **transaction)
    .await?
    .into_iter()
    .map(|row| SourceSnapshot {
        id: row.get("id"),
        source_type: row.get("source_type"),
        recipe_id: row.get("recipe_id"),
        label: row.get("label"),
        created_at: row.get("created_at"),
    })
    .collect();
    Ok(Some(ShoppingSnapshot {
        id: row.get("id"),
        name: row.get("name"),
        normalized_name: row.get("normalized_name"),
        canonical_ingredient_id: row.get("canonical_ingredient_id"),
        choice_key: row.get("choice_key"),
        choice_alternatives: row
            .get::<Option<String>, _>("choice_json")
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(|error| AppError::Data(format!("invalid shopping choice: {error}")))?
            .unwrap_or_default(),
        version: row.get("version"),
        sources,
    }))
}

async fn load_shopping_snapshot_by_identity(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    canonical_id: Option<&str>,
    normalized_name: &str,
) -> AppResult<Option<ShoppingSnapshot>> {
    let id: Option<String> = if let Some(canonical_id) = canonical_id {
        sqlx::query_scalar(
            "SELECT id FROM shopping_items
             WHERE owner_id = ? AND canonical_ingredient_id = ? AND completed_at IS NULL",
        )
        .bind(owner_id)
        .bind(canonical_id)
        .fetch_optional(&mut **transaction)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT id FROM shopping_items
             WHERE owner_id = ? AND canonical_ingredient_id IS NULL
               AND normalized_name = ? AND completed_at IS NULL",
        )
        .bind(owner_id)
        .bind(normalized_name)
        .fetch_optional(&mut **transaction)
        .await?
    };
    match id {
        Some(id) => load_shopping_snapshot(transaction, owner_id, &id).await,
        None => Ok(None),
    }
}

async fn has_automatic_identity(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    item: &ShoppingSnapshot,
) -> AppResult<bool> {
    if item.choice_key.is_some() {
        return Ok(false);
    }
    let exists: Option<i64> = if let Some(canonical_id) = item.canonical_ingredient_id.as_deref() {
        sqlx::query_scalar(
            "SELECT 1 FROM pantry_items
             WHERE owner_id = ? AND status = 'out'
               AND auto_add_to_shopping_list_when_out = 1
               AND canonical_ingredient_id = ?",
        )
        .bind(owner_id)
        .bind(canonical_id)
        .fetch_optional(&mut **transaction)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT 1 FROM pantry_items
             WHERE owner_id = ? AND status = 'out'
               AND auto_add_to_shopping_list_when_out = 1
               AND canonical_ingredient_id IS NULL AND normalized_name = ?",
        )
        .bind(owner_id)
        .bind(&item.normalized_name)
        .fetch_optional(&mut **transaction)
        .await?
    };
    Ok(exists.is_some())
}

async fn load_pantry_snapshot(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    pantry_id: &str,
) -> AppResult<Option<PantrySnapshot>> {
    let row = sqlx::query(
        "SELECT id, name, normalized_name, status, quantity_note, confirmed_at, expires_on,
                canonical_ingredient_id, auto_add_to_shopping_list_when_out, version
         FROM pantry_items WHERE id = ? AND owner_id = ?",
    )
    .bind(pantry_id)
    .bind(owner_id)
    .fetch_optional(&mut **transaction)
    .await?;
    Ok(row.map(|row| PantrySnapshot {
        id: row.get("id"),
        name: row.get("name"),
        normalized_name: row.get("normalized_name"),
        status: row.get("status"),
        quantity_note: row.get("quantity_note"),
        confirmed_at: row.get("confirmed_at"),
        expires_on: row.get("expires_on"),
        canonical_ingredient_id: row.get("canonical_ingredient_id"),
        auto_add_to_shopping_list_when_out: row.get("auto_add_to_shopping_list_when_out"),
        version: row.get("version"),
    }))
}

fn pantry_from_snapshot(item: PantrySnapshot) -> PantryItem {
    PantryItem {
        id: item.id,
        name: item.name,
        status: PantryStatus::parse(&item.status),
        quantity_note: item.quantity_note,
        confirmed_at: item.confirmed_at,
        expires_on: item.expires_on,
        canonical_ingredient_id: item.canonical_ingredient_id,
        aliases: Vec::new(),
        auto_add_to_shopping_list_when_out: item.auto_add_to_shopping_list_when_out,
        version: item.version,
    }
}

async fn restore_pantry(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    item: &PantrySnapshot,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE pantry_items SET name = ?, normalized_name = ?, status = ?, quantity_note = ?,
           confirmed_at = ?, expires_on = ?, canonical_ingredient_id = ?,
           auto_add_to_shopping_list_when_out = ?, version = ?, updated_at = CURRENT_TIMESTAMP
         WHERE id = ? AND owner_id = ?",
    )
    .bind(&item.name)
    .bind(&item.normalized_name)
    .bind(&item.status)
    .bind(&item.quantity_note)
    .bind(&item.confirmed_at)
    .bind(&item.expires_on)
    .bind(&item.canonical_ingredient_id)
    .bind(item.auto_add_to_shopping_list_when_out)
    .bind(item.version)
    .bind(&item.id)
    .bind(owner_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn restore_shopping(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    item: &ShoppingSnapshot,
) -> AppResult<()> {
    let item_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM shopping_items
         WHERE owner_id = ? AND completed_at IS NULL",
    )
    .bind(owner_id)
    .fetch_one(&mut **transaction)
    .await?;
    if item_count >= MAX_SHOPPING_ITEMS_PER_OWNER {
        return Err(AppError::Validation(
            "shopping list item limit exceeded".into(),
        ));
    }
    let source_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM shopping_item_sources s
         JOIN shopping_items i ON i.id = s.item_id
         WHERE i.owner_id = ?",
    )
    .bind(owner_id)
    .fetch_one(&mut **transaction)
    .await?;
    if source_count + item.sources.len() as i64 > MAX_SHOPPING_SOURCES_PER_OWNER {
        return Err(AppError::Validation(
            "shopping list source limit exceeded".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO shopping_items
         (id, owner_id, name, normalized_name, canonical_ingredient_id, completed_at,
          created_at, updated_at, version, choice_key, choice_json)
         VALUES (?, ?, ?, ?, ?, NULL, ?, ?, ?, ?, ?)",
    )
    .bind(&item.id)
    .bind(owner_id)
    .bind(&item.name)
    .bind(&item.normalized_name)
    .bind(&item.canonical_ingredient_id)
    .bind(Utc::now().timestamp())
    .bind(Utc::now().timestamp())
    .bind(item.version)
    .bind(&item.choice_key)
    .bind(
        (!item.choice_alternatives.is_empty())
            .then(|| serde_json::to_string(&item.choice_alternatives))
            .transpose()?,
    )
    .execute(&mut **transaction)
    .await?;
    for source in &item.sources {
        sqlx::query(
            "INSERT INTO shopping_item_sources
             (id, item_id, source_type, recipe_id, label, created_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&source.id)
        .bind(&item.id)
        .bind(&source.source_type)
        .bind(&source.recipe_id)
        .bind(&source.label)
        .bind(source.created_at)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn upsert_stored_item(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    display_name: &str,
    canonical_id: Option<&str>,
) -> AppResult<(String, i64)> {
    let (name, normalized) = validate_name(display_name)?;
    validate_selected_canonical(transaction, canonical_id).await?;
    let existing: Option<String> = if let Some(canonical_id) = canonical_id {
        sqlx::query_scalar(
            "SELECT id FROM shopping_items
             WHERE owner_id = ? AND canonical_ingredient_id = ?",
        )
        .bind(owner_id)
        .bind(canonical_id)
        .fetch_optional(&mut **transaction)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT id FROM shopping_items
             WHERE owner_id = ? AND canonical_ingredient_id IS NULL AND normalized_name = ?",
        )
        .bind(owner_id)
        .bind(&normalized)
        .fetch_optional(&mut **transaction)
        .await?
    };
    if existing.is_none() {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM shopping_items WHERE owner_id = ?")
                .bind(owner_id)
                .fetch_one(&mut **transaction)
                .await?;
        if count >= MAX_SHOPPING_ITEMS_PER_OWNER {
            return Err(AppError::Validation(
                "shopping list item limit exceeded".into(),
            ));
        }
    }
    sqlx::query_as(
        "INSERT INTO shopping_items
         (id, owner_id, name, normalized_name, canonical_ingredient_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT DO UPDATE SET completed_at = NULL, version = shopping_items.version + 1,
           updated_at = excluded.updated_at RETURNING id, version",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(owner_id)
    .bind(name)
    .bind(normalized)
    .bind(canonical_id)
    .bind(Utc::now().timestamp())
    .bind(Utc::now().timestamp())
    .fetch_one(&mut **transaction)
    .await
    .map_err(map_unique_conflict)
}

async fn upsert_choice_item(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    alternatives: &[ShoppingChoiceAlternative],
) -> AppResult<(String, i64)> {
    if alternatives.len() < 2 || alternatives.len() > 8 {
        return Err(AppError::Validation(
            "shopping choices require between 2 and 8 alternatives".into(),
        ));
    }
    for alternative in alternatives {
        let existing: Option<(String, i64)> =
            if let Some(canonical_id) = alternative.canonical_ingredient_id.as_deref() {
                sqlx::query_as(
                    "SELECT id, version FROM shopping_items
                     WHERE owner_id = ? AND completed_at IS NULL
                       AND choice_key IS NULL AND canonical_ingredient_id = ?",
                )
                .bind(owner_id)
                .bind(canonical_id)
                .fetch_optional(&mut **transaction)
                .await?
            } else {
                sqlx::query_as(
                    "SELECT id, version FROM shopping_items
                     WHERE owner_id = ? AND completed_at IS NULL
                       AND choice_key IS NULL AND canonical_ingredient_id IS NULL
                       AND normalized_name = ?",
                )
                .bind(owner_id)
                .bind(normalize_ingredient(&alternative.name))
                .fetch_optional(&mut **transaction)
                .await?
            };
        if let Some(existing) = existing {
            return Ok(existing);
        }
    }
    let mut identities = alternatives
        .iter()
        .map(|alternative| {
            alternative
                .canonical_ingredient_id
                .as_deref()
                .map(|id| format!("id:{id}"))
                .unwrap_or_else(|| format!("name:{}", normalize_ingredient(&alternative.name)))
        })
        .collect::<Vec<_>>();
    identities.sort();
    identities.dedup();
    if identities.len() != alternatives.len() {
        return Err(AppError::Validation(
            "shopping choice alternatives must have distinct identities".into(),
        ));
    }
    let choice_json =
        serde_json::to_string(alternatives).map_err(|error| AppError::Data(error.to_string()))?;
    let choice_key = exact_choice_key(identities);
    let display = alternatives
        .iter()
        .map(|alternative| alternative.name.as_str())
        .collect::<Vec<_>>()
        .join(" / ");
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM shopping_items WHERE owner_id = ? AND completed_at IS NULL",
    )
    .bind(owner_id)
    .fetch_one(&mut **transaction)
    .await?;
    if count >= MAX_SHOPPING_ITEMS_PER_OWNER {
        return Err(AppError::Validation(
            "shopping list item limit exceeded".into(),
        ));
    }
    sqlx::query_as(
        "INSERT INTO shopping_items
         (id, owner_id, name, normalized_name, canonical_ingredient_id, choice_key,
          choice_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, NULL, ?, ?, ?, ?)
         ON CONFLICT(owner_id, choice_key) WHERE choice_key IS NOT NULL
         DO UPDATE SET completed_at = NULL, version = shopping_items.version + 1,
           updated_at = excluded.updated_at
         RETURNING id, version",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(owner_id)
    .bind(&display)
    .bind(format!("choice:{choice_key}"))
    .bind(choice_key)
    .bind(choice_json)
    .bind(Utc::now().timestamp())
    .bind(Utc::now().timestamp())
    .fetch_one(&mut **transaction)
    .await
    .map_err(map_unique_conflict)
}

async fn load_aliases_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    canonical_id: Option<&str>,
) -> AppResult<Vec<String>> {
    let Some(canonical_id) = canonical_id else {
        return Ok(Vec::new());
    };
    let aliases = sqlx::query_scalar(
        "SELECT alias_normalized FROM ingredient_aliases
         WHERE owner_id = ? AND canonical_ingredient_id = ?
         ORDER BY alias_normalized LIMIT ?",
    )
    .bind(owner_id)
    .bind(canonical_id)
    .bind(MAX_INGREDIENT_ALIASES_PER_OWNER + 1)
    .fetch_all(&mut **transaction)
    .await?;
    if aliases.len() as i64 > MAX_INGREDIENT_ALIASES_PER_OWNER {
        return Err(AppError::Validation(
            "ingredient alias limit exceeded".into(),
        ));
    }
    Ok(aliases)
}

async fn insert_source(
    transaction: &mut Transaction<'_, Sqlite>,
    item_id: &str,
    source_type: &str,
    recipe_id: Option<&str>,
    label: &str,
) -> AppResult<()> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM shopping_item_sources
         WHERE item_id = ? AND source_type = ? AND recipe_id IS ?",
    )
    .bind(item_id)
    .bind(source_type)
    .bind(recipe_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if exists.is_some() {
        return Ok(());
    }
    let source_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM shopping_item_sources s
         JOIN shopping_items i ON i.id = s.item_id
         WHERE i.owner_id = (SELECT owner_id FROM shopping_items WHERE id = ?)",
    )
    .bind(item_id)
    .fetch_one(&mut **transaction)
    .await?;
    if source_count >= MAX_SHOPPING_SOURCES_PER_OWNER {
        return Err(AppError::Validation(
            "shopping list source limit exceeded".into(),
        ));
    }
    sqlx::query(
        "INSERT OR IGNORE INTO shopping_item_sources
         (id, item_id, source_type, recipe_id, label, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(item_id)
    .bind(source_type)
    .bind(recipe_id)
    .bind(label)
    .bind(Utc::now().timestamp())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn validate_selected_canonical(
    transaction: &mut Transaction<'_, Sqlite>,
    canonical_id: Option<&str>,
) -> AppResult<()> {
    let Some(canonical_id) = canonical_id else {
        return Ok(());
    };
    let visible: Option<i64> = sqlx::query_scalar("SELECT 1 FROM ingredient_catalog WHERE id = ?")
        .bind(canonical_id)
        .fetch_optional(&mut **transaction)
        .await?;
    visible.map(|_| ()).ok_or_else(|| {
        AppError::Validation(
            "selected canonical ingredient is not available to this account".into(),
        )
    })
}

async fn require_manual_source(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    item_id: &str,
) -> AppResult<()> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM shopping_items i JOIN shopping_item_sources s ON s.item_id = i.id
         WHERE i.id = ? AND i.owner_id = ? AND s.source_type = 'manual'",
    )
    .bind(item_id)
    .bind(owner_id)
    .fetch_optional(&mut **transaction)
    .await?;
    exists.map(|_| ()).ok_or(AppError::NotFound)
}

fn validate_name(value: &str) -> AppResult<(String, String)> {
    let name = clean_display_name(value);
    let normalized = normalize_ingredient(&name);
    if normalized.is_empty() {
        return Err(AppError::Validation(
            "shopping item name is required".into(),
        ));
    }
    if name.chars().count() > 120 {
        return Err(AppError::Validation(
            "shopping item names may use at most 120 characters".into(),
        ));
    }
    Ok((name, normalized))
}

fn validate_purchase_input(input: &PurchaseInput) -> AppResult<()> {
    if input.quantity_note.chars().count() > 240 {
        return Err(AppError::Validation(
            "quantity notes may use at most 240 characters".into(),
        ));
    }
    if let Some(date) = &input.expires_on {
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| AppError::Validation("expiration must be an ISO date".into()))?;
    }
    if input.create_using_current_name && input.pantry_item_id.is_some() {
        return Err(AppError::Validation(
            "choose either an existing pantry item or create using the current name".into(),
        ));
    }
    Ok(())
}

fn idempotency_key(headers: &HeaderMap, now: i64) -> AppResult<String> {
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::Validation("Idempotency-Key header is required".into()))?;
    if !(8..=128).contains(&key.len())
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(AppError::Validation(
            "Idempotency-Key must contain 8 to 128 URL-safe characters".into(),
        ));
    }
    let timestamp = key
        .split_once('.')
        .and_then(|(value, _)| value.parse::<i64>().ok())
        .ok_or_else(|| {
            AppError::Validation(
                "Idempotency-Key must start with its Unix creation timestamp".into(),
            )
        })?;
    if timestamp > now + 300 {
        return Err(AppError::Validation(
            "Idempotency-Key creation timestamp is in the future".into(),
        ));
    }
    if timestamp < now - PURCHASE_REPLAY_SECONDS {
        return Err(AppError::Conflict(
            "Idempotency-Key replay window expired; reload before retrying".into(),
        ));
    }
    Ok(key.to_string())
}

async fn replay_purchase(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    key: &str,
    fingerprint: &[u8; 32],
) -> AppResult<Option<PurchaseResponse>> {
    let row = sqlx::query(
        "SELECT request_fingerprint, response_json FROM shopping_purchase_actions
         WHERE owner_id = ? AND idempotency_key = ?",
    )
    .bind(owner_id)
    .bind(key)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let previous: Vec<u8> = row.get("request_fingerprint");
    if previous.as_slice() != fingerprint {
        return Err(AppError::Conflict(
            "Idempotency-Key was already used for a different purchase".into(),
        ));
    }
    serde_json::from_str(&row.get::<String, _>("response_json"))
        .map(Some)
        .map_err(|error| AppError::Data(error.to_string()))
}

async fn replay_delete(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    key: &str,
    fingerprint: &[u8; 32],
) -> AppResult<Option<DeleteResponse>> {
    let row = sqlx::query(
        "SELECT request_fingerprint, response_json FROM shopping_delete_actions
         WHERE owner_id = ? AND idempotency_key = ?",
    )
    .bind(owner_id)
    .bind(key)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let previous: Vec<u8> = row.get("request_fingerprint");
    if previous.as_slice() != fingerprint {
        return Err(AppError::Conflict(
            "Idempotency-Key was already used for a different shopping deletion".into(),
        ));
    }
    serde_json::from_str(&row.get::<String, _>("response_json"))
        .map(Some)
        .map_err(|error| AppError::Data(error.to_string()))
}

async fn cleanup_delete_actions(
    transaction: &mut Transaction<'_, Sqlite>,
    now: i64,
) -> AppResult<()> {
    sqlx::query(
        "DELETE FROM shopping_delete_actions WHERE rowid IN (
           SELECT rowid FROM shopping_delete_actions
           WHERE created_at < ? ORDER BY created_at LIMIT ?)",
    )
    .bind(now - PURCHASE_REPLAY_SECONDS)
    .bind(PURCHASE_CLEANUP_BATCH)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn enforce_delete_capacity(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    now: i64,
) -> AppResult<()> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM shopping_delete_actions WHERE owner_id = ? AND created_at >= ?",
    )
    .bind(owner_id)
    .bind(now - PURCHASE_REPLAY_SECONDS)
    .fetch_one(&mut **transaction)
    .await?;
    if count >= DELETE_ACTION_CAP {
        Err(AppError::RateLimited)
    } else {
        Ok(())
    }
}

async fn enforce_delete_rate_limit(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    now: i64,
) -> AppResult<()> {
    let window = now - now.rem_euclid(PURCHASE_RATE_WINDOW_SECONDS);
    let count: i64 = sqlx::query_scalar(
        "INSERT INTO public_api_rate_limits (rate_key, window_started_at, request_count)
         VALUES (?, ?, 1) ON CONFLICT(rate_key) DO UPDATE SET
           window_started_at = CASE WHEN public_api_rate_limits.window_started_at < excluded.window_started_at
             THEN excluded.window_started_at ELSE public_api_rate_limits.window_started_at END,
           request_count = CASE WHEN public_api_rate_limits.window_started_at < excluded.window_started_at
             THEN 1 ELSE public_api_rate_limits.request_count + 1 END RETURNING request_count",
    )
    .bind(format!("shopping-delete:{owner_id}"))
    .bind(window)
    .fetch_one(&mut **transaction)
    .await?;
    if count > DELETE_RATE_LIMIT {
        Err(AppError::RateLimited)
    } else {
        Ok(())
    }
}

async fn cleanup_purchase_actions(
    transaction: &mut Transaction<'_, Sqlite>,
    now: i64,
) -> AppResult<()> {
    sqlx::query(
        "DELETE FROM shopping_purchase_actions WHERE rowid IN (
           SELECT rowid FROM shopping_purchase_actions
           WHERE created_at < ? ORDER BY created_at LIMIT ?)",
    )
    .bind(now - PURCHASE_REPLAY_SECONDS)
    .bind(PURCHASE_CLEANUP_BATCH)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn enforce_purchase_capacity(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    now: i64,
) -> AppResult<()> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM shopping_purchase_actions WHERE owner_id = ? AND created_at >= ?",
    )
    .bind(owner_id)
    .bind(now - PURCHASE_REPLAY_SECONDS)
    .fetch_one(&mut **transaction)
    .await?;
    if count >= PURCHASE_ACTION_CAP {
        Err(AppError::RateLimited)
    } else {
        Ok(())
    }
}

async fn enforce_purchase_rate_limit(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    now: i64,
) -> AppResult<()> {
    let window = now - now.rem_euclid(PURCHASE_RATE_WINDOW_SECONDS);
    let count: i64 = sqlx::query_scalar(
        "INSERT INTO public_api_rate_limits (rate_key, window_started_at, request_count)
         VALUES (?, ?, 1) ON CONFLICT(rate_key) DO UPDATE SET
           window_started_at = CASE WHEN public_api_rate_limits.window_started_at < excluded.window_started_at
             THEN excluded.window_started_at ELSE public_api_rate_limits.window_started_at END,
           request_count = CASE WHEN public_api_rate_limits.window_started_at < excluded.window_started_at
             THEN 1 ELSE public_api_rate_limits.request_count + 1 END RETURNING request_count",
    )
    .bind(format!("shopping-purchase:{owner_id}"))
    .bind(window)
    .fetch_one(&mut **transaction)
    .await?;
    if count > PURCHASE_RATE_LIMIT {
        Err(AppError::RateLimited)
    } else {
        Ok(())
    }
}

pub(crate) fn dedupe_key(canonical_id: Option<&str>, normalized: &str) -> String {
    canonical_id
        .map(|id| format!("c:{id}"))
        .unwrap_or_else(|| format!("n:{normalized}"))
}

fn map_unique_conflict(error: sqlx::Error) -> AppError {
    if matches!(&error, sqlx::Error::Database(database) if database.is_unique_violation()) {
        AppError::Conflict("that ingredient already exists with incompatible identity".into())
    } else {
        AppError::Database(error)
    }
}

fn state_etag<T: Serialize>(value: &T) -> AppResult<HeaderValue> {
    let digest = Sha256::digest(serde_json::to_vec(value)?);
    HeaderValue::from_str(&format!("\"shopping-{:x}\"", digest))
        .map_err(|error| AppError::Data(error.to_string()))
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        Self::Data(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, name: &str, canonical_id: Option<&str>) -> PantryPurchaseCandidate {
        PantryPurchaseCandidate {
            id: id.into(),
            name: name.into(),
            status: PantryStatus::Have,
            quantity_note: String::new(),
            expires_on: None,
            canonical_ingredient_id: canonical_id.map(str::to_string),
            auto_add_to_shopping_list_when_out: false,
            version: 1,
        }
    }

    #[test]
    fn deterministic_reuse_requires_one_exact_identity() {
        let canonical = vec![
            candidate("exact", "Scallion", Some("canonical-a")),
            candidate("fuzzy", "Scallions", Some("canonical-b")),
        ];
        assert_eq!(
            deterministic_reuse(&canonical, Some("canonical-a"), "scallion"),
            Some("exact".into())
        );
        assert_eq!(
            deterministic_reuse(&canonical, Some("canonical-c"), "scallion"),
            None
        );

        let unresolved = vec![
            candidate("same", "  Milk ", None),
            candidate("canonical", "Milk", Some("canonical-milk")),
        ];
        assert_eq!(
            deterministic_reuse(&unresolved, None, "milk"),
            Some("same".into())
        );
        let ambiguous = vec![
            candidate("one", "Milk", None),
            candidate("two", "Ｍｉｌｋ", None),
        ];
        assert_eq!(deterministic_reuse(&ambiguous, None, "milk"), None);
    }
}
