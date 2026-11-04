use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{
        header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, SET_COOKIE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{IntoResponse, Response},
    Json,
};
use axum_extra::extract::cookie::CookieJar;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use rust_web_auth_core::UserId;
use rust_web_auth_http::{expired_cookie, issued_cookie};
use rust_web_auth_passkey::PublicKeyCredential;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    routes::{CurrentAuth, CurrentUser},
    AppState,
};

const REAUTH_SECONDS: i64 = 10 * 60;
const CEREMONY_SECONDS: i64 = 5 * 60;
const MAX_REPORT_DETAILS: usize = 500;
const MAX_APPEAL_REASON: usize = 1_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimStartInput {
    code: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimFinishInput {
    code: String,
    credential: PublicKeyCredential,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportInput {
    reason: String,
    #[serde(default)]
    details: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppealInput {
    reason: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminMutationInput {
    pub expected_recipe_version: i64,
    pub expected_moderation_version: i64,
    pub expected_case_version: Option<i64>,
    pub reason_code: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppealResolutionInput {
    pub resolution: String,
    pub expected_recipe_version: i64,
    pub expected_moderation_version: i64,
    pub expected_case_version: i64,
    pub expected_appeal_version: i64,
    pub reason_code: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorQuery {
    cursor: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Notification {
    id: String,
    recipe_id: String,
    kind: String,
    reason_code: String,
    created_at: i64,
    read_at: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueueItem {
    recipe_id: String,
    title: String,
    author_handle: String,
    review_state: String,
    moderation_state: String,
    recipe_version: i64,
    moderation_version: i64,
    case_id: Option<String>,
    case_version: Option<i64>,
    report_count: i64,
    active_appeal: bool,
    updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueuePage {
    items: Vec<QueueItem>,
    next_cursor: Option<String>,
}

pub(crate) async fn delete_owned_recipe(
    state: &AppState,
    user_id: &str,
    recipe_id: &str,
) -> AppResult<()> {
    let now = Utc::now().timestamp();
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let owned: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM recipes WHERE id = ? AND owner_id = ?")
            .bind(recipe_id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if owned.is_none() {
        return Err(AppError::NotFound);
    }
    let case_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM moderation_cases
         WHERE recipe_id = ? AND status IN ('open', 'restricted')",
    )
    .bind(recipe_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(case_id) = case_id.as_deref() {
        sqlx::query(
            "UPDATE moderation_cases
             SET status = 'content_removed', version = version + 1,
                 updated_at = ?, resolved_at = ?
             WHERE id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(case_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE moderation_appeals
             SET status = 'content_removed', version = version + 1, resolved_at = ?
             WHERE case_id = ? AND status = 'active'",
        )
        .bind(now)
        .bind(case_id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("DELETE FROM recipes WHERE id = ? AND owner_id = ?")
        .bind(recipe_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    audit(
        &mut tx,
        "recipe_content_removed",
        None,
        Some(user_id),
        Some(recipe_id),
        case_id.as_deref(),
        None,
        None,
        None,
        None,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn claim_start(
    State(state): State<AppState>,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    axum::Extension(CurrentAuth(auth)): axum::Extension<CurrentAuth>,
    Json(input): Json<ClaimStartInput>,
) -> AppResult<impl IntoResponse> {
    rate_limit(&state, &format!("claim:{}", user.id), 10, 60 * 60).await?;
    let code_digest = claim_digest(&input.code)?;
    let now = Utc::now().timestamp();
    let valid: i64 = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM capability_claim_codes
           WHERE code_digest = ? AND capability = 'content_moderation'
             AND consumed_at IS NULL AND expires_at > ?
         )",
    )
    .bind(code_digest.as_slice())
    .bind(now)
    .fetch_one(&state.pool)
    .await?;
    let passkeys: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM passkey_credentials WHERE user_id = ? AND revoked_at IS NULL",
    )
    .bind(&user.id)
    .fetch_one(&state.pool)
    .await?;
    if valid == 0 || passkeys == 0 {
        return Err(AppError::NotFound);
    }
    let user_id = UserId::new(user.id.clone()).map_err(|_| AppError::AuthUnavailable)?;
    let start = state
        .auth
        .passwordless
        .passkeys
        .start_authentication(user_id, Utc::now())
        .await
        .map_err(crate::passwordless::map_passkey_error)?;
    remember_attempt(
        &state,
        start.ceremony_token.expose_for_cookie(),
        auth.session_id.to_string().as_str(),
        &user.id,
        "moderator_claim",
        Some(&code_digest),
    )
    .await?;
    let cookie = issued_cookie(
        &state.auth.admin_reauth_cookie,
        start.ceremony_token.expose_for_cookie(),
        CEREMONY_SECONDS as u64,
    )
    .map_err(|_| AppError::AuthUnavailable)?;
    Ok(([(SET_COOKIE, cookie)], Json(start.challenge)))
}

pub(crate) async fn claim_finish(
    State(state): State<AppState>,
    jar: CookieJar,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    axum::Extension(CurrentAuth(auth)): axum::Extension<CurrentAuth>,
    Json(input): Json<ClaimFinishInput>,
) -> AppResult<impl IntoResponse> {
    let token = reauth_cookie(&state, &jar)?;
    let code_digest = claim_digest(&input.code)?;
    consume_attempt(
        &state,
        token,
        auth.session_id.to_string().as_str(),
        &user.id,
        "moderator_claim",
        Some(&code_digest),
    )
    .await?;
    let verified = state
        .auth
        .passwordless
        .passkeys
        .finish_authentication(token, &input.credential, Utc::now())
        .await
        .map_err(crate::passwordless::map_passkey_error)?;
    if verified.as_str() != user.id {
        return Err(AppError::NotFound);
    }

    let now = Utc::now().timestamp();
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let code_id: String = sqlx::query_scalar(
        "SELECT id FROM capability_claim_codes
         WHERE code_digest = ? AND capability = 'content_moderation'
           AND consumed_at IS NULL AND expires_at > ?",
    )
    .bind(code_digest.as_slice())
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let existing_assignment = sqlx::query(
        "SELECT id, revoked_at FROM capability_assignments
         WHERE user_id = ? AND capability = 'content_moderation'",
    )
    .bind(&user.id)
    .fetch_optional(&mut *tx)
    .await?;
    let assignment_id = if let Some(existing) = existing_assignment {
        if existing.get::<Option<i64>, _>("revoked_at").is_none() {
            return Err(AppError::Conflict(
                "content moderator capability is already assigned".into(),
            ));
        }
        let id: String = existing.get("id");
        sqlx::query(
            "UPDATE capability_assignments
             SET created_at = ?, revoked_at = NULL
             WHERE id = ? AND revoked_at IS NOT NULL",
        )
        .bind(now)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        id
    } else {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO capability_assignments (id, user_id, capability, created_at)
             VALUES (?, ?, 'content_moderation', ?)",
        )
        .bind(&id)
        .bind(&user.id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        id
    };
    sqlx::query(
        "UPDATE capability_claim_codes
         SET consumed_at = ?, consumed_by_assignment_id = ?
         WHERE id = ? AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(&assignment_id)
    .bind(code_id)
    .execute(&mut *tx)
    .await?;
    let authorization_version: i64 = sqlx::query_scalar(
        "UPDATE users SET authorization_version = authorization_version + 1
         WHERE id = ? RETURNING authorization_version",
    )
    .bind(&user.id)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE auth_sessions
         SET authorization_version = ?, passkey_verified_at = ?
         WHERE session_id = ? AND user_id = ?",
    )
    .bind(authorization_version)
    .bind(now)
    .bind(auth.session_id.to_string())
    .bind(&user.id)
    .execute(&mut *tx)
    .await?;
    audit(
        &mut tx,
        "moderator_claimed",
        Some(&assignment_id),
        Some(&user.id),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok((
        [(
            SET_COOKIE,
            expired_cookie(&state.auth.admin_reauth_cookie)
                .map_err(|_| AppError::AuthUnavailable)?,
        )],
        Json(json!({"capability": "content_moderation"})),
    ))
}

pub(crate) async fn reauth_start(
    State(state): State<AppState>,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    axum::Extension(CurrentAuth(auth)): axum::Extension<CurrentAuth>,
) -> AppResult<impl IntoResponse> {
    require_moderator(&state, &user.id).await?;
    rate_limit(&state, &format!("reauth:{}", user.id), 20, 60 * 60).await?;
    let user_id = UserId::new(user.id.clone()).map_err(|_| AppError::AuthUnavailable)?;
    let start = state
        .auth
        .passwordless
        .passkeys
        .start_authentication(user_id, Utc::now())
        .await
        .map_err(crate::passwordless::map_passkey_error)?;
    remember_attempt(
        &state,
        start.ceremony_token.expose_for_cookie(),
        auth.session_id.to_string().as_str(),
        &user.id,
        "admin_reauth",
        None,
    )
    .await?;
    let cookie = issued_cookie(
        &state.auth.admin_reauth_cookie,
        start.ceremony_token.expose_for_cookie(),
        CEREMONY_SECONDS as u64,
    )
    .map_err(|_| AppError::AuthUnavailable)?;
    Ok(([(SET_COOKIE, cookie)], Json(start.challenge)))
}

pub(crate) async fn reauth_finish(
    State(state): State<AppState>,
    jar: CookieJar,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    axum::Extension(CurrentAuth(auth)): axum::Extension<CurrentAuth>,
    Json(credential): Json<PublicKeyCredential>,
) -> AppResult<impl IntoResponse> {
    require_moderator(&state, &user.id).await?;
    let token = reauth_cookie(&state, &jar)?;
    consume_attempt(
        &state,
        token,
        auth.session_id.to_string().as_str(),
        &user.id,
        "admin_reauth",
        None,
    )
    .await?;
    let verified = state
        .auth
        .passwordless
        .passkeys
        .finish_authentication(token, &credential, Utc::now())
        .await
        .map_err(crate::passwordless::map_passkey_error)?;
    if verified.as_str() != user.id {
        return Err(AppError::NotFound);
    }
    sqlx::query(
        "UPDATE auth_sessions SET passkey_verified_at = ?
         WHERE session_id = ? AND user_id = ?",
    )
    .bind(Utc::now().timestamp())
    .bind(auth.session_id.to_string())
    .bind(&user.id)
    .execute(&state.pool)
    .await?;
    Ok((
        [(
            SET_COOKIE,
            expired_cookie(&state.auth.admin_reauth_cookie)
                .map_err(|_| AppError::AuthUnavailable)?,
        )],
        StatusCode::NO_CONTENT,
    ))
}

pub(crate) async fn report_recipe(
    State(state): State<AppState>,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    Path(recipe_id): Path<String>,
    Json(input): Json<ReportInput>,
) -> AppResult<(StatusCode, Json<Value>)> {
    validate_report_reason(&input.reason)?;
    let details = plain_text("details", &input.details, MAX_REPORT_DETAILS, true)?;
    rate_limit(&state, &format!("report:{}", user.id), 20, 60 * 60).await?;
    let now = Utc::now().timestamp();
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let row = sqlx::query(
        "SELECT r.owner_id, r.version, r.title, r.description, r.instructions,
                u.public_handle, i.id AS image_id, i.image_bytes,
                i.media_type AS image_media_type, i.sha256 AS image_sha256
         FROM public_eligible_recipes r
         JOIN users u ON u.id = r.owner_id
         LEFT JOIN recipe_images i ON i.recipe_id = r.id
         WHERE r.id = ?",
    )
    .bind(&recipe_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    if row.get::<String, _>("owner_id") == user.id {
        return Err(AppError::NotFound);
    }
    let recipe_version: i64 = row.get("version");
    if let Some(existing) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM moderation_reports
         WHERE reporter_user_id = ? AND recipe_id = ? AND recipe_version = ? AND reason = ?",
    )
    .bind(&user.id)
    .bind(&recipe_id)
    .bind(recipe_version)
    .bind(&input.reason)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok((
            StatusCode::OK,
            Json(json!({"id": existing, "duplicate": true})),
        ));
    }
    let case_id = if let Some(id) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM moderation_cases
         WHERE recipe_id = ? AND status IN ('open', 'restricted')",
    )
    .bind(&recipe_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        id
    } else {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO moderation_cases (id, recipe_id, created_at, updated_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&recipe_id)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO moderation_evidence_snapshots
             (case_id, recipe_id, owner_user_id, recipe_version, title, description, instructions,
              author_handle, image_id, image_bytes, image_media_type, image_sha256, captured_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&recipe_id)
        .bind(row.get::<String, _>("owner_id"))
        .bind(recipe_version)
        .bind(truncate(&row.get::<String, _>("title"), 120))
        .bind(truncate(&row.get::<String, _>("description"), 2_000))
        .bind(truncate(&row.get::<String, _>("instructions"), 20_000))
        .bind(row.get::<String, _>("public_handle"))
        .bind(row.get::<Option<String>, _>("image_id"))
        .bind(row.get::<Option<Vec<u8>>, _>("image_bytes"))
        .bind(row.get::<Option<String>, _>("image_media_type"))
        .bind(row.get::<Option<Vec<u8>>, _>("image_sha256"))
        .bind(now)
        .execute(&mut *tx)
        .await?;
        id
    };
    let report_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO moderation_reports
         (id, case_id, recipe_id, reporter_user_id, recipe_version, reason, details, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&report_id)
    .bind(&case_id)
    .bind(&recipe_id)
    .bind(&user.id)
    .bind(recipe_version)
    .bind(&input.reason)
    .bind(details)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE moderation_cases SET version = version + 1, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(&case_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": report_id, "duplicate": false})),
    ))
}

pub(crate) async fn list_notifications(
    State(state): State<AppState>,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
) -> AppResult<Json<Vec<Notification>>> {
    let rows = sqlx::query(
        "SELECT id, recipe_id, kind, reason_code, created_at, read_at
         FROM owner_notifications WHERE owner_user_id = ?
         ORDER BY created_at DESC, id DESC LIMIT 100",
    )
    .bind(&user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| Notification {
                id: row.get("id"),
                recipe_id: row.get("recipe_id"),
                kind: row.get("kind"),
                reason_code: row.get("reason_code"),
                created_at: row.get("created_at"),
                read_at: row.get("read_at"),
            })
            .collect(),
    ))
}

pub(crate) async fn read_notification(
    State(state): State<AppState>,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let changed = sqlx::query(
        "UPDATE owner_notifications SET read_at = COALESCE(read_at, ?)
         WHERE id = ? AND owner_user_id = ?",
    )
    .bind(Utc::now().timestamp())
    .bind(id)
    .bind(user.id)
    .execute(&state.pool)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_appeals(
    State(state): State<AppState>,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    Path(recipe_id): Path<String>,
) -> AppResult<Json<Vec<Value>>> {
    let owner: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM recipes WHERE id = ? AND owner_id = ?")
            .bind(&recipe_id)
            .bind(&user.id)
            .fetch_optional(&state.pool)
            .await?;
    if owner.is_none() {
        return Err(AppError::NotFound);
    }
    let rows = sqlx::query(
        "SELECT id, restriction_epoch, status, reason, resolution_reason, version,
                created_at, resolved_at
         FROM moderation_appeals WHERE recipe_id = ? ORDER BY created_at DESC",
    )
    .bind(recipe_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| {
                json!({
                    "id": row.get::<String, _>("id"),
                    "restrictionEpoch": row.get::<i64, _>("restriction_epoch"),
                    "status": row.get::<String, _>("status"),
                    "reason": row.get::<String, _>("reason"),
                    "resolutionReason": row.get::<Option<String>, _>("resolution_reason"),
                    "version": row.get::<i64, _>("version"),
                    "createdAt": row.get::<i64, _>("created_at"),
                    "resolvedAt": row.get::<Option<i64>, _>("resolved_at")
                })
            })
            .collect(),
    ))
}

pub(crate) async fn create_appeal(
    State(state): State<AppState>,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    Path(recipe_id): Path<String>,
    Json(input): Json<AppealInput>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let reason = plain_text("reason", &input.reason, MAX_APPEAL_REASON, false)?;
    rate_limit(&state, &format!("appeal:{}", user.id), 5, 60 * 60).await?;
    let now = Utc::now().timestamp();
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let row = sqlx::query(
        "SELECT restriction_epoch FROM recipes
         WHERE id = ? AND owner_id = ? AND moderation_state = 'restricted'",
    )
    .bind(&recipe_id)
    .bind(&user.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let case_id: String = sqlx::query_scalar(
        "SELECT id FROM moderation_cases
         WHERE recipe_id = ? AND status = 'restricted'",
    )
    .bind(&recipe_id)
    .fetch_one(&mut *tx)
    .await?;
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO moderation_appeals
         (id, case_id, recipe_id, owner_user_id, restriction_epoch, reason, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&case_id)
    .bind(&recipe_id)
    .bind(&user.id)
    .bind(row.get::<i64, _>("restriction_epoch"))
    .bind(reason)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(map_unique_conflict)?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id, "version": 1}))))
}

pub(crate) async fn queue(
    State(state): State<AppState>,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    Query(query): Query<CursorQuery>,
) -> AppResult<Json<QueuePage>> {
    require_moderator(&state, &user.id).await?;
    rate_limit(&state, &format!("queue:{}", user.id), 240, 60 * 60).await?;
    let limit = query.limit.unwrap_or(30);
    if !(1..=100).contains(&limit) {
        return Err(AppError::Validation(
            "limit must be between 1 and 100".into(),
        ));
    }
    let cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let rows = sqlx::query(
        "SELECT r.id, r.title, u.public_handle, r.moderation_review_state,
                r.moderation_state, r.version, r.moderation_version, r.updated_at,
                c.id AS case_id, c.version AS case_version,
                (SELECT COUNT(*) FROM moderation_reports mr WHERE mr.case_id = c.id) AS report_count,
                EXISTS(SELECT 1 FROM moderation_appeals a
                       WHERE a.recipe_id = r.id AND a.status = 'active') AS active_appeal
         FROM recipes r
         JOIN users u ON u.id = r.owner_id
         LEFT JOIN moderation_cases c ON c.recipe_id = r.id
              AND c.status IN ('open', 'restricted')
         WHERE r.owner_id <> ?
           AND ((r.publication_intent = 'public' AND r.moderation_review_state = 'unreviewed')
                OR c.id IS NOT NULL
                OR EXISTS(SELECT 1 FROM moderation_appeals a
                          WHERE a.recipe_id = r.id AND a.status = 'active'))
           AND (? IS NULL OR r.updated_at < ? OR (r.updated_at = ? AND r.id < ?))
         ORDER BY r.updated_at DESC, r.id DESC
         LIMIT ?",
    )
    .bind(&user.id)
    .bind(cursor.as_ref().map(|value| value.0.as_str()))
    .bind(cursor.as_ref().map(|value| value.0.as_str()))
    .bind(cursor.as_ref().map(|value| value.0.as_str()))
    .bind(cursor.as_ref().map(|value| value.1.as_str()))
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;
    let has_more = rows.len() as i64 > limit;
    let items = rows
        .into_iter()
        .take(limit as usize)
        .map(|row| QueueItem {
            recipe_id: row.get("id"),
            title: row.get("title"),
            author_handle: row.get("public_handle"),
            review_state: row.get("moderation_review_state"),
            moderation_state: row.get("moderation_state"),
            recipe_version: row.get("version"),
            moderation_version: row.get("moderation_version"),
            case_id: row.get("case_id"),
            case_version: row.get("case_version"),
            report_count: row.get("report_count"),
            active_appeal: row.get::<i64, _>("active_appeal") != 0,
            updated_at: row.get("updated_at"),
        })
        .collect::<Vec<_>>();
    let next_cursor = has_more
        .then(|| {
            items
                .last()
                .map(|item| encode_cursor(&item.updated_at, &item.recipe_id))
        })
        .flatten();
    Ok(Json(QueuePage { items, next_cursor }))
}

pub(crate) async fn case_detail(
    State(state): State<AppState>,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    axum::Extension(CurrentAuth(auth)): axum::Extension<CurrentAuth>,
    Path(case_id): Path<String>,
) -> AppResult<Json<Value>> {
    require_fresh_moderator(&state, &user.id, &auth).await?;
    rate_limit(&state, &format!("case:{}", user.id), 120, 60 * 60).await?;
    let row = sqlx::query(
        "SELECT c.id, c.recipe_id, c.status, c.version, c.created_at, c.updated_at,
                e.recipe_version, e.title, e.description, e.instructions,
                e.author_handle, e.image_id, e.owner_user_id,
                r.version AS current_recipe_version,
                r.moderation_version AS current_moderation_version
         FROM moderation_cases c
         JOIN moderation_evidence_snapshots e ON e.case_id = c.id
         LEFT JOIN recipes r ON r.id = c.recipe_id
         WHERE c.id = ?",
    )
    .bind(&case_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    if row.get::<String, _>("owner_user_id") == user.id {
        return Err(AppError::NotFound);
    }
    let reports = sqlx::query(
        "SELECT reason, details, recipe_version, created_at
         FROM moderation_reports WHERE case_id = ? ORDER BY created_at",
    )
    .bind(&case_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|report| {
        json!({
            "reason": report.get::<String, _>("reason"),
            "details": report.get::<String, _>("details"),
            "recipeVersion": report.get::<i64, _>("recipe_version"),
            "createdAt": report.get::<i64, _>("created_at")
        })
    })
    .collect::<Vec<_>>();
    let appeal = sqlx::query(
        "SELECT id, reason, version, created_at
         FROM moderation_appeals
         WHERE case_id = ? AND status = 'active'",
    )
    .bind(&case_id)
    .fetch_optional(&state.pool)
    .await?
    .map(|appeal| {
        json!({
            "id": appeal.get::<String, _>("id"),
            "reason": appeal.get::<String, _>("reason"),
            "version": appeal.get::<i64, _>("version"),
            "createdAt": appeal.get::<i64, _>("created_at")
        })
    });
    Ok(Json(json!({
        "id": row.get::<String, _>("id"),
        "recipeId": row.get::<String, _>("recipe_id"),
        "status": row.get::<String, _>("status"),
        "version": row.get::<i64, _>("version"),
        "createdAt": row.get::<i64, _>("created_at"),
        "updatedAt": row.get::<i64, _>("updated_at"),
        "recipeVersion": row.get::<Option<i64>, _>("current_recipe_version"),
        "moderationVersion": row.get::<Option<i64>, _>("current_moderation_version"),
        "evidence": {
            "recipeVersion": row.get::<i64, _>("recipe_version"),
            "title": row.get::<String, _>("title"),
            "description": row.get::<String, _>("description"),
            "instructions": row.get::<String, _>("instructions"),
            "authorHandle": row.get::<String, _>("author_handle"),
            "imageId": row.get::<Option<String>, _>("image_id")
        },
        "reports": reports,
        "appeal": appeal
    })))
}

pub(crate) async fn case_evidence_image(
    State(state): State<AppState>,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    axum::Extension(CurrentAuth(auth)): axum::Extension<CurrentAuth>,
    Path(case_id): Path<String>,
) -> AppResult<Response> {
    require_fresh_moderator(&state, &user.id, &auth).await?;
    rate_limit(&state, &format!("case-image:{}", user.id), 120, 60 * 60).await?;
    let row = sqlx::query(
        "SELECT e.owner_user_id, e.image_bytes, e.image_media_type
         FROM moderation_cases c
         JOIN moderation_evidence_snapshots e ON e.case_id = c.id
         WHERE c.id = ? AND e.image_bytes IS NOT NULL",
    )
    .bind(&case_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    if row.get::<String, _>("owner_user_id") == user.id {
        return Err(AppError::NotFound);
    }
    let bytes: Vec<u8> = row.get("image_bytes");
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, row.get::<String, _>("image_media_type"))
        .header(CONTENT_LENGTH, bytes.len().to_string())
        .header(CACHE_CONTROL, "private, no-store")
        .header(
            "x-content-type-options",
            HeaderValue::from_static("nosniff"),
        )
        .body(Body::from(bytes))
        .map_err(|_| AppError::Data("invalid evidence image response".into()))
}

pub(crate) async fn mark_reviewed(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    Path(recipe_id): Path<String>,
    Json(input): Json<AdminMutationInput>,
) -> AppResult<Json<Value>> {
    let assignment = require_moderator(&state, &user.id).await?;
    rate_limit(&state, &format!("mutation:{assignment}"), 120, 60 * 60).await?;
    same_origin(&state, &headers)?;
    validate_review_reason(&input.reason_code)?;
    let key = idempotency_key(&headers)?;
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    ensure_not_owned_recipe(&mut tx, &recipe_id, &user.id).await?;
    if let Some(value) = replay(
        &mut tx,
        &assignment,
        "review",
        MutationTarget::recipe(&recipe_id),
        &key,
        &input,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(Json(value));
    }
    let active_case = sqlx::query(
        "SELECT c.id, c.version, e.recipe_version,
                r.version AS current_recipe_version,
                r.moderation_version AS current_moderation_version
         FROM moderation_cases c
         JOIN moderation_evidence_snapshots e ON e.case_id = c.id
         JOIN recipes r ON r.id = c.recipe_id
         WHERE c.recipe_id = ? AND c.status = 'open'",
    )
    .bind(&recipe_id)
    .fetch_optional(&mut *tx)
    .await?;
    match (active_case.as_ref(), input.expected_case_version) {
        (Some(case), Some(expected))
            if case.get::<i64, _>("version") == expected
                && case.get::<i64, _>("recipe_version")
                    == case.get::<i64, _>("current_recipe_version") => {}
        (Some(case), _) => {
            return Err(AppError::ModerationConflict {
                message: "active moderation case requires current-version evidence".into(),
                recipe_version: Some(case.get("current_recipe_version")),
                moderation_version: Some(case.get("current_moderation_version")),
                case_version: Some(case.get("version")),
                appeal_version: None,
            });
        }
        (None, None) => {}
        (None, Some(_)) => {
            return Err(AppError::ModerationConflict {
                message: "moderation case no longer active".into(),
                recipe_version: Some(input.expected_recipe_version),
                moderation_version: Some(input.expected_moderation_version),
                case_version: None,
                appeal_version: None,
            });
        }
    }
    let changed = sqlx::query(
        "UPDATE recipes
         SET moderation_review_state = 'reviewed',
             moderation_version = moderation_version + 1
         WHERE id = ? AND owner_id <> ? AND version = ? AND moderation_version = ?
           AND moderation_state = 'clear'
         RETURNING version, moderation_version",
    )
    .bind(&recipe_id)
    .bind(&user.id)
    .bind(input.expected_recipe_version)
    .bind(input.expected_moderation_version)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(changed) = changed else {
        return Err(recipe_version_conflict(
            &mut tx,
            &recipe_id,
            "recipe or moderation version changed",
        )
        .await?);
    };
    let (case_id, case_version) = if let Some(case) = active_case {
        let case_id: String = case.get("id");
        let case_version: i64 = sqlx::query_scalar(
            "UPDATE moderation_cases
             SET status = 'resolved', version = version + 1,
                 updated_at = ?, resolved_at = ?
             WHERE id = ? AND status = 'open' AND version = ?
             RETURNING version",
        )
        .bind(Utc::now().timestamp())
        .bind(Utc::now().timestamp())
        .bind(&case_id)
        .bind(input.expected_case_version.unwrap_or_default())
        .fetch_one(&mut *tx)
        .await?;
        (Some(case_id), Some(case_version))
    } else {
        (None, None)
    };
    let value = json!({
        "recipeVersion": changed.get::<i64, _>("version"),
        "moderationVersion": changed.get::<i64, _>("moderation_version"),
        "caseId": case_id,
        "caseVersion": case_version
    });
    record_replay(
        &mut tx,
        &assignment,
        "review",
        MutationTarget::recipe(&recipe_id),
        &key,
        &input,
        &value,
    )
    .await?;
    audit(
        &mut tx,
        "recipe_reviewed",
        Some(&assignment),
        Some(&user.id),
        Some(&recipe_id),
        value["caseId"].as_str(),
        None,
        Some(&input.reason_code),
        Some(input.expected_recipe_version),
        Some(value["moderationVersion"].as_i64().unwrap_or_default()),
        value["caseVersion"].as_i64(),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(value))
}

pub(crate) async fn restrict(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    axum::Extension(CurrentAuth(auth)): axum::Extension<CurrentAuth>,
    Path(recipe_id): Path<String>,
    Json(input): Json<AdminMutationInput>,
) -> AppResult<Json<Value>> {
    let assignment = require_fresh_moderator(&state, &user.id, &auth).await?;
    rate_limit(&state, &format!("mutation:{assignment}"), 120, 60 * 60).await?;
    same_origin(&state, &headers)?;
    validate_action_reason(&input.reason_code)?;
    let key = idempotency_key(&headers)?;
    let now = Utc::now().timestamp();
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    ensure_not_owned_recipe(&mut tx, &recipe_id, &user.id).await?;
    if let Some(value) = replay(
        &mut tx,
        &assignment,
        "restrict",
        MutationTarget::recipe(&recipe_id),
        &key,
        &input,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(Json(value));
    }
    let changed = sqlx::query(
        "UPDATE recipes
         SET moderation_state = 'restricted', visibility = 'private',
             moderation_review_state = 'reviewed',
             moderation_version = moderation_version + 1,
             restriction_epoch = restriction_epoch + 1
         WHERE id = ? AND owner_id <> ? AND version = ? AND moderation_version = ?
           AND moderation_state = 'clear'
         RETURNING owner_id, version, moderation_version, restriction_epoch",
    )
    .bind(&recipe_id)
    .bind(&user.id)
    .bind(input.expected_recipe_version)
    .bind(input.expected_moderation_version)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(changed) = changed else {
        return Err(recipe_version_conflict(
            &mut tx,
            &recipe_id,
            "recipe or moderation version changed",
        )
        .await?);
    };
    let (case_id, case_created) = active_or_new_case(&mut tx, &recipe_id, now).await?;
    let current_case = sqlx::query(
        "SELECT c.version, e.recipe_version
         FROM moderation_cases c
         JOIN moderation_evidence_snapshots e ON e.case_id = c.id
         WHERE c.id = ?",
    )
    .bind(&case_id)
    .fetch_one(&mut *tx)
    .await?;
    let current_case_version: i64 = current_case.get("version");
    let case_version_matches = if case_created {
        input.expected_case_version.is_none()
    } else {
        input.expected_case_version == Some(current_case_version)
    };
    if !case_version_matches
        || current_case.get::<i64, _>("recipe_version") != changed.get::<i64, _>("version")
    {
        return Err(AppError::ModerationConflict {
            message: if case_created {
                "moderation case did not previously exist".into()
            } else {
                "active moderation case requires current-version evidence".into()
            },
            recipe_version: Some(input.expected_recipe_version),
            moderation_version: Some(input.expected_moderation_version),
            case_version: Some(current_case_version),
            appeal_version: None,
        });
    }
    let case_version: i64 = sqlx::query_scalar(
        "UPDATE moderation_cases
         SET status = 'restricted', version = version + 1, updated_at = ?
         WHERE id = ? RETURNING version",
    )
    .bind(now)
    .bind(&case_id)
    .fetch_one(&mut *tx)
    .await?;
    rotate_image(&mut tx, &recipe_id, now).await?;
    sqlx::query(
        "INSERT INTO owner_notifications
         (id, owner_user_id, recipe_id, kind, reason_code, created_at)
         VALUES (?, ?, ?, 'recipe_restricted', ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(changed.get::<String, _>("owner_id"))
    .bind(&recipe_id)
    .bind(&input.reason_code)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE release_compatibility
         SET minimum_release = COALESCE(minimum_release, 'v0.6.0'),
             first_restriction_at = COALESCE(first_restriction_at, ?)
         WHERE id = 1",
    )
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let value = json!({
        "recipeVersion": changed.get::<i64, _>("version"),
        "moderationVersion": changed.get::<i64, _>("moderation_version"),
        "caseId": case_id,
        "caseVersion": case_version,
        "restrictionEpoch": changed.get::<i64, _>("restriction_epoch")
    });
    record_replay(
        &mut tx,
        &assignment,
        "restrict",
        MutationTarget::recipe(&recipe_id),
        &key,
        &input,
        &value,
    )
    .await?;
    audit(
        &mut tx,
        "recipe_restricted",
        Some(&assignment),
        Some(&user.id),
        Some(&recipe_id),
        Some(value["caseId"].as_str().unwrap_or_default()),
        None,
        Some(&input.reason_code),
        Some(input.expected_recipe_version),
        Some(value["moderationVersion"].as_i64().unwrap_or_default()),
        Some(case_version),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(value))
}

pub(crate) async fn reinstate(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    axum::Extension(CurrentAuth(auth)): axum::Extension<CurrentAuth>,
    Path(recipe_id): Path<String>,
    Json(input): Json<AdminMutationInput>,
) -> AppResult<Json<Value>> {
    let assignment = require_fresh_moderator(&state, &user.id, &auth).await?;
    rate_limit(&state, &format!("mutation:{assignment}"), 120, 60 * 60).await?;
    same_origin(&state, &headers)?;
    validate_action_reason(&input.reason_code)?;
    let key = idempotency_key(&headers)?;
    let now = Utc::now().timestamp();
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    ensure_not_owned_recipe(&mut tx, &recipe_id, &user.id).await?;
    if let Some(value) = replay(
        &mut tx,
        &assignment,
        "reinstate",
        MutationTarget::recipe(&recipe_id),
        &key,
        &input,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(Json(value));
    }
    let moderation_case = sqlx::query(
        "SELECT c.id, c.version, e.recipe_version AS evidence_recipe_version,
                r.version AS current_recipe_version, r.moderation_version
         FROM moderation_cases c
         JOIN moderation_evidence_snapshots e ON e.case_id = c.id
         JOIN recipes r ON r.id = c.recipe_id
         WHERE c.recipe_id = ? AND c.status = 'restricted'",
    )
    .bind(&recipe_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let current_case_version: i64 = moderation_case.get("version");
    let current_recipe_version: i64 = moderation_case.get("current_recipe_version");
    let current_moderation_version: i64 = moderation_case.get("moderation_version");
    if input.expected_case_version != Some(current_case_version)
        || moderation_case.get::<i64, _>("evidence_recipe_version") != current_recipe_version
    {
        return Err(AppError::ModerationConflict {
            message: "current recipe version has not been reviewed in this case".into(),
            recipe_version: Some(current_recipe_version),
            moderation_version: Some(current_moderation_version),
            case_version: Some(current_case_version),
            appeal_version: None,
        });
    }
    let changed = sqlx::query(
        "UPDATE recipes
         SET moderation_state = 'clear',
             visibility = CASE WHEN publication_intent = 'public' THEN 'public' ELSE 'private' END,
             moderation_review_state = 'reviewed',
             moderation_version = moderation_version + 1
         WHERE id = ? AND owner_id <> ? AND version = ? AND moderation_version = ?
           AND moderation_state = 'restricted'
         RETURNING owner_id, version, moderation_version",
    )
    .bind(&recipe_id)
    .bind(&user.id)
    .bind(input.expected_recipe_version)
    .bind(input.expected_moderation_version)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(changed) = changed else {
        return Err(recipe_version_conflict(
            &mut tx,
            &recipe_id,
            "recipe or moderation version changed",
        )
        .await?);
    };
    let case_id: String = moderation_case.get("id");
    let case_version: Option<i64> = sqlx::query_scalar(
        "UPDATE moderation_cases
         SET status = 'reinstated', version = version + 1, updated_at = ?, resolved_at = ?
         WHERE id = ? AND version = ? RETURNING version",
    )
    .bind(now)
    .bind(now)
    .bind(&case_id)
    .bind(current_case_version)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(case_version) = case_version else {
        let current = sqlx::query_scalar("SELECT version FROM moderation_cases WHERE id = ?")
            .bind(&case_id)
            .fetch_optional(&mut *tx)
            .await?;
        return Err(AppError::ModerationConflict {
            message: "moderation case version changed".into(),
            recipe_version: Some(input.expected_recipe_version),
            moderation_version: Some(input.expected_moderation_version),
            case_version: current,
            appeal_version: None,
        });
    };
    sqlx::query(
        "UPDATE moderation_appeals
         SET status = 'reinstated', resolution_reason = ?, version = version + 1, resolved_at = ?
         WHERE recipe_id = ? AND status = 'active'",
    )
    .bind(&input.reason_code)
    .bind(now)
    .bind(&recipe_id)
    .execute(&mut *tx)
    .await?;
    rotate_image(&mut tx, &recipe_id, now).await?;
    sqlx::query(
        "INSERT INTO owner_notifications
         (id, owner_user_id, recipe_id, kind, reason_code, created_at)
         VALUES (?, ?, ?, 'recipe_reinstated', ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(changed.get::<String, _>("owner_id"))
    .bind(&recipe_id)
    .bind(&input.reason_code)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let value = json!({
        "recipeVersion": changed.get::<i64, _>("version"),
        "moderationVersion": changed.get::<i64, _>("moderation_version"),
        "caseId": case_id,
        "caseVersion": case_version
    });
    record_replay(
        &mut tx,
        &assignment,
        "reinstate",
        MutationTarget::recipe(&recipe_id),
        &key,
        &input,
        &value,
    )
    .await?;
    audit(
        &mut tx,
        "recipe_reinstated",
        Some(&assignment),
        Some(&user.id),
        Some(&recipe_id),
        Some(value["caseId"].as_str().unwrap_or_default()),
        None,
        Some(&input.reason_code),
        Some(input.expected_recipe_version),
        Some(value["moderationVersion"].as_i64().unwrap_or_default()),
        Some(case_version),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(value))
}

pub(crate) async fn resolve_appeal(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    axum::Extension(CurrentAuth(auth)): axum::Extension<CurrentAuth>,
    Path(appeal_id): Path<String>,
    Json(input): Json<AppealResolutionInput>,
) -> AppResult<Json<Value>> {
    if input.resolution == "reinstate" {
        let assignment = require_fresh_moderator(&state, &user.id, &auth).await?;
        rate_limit(&state, &format!("mutation:{assignment}"), 120, 60 * 60).await?;
        same_origin(&state, &headers)?;
        validate_action_reason(&input.reason_code)?;
        let key = idempotency_key(&headers)?;
        let now = Utc::now().timestamp();
        let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(value) = replay(
            &mut tx,
            &assignment,
            "appeal_reinstate",
            MutationTarget::appeal(&appeal_id),
            &key,
            &input,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(Json(value));
        }
        let appeal = sqlx::query(
            "SELECT a.recipe_id, a.case_id, a.owner_user_id
             FROM moderation_appeals a
             JOIN recipes r ON r.id = a.recipe_id
             JOIN moderation_evidence_snapshots e ON e.case_id = a.case_id
             WHERE a.id = ? AND a.status = 'active' AND a.version = ?
               AND r.version = ? AND r.moderation_version = ?
               AND r.moderation_state = 'restricted'
               AND e.recipe_version = r.version",
        )
        .bind(&appeal_id)
        .bind(input.expected_appeal_version)
        .bind(input.expected_recipe_version)
        .bind(input.expected_moderation_version)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(appeal) = appeal else {
            return Err(appeal_version_conflict(
                &mut tx,
                &appeal_id,
                "appeal or recipe version changed",
            )
            .await?);
        };
        if appeal.get::<String, _>("owner_user_id") == user.id {
            return Err(AppError::NotFound);
        }
        let recipe_id: String = appeal.get("recipe_id");
        let case_id: String = appeal.get("case_id");
        let changed = sqlx::query(
            "UPDATE recipes
             SET moderation_state = 'clear',
                 visibility = CASE WHEN publication_intent = 'public' THEN 'public' ELSE 'private' END,
                 moderation_review_state = 'reviewed',
                 moderation_version = moderation_version + 1
             WHERE id = ? AND version = ? AND moderation_version = ?
             RETURNING owner_id, version, moderation_version",
        )
        .bind(&recipe_id)
        .bind(input.expected_recipe_version)
        .bind(input.expected_moderation_version)
        .fetch_one(&mut *tx)
        .await?;
        let case_version: Option<i64> = sqlx::query_scalar(
            "UPDATE moderation_cases
             SET status = 'reinstated', version = version + 1, updated_at = ?, resolved_at = ?
             WHERE id = ? AND version = ? RETURNING version",
        )
        .bind(now)
        .bind(now)
        .bind(&case_id)
        .bind(input.expected_case_version)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(case_version) = case_version else {
            return Err(appeal_version_conflict(
                &mut tx,
                &appeal_id,
                "moderation case version changed",
            )
            .await?);
        };
        let appeal_version: Option<i64> = sqlx::query_scalar(
            "UPDATE moderation_appeals
             SET status = 'reinstated', resolution_reason = ?,
                 version = version + 1, resolved_at = ?
             WHERE id = ? AND status = 'active' AND version = ?
             RETURNING version",
        )
        .bind(&input.reason_code)
        .bind(now)
        .bind(&appeal_id)
        .bind(input.expected_appeal_version)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(appeal_version) = appeal_version else {
            return Err(
                appeal_version_conflict(&mut tx, &appeal_id, "appeal version changed").await?,
            );
        };
        rotate_image(&mut tx, &recipe_id, now).await?;
        sqlx::query(
            "INSERT INTO owner_notifications
             (id, owner_user_id, recipe_id, kind, reason_code, created_at)
             VALUES (?, ?, ?, 'recipe_reinstated', ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(changed.get::<String, _>("owner_id"))
        .bind(&recipe_id)
        .bind(&input.reason_code)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let value = json!({
            "recipeVersion": changed.get::<i64, _>("version"),
            "moderationVersion": changed.get::<i64, _>("moderation_version"),
            "caseId": case_id,
            "caseVersion": case_version,
            "appealVersion": appeal_version
        });
        record_replay(
            &mut tx,
            &assignment,
            "appeal_reinstate",
            MutationTarget::appeal(&appeal_id),
            &key,
            &input,
            &value,
        )
        .await?;
        audit(
            &mut tx,
            "appeal_reinstated",
            Some(&assignment),
            Some(&user.id),
            Some(&recipe_id),
            Some(value["caseId"].as_str().unwrap_or_default()),
            Some(&appeal_id),
            Some(&input.reason_code),
            Some(input.expected_recipe_version),
            Some(value["moderationVersion"].as_i64().unwrap_or_default()),
            Some(case_version),
        )
        .await?;
        tx.commit().await?;
        return Ok(Json(value));
    }
    if input.resolution != "uphold" {
        return Err(AppError::Validation(
            "resolution must be reinstate or uphold".into(),
        ));
    }
    let assignment = require_fresh_moderator(&state, &user.id, &auth).await?;
    rate_limit(&state, &format!("mutation:{assignment}"), 120, 60 * 60).await?;
    same_origin(&state, &headers)?;
    validate_action_reason(&input.reason_code)?;
    let key = idempotency_key(&headers)?;
    let now = Utc::now().timestamp();
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    if let Some(value) = replay(
        &mut tx,
        &assignment,
        "appeal_uphold",
        MutationTarget::appeal(&appeal_id),
        &key,
        &input,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(Json(value));
    }
    let appeal = sqlx::query(
        "SELECT a.recipe_id, a.case_id, a.owner_user_id
         FROM moderation_appeals a JOIN recipes r ON r.id = a.recipe_id
         WHERE a.id = ? AND a.status = 'active' AND a.version = ?
           AND r.version = ? AND r.moderation_version = ?",
    )
    .bind(&appeal_id)
    .bind(input.expected_appeal_version)
    .bind(input.expected_recipe_version)
    .bind(input.expected_moderation_version)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(appeal) = appeal else {
        return Err(appeal_version_conflict(
            &mut tx,
            &appeal_id,
            "appeal or recipe version changed",
        )
        .await?);
    };
    if appeal.get::<String, _>("owner_user_id") == user.id {
        return Err(AppError::NotFound);
    }
    let case_id: String = appeal.get("case_id");
    let case_version: Option<i64> = sqlx::query_scalar(
        "UPDATE moderation_cases
         SET status = 'upheld', version = version + 1, updated_at = ?, resolved_at = ?
         WHERE id = ? AND version = ? RETURNING version",
    )
    .bind(now)
    .bind(now)
    .bind(&case_id)
    .bind(input.expected_case_version)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(case_version) = case_version else {
        return Err(appeal_version_conflict(
            &mut tx,
            &appeal_id,
            "moderation case version changed",
        )
        .await?);
    };
    let appeal_version: i64 = sqlx::query_scalar(
        "UPDATE moderation_appeals
         SET status = 'upheld', resolution_reason = ?, version = version + 1, resolved_at = ?
         WHERE id = ? AND version = ? RETURNING version",
    )
    .bind(&input.reason_code)
    .bind(now)
    .bind(&appeal_id)
    .bind(input.expected_appeal_version)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO owner_notifications
         (id, owner_user_id, recipe_id, kind, reason_code, created_at)
         VALUES (?, ?, ?, 'appeal_upheld', ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(appeal.get::<String, _>("owner_user_id"))
    .bind(appeal.get::<String, _>("recipe_id"))
    .bind(&input.reason_code)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let value = json!({"appealVersion": appeal_version, "caseVersion": case_version});
    record_replay(
        &mut tx,
        &assignment,
        "appeal_uphold",
        MutationTarget::appeal(&appeal_id),
        &key,
        &input,
        &value,
    )
    .await?;
    audit(
        &mut tx,
        "appeal_upheld",
        Some(&assignment),
        Some(&user.id),
        Some(appeal.get::<String, _>("recipe_id").as_str()),
        Some(&case_id),
        Some(&appeal_id),
        Some(&input.reason_code),
        Some(input.expected_recipe_version),
        Some(input.expected_moderation_version),
        Some(case_version),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(value))
}

pub(crate) async fn audit_events(
    State(state): State<AppState>,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    Query(query): Query<CursorQuery>,
) -> AppResult<Json<Value>> {
    require_moderator(&state, &user.id).await?;
    rate_limit(&state, &format!("audit:{}", user.id), 240, 60 * 60).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let cursor = query
        .cursor
        .as_deref()
        .map(decode_audit_cursor)
        .transpose()?;
    let rows = sqlx::query(
        "SELECT id, event_type, assignment_id, recipe_id, case_id, appeal_id, reason_code,
                recipe_version, moderation_version, case_version, created_at
         FROM moderation_audit_events
         WHERE (? IS NULL OR created_at < ? OR (created_at = ? AND id < ?))
         ORDER BY created_at DESC, id DESC LIMIT ?",
    )
    .bind(cursor.as_ref().map(|value| value.0))
    .bind(cursor.as_ref().map(|value| value.0))
    .bind(cursor.as_ref().map(|value| value.0))
    .bind(cursor.as_ref().map(|value| value.1.as_str()))
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;
    let has_more = rows.len() as i64 > limit;
    let items = rows
        .into_iter()
        .take(limit as usize)
        .map(audit_json)
        .collect::<Vec<_>>();
    let next = has_more
        .then(|| {
            items.last().and_then(|value| {
                Some(encode_audit_cursor(
                    value["createdAt"].as_i64()?,
                    value["id"].as_str()?,
                ))
            })
        })
        .flatten();
    Ok(Json(json!({"items": items, "nextCursor": next})))
}

pub(crate) async fn audit_detail(
    State(state): State<AppState>,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    axum::Extension(CurrentAuth(auth)): axum::Extension<CurrentAuth>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    require_fresh_moderator(&state, &user.id, &auth).await?;
    rate_limit(&state, &format!("audit:{}", user.id), 120, 60 * 60).await?;
    let row = sqlx::query(
        "SELECT id, event_type, assignment_id, recipe_id, case_id, appeal_id, reason_code,
                recipe_version, moderation_version, case_version, created_at
         FROM moderation_audit_events WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(audit_json(row)))
}

pub(crate) async fn audit_export(
    State(state): State<AppState>,
    axum::Extension(CurrentUser(user)): axum::Extension<CurrentUser>,
    axum::Extension(CurrentAuth(auth)): axum::Extension<CurrentAuth>,
) -> AppResult<Response> {
    require_fresh_moderator(&state, &user.id, &auth).await?;
    rate_limit(&state, &format!("audit-export:{}", user.id), 10, 60 * 60).await?;
    let rows = sqlx::query(
        "SELECT id, event_type, assignment_id, recipe_id, case_id, appeal_id, reason_code,
                recipe_version, moderation_version, case_version, created_at
         FROM moderation_audit_events ORDER BY created_at DESC, id DESC LIMIT 10000",
    )
    .fetch_all(&state.pool)
    .await?;
    let body = serde_json::to_vec(&rows.into_iter().map(audit_json).collect::<Vec<_>>())
        .map_err(|error| AppError::Data(error.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header(
            "content-disposition",
            "attachment; filename=\"moderation-audit.json\"",
        )
        .body(axum::body::Body::from(body))
        .map_err(|_| AppError::Data("invalid audit export response".into()))
}

async fn remember_attempt(
    state: &AppState,
    token: &str,
    session_id: &str,
    user_id: &str,
    purpose: &str,
    claim_code_digest: Option<&[u8; 32]>,
) -> AppResult<()> {
    let digest = Sha256::digest(token.as_bytes());
    sqlx::query(
        "INSERT INTO admin_passkey_attempts
         (token_digest, session_id, user_id, purpose, claim_code_digest, expires_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(digest.as_slice())
    .bind(session_id)
    .bind(user_id)
    .bind(purpose)
    .bind(claim_code_digest.map(|value| value.as_slice()))
    .bind(Utc::now().timestamp() + CEREMONY_SECONDS)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn consume_attempt(
    state: &AppState,
    token: &str,
    session_id: &str,
    user_id: &str,
    purpose: &str,
    claim_code_digest: Option<&[u8; 32]>,
) -> AppResult<()> {
    let digest = Sha256::digest(token.as_bytes());
    let changed = sqlx::query(
        "DELETE FROM admin_passkey_attempts
         WHERE token_digest = ? AND session_id = ? AND user_id = ? AND purpose = ?
           AND expires_at > ?
           AND ((claim_code_digest IS NULL AND ? IS NULL) OR claim_code_digest = ?)",
    )
    .bind(digest.as_slice())
    .bind(session_id)
    .bind(user_id)
    .bind(purpose)
    .bind(Utc::now().timestamp())
    .bind(claim_code_digest.map(|value| value.as_slice()))
    .bind(claim_code_digest.map(|value| value.as_slice()))
    .execute(&state.pool)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

fn reauth_cookie<'a>(state: &AppState, jar: &'a CookieJar) -> AppResult<&'a str> {
    jar.get(state.auth.admin_reauth_cookie.name())
        .map(|cookie| cookie.value())
        .ok_or(AppError::NotFound)
}

async fn require_moderator(state: &AppState, user_id: &str) -> AppResult<String> {
    sqlx::query_scalar(
        "SELECT id FROM capability_assignments
         WHERE user_id = ? AND capability = 'content_moderation' AND revoked_at IS NULL",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)
}

async fn ensure_not_owned_recipe(
    tx: &mut Transaction<'_, Sqlite>,
    recipe_id: &str,
    moderator_user_id: &str,
) -> AppResult<()> {
    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM recipes WHERE id = ?")
        .bind(recipe_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(AppError::NotFound)?;
    if owner_id == moderator_user_id {
        return Err(AppError::NotFound);
    }
    Ok(())
}

async fn recipe_version_conflict(
    tx: &mut Transaction<'_, Sqlite>,
    recipe_id: &str,
    message: &str,
) -> AppResult<AppError> {
    let versions = sqlx::query("SELECT version, moderation_version FROM recipes WHERE id = ?")
        .bind(recipe_id)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(AppError::ModerationConflict {
        message: message.into(),
        recipe_version: versions.as_ref().map(|row| row.get("version")),
        moderation_version: versions.as_ref().map(|row| row.get("moderation_version")),
        case_version: None,
        appeal_version: None,
    })
}

async fn appeal_version_conflict(
    tx: &mut Transaction<'_, Sqlite>,
    appeal_id: &str,
    message: &str,
) -> AppResult<AppError> {
    let versions = sqlx::query(
        "SELECT r.version AS recipe_version, r.moderation_version,
                c.version AS case_version, a.version AS appeal_version
         FROM moderation_appeals a
         LEFT JOIN recipes r ON r.id = a.recipe_id
         LEFT JOIN moderation_cases c ON c.id = a.case_id
         WHERE a.id = ?",
    )
    .bind(appeal_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(AppError::ModerationConflict {
        message: message.into(),
        recipe_version: versions
            .as_ref()
            .and_then(|row| row.get::<Option<i64>, _>("recipe_version")),
        moderation_version: versions
            .as_ref()
            .and_then(|row| row.get::<Option<i64>, _>("moderation_version")),
        case_version: versions
            .as_ref()
            .and_then(|row| row.get::<Option<i64>, _>("case_version")),
        appeal_version: versions
            .as_ref()
            .map(|row| row.get::<i64, _>("appeal_version")),
    })
}

async fn require_fresh_moderator(
    state: &AppState,
    user_id: &str,
    auth: &rust_web_auth_core::AuthenticatedContext,
) -> AppResult<String> {
    let assignment = require_moderator(state, user_id).await?;
    let verified: Option<i64> = sqlx::query_scalar(
        "SELECT passkey_verified_at FROM auth_sessions
         WHERE session_id = ? AND user_id = ?",
    )
    .bind(auth.session_id.to_string())
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .flatten();
    if verified.is_none_or(|value| value < Utc::now().timestamp() - REAUTH_SECONDS) {
        return Err(AppError::AdminReauthRequired);
    }
    Ok(assignment)
}

fn same_origin(state: &AppState, headers: &HeaderMap) -> AppResult<()> {
    let origin = headers
        .get_all("origin")
        .iter()
        .map(|value| value.to_str().ok())
        .collect::<Option<Vec<_>>>()
        .ok_or(AppError::Forbidden)?;
    if origin.len() != 1 || origin[0] != state.config.public_origin {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

fn claim_digest(code: &str) -> AppResult<[u8; 32]> {
    let code = code.trim();
    if !(32..=128).contains(&code.len())
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(AppError::NotFound);
    }
    Ok(Sha256::digest(code.as_bytes()).into())
}

fn idempotency_key(headers: &HeaderMap) -> AppResult<String> {
    let value = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::Validation("Idempotency-Key header is required".into()))?;
    if !(8..=128).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(AppError::Validation(
            "Idempotency-Key must contain 8 to 128 URL-safe characters".into(),
        ));
    }
    Ok(value.to_string())
}

async fn replay<T: Serialize>(
    tx: &mut Transaction<'_, Sqlite>,
    assignment_id: &str,
    action: &str,
    target: MutationTarget<'_>,
    key: &str,
    input: &T,
) -> AppResult<Option<Value>> {
    let fingerprint = fingerprint(action, target, input)?;
    let row = sqlx::query(
        "SELECT resource_type, resource_id, request_fingerprint, result_json
         FROM admin_mutation_requests
         WHERE assignment_id = ? AND action = ? AND idempotency_key = ?",
    )
    .bind(assignment_id)
    .bind(action)
    .bind(key)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.get::<String, _>("resource_type") != target.kind
        || row.get::<String, _>("resource_id") != target.id
        || row.get::<Vec<u8>, _>("request_fingerprint").as_slice() != fingerprint
    {
        return Err(AppError::Conflict(
            "Idempotency-Key was already used for a different request".into(),
        ));
    }
    serde_json::from_str(row.get::<String, _>("result_json").as_str())
        .map(Some)
        .map_err(|error| AppError::Data(error.to_string()))
}

async fn record_replay<T: Serialize>(
    tx: &mut Transaction<'_, Sqlite>,
    assignment_id: &str,
    action: &str,
    target: MutationTarget<'_>,
    key: &str,
    input: &T,
    result: &Value,
) -> AppResult<()> {
    let now = Utc::now().timestamp();
    sqlx::query(
        "DELETE FROM admin_mutation_requests
         WHERE rowid IN (
           SELECT rowid FROM admin_mutation_requests
           WHERE created_at < ?
           ORDER BY created_at
           LIMIT 256
         )",
    )
    .bind(now - 7 * 24 * 60 * 60)
    .execute(&mut **tx)
    .await?;
    let active: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM admin_mutation_requests WHERE assignment_id = ?")
            .bind(assignment_id)
            .fetch_one(&mut **tx)
            .await?;
    if active >= 512 {
        return Err(AppError::RateLimited);
    }
    sqlx::query(
        "INSERT INTO admin_mutation_requests
         (assignment_id, action, resource_type, resource_id, idempotency_key,
          request_fingerprint, result_json, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(assignment_id)
    .bind(action)
    .bind(target.kind)
    .bind(target.id)
    .bind(key)
    .bind(fingerprint(action, target, input)?.as_slice())
    .bind(serde_json::to_string(result).map_err(|error| AppError::Data(error.to_string()))?)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn fingerprint<T: Serialize>(
    action: &str,
    target: MutationTarget<'_>,
    input: &T,
) -> AppResult<[u8; 32]> {
    let bytes = serde_json::to_vec(&(action, target.kind, target.id, input))
        .map_err(|error| AppError::Data(error.to_string()))?;
    Ok(Sha256::digest(bytes).into())
}

#[derive(Clone, Copy)]
struct MutationTarget<'a> {
    kind: &'static str,
    id: &'a str,
}

impl<'a> MutationTarget<'a> {
    fn recipe(id: &'a str) -> Self {
        Self { kind: "recipe", id }
    }

    fn appeal(id: &'a str) -> Self {
        Self { kind: "appeal", id }
    }
}

async fn active_or_new_case(
    tx: &mut Transaction<'_, Sqlite>,
    recipe_id: &str,
    now: i64,
) -> AppResult<(String, bool)> {
    if let Some(id) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM moderation_cases
         WHERE recipe_id = ? AND status IN ('open', 'restricted')",
    )
    .bind(recipe_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok((id, false));
    }
    let row = sqlx::query(
        "SELECT r.owner_id, r.version, r.title, r.description, r.instructions, u.public_handle,
                i.id AS image_id, i.image_bytes, i.media_type AS image_media_type,
                i.sha256 AS image_sha256
         FROM recipes r JOIN users u ON u.id = r.owner_id
         LEFT JOIN recipe_images i ON i.recipe_id = r.id
         WHERE r.id = ?",
    )
    .bind(recipe_id)
    .fetch_one(&mut **tx)
    .await?;
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO moderation_cases (id, recipe_id, created_at, updated_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(recipe_id)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO moderation_evidence_snapshots
         (case_id, recipe_id, owner_user_id, recipe_version, title, description, instructions,
          author_handle, image_id, image_bytes, image_media_type, image_sha256, captured_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(recipe_id)
    .bind(row.get::<String, _>("owner_id"))
    .bind(row.get::<i64, _>("version"))
    .bind(truncate(&row.get::<String, _>("title"), 120))
    .bind(truncate(&row.get::<String, _>("description"), 2_000))
    .bind(truncate(&row.get::<String, _>("instructions"), 20_000))
    .bind(row.get::<String, _>("public_handle"))
    .bind(row.get::<Option<String>, _>("image_id"))
    .bind(row.get::<Option<Vec<u8>>, _>("image_bytes"))
    .bind(row.get::<Option<String>, _>("image_media_type"))
    .bind(row.get::<Option<Vec<u8>>, _>("image_sha256"))
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok((id, true))
}

async fn rotate_image(
    tx: &mut Transaction<'_, Sqlite>,
    recipe_id: &str,
    now: i64,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE recipe_images
         SET id = ?, version = version + 1, updated_at = ?
         WHERE recipe_id = ?",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(now)
    .bind(recipe_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn audit(
    tx: &mut Transaction<'_, Sqlite>,
    event_type: &str,
    assignment_id: Option<&str>,
    actor_user_id: Option<&str>,
    recipe_id: Option<&str>,
    case_id: Option<&str>,
    appeal_id: Option<&str>,
    reason_code: Option<&str>,
    recipe_version: Option<i64>,
    moderation_version: Option<i64>,
    case_version: Option<i64>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO moderation_audit_events
         (id, event_type, assignment_id, actor_user_id, recipe_id, case_id, appeal_id,
          reason_code, recipe_version, moderation_version, case_version, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(event_type)
    .bind(assignment_id)
    .bind(actor_user_id)
    .bind(recipe_id)
    .bind(case_id)
    .bind(appeal_id)
    .bind(reason_code)
    .bind(recipe_version)
    .bind(moderation_version)
    .bind(case_version)
    .bind(Utc::now().timestamp())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn rate_limit(state: &AppState, key: &str, limit: i64, window: i64) -> AppResult<()> {
    let now = Utc::now().timestamp();
    let start = now - now.rem_euclid(window);
    let count: i64 = sqlx::query_scalar(
        "INSERT INTO moderation_rate_limits (rate_key, window_started_at, request_count)
         VALUES (?, ?, 1)
         ON CONFLICT(rate_key) DO UPDATE SET
           window_started_at = CASE WHEN moderation_rate_limits.window_started_at < excluded.window_started_at
             THEN excluded.window_started_at ELSE moderation_rate_limits.window_started_at END,
           request_count = CASE WHEN moderation_rate_limits.window_started_at < excluded.window_started_at
             THEN 1 ELSE moderation_rate_limits.request_count + 1 END
         RETURNING request_count",
    )
    .bind(key)
    .bind(start)
    .fetch_one(&state.pool)
    .await?;
    if count > limit {
        return Err(AppError::RateLimited);
    }
    Ok(())
}

fn validate_report_reason(value: &str) -> AppResult<()> {
    match value {
        "spam"
        | "unsafe_or_harmful"
        | "harassment_or_hate"
        | "privacy"
        | "copyright_or_attribution"
        | "misleading"
        | "other" => Ok(()),
        _ => Err(AppError::Validation("invalid report reason".into())),
    }
}

fn validate_action_reason(value: &str) -> AppResult<()> {
    match value {
        "policy_violation" | "safety" | "privacy" | "copyright" | "spam" | "appeal_reviewed"
        | "other" => Ok(()),
        _ => Err(AppError::Validation(
            "invalid moderation reason code".into(),
        )),
    }
}

fn validate_review_reason(value: &str) -> AppResult<()> {
    match value {
        "no_action_needed" | "report_resolved" | "appeal_reviewed" | "other" => Ok(()),
        _ => Err(AppError::Validation("invalid review reason code".into())),
    }
}

fn plain_text(field: &str, value: &str, max: usize, allow_empty: bool) -> AppResult<String> {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    let value = normalized.trim();
    if !allow_empty && value.is_empty() {
        return Err(AppError::Validation(format!("{field} is required")));
    }
    if value.chars().count() > max
        || value
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(AppError::Validation(format!(
            "{field} must contain at most {max} visible characters"
        )));
    }
    Ok(value.to_string())
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn map_unique_conflict(error: sqlx::Error) -> AppError {
    if error
        .as_database_error()
        .is_some_and(|database| database.is_unique_violation())
    {
        AppError::Conflict("an appeal already exists for this restriction".into())
    } else {
        AppError::Database(error)
    }
}

fn encode_cursor(updated_at: &str, id: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("{updated_at}\n{id}"))
}

fn decode_cursor(value: &str) -> AppResult<(String, String)> {
    let decoded = URL_SAFE_NO_PAD
        .decode(value)
        .ok()
        .and_then(|value| String::from_utf8(value).ok())
        .ok_or_else(|| AppError::Validation("invalid cursor".into()))?;
    decoded
        .split_once('\n')
        .map(|(left, right)| (left.to_string(), right.to_string()))
        .filter(|(left, right)| !left.is_empty() && !right.is_empty())
        .ok_or_else(|| AppError::Validation("invalid cursor".into()))
}

fn encode_audit_cursor(created_at: i64, id: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("{created_at}\n{id}"))
}

fn decode_audit_cursor(value: &str) -> AppResult<(i64, String)> {
    let (created, id) = decode_cursor(value)?;
    Ok((
        created
            .parse()
            .map_err(|_| AppError::Validation("invalid cursor".into()))?,
        id,
    ))
}

fn audit_json(row: sqlx::sqlite::SqliteRow) -> Value {
    json!({
        "id": row.get::<String, _>("id"),
        "eventType": row.get::<String, _>("event_type"),
        "assignmentId": row.get::<Option<String>, _>("assignment_id"),
        "recipeId": row.get::<Option<String>, _>("recipe_id"),
        "caseId": row.get::<Option<String>, _>("case_id"),
        "appealId": row.get::<Option<String>, _>("appeal_id"),
        "reasonCode": row.get::<Option<String>, _>("reason_code"),
        "recipeVersion": row.get::<Option<i64>, _>("recipe_version"),
        "moderationVersion": row.get::<Option<i64>, _>("moderation_version"),
        "caseVersion": row.get::<Option<i64>, _>("case_version"),
        "createdAt": row.get::<i64, _>("created_at")
    })
}
