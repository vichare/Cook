use axum::{
    extract::{Path, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    Extension, Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use url::Url;
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    models::{IngredientAlternativeInput, IngredientGroupInput, IngredientInput, Recipe},
    routes::{
        insert_ingredient_groups, load_visible_recipes, validate_ingredient_groups, CurrentUser,
    },
    AppState,
};

pub const SCOPE_IMPORT: &str = "recipes:import";
pub const SCOPE_RECIPES_READ: &str = "recipes:read";
pub const SCOPE_RECIPES_WRITE: &str = "recipes:write";
pub const SCOPE_IMAGES_WRITE: &str = "recipes:images:write";
pub const SCOPE_RECIPES_PUBLISH: &str = "recipes:publish";
const IMPORT_RATE_WINDOW_SECONDS: i64 = 60 * 60;
const IMPORT_RATE_LIMIT: i64 = 20;
const READ_RATE_LIMIT: i64 = 240;
const RECIPE_WRITE_RATE_LIMIT: i64 = 600;
const IMAGE_WRITE_RATE_LIMIT: i64 = 300;
const PUBLISH_RATE_LIMIT: i64 = 60;
const CAPABILITIES_RATE_LIMIT: i64 = 120;
const ACTIVE_DRAFT_QUOTA: i64 = 50;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateImportTokenInput {
    name: String,
    expires_in_days: Option<i64>,
    preset: Option<String>,
    #[serde(default)]
    confirm_publish_risk: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportToken {
    id: String,
    name: String,
    prefix: String,
    scopes: Vec<String>,
    created_at: String,
    last_used_at: Option<String>,
    expires_at: Option<String>,
    revoked_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedImportToken {
    token: ImportToken,
    secret: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCapabilities {
    prefix: String,
    scopes: Vec<String>,
    expires_at: Option<String>,
    preset: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportDraftSubmission {
    schema_version: String,
    source_url: String,
    source_title: Option<String>,
    source_author: Option<String>,
    source_license: Option<String>,
    source_attribution: Option<String>,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    difficulty: Option<i64>,
    #[serde(default)]
    estimated_calories: Option<i64>,
    #[serde(default)]
    ingredients: Vec<IngredientInput>,
    #[serde(default)]
    ingredient_model: Option<String>,
    #[serde(default)]
    ingredient_groups: Option<Vec<IngredientGroupInput>>,
    steps: Vec<String>,
    #[serde(default)]
    warnings: Vec<String>,
    confidence: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportDraftUpdate {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    difficulty: Option<i64>,
    #[serde(default)]
    estimated_calories: Option<i64>,
    #[serde(default)]
    ingredients: Vec<IngredientInput>,
    #[serde(default)]
    ingredient_model: Option<String>,
    #[serde(default)]
    ingredient_groups: Option<Vec<IngredientGroupInput>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDraft {
    id: String,
    source_url: String,
    source_title: Option<String>,
    source_author: Option<String>,
    source_license: Option<String>,
    source_attribution: Option<String>,
    title: String,
    description: String,
    instructions: String,
    difficulty: Option<i64>,
    estimated_calories: Option<i64>,
    ingredient_model: String,
    ingredient_groups: Vec<IngredientGroupInput>,
    ingredients: Vec<IngredientInput>,
    warnings: Vec<String>,
    confidence: Option<f64>,
    status: String,
    accepted_recipe_id: Option<String>,
    review_url: String,
    created_at: String,
    updated_at: String,
}

pub(crate) async fn list_import_tokens(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<Vec<ImportToken>>> {
    let rows = sqlx::query(
        "SELECT id, name, token_prefix, scopes, created_at, last_used_at, expires_at, revoked_at
         FROM import_tokens WHERE owner_id = ? ORDER BY created_at DESC",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.into_iter().map(token_from_row).collect()))
}

pub(crate) async fn create_import_token(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(input): Json<CreateImportTokenInput>,
) -> AppResult<(StatusCode, Json<CreatedImportToken>)> {
    let name = validate_short_text("token name", &input.name, 80, false)?;
    let scopes = match input.preset.as_deref().unwrap_or("import") {
        "import" => vec![SCOPE_IMPORT.to_string()],
        "manage" => vec![
            SCOPE_RECIPES_READ.to_string(),
            SCOPE_RECIPES_WRITE.to_string(),
            SCOPE_IMAGES_WRITE.to_string(),
        ],
        "publish" if input.confirm_publish_risk => vec![
            SCOPE_RECIPES_READ.to_string(),
            SCOPE_RECIPES_WRITE.to_string(),
            SCOPE_IMAGES_WRITE.to_string(),
            SCOPE_RECIPES_PUBLISH.to_string(),
        ],
        "publish" => {
            return Err(AppError::Validation(
                "confirmPublishRisk must be true for the publish preset".into(),
            ))
        }
        _ => {
            return Err(AppError::Validation(
                "preset must be import, manage, or publish".into(),
            ))
        }
    };
    let expires_at = match input.expires_in_days {
        Some(days) if (1..=365).contains(&days) => Some(Utc::now() + Duration::days(days)),
        Some(_) => {
            return Err(AppError::Validation(
                "expiresInDays must be between 1 and 365".into(),
            ))
        }
        None => None,
    };
    let id = Uuid::new_v4().to_string();
    let secret = format!(
        "cook_imp_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    let prefix = secret.chars().take(17).collect::<String>();
    let created_at = Utc::now();
    sqlx::query(
        "INSERT INTO import_tokens
         (id, owner_id, name, token_digest, token_prefix, scope, scopes, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&user.id)
    .bind(&name)
    .bind(token_digest(&secret).as_slice())
    .bind(&prefix)
    .bind(SCOPE_IMPORT)
    .bind(scopes.join(" "))
    .bind(created_at.timestamp())
    .bind(expires_at.map(|value| value.timestamp()))
    .execute(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedImportToken {
            token: ImportToken {
                id,
                name,
                prefix,
                scopes,
                created_at: created_at.to_rfc3339(),
                last_used_at: None,
                expires_at: expires_at.map(|value| value.to_rfc3339()),
                revoked_at: None,
            },
            secret,
        }),
    ))
}

pub(crate) async fn revoke_import_token(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let result = sqlx::query(
        "UPDATE import_tokens SET revoked_at = COALESCE(revoked_at, ?)
         WHERE id = ? AND owner_id = ?",
    )
    .bind(Utc::now().timestamp())
    .bind(id)
    .bind(user.id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn inspect_ai_capabilities(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<AiCapabilities>> {
    let principal = authenticate_token_inner(
        &state.pool,
        &headers,
        None,
        "capabilities",
        CAPABILITIES_RATE_LIMIT,
    )
    .await?;
    let preset = token_preset(&principal.scopes).map(str::to_string);
    let capabilities = AiCapabilities {
        prefix: principal.prefix.clone(),
        scopes: principal.scopes.clone(),
        expires_at: principal.expires_at.map(timestamp),
        preset,
    };
    let mut transaction = state.pool.begin().await?;
    record_token_audit(
        &mut transaction,
        &principal,
        "token.capabilities",
        None,
        Utc::now().timestamp(),
    )
    .await?;
    transaction.commit().await?;
    Ok(Json(capabilities))
}

pub(crate) async fn create_import_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ImportDraftSubmission>,
) -> AppResult<(StatusCode, Json<ImportDraft>)> {
    validate_submission(&input)?;
    let principal = authenticate_token(&state.pool, &headers, SCOPE_IMPORT).await?;
    let token_id = principal.token_id.clone();
    let owner_id = principal.owner_id.clone();
    let now = Utc::now();
    let id = Uuid::new_v4().to_string();
    let instructions = input
        .steps
        .iter()
        .map(|step| step.trim())
        .collect::<Vec<_>>()
        .join("\n\n");
    let warnings_json = serde_json::to_string(&input.warnings)
        .map_err(|_| AppError::Validation("warnings are invalid".into()))?;
    let mut transaction = state.pool.begin().await?;
    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM import_drafts WHERE owner_id = ? AND status = 'draft'",
    )
    .bind(&owner_id)
    .fetch_one(&mut *transaction)
    .await?;
    if active_count >= ACTIVE_DRAFT_QUOTA {
        return Err(AppError::RateLimited);
    }
    sqlx::query(
        "INSERT INTO import_drafts
         (id, owner_id, source_url, source_title, source_author, source_license,
          source_attribution, title, description,
          instructions, difficulty, estimated_calories, warnings_json, confidence, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&owner_id)
    .bind(input.source_url.trim())
    .bind(trim_optional(input.source_title.as_deref()))
    .bind(trim_optional(input.source_author.as_deref()))
    .bind(trim_optional(input.source_license.as_deref()))
    .bind(trim_optional(input.source_attribution.as_deref()))
    .bind(input.title.trim())
    .bind(input.description.trim())
    .bind(&instructions)
    .bind(input.difficulty)
    .bind(input.estimated_calories)
    .bind(&warnings_json)
    .bind(input.confidence)
    .bind(now.timestamp())
    .bind(now.timestamp())
    .execute(&mut *transaction)
    .await?;
    insert_draft_groups(&mut transaction, &id, &submission_groups(&input)?).await?;
    record_token_audit(
        &mut transaction,
        &principal,
        "import_draft.create",
        Some(&id),
        now.timestamp(),
    )
    .await?;
    transaction.commit().await?;
    tracing::info!(token_id = %token_id, "recipe import draft created");
    Ok((
        StatusCode::CREATED,
        Json(load_import_draft(&state.pool, &owner_id, &id).await?),
    ))
}

pub(crate) async fn list_import_drafts(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<Vec<ImportDraft>>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM import_drafts WHERE owner_id = ? ORDER BY updated_at DESC",
    )
    .bind(&user.id)
    .fetch_all(&state.pool)
    .await?;
    let mut drafts = Vec::with_capacity(ids.len());
    for id in ids {
        drafts.push(load_import_draft(&state.pool, &user.id, &id).await?);
    }
    Ok(Json(drafts))
}

pub(crate) async fn get_import_draft(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<Json<ImportDraft>> {
    Ok(Json(load_import_draft(&state.pool, &user.id, &id).await?))
}

pub(crate) async fn update_import_draft(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(input): Json<ImportDraftUpdate>,
) -> AppResult<Json<ImportDraft>> {
    validate_draft_update(&input)?;
    let mut transaction = state.pool.begin().await?;
    if input.ingredient_groups.is_none() {
        let requires_groups: Option<i64> = sqlx::query_scalar(
            "SELECT 1
             FROM import_draft_ingredient_groups g
             JOIN import_drafts d ON d.id = g.draft_id
             WHERE d.id = ? AND d.owner_id = ? AND d.status = 'draft'
               AND (g.optional = 1 OR g.note <> '' OR
                    (SELECT COUNT(*) FROM import_draft_ingredient_alternatives a
                     WHERE a.group_id = g.id) > 1)
             LIMIT 1",
        )
        .bind(&id)
        .bind(&user.id)
        .fetch_optional(&mut *transaction)
        .await?;
        if requires_groups.is_some() {
            return Err(AppError::Validation("ingredient_groups_required".into()));
        }
    }
    let result = sqlx::query(
        "UPDATE import_drafts
         SET title = ?, description = ?, instructions = ?, difficulty = ?,
             estimated_calories = ?, updated_at = ?
         WHERE id = ? AND owner_id = ? AND status = 'draft'",
    )
    .bind(input.title.trim())
    .bind(input.description.trim())
    .bind(input.instructions.trim())
    .bind(input.difficulty)
    .bind(input.estimated_calories)
    .bind(Utc::now().timestamp())
    .bind(&id)
    .bind(&user.id)
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() == 0 {
        return draft_write_error(&mut transaction, &user.id, &id).await;
    }
    sqlx::query("DELETE FROM import_draft_ingredients WHERE draft_id = ?")
        .bind(&id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM import_draft_ingredient_groups WHERE draft_id = ?")
        .bind(&id)
        .execute(&mut *transaction)
        .await?;
    insert_draft_groups(&mut transaction, &id, &update_groups(&input)?).await?;
    transaction.commit().await?;
    Ok(Json(load_import_draft(&state.pool, &user.id, &id).await?))
}

pub(crate) async fn discard_import_draft(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let result = sqlx::query(
        "UPDATE import_drafts SET status = 'discarded', updated_at = ?
         WHERE id = ? AND owner_id = ? AND status = 'draft'",
    )
    .bind(Utc::now().timestamp())
    .bind(&id)
    .bind(&user.id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn accept_import_draft(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<(StatusCode, Json<Recipe>)> {
    let mut transaction = state.pool.begin().await?;
    let row = sqlx::query(
        "SELECT source_url, source_title, source_author, source_license, source_attribution,
           title, description, instructions,
           difficulty, estimated_calories,
           warnings_json, confidence, status
         FROM import_drafts WHERE id = ? AND owner_id = ?",
    )
    .bind(&id)
    .bind(&user.id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::NotFound)?;
    let status: String = row.get("status");
    if status != "draft" {
        return Err(AppError::Conflict(
            "import draft has already been consumed or discarded".into(),
        ));
    }
    let groups = load_draft_groups_tx(&mut transaction, &id).await?;
    let recipe_id = Uuid::new_v4().to_string();
    let source_title: Option<String> = row.get("source_title");
    let source_author: Option<String> = row.get("source_author");
    let attribution = row
        .get::<Option<String>, _>("source_attribution")
        .or_else(|| match (&source_title, &source_author) {
            (Some(title), Some(author)) => Some(format!("{title} — {author}")),
            (Some(title), None) => Some(title.clone()),
            (None, Some(author)) => Some(author.clone()),
            (None, None) => None,
        });
    sqlx::query(
        "INSERT INTO recipes
         (id, owner_id, title, description, instructions, visibility, source_url,
          difficulty, estimated_calories, source_license, source_attribution, source_title, source_author,
          source_warnings, source_confidence)
         VALUES (?, ?, ?, ?, ?, 'private', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&recipe_id)
    .bind(&user.id)
    .bind(row.get::<String, _>("title"))
    .bind(row.get::<String, _>("description"))
    .bind(row.get::<String, _>("instructions"))
    .bind(row.get::<String, _>("source_url"))
    .bind(row.get::<Option<i64>, _>("difficulty"))
    .bind(row.get::<Option<i64>, _>("estimated_calories"))
    .bind(row.get::<Option<String>, _>("source_license"))
    .bind(attribution)
    .bind(source_title)
    .bind(source_author)
    .bind(row.get::<String, _>("warnings_json"))
    .bind(row.get::<Option<f64>, _>("confidence"))
    .execute(&mut *transaction)
    .await?;
    insert_ingredient_groups(&mut transaction, &recipe_id, &user.id, &groups).await?;
    let result = sqlx::query(
        "UPDATE import_drafts
         SET status = 'consumed', accepted_recipe_id = ?, consumed_at = ?, updated_at = ?
         WHERE id = ? AND owner_id = ? AND status = 'draft'",
    )
    .bind(&recipe_id)
    .bind(Utc::now().timestamp())
    .bind(Utc::now().timestamp())
    .bind(&id)
    .bind(&user.id)
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::Conflict(
            "import draft was consumed concurrently".into(),
        ));
    }
    transaction.commit().await?;
    let recipe = load_visible_recipes(&state.pool, &user.id, Some(&recipe_id))
        .await?
        .into_iter()
        .next()
        .ok_or(AppError::NotFound)?;
    Ok((StatusCode::CREATED, Json(recipe)))
}

#[derive(Clone, Debug)]
pub struct TokenPrincipal {
    pub token_id: String,
    pub owner_id: String,
    pub prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<i64>,
}

pub async fn authenticate_token(
    pool: &SqlitePool,
    headers: &HeaderMap,
    required_scope: &str,
) -> AppResult<TokenPrincipal> {
    authenticate_token_inner(
        pool,
        headers,
        Some(required_scope),
        required_scope,
        rate_limit_for_scope(required_scope),
    )
    .await
}

async fn authenticate_token_inner(
    pool: &SqlitePool,
    headers: &HeaderMap,
    required_scope: Option<&str>,
    rate_limit_bucket: &str,
    rate_limit: i64,
) -> AppResult<TokenPrincipal> {
    let secret = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
        .ok_or(AppError::Unauthorized)?;
    let now = Utc::now().timestamp();
    let mut transaction = pool.begin().await?;
    let row = sqlx::query(
        "SELECT id, owner_id, token_prefix, scopes, expires_at, revoked_at
         FROM import_tokens WHERE token_digest = ?",
    )
    .bind(token_digest(secret).as_slice())
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::Unauthorized)?;
    if row.get::<Option<i64>, _>("revoked_at").is_some()
        || row
            .get::<Option<i64>, _>("expires_at")
            .is_some_and(|expires_at| expires_at <= now)
    {
        return Err(AppError::Unauthorized);
    }
    let scopes = row
        .get::<String, _>("scopes")
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if required_scope.is_some_and(|required| !scopes.iter().any(|scope| scope == required)) {
        return Err(AppError::Forbidden);
    }
    let token_id: String = row.get("id");
    enforce_token_rate_limit(
        &mut transaction,
        &token_id,
        rate_limit_bucket,
        rate_limit,
        now,
    )
    .await?;
    sqlx::query("UPDATE import_tokens SET last_used_at = ? WHERE id = ?")
        .bind(now)
        .bind(&token_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(TokenPrincipal {
        token_id,
        owner_id: row.get("owner_id"),
        prefix: row.get("token_prefix"),
        scopes,
        expires_at: row.get("expires_at"),
    })
}

fn token_preset(scopes: &[String]) -> Option<&'static str> {
    let has = |scope: &str| scopes.iter().any(|candidate| candidate == scope);
    if has(SCOPE_RECIPES_READ)
        && has(SCOPE_RECIPES_WRITE)
        && has(SCOPE_IMAGES_WRITE)
        && has(SCOPE_RECIPES_PUBLISH)
    {
        Some("publish")
    } else if has(SCOPE_RECIPES_READ) && has(SCOPE_RECIPES_WRITE) && has(SCOPE_IMAGES_WRITE) {
        Some("manage")
    } else if scopes.len() == 1 && has(SCOPE_IMPORT) {
        Some("import")
    } else {
        None
    }
}

pub async fn record_token_audit(
    transaction: &mut Transaction<'_, Sqlite>,
    principal: &TokenPrincipal,
    action: &str,
    recipe_id: Option<&str>,
    now: i64,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO ai_token_audit_events
         (id, token_id, token_prefix, action, recipe_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&principal.token_id)
    .bind(&principal.prefix)
    .bind(action)
    .bind(recipe_id)
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn enforce_token_rate_limit(
    transaction: &mut Transaction<'_, Sqlite>,
    token_id: &str,
    bucket: &str,
    limit: i64,
    now: i64,
) -> AppResult<()> {
    let row: Option<(i64, i64)> = sqlx::query_as(
        "SELECT window_started_at, request_count FROM ai_token_rate_limits
         WHERE token_id = ? AND bucket = ?",
    )
    .bind(token_id)
    .bind(bucket)
    .fetch_optional(&mut **transaction)
    .await?;
    match row {
        Some((window, count)) if now - window < IMPORT_RATE_WINDOW_SECONDS => {
            if count >= limit {
                return Err(AppError::RateLimited);
            }
            sqlx::query(
                "UPDATE ai_token_rate_limits SET request_count = request_count + 1
                 WHERE token_id = ? AND bucket = ?",
            )
            .bind(token_id)
            .bind(bucket)
            .execute(&mut **transaction)
            .await?;
        }
        _ => {
            sqlx::query(
                "INSERT INTO ai_token_rate_limits
                 (token_id, bucket, window_started_at, request_count)
                 VALUES (?, ?, ?, 1)
                 ON CONFLICT(token_id, bucket) DO UPDATE SET
                   window_started_at = excluded.window_started_at, request_count = 1",
            )
            .bind(token_id)
            .bind(bucket)
            .bind(now)
            .execute(&mut **transaction)
            .await?;
        }
    }
    Ok(())
}

fn rate_limit_for_scope(scope: &str) -> i64 {
    match scope {
        SCOPE_RECIPES_READ => READ_RATE_LIMIT,
        SCOPE_RECIPES_WRITE => RECIPE_WRITE_RATE_LIMIT,
        SCOPE_IMAGES_WRITE => IMAGE_WRITE_RATE_LIMIT,
        SCOPE_RECIPES_PUBLISH => PUBLISH_RATE_LIMIT,
        _ => IMPORT_RATE_LIMIT,
    }
}

fn timestamp(value: i64) -> String {
    DateTime::from_timestamp(value, 0)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .to_rfc3339()
}

fn validate_submission(input: &ImportDraftSubmission) -> AppResult<()> {
    if input.schema_version != "1.0" && input.schema_version != "2.0" {
        return Err(AppError::Validation(
            "schemaVersion must be 1.0 or 2.0".into(),
        ));
    }
    let url = Url::parse(input.source_url.trim())
        .map_err(|_| AppError::Validation("sourceUrl must be a valid URL".into()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AppError::Validation(
            "sourceUrl must use http or https and include a host".into(),
        ));
    }
    validate_short_text("sourceUrl", &input.source_url, 2048, false)?;
    validate_optional_text("sourceTitle", input.source_title.as_deref(), 200)?;
    validate_optional_text("sourceAuthor", input.source_author.as_deref(), 200)?;
    validate_optional_text("sourceLicense", input.source_license.as_deref(), 200)?;
    validate_optional_text(
        "sourceAttribution",
        input.source_attribution.as_deref(),
        1000,
    )?;
    validate_short_text("title", &input.title, 120, false)?;
    validate_short_text("description", &input.description, 2000, true)?;
    validate_recipe_metadata(input.difficulty, input.estimated_calories)?;
    validate_ingredient_groups(&submission_groups(input)?)?;
    if input.steps.is_empty() || input.steps.len() > 50 {
        return Err(AppError::Validation(
            "steps must contain between 1 and 50 items".into(),
        ));
    }
    let mut total_step_chars = 0;
    for step in &input.steps {
        total_step_chars += validate_short_text("step", step, 2000, false)?
            .chars()
            .count();
    }
    if total_step_chars > 20_000 {
        return Err(AppError::Validation(
            "combined steps must be at most 20000 characters".into(),
        ));
    }
    if input.warnings.len() > 20 {
        return Err(AppError::Validation(
            "warnings must contain at most 20 items".into(),
        ));
    }
    for warning in &input.warnings {
        validate_short_text("warning", warning, 500, false)?;
    }
    if input
        .confidence
        .is_some_and(|confidence| !confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
    {
        return Err(AppError::Validation(
            "confidence must be between 0 and 1".into(),
        ));
    }
    Ok(())
}

fn validate_draft_update(input: &ImportDraftUpdate) -> AppResult<()> {
    validate_short_text("title", &input.title, 120, false)?;
    validate_short_text("description", &input.description, 2000, true)?;
    validate_short_text("instructions", &input.instructions, 20_000, false)?;
    validate_recipe_metadata(input.difficulty, input.estimated_calories)?;
    validate_ingredient_groups(&update_groups(input)?)
}

fn legacy_groups(ingredients: &[IngredientInput]) -> Vec<IngredientGroupInput> {
    ingredients
        .iter()
        .enumerate()
        .map(|(index, ingredient)| IngredientGroupInput {
            id: Uuid::new_v4().to_string(),
            optional: false,
            note: String::new(),
            sort_order: index as i64,
            alternatives: vec![IngredientAlternativeInput {
                id: Uuid::new_v4().to_string(),
                name: ingredient.name.clone(),
                quantity_note: ingredient.quantity_note.clone(),
                canonical_ingredient_id: ingredient.canonical_ingredient_id.clone(),
                sort_order: 0,
            }],
        })
        .collect()
}

fn groups_contract(
    ingredients: &[IngredientInput],
    ingredient_model: Option<&str>,
    ingredient_groups: Option<&[IngredientGroupInput]>,
) -> AppResult<Vec<IngredientGroupInput>> {
    if ingredient_groups.is_some() && !ingredients.is_empty() {
        return Err(AppError::Validation(
            "ingredients and ingredientGroups cannot both be provided".into(),
        ));
    }
    if let Some(groups) = ingredient_groups {
        if ingredient_model != Some("groups-v1") {
            return Err(AppError::Validation(
                "ingredientModel must be groups-v1 when ingredientGroups is provided".into(),
            ));
        }
        Ok(groups.to_vec())
    } else {
        if ingredient_model.is_some() {
            return Err(AppError::Validation(
                "ingredientModel is only valid with ingredientGroups".into(),
            ));
        }
        Ok(legacy_groups(ingredients))
    }
}

fn submission_groups(input: &ImportDraftSubmission) -> AppResult<Vec<IngredientGroupInput>> {
    groups_contract(
        &input.ingredients,
        input.ingredient_model.as_deref(),
        input.ingredient_groups.as_deref(),
    )
}

fn update_groups(input: &ImportDraftUpdate) -> AppResult<Vec<IngredientGroupInput>> {
    groups_contract(
        &input.ingredients,
        input.ingredient_model.as_deref(),
        input.ingredient_groups.as_deref(),
    )
}

fn validate_recipe_metadata(
    difficulty: Option<i64>,
    estimated_calories: Option<i64>,
) -> AppResult<()> {
    if difficulty.is_some_and(|value| !(1..=5).contains(&value)) {
        return Err(AppError::Validation(
            "difficulty must be an integer from 1 to 5".into(),
        ));
    }
    if estimated_calories.is_some_and(|value| !(1..=100_000).contains(&value)) {
        return Err(AppError::Validation(
            "estimatedCalories must be an integer from 1 to 100000".into(),
        ));
    }
    Ok(())
}

fn validate_optional_text(field: &str, value: Option<&str>, max: usize) -> AppResult<()> {
    if let Some(value) = value {
        validate_short_text(field, value, max, true)?;
    }
    Ok(())
}

fn validate_short_text(
    field: &str,
    value: &str,
    max: usize,
    allow_empty: bool,
) -> AppResult<String> {
    let value = value.trim();
    if !allow_empty && value.is_empty() {
        return Err(AppError::Validation(format!("{field} is required")));
    }
    if value.chars().count() > max {
        return Err(AppError::Validation(format!(
            "{field} must be at most {max} characters"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(AppError::Validation(format!(
            "{field} cannot contain control characters"
        )));
    }
    Ok(value.to_string())
}

async fn load_import_draft(pool: &SqlitePool, owner_id: &str, id: &str) -> AppResult<ImportDraft> {
    let row = sqlx::query(
        "SELECT id, source_url, source_title, source_author, source_license, source_attribution,
           title, description, instructions,
           difficulty, estimated_calories,
           warnings_json, confidence, status, accepted_recipe_id, created_at, updated_at
         FROM import_drafts WHERE id = ? AND owner_id = ?",
    )
    .bind(id)
    .bind(owner_id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(ImportDraft {
        id: row.get("id"),
        source_url: row.get("source_url"),
        source_title: row.get("source_title"),
        source_author: row.get("source_author"),
        source_license: row.get("source_license"),
        source_attribution: row.get("source_attribution"),
        title: row.get("title"),
        description: row.get("description"),
        instructions: row.get("instructions"),
        difficulty: row.get("difficulty"),
        estimated_calories: row.get("estimated_calories"),
        ingredient_model: "groups-v1".into(),
        ingredient_groups: load_draft_groups(pool, id).await?,
        ingredients: load_draft_ingredients(pool, id).await?,
        warnings: serde_json::from_str(&row.get::<String, _>("warnings_json"))
            .map_err(|error| AppError::Data(format!("invalid stored import warnings: {error}")))?,
        confidence: row.get("confidence"),
        status: row.get("status"),
        accepted_recipe_id: row.get("accepted_recipe_id"),
        review_url: format!("/imports/{id}"),
        created_at: timestamp_to_string(row.get("created_at")),
        updated_at: timestamp_to_string(row.get("updated_at")),
    })
}

async fn load_draft_ingredients(pool: &SqlitePool, id: &str) -> AppResult<Vec<IngredientInput>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, quantity_note FROM import_draft_ingredients
         WHERE draft_id = ? ORDER BY sort_order",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, quantity_note)| IngredientInput {
            name,
            quantity_note,
            canonical_ingredient_id: None,
        })
        .collect())
}

async fn load_draft_groups(pool: &SqlitePool, id: &str) -> AppResult<Vec<IngredientGroupInput>> {
    let rows = sqlx::query(
        "SELECT g.external_id AS group_id, g.optional, g.note, g.sort_order AS group_sort,
                a.external_id AS alternative_id, a.name, a.quantity_note,
                a.canonical_ingredient_id, a.sort_order AS alternative_sort
         FROM import_draft_ingredient_groups g
         JOIN import_draft_ingredient_alternatives a ON a.group_id = g.id
         WHERE g.draft_id = ? ORDER BY g.sort_order, a.sort_order",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    Ok(draft_groups_from_rows(rows))
}

async fn load_draft_groups_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> AppResult<Vec<IngredientGroupInput>> {
    let rows = sqlx::query(
        "SELECT g.external_id AS group_id, g.optional, g.note, g.sort_order AS group_sort,
                a.external_id AS alternative_id, a.name, a.quantity_note,
                a.canonical_ingredient_id, a.sort_order AS alternative_sort
         FROM import_draft_ingredient_groups g
         JOIN import_draft_ingredient_alternatives a ON a.group_id = g.id
         WHERE g.draft_id = ? ORDER BY g.sort_order, a.sort_order",
    )
    .bind(id)
    .fetch_all(&mut **transaction)
    .await?;
    Ok(draft_groups_from_rows(rows))
}

fn draft_groups_from_rows(rows: Vec<sqlx::sqlite::SqliteRow>) -> Vec<IngredientGroupInput> {
    let mut groups = Vec::<IngredientGroupInput>::new();
    for row in rows {
        let group_id: String = row.get("group_id");
        if groups.last().is_none_or(|group| group.id != group_id) {
            groups.push(IngredientGroupInput {
                id: group_id,
                optional: row.get("optional"),
                note: row.get("note"),
                sort_order: row.get("group_sort"),
                alternatives: Vec::new(),
            });
        }
        groups
            .last_mut()
            .expect("group was inserted")
            .alternatives
            .push(IngredientAlternativeInput {
                id: row.get("alternative_id"),
                name: row.get("name"),
                quantity_note: row.get("quantity_note"),
                canonical_ingredient_id: row.get("canonical_ingredient_id"),
                sort_order: row.get("alternative_sort"),
            });
    }
    groups
}

async fn insert_draft_groups(
    transaction: &mut Transaction<'_, Sqlite>,
    draft_id: &str,
    groups: &[IngredientGroupInput],
) -> AppResult<()> {
    let mut groups = groups.iter().collect::<Vec<_>>();
    groups.sort_by_key(|group| group.sort_order);
    for group in groups {
        let persisted_group_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO import_draft_ingredient_groups
             (id, draft_id, external_id, optional, note, sort_order)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&persisted_group_id)
        .bind(draft_id)
        .bind(&group.id)
        .bind(group.optional)
        .bind(group.note.trim())
        .bind(group.sort_order)
        .execute(&mut **transaction)
        .await?;
        let mut alternatives = group.alternatives.iter().collect::<Vec<_>>();
        alternatives.sort_by_key(|alternative| alternative.sort_order);
        for alternative in alternatives {
            sqlx::query(
                "INSERT INTO import_draft_ingredient_alternatives
                 (id, group_id, external_id, name, quantity_note,
                  canonical_ingredient_id, sort_order)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(&persisted_group_id)
            .bind(&alternative.id)
            .bind(alternative.name.trim())
            .bind(alternative.quantity_note.trim())
            .bind(&alternative.canonical_ingredient_id)
            .bind(alternative.sort_order)
            .execute(&mut **transaction)
            .await?;
        }
        let display = group
            .alternatives
            .iter()
            .map(|alternative| alternative.name.trim())
            .collect::<Vec<_>>()
            .join(" / ");
        sqlx::query(
            "INSERT INTO import_draft_ingredients
             (id, draft_id, name, quantity_note, sort_order) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(draft_id)
        .bind(display)
        .bind(if group.alternatives.len() == 1 {
            group.alternatives[0].quantity_note.trim()
        } else {
            group.note.trim()
        })
        .bind(group.sort_order)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn draft_write_error<T>(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    id: &str,
) -> AppResult<T> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM import_drafts WHERE id = ? AND owner_id = ?")
            .bind(id)
            .bind(owner_id)
            .fetch_optional(&mut **transaction)
            .await?;
    match status {
        Some(_) => Err(AppError::Conflict(
            "only pending import drafts can be changed".into(),
        )),
        None => Err(AppError::NotFound),
    }
}

fn token_from_row(row: sqlx::sqlite::SqliteRow) -> ImportToken {
    ImportToken {
        id: row.get("id"),
        name: row.get("name"),
        prefix: row.get("token_prefix"),
        scopes: row
            .get::<String, _>("scopes")
            .split_whitespace()
            .map(str::to_string)
            .collect(),
        created_at: timestamp_to_string(row.get("created_at")),
        last_used_at: row
            .get::<Option<i64>, _>("last_used_at")
            .map(timestamp_to_string),
        expires_at: row
            .get::<Option<i64>, _>("expires_at")
            .map(timestamp_to_string),
        revoked_at: row
            .get::<Option<i64>, _>("revoked_at")
            .map(timestamp_to_string),
    }
}

fn token_digest(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

fn timestamp_to_string(timestamp: i64) -> String {
    DateTime::from_timestamp(timestamp, 0)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .to_rfc3339()
}

fn trim_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_scope_rate_limits_preserve_read_boundaries_and_allow_bulk_writes() {
        assert_eq!(rate_limit_for_scope(SCOPE_IMPORT), 20);
        assert_eq!(rate_limit_for_scope(SCOPE_RECIPES_READ), 240);
        assert_eq!(rate_limit_for_scope(SCOPE_RECIPES_WRITE), 600);
        assert_eq!(rate_limit_for_scope(SCOPE_IMAGES_WRITE), 300);
        assert_eq!(rate_limit_for_scope(SCOPE_RECIPES_PUBLISH), 60);
        assert_eq!(CAPABILITIES_RATE_LIMIT, 120);
    }

    #[test]
    fn import_payload_rejects_non_http_urls_and_oversized_ingredients() {
        let mut input = ImportDraftSubmission {
            schema_version: "1.0".into(),
            source_url: "file:///etc/passwd".into(),
            source_title: None,
            source_author: None,
            source_license: None,
            source_attribution: None,
            title: "Soup".into(),
            description: String::new(),
            difficulty: None,
            estimated_calories: None,
            ingredients: vec![IngredientInput {
                name: "water".into(),
                quantity_note: String::new(),
                canonical_ingredient_id: None,
            }],
            ingredient_model: None,
            ingredient_groups: None,
            steps: vec!["Stir.".into()],
            warnings: vec![],
            confidence: Some(0.8),
        };
        assert!(validate_submission(&input).is_err());
        input.source_url = "https://example.test/recipe".into();
        input.ingredients = (0..101)
            .map(|index| IngredientInput {
                name: format!("ingredient {index}"),
                quantity_note: String::new(),
                canonical_ingredient_id: None,
            })
            .collect();
        assert!(validate_submission(&input).is_err());
    }

    #[test]
    fn v2_import_preserves_optional_or_groups_and_rejects_legacy_mix() {
        let group = IngredientGroupInput {
            id: "fat".into(),
            optional: true,
            note: "for frying".into(),
            sort_order: 0,
            alternatives: vec![
                IngredientAlternativeInput {
                    id: "butter".into(),
                    name: "butter".into(),
                    quantity_note: "1 tbsp".into(),
                    canonical_ingredient_id: None,
                    sort_order: 0,
                },
                IngredientAlternativeInput {
                    id: "oil".into(),
                    name: "oil".into(),
                    quantity_note: "1 tbsp".into(),
                    canonical_ingredient_id: None,
                    sort_order: 1,
                },
            ],
        };
        let mut input = ImportDraftSubmission {
            schema_version: "2.0".into(),
            source_url: "https://example.test/recipe".into(),
            source_title: None,
            source_author: None,
            source_license: None,
            source_attribution: None,
            title: "Soup".into(),
            description: String::new(),
            difficulty: None,
            estimated_calories: None,
            ingredients: Vec::new(),
            ingredient_model: Some("groups-v1".into()),
            ingredient_groups: Some(vec![group]),
            steps: vec!["Cook.".into()],
            warnings: Vec::new(),
            confidence: None,
        };
        assert!(validate_submission(&input).is_ok());
        input.ingredients.push(IngredientInput {
            name: "salt".into(),
            quantity_note: String::new(),
            canonical_ingredient_id: None,
        });
        assert!(validate_submission(&input).is_err());
    }
}
