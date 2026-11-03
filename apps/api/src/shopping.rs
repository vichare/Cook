use std::collections::BTreeMap;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    ingredients::{clean_display_name, normalize_ingredient},
    models::{ShoppingItem, ShoppingItemInput, ShoppingItemSource},
    routes::CurrentUser,
    AppState,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecipeShoppingInput {
    name: String,
    #[serde(default)]
    canonical_ingredient_id: Option<String>,
}

#[derive(Debug)]
struct StoredItem {
    id: String,
    name: String,
    canonical_ingredient_id: Option<String>,
    completed_at: Option<i64>,
    sources: Vec<ShoppingItemSource>,
}

pub async fn list_shopping_items(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<Vec<ShoppingItem>>> {
    Ok(Json(load_shopping_items(&state, &user.id).await?))
}

pub async fn create_manual_item(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(input): Json<ShoppingItemInput>,
) -> AppResult<(StatusCode, Json<Vec<ShoppingItem>>)> {
    let mut transaction = state.pool.begin().await?;
    let item_id = upsert_stored_item(
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
    validate_selected_canonical(
        &mut transaction,
        &user.id,
        input.canonical_ingredient_id.as_deref(),
    )
    .await?;
    let result = sqlx::query(
        "UPDATE shopping_items
         SET name = ?, normalized_name = ?, canonical_ingredient_id = ?,
             completed_at = NULL, updated_at = ?
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

pub async fn complete_manual_item(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(item_id): Path<String>,
) -> AppResult<Json<Vec<ShoppingItem>>> {
    let mut transaction = state.pool.begin().await?;
    require_manual_source(&mut transaction, &user.id, &item_id).await?;
    if stored_item_has_active_auto_source(&mut transaction, &user.id, &item_id).await? {
        return Err(AppError::Validation(
            "automatic restock items must be marked purchased through their pantry item".into(),
        ));
    }
    let result = sqlx::query(
        "UPDATE shopping_items SET completed_at = ?, updated_at = ?
         WHERE id = ? AND owner_id = ?",
    )
    .bind(Utc::now().timestamp())
    .bind(Utc::now().timestamp())
    .bind(item_id)
    .bind(&user.id)
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::NotFound);
    }
    transaction.commit().await?;
    Ok(Json(load_shopping_items(&state, &user.id).await?))
}

pub async fn delete_manual_item(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(item_id): Path<String>,
) -> AppResult<Json<Vec<ShoppingItem>>> {
    let mut transaction = state.pool.begin().await?;
    require_manual_source(&mut transaction, &user.id, &item_id).await?;
    sqlx::query(
        "DELETE FROM shopping_item_sources
         WHERE item_id = ? AND source_type = 'manual'",
    )
    .bind(&item_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "DELETE FROM shopping_items
         WHERE id = ? AND owner_id = ?
           AND NOT EXISTS (SELECT 1 FROM shopping_item_sources WHERE item_id = ?)",
    )
    .bind(&item_id)
    .bind(&user.id)
    .bind(&item_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Json(load_shopping_items(&state, &user.id).await?))
}

pub async fn mark_automatic_purchased(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(pantry_id): Path<String>,
) -> AppResult<Json<Vec<ShoppingItem>>> {
    let mut transaction = state.pool.begin().await?;
    let row = sqlx::query(
        "SELECT normalized_name, canonical_ingredient_id
         FROM pantry_items
         WHERE id = ? AND owner_id = ? AND status = 'out'
           AND auto_add_to_shopping_list_when_out = 1",
    )
    .bind(&pantry_id)
    .bind(&user.id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::NotFound)?;
    let normalized_name: String = row.get("normalized_name");
    let canonical_id: Option<String> = row.get("canonical_ingredient_id");
    let now = Utc::now();
    sqlx::query(
        "UPDATE pantry_items SET status = 'have', confirmed_at = ?, updated_at = CURRENT_TIMESTAMP
         WHERE id = ? AND owner_id = ?",
    )
    .bind(now.to_rfc3339())
    .bind(&pantry_id)
    .bind(&user.id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE shopping_items SET completed_at = ?, updated_at = ?
         WHERE owner_id = ?
           AND ((? IS NOT NULL AND canonical_ingredient_id = ?)
             OR (? IS NULL AND canonical_ingredient_id IS NULL AND normalized_name = ?))",
    )
    .bind(now.timestamp())
    .bind(now.timestamp())
    .bind(&user.id)
    .bind(&canonical_id)
    .bind(&canonical_id)
    .bind(&canonical_id)
    .bind(&normalized_name)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Json(load_shopping_items(&state, &user.id).await?))
}

pub async fn add_recipe_ingredient(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(recipe_id): Path<String>,
    Json(input): Json<RecipeShoppingInput>,
) -> AppResult<(StatusCode, Json<Vec<ShoppingItem>>)> {
    let mut transaction = state.pool.begin().await?;
    let normalized = normalize_ingredient(&input.name);
    let ingredient = sqlx::query(
        "SELECT r.title, ri.name, ri.canonical_ingredient_id
         FROM recipes r
         JOIN recipe_ingredients ri ON ri.recipe_id = r.id
         WHERE r.id = ? AND (r.owner_id = ? OR r.visibility = 'public')
           AND ((? IS NOT NULL AND ri.canonical_ingredient_id = ?)
             OR (? IS NULL AND ri.normalized_name = ?))
         ORDER BY ri.sort_order
         LIMIT 1",
    )
    .bind(&recipe_id)
    .bind(&user.id)
    .bind(&input.canonical_ingredient_id)
    .bind(&input.canonical_ingredient_id)
    .bind(&input.canonical_ingredient_id)
    .bind(&normalized)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::NotFound)?;
    let recipe_title: String = ingredient.get("title");
    let ingredient_name: String = ingredient.get("name");
    let canonical_id: Option<String> = ingredient.get("canonical_ingredient_id");
    let item_id = upsert_stored_item(
        &mut transaction,
        &user.id,
        &ingredient_name,
        canonical_id.as_deref(),
    )
    .await?;
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

async fn load_shopping_items(state: &AppState, owner_id: &str) -> AppResult<Vec<ShoppingItem>> {
    let rows = sqlx::query(
        "SELECT id, name, normalized_name, canonical_ingredient_id, completed_at
         FROM shopping_items WHERE owner_id = ? ORDER BY completed_at, normalized_name, id",
    )
    .bind(owner_id)
    .fetch_all(&state.pool)
    .await?;
    let mut items = BTreeMap::<String, StoredItem>::new();
    for row in rows {
        let canonical_id: Option<String> = row.get("canonical_ingredient_id");
        let normalized_name: String = row.get("normalized_name");
        let id: String = row.get("id");
        let sources = load_sources(state, &id).await?;
        items.insert(
            dedupe_key(canonical_id.as_deref(), &normalized_name),
            StoredItem {
                id,
                name: row.get("name"),
                canonical_ingredient_id: canonical_id,
                completed_at: row.get("completed_at"),
                sources,
            },
        );
    }
    let auto_rows = sqlx::query(
        "SELECT id, name, normalized_name, canonical_ingredient_id
         FROM pantry_items
         WHERE owner_id = ? AND status = 'out'
           AND auto_add_to_shopping_list_when_out = 1
         ORDER BY normalized_name, id",
    )
    .bind(owner_id)
    .fetch_all(&state.pool)
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
            canonical_ingredient_id: canonical_id,
            completed_at: None,
            sources: Vec::new(),
        });
        item.completed_at = None;
        if !item.sources.iter().any(|source| source.kind == "automatic") {
            item.sources.push(ShoppingItemSource {
                kind: "automatic".into(),
                label: String::new(),
                recipe_id: None,
            });
        }
    }
    Ok(items
        .into_iter()
        .map(|(key, item)| ShoppingItem {
            id: item.id,
            name: item.name,
            canonical_ingredient_id: item.canonical_ingredient_id,
            automatic: automatic_ids.contains_key(&key),
            completed_at: item.completed_at.and_then(timestamp),
            pantry_item_id: automatic_ids.get(&key).cloned(),
            sources: item.sources,
        })
        .collect())
}

async fn load_sources(state: &AppState, item_id: &str) -> AppResult<Vec<ShoppingItemSource>> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT source_type, label, recipe_id
         FROM shopping_item_sources WHERE item_id = ? ORDER BY source_type, label",
    )
    .bind(item_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(kind, label, recipe_id)| ShoppingItemSource {
            kind,
            label,
            recipe_id,
        })
        .collect())
}

async fn upsert_stored_item(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    display_name: &str,
    canonical_id: Option<&str>,
) -> AppResult<String> {
    let (name, normalized) = validate_name(display_name)?;
    validate_selected_canonical(transaction, owner_id, canonical_id).await?;
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().timestamp();
    sqlx::query_scalar(
        "INSERT INTO shopping_items
         (id, owner_id, name, normalized_name, canonical_ingredient_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT DO UPDATE SET completed_at = NULL, updated_at = excluded.updated_at
         RETURNING id",
    )
    .bind(&id)
    .bind(owner_id)
    .bind(name)
    .bind(normalized)
    .bind(canonical_id)
    .bind(now)
    .bind(now)
    .fetch_one(&mut **transaction)
    .await
    .map_err(map_unique_conflict)
}

async fn insert_source(
    transaction: &mut Transaction<'_, Sqlite>,
    item_id: &str,
    source_type: &str,
    recipe_id: Option<&str>,
    label: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO shopping_item_sources
         (id, item_id, source_type, recipe_id, label, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
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
    _owner_id: &str,
    canonical_id: Option<&str>,
) -> AppResult<()> {
    let Some(canonical_id) = canonical_id else {
        return Ok(());
    };
    let visible: Option<i64> = sqlx::query_scalar("SELECT 1 FROM ingredient_catalog WHERE id = ?")
        .bind(canonical_id)
        .fetch_optional(&mut **transaction)
        .await?;
    if visible.is_none() {
        return Err(AppError::Validation(
            "selected canonical ingredient is not available to this account".into(),
        ));
    }
    Ok(())
}

async fn require_manual_source(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    item_id: &str,
) -> AppResult<()> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM shopping_items i
         JOIN shopping_item_sources s ON s.item_id = i.id
         WHERE i.id = ? AND i.owner_id = ? AND s.source_type = 'manual'",
    )
    .bind(item_id)
    .bind(owner_id)
    .fetch_optional(&mut **transaction)
    .await?;
    exists.map(|_| ()).ok_or(AppError::NotFound)
}

async fn stored_item_has_active_auto_source(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    item_id: &str,
) -> AppResult<bool> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM shopping_items i
         JOIN pantry_items p ON p.owner_id = i.owner_id
          AND ((i.canonical_ingredient_id IS NOT NULL
                AND p.canonical_ingredient_id = i.canonical_ingredient_id)
            OR (i.canonical_ingredient_id IS NULL
                AND p.canonical_ingredient_id IS NULL
                AND p.normalized_name = i.normalized_name))
         WHERE i.id = ? AND i.owner_id = ? AND p.status = 'out'
           AND p.auto_add_to_shopping_list_when_out = 1",
    )
    .bind(item_id)
    .bind(owner_id)
    .fetch_optional(&mut **transaction)
    .await?;
    Ok(exists.is_some())
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

fn dedupe_key(canonical_id: Option<&str>, normalized: &str) -> String {
    canonical_id
        .map(|id| format!("c:{id}"))
        .unwrap_or_else(|| format!("n:{normalized}"))
}

fn timestamp(value: i64) -> Option<String> {
    DateTime::from_timestamp(value, 0).map(|value| value.to_rfc3339())
}

fn map_unique_conflict(error: sqlx::Error) -> AppError {
    if matches!(&error, sqlx::Error::Database(database) if database.is_unique_violation()) {
        AppError::Conflict("that ingredient is already on the shopping list".into())
    } else {
        AppError::Database(error)
    }
}
