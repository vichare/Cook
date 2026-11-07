use axum::{
    extract::State,
    http::{header::SET_COOKIE, HeaderMap},
    response::IntoResponse,
    Extension, Json,
};
use axum_extra::extract::cookie::CookieJar;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, Rng, RngCore};
use rust_web_auth_core::UserId;
use rust_web_auth_http::{expired_cookie, issued_cookie};
use rust_web_auth_passkey::PublicKeyCredential;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    passwordless::{normalize_email, EMAIL_IDENTITY_ISSUER},
    routes::{issue_assisted_user_session, rate_limit_key, CurrentAuth, CurrentUser},
    AppState,
};

const OTP_LIFETIME_SECONDS: i64 = 10 * 60;
const ASSISTED_SESSION_SECONDS: i64 = 60 * 60;
const CLAIM_LIFETIME_SECONDS: i64 = 5 * 60;
const REAUTH_SECONDS: i64 = 10 * 60;
const MAX_NOTE_CHARS: usize = 120;
const PURPOSE: &str = "account_assistant_claim";

type HmacSha256 = Hmac<Sha256>;

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
pub struct InspectInput {
    email: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueInput {
    email: String,
    reason_code: String,
    #[serde(default)]
    note: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectResponse {
    normalized_email: String,
    target_status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueResponse {
    challenge_id: String,
    code: String,
    expires_at: String,
    target_status: &'static str,
}

struct ChallengeRow {
    normalized_email: String,
    code_digest: Vec<u8>,
    assignment_id: String,
    assignment_alias: String,
    reason_code: String,
    target_existed: bool,
    expires_at: i64,
    attempts_remaining: i64,
}

pub(crate) async fn claim_start(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentAuth(auth)): Extension<CurrentAuth>,
    Json(input): Json<ClaimStartInput>,
) -> AppResult<impl IntoResponse> {
    ensure_not_assisted_session(&state, &auth).await?;
    check_rate_limits(
        &state,
        &format!("claim:{}", user.id),
        10,
        60 * 60,
        Utc::now().timestamp(),
    )
    .await?;
    let code_digest = claim_digest(&input.code)?;
    let now = Utc::now().timestamp();
    let valid: i64 = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM account_assistant_claim_codes
           WHERE code_digest = ? AND consumed_at IS NULL AND expires_at > ?
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
    let start = state
        .auth
        .passwordless
        .passkeys
        .start_authentication(
            UserId::new(user.id.clone()).map_err(|_| AppError::AuthUnavailable)?,
            Utc::now(),
        )
        .await
        .map_err(crate::passwordless::map_passkey_error)?;
    remember_attempt(
        &state,
        start.ceremony_token.expose_for_cookie(),
        &auth.session_id.to_string(),
        &user.id,
        Some(&code_digest),
    )
    .await?;
    let cookie = issued_cookie(
        &state.auth.admin_reauth_cookie,
        start.ceremony_token.expose_for_cookie(),
        CLAIM_LIFETIME_SECONDS as u64,
    )
    .map_err(|_| AppError::AuthUnavailable)?;
    Ok(([(SET_COOKIE, cookie)], Json(start.challenge)))
}

pub(crate) async fn claim_finish(
    State(state): State<AppState>,
    jar: CookieJar,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentAuth(auth)): Extension<CurrentAuth>,
    Json(input): Json<ClaimFinishInput>,
) -> AppResult<impl IntoResponse> {
    ensure_not_assisted_session(&state, &auth).await?;
    let token = jar
        .get(state.auth.admin_reauth_cookie.name())
        .map(|cookie| cookie.value())
        .ok_or(AppError::NotFound)?;
    let code_digest = claim_digest(&input.code)?;
    consume_attempt(
        &state,
        token,
        &auth.session_id.to_string(),
        &user.id,
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
        "SELECT id FROM account_assistant_claim_codes
         WHERE code_digest = ? AND consumed_at IS NULL AND expires_at > ?",
    )
    .bind(code_digest.as_slice())
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let (assignment_id, alias) = grant_assignment(&mut tx, &user.id, now).await?;
    sqlx::query(
        "UPDATE account_assistant_claim_codes
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
        "UPDATE auth_sessions SET authorization_version = ?, passkey_verified_at = ?
         WHERE session_id = ? AND user_id = ?",
    )
    .bind(authorization_version)
    .bind(now)
    .bind(auth.session_id.to_string())
    .bind(&user.id)
    .execute(&mut *tx)
    .await?;
    insert_audit(
        &mut tx,
        "account_assistant_claimed",
        Some(&assignment_id),
        Some(&alias),
        Some(&user.id),
        None,
        None,
        None,
        None,
        now,
    )
    .await?;
    tx.commit().await?;
    Ok((
        [(
            SET_COOKIE,
            expired_cookie(&state.auth.admin_reauth_cookie)
                .map_err(|_| AppError::AuthUnavailable)?,
        )],
        Json(serde_json::json!({"capability": "account_assistance"})),
    ))
}

pub(crate) async fn inspect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentAuth(auth)): Extension<CurrentAuth>,
    Json(input): Json<InspectInput>,
) -> AppResult<Json<InspectResponse>> {
    let _assignment = require_fresh_assistant(&state, &user.id, &auth).await?;
    same_origin(&state, &headers)?;
    let normalized = normalize_email(&input.email).map_err(crate::passwordless::map_email_error)?;
    check_rate_limits(
        &state,
        &format!("inspect:{}", user.id),
        30,
        60 * 60,
        Utc::now().timestamp(),
    )
    .await?;
    let exists = email_user(&state, &normalized).await?.is_some();
    Ok(Json(InspectResponse {
        normalized_email: normalized,
        target_status: if exists { "existing" } else { "new" },
    }))
}

pub(crate) async fn issue_otp(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentAuth(auth)): Extension<CurrentAuth>,
    Json(input): Json<IssueInput>,
) -> AppResult<Json<IssueResponse>> {
    let (assignment_id, assignment_alias) =
        require_fresh_assistant(&state, &user.id, &auth).await?;
    same_origin(&state, &headers)?;
    let normalized = normalize_email(&input.email).map_err(crate::passwordless::map_email_error)?;
    let reason_code = validate_reason(&input.reason_code)?;
    let note = input.note.trim();
    if note.chars().count() > MAX_NOTE_CHARS || note.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "assistance note must contain at most 120 visible characters".into(),
        ));
    }

    let now = Utc::now().timestamp();
    let email_digest = keyed_digest(
        &state.config.email_otp_digest_key,
        b"account-assistance-email",
        normalized.as_bytes(),
    );
    let ip_partition = rate_limit_key(&headers, &state.config.email_otp_digest_key);
    enforce_issue_limits(&state, &assignment_id, &email_digest, &ip_partition, now).await?;

    let target_user = email_user(&state, &normalized).await?;
    let target_existed = target_user.is_some();
    let challenge_id = format!("aa_{}", random_token());
    let code = format!("{:08}", OsRng.gen_range(0..100_000_000u32));
    let challenge_digest = keyed_digest(
        &state.config.email_otp_digest_key,
        b"account-assistance-challenge",
        challenge_id.as_bytes(),
    );
    let code_digest = keyed_digest(
        &state.config.email_otp_digest_key,
        &challenge_digest,
        code.as_bytes(),
    );
    let expires_at = now + OTP_LIFETIME_SECONDS;
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query(
        "INSERT INTO account_assistance_otp_challenges
         (challenge_digest, email_digest, normalized_email, code_digest,
          issuer_assignment_id, issuer_assignment_alias, reason_code, note_present,
          target_existed_at_issue, created_at, expires_at, attempts_remaining)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 5)
         ON CONFLICT(email_digest) DO UPDATE SET
           challenge_digest = excluded.challenge_digest,
           normalized_email = excluded.normalized_email,
           code_digest = excluded.code_digest,
           issuer_assignment_id = excluded.issuer_assignment_id,
           issuer_assignment_alias = excluded.issuer_assignment_alias,
           reason_code = excluded.reason_code,
           note_present = excluded.note_present,
           target_existed_at_issue = excluded.target_existed_at_issue,
           created_at = excluded.created_at,
           expires_at = excluded.expires_at,
           attempts_remaining = 5",
    )
    .bind(challenge_digest.as_slice())
    .bind(email_digest.as_slice())
    .bind(&normalized)
    .bind(code_digest.as_slice())
    .bind(&assignment_id)
    .bind(&assignment_alias)
    .bind(reason_code)
    .bind(i64::from(!note.is_empty()))
    .bind(i64::from(target_existed))
    .bind(now)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    if let Some(target_user) = target_user.as_deref() {
        insert_notification(
            &mut tx,
            target_user,
            "account_assistance_issued",
            &assignment_alias,
            reason_code,
            now,
        )
        .await?;
    }
    insert_audit(
        &mut tx,
        "assisted_otp_issued",
        Some(&assignment_id),
        Some(&assignment_alias),
        Some(&user.id),
        Some(&email_digest),
        Some(target_existed),
        Some(reason_code),
        Some(!note.is_empty()),
        now,
    )
    .await?;
    tx.commit().await?;

    Ok(Json(IssueResponse {
        challenge_id,
        code,
        expires_at: chrono::DateTime::from_timestamp(expires_at, 0)
            .ok_or_else(|| AppError::Data("invalid assisted OTP expiry".into()))?
            .to_rfc3339(),
        target_status: if target_existed { "existing" } else { "new" },
    }))
}

pub(crate) async fn try_verify(
    state: &AppState,
    headers: &HeaderMap,
    challenge_id: &str,
    code: &str,
) -> AppResult<Option<impl IntoResponse>> {
    let challenge_digest = keyed_digest(
        &state.config.email_otp_digest_key,
        b"account-assistance-challenge",
        challenge_id.as_bytes(),
    );
    let exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM account_assistance_otp_challenges WHERE challenge_digest = ?
         )",
    )
    .bind(challenge_digest.as_slice())
    .fetch_one(&state.pool)
    .await?;
    if exists == 0 {
        if challenge_id.starts_with("aa_") {
            return Err(AppError::AssistedOtpInvalid);
        }
        return Ok(None);
    }
    let now = Utc::now().timestamp();
    let ip_partition = rate_limit_key(headers, &state.config.email_otp_digest_key);
    check_rate_limits(
        state,
        &format!("verify-ip:{ip_partition}"),
        60,
        15 * 60,
        now,
    )
    .await?;

    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let row = sqlx::query(
        "SELECT normalized_email, code_digest, issuer_assignment_id,
                issuer_assignment_alias, reason_code, target_existed_at_issue,
                expires_at, attempts_remaining
         FROM account_assistance_otp_challenges WHERE challenge_digest = ?",
    )
    .bind(challenge_digest.as_slice())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::AssistedOtpInvalid)?;
    let row = ChallengeRow {
        normalized_email: row.get("normalized_email"),
        code_digest: row.get("code_digest"),
        assignment_id: row.get("issuer_assignment_id"),
        assignment_alias: row.get("issuer_assignment_alias"),
        reason_code: row.get("reason_code"),
        target_existed: row.get::<i64, _>("target_existed_at_issue") != 0,
        expires_at: row.get("expires_at"),
        attempts_remaining: row.get("attempts_remaining"),
    };
    if row.expires_at <= now {
        sqlx::query("DELETE FROM account_assistance_otp_challenges WHERE challenge_digest = ?")
            .bind(challenge_digest.as_slice())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(AppError::AssistedOtpInvalid);
    }
    let mut verifier = HmacSha256::new_from_slice(&state.config.email_otp_digest_key)
        .map_err(|_| AppError::AuthUnavailable)?;
    verifier.update(&challenge_digest);
    verifier.update(&[0]);
    verifier.update(code.as_bytes());
    let valid = verifier.verify_slice(&row.code_digest).is_ok();
    if !valid {
        if row.attempts_remaining <= 1 {
            sqlx::query("DELETE FROM account_assistance_otp_challenges WHERE challenge_digest = ?")
                .bind(challenge_digest.as_slice())
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query(
                "UPDATE account_assistance_otp_challenges
                 SET attempts_remaining = attempts_remaining - 1 WHERE challenge_digest = ?",
            )
            .bind(challenge_digest.as_slice())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        return Err(AppError::AssistedOtpInvalid);
    }
    let active: i64 = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM account_assistant_assignments
           WHERE id = ? AND alias = ? AND revoked_at IS NULL
         )",
    )
    .bind(&row.assignment_id)
    .bind(&row.assignment_alias)
    .fetch_one(&mut *tx)
    .await?;
    if active == 0 {
        sqlx::query("DELETE FROM account_assistance_otp_challenges WHERE challenge_digest = ?")
            .bind(challenge_digest.as_slice())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(AppError::AssistedOtpInvalid);
    }
    sqlx::query("DELETE FROM account_assistance_otp_challenges WHERE challenge_digest = ?")
        .bind(challenge_digest.as_slice())
        .execute(&mut *tx)
        .await?;
    let (user_id, existed) = resolve_assisted_user(&mut tx, &row.normalized_email, now).await?;
    if !existed {
        insert_notification(
            &mut tx,
            &user_id,
            "account_assistance_issued",
            &row.assignment_alias,
            &row.reason_code,
            now,
        )
        .await?;
    }
    insert_notification(
        &mut tx,
        &user_id,
        "account_assistance_consumed",
        &row.assignment_alias,
        &row.reason_code,
        now,
    )
    .await?;
    let email_digest = keyed_digest(
        &state.config.email_otp_digest_key,
        b"account-assistance-email",
        row.normalized_email.as_bytes(),
    );
    insert_audit(
        &mut tx,
        "assisted_otp_consumed",
        Some(&row.assignment_id),
        Some(&row.assignment_alias),
        None,
        Some(&email_digest),
        Some(row.target_existed),
        Some(&row.reason_code),
        None,
        now,
    )
    .await?;
    tx.commit().await?;
    let response = issue_assisted_user_session(
        state,
        UserId::new(user_id).map_err(|_| AppError::AuthUnavailable)?,
        &row.assignment_id,
        &row.assignment_alias,
        now,
        ASSISTED_SESSION_SECONDS,
    )
    .await?;
    Ok(Some(response))
}

pub(crate) async fn ensure_not_assisted_session(
    state: &AppState,
    auth: &rust_web_auth_core::AuthenticatedContext,
) -> AppResult<()> {
    if is_assisted_session(state, auth).await? {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

pub(crate) async fn is_assisted_session(
    state: &AppState,
    auth: &rust_web_auth_core::AuthenticatedContext,
) -> AppResult<bool> {
    let assisted: i64 = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM auth_sessions
           WHERE session_id = ? AND auth_method = 'admin_assisted_otp'
         )",
    )
    .bind(auth.session_id.to_string())
    .fetch_one(&state.pool)
    .await?;
    Ok(assisted != 0)
}

pub(crate) async fn require_any_admin(state: &AppState, user_id: &str) -> AppResult<()> {
    let exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM capability_assignments
           WHERE user_id = ? AND capability = 'content_moderation' AND revoked_at IS NULL
           UNION ALL
           SELECT 1 FROM account_assistant_assignments
           WHERE user_id = ? AND revoked_at IS NULL
         )",
    )
    .bind(user_id)
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;
    if exists == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

pub async fn revoke_assignment(
    pool: &sqlx::SqlitePool,
    assignment_id: &str,
    now: i64,
) -> AppResult<(String, String)> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let row = sqlx::query(
        "SELECT user_id, alias FROM account_assistant_assignments
         WHERE id = ? AND revoked_at IS NULL",
    )
    .bind(assignment_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let user_id: String = row.get("user_id");
    let alias: String = row.get("alias");
    sqlx::query("UPDATE account_assistant_assignments SET revoked_at = ? WHERE id = ?")
        .bind(now)
        .bind(assignment_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM account_assistance_otp_challenges WHERE issuer_assignment_id = ?")
        .bind(assignment_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE users SET authorization_version = authorization_version + 1 WHERE id = ?")
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_sessions WHERE user_id = ?")
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
    insert_audit(
        &mut tx,
        "account_assistant_revoked",
        Some(assignment_id),
        Some(&alias),
        None,
        None,
        None,
        None,
        None,
        now,
    )
    .await?;
    tx.commit().await?;
    Ok((user_id, alias))
}

async fn grant_assignment(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: &str,
    now: i64,
) -> AppResult<(String, String)> {
    let active: i64 = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM account_assistant_assignments
           WHERE user_id = ? AND revoked_at IS NULL
         )",
    )
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await?;
    if active != 0 {
        return Err(AppError::Conflict(
            "account assistant capability is already assigned".into(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    let alias = random_alias();
    sqlx::query(
        "INSERT INTO account_assistant_assignments
         (id, user_id, alias, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(&alias)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok((id, alias))
}

async fn require_fresh_assistant(
    state: &AppState,
    user_id: &str,
    auth: &rust_web_auth_core::AuthenticatedContext,
) -> AppResult<(String, String)> {
    ensure_not_assisted_session(state, auth).await?;
    let assignment = sqlx::query(
        "SELECT id, alias FROM account_assistant_assignments
         WHERE user_id = ? AND revoked_at IS NULL",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
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
    Ok((assignment.get("id"), assignment.get("alias")))
}

async fn remember_attempt(
    state: &AppState,
    token: &str,
    session_id: &str,
    user_id: &str,
    code_digest: Option<&[u8; 32]>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT OR REPLACE INTO admin_passkey_attempts
         (token_digest, session_id, user_id, purpose, claim_code_digest, expires_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(Sha256::digest(token.as_bytes()).as_slice())
    .bind(session_id)
    .bind(user_id)
    .bind(PURPOSE)
    .bind(code_digest.map(|value| value.as_slice()))
    .bind(Utc::now().timestamp() + CLAIM_LIFETIME_SECONDS)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn consume_attempt(
    state: &AppState,
    token: &str,
    session_id: &str,
    user_id: &str,
    code_digest: Option<&[u8; 32]>,
) -> AppResult<()> {
    let deleted = sqlx::query(
        "DELETE FROM admin_passkey_attempts
         WHERE token_digest = ? AND session_id = ? AND user_id = ? AND purpose = ?
           AND expires_at > ?
           AND ((claim_code_digest IS NULL AND ? IS NULL) OR claim_code_digest = ?)",
    )
    .bind(Sha256::digest(token.as_bytes()).as_slice())
    .bind(session_id)
    .bind(user_id)
    .bind(PURPOSE)
    .bind(Utc::now().timestamp())
    .bind(code_digest.map(|value| value.as_slice()))
    .bind(code_digest.map(|value| value.as_slice()))
    .execute(&state.pool)
    .await?;
    if deleted.rows_affected() != 1 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

async fn resolve_assisted_user(
    tx: &mut Transaction<'_, Sqlite>,
    normalized_email: &str,
    now: i64,
) -> AppResult<(String, bool)> {
    if let Some(user_id) = sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM auth_identities WHERE issuer = ? AND subject = ?",
    )
    .bind(EMAIL_IDENTITY_ISSUER)
    .bind(normalized_email)
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok((user_id, true));
    }
    let user_id = Uuid::new_v4().to_string();
    let display_name = normalized_email
        .split_once('@')
        .map(|(local, _)| local)
        .filter(|value| !value.is_empty())
        .unwrap_or("Cook user");
    sqlx::query("INSERT INTO users (id, email, profile_email, display_name) VALUES (?, ?, ?, ?)")
        .bind(&user_id)
        .bind(format!("email-{user_id}@identity.invalid"))
        .bind(normalized_email)
        .bind(display_name)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO auth_identities
         (issuer, subject, user_id, created_at, verified_email, verification_provenance)
         VALUES (?, ?, ?, ?, NULL, 'admin_assisted')",
    )
    .bind(EMAIL_IDENTITY_ISSUER)
    .bind(normalized_email)
    .bind(&user_id)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok((user_id, false))
}

async fn email_user(state: &AppState, normalized_email: &str) -> AppResult<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT user_id FROM auth_identities WHERE issuer = ? AND subject = ?")
            .bind(EMAIL_IDENTITY_ISSUER)
            .bind(normalized_email)
            .fetch_optional(&state.pool)
            .await?,
    )
}

async fn enforce_issue_limits(
    state: &AppState,
    assignment_id: &str,
    email_digest: &[u8; 32],
    ip_partition: &str,
    now: i64,
) -> AppResult<()> {
    let target_key = URL_SAFE_NO_PAD.encode(email_digest);
    let checks = [
        (format!("issuer:{assignment_id}"), 5, 60 * 60),
        (format!("target:{target_key}"), 3, 24 * 60 * 60),
        (format!("ip:{ip_partition}"), 30, 60 * 60),
        ("global".to_string(), 100, 60 * 60),
    ];
    let mut retry_after = 0u64;
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    for (key, limit, window) in checks {
        retry_after = retry_after.max(increment_limit(&mut tx, &key, limit, window, now).await?);
    }
    tx.commit().await?;
    if retry_after > 0 {
        return Err(AppError::RateLimitedWithRetry(retry_after));
    }
    Ok(())
}

async fn check_rate_limits(
    state: &AppState,
    key: &str,
    limit: i64,
    window: i64,
    now: i64,
) -> AppResult<()> {
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let retry = increment_limit(&mut tx, key, limit, window, now).await?;
    tx.commit().await?;
    if retry > 0 {
        return Err(AppError::RateLimitedWithRetry(retry));
    }
    Ok(())
}

async fn increment_limit(
    tx: &mut Transaction<'_, Sqlite>,
    key: &str,
    limit: i64,
    window: i64,
    now: i64,
) -> AppResult<u64> {
    let cutoff = now - window;
    let row = sqlx::query(
        "INSERT INTO account_assistance_rate_limits
         (rate_key, window_started_at, request_count) VALUES (?, ?, 1)
         ON CONFLICT(rate_key) DO UPDATE SET
           window_started_at = CASE
             WHEN account_assistance_rate_limits.window_started_at <= ?
             THEN excluded.window_started_at
             ELSE account_assistance_rate_limits.window_started_at END,
           request_count = CASE
             WHEN account_assistance_rate_limits.window_started_at <= ? THEN 1
             ELSE account_assistance_rate_limits.request_count + 1 END
         RETURNING window_started_at, request_count",
    )
    .bind(key)
    .bind(now)
    .bind(cutoff)
    .bind(cutoff)
    .fetch_one(&mut **tx)
    .await?;
    let started: i64 = row.get("window_started_at");
    let count: i64 = row.get("request_count");
    Ok(if count > limit {
        u64::try_from((started + window - now).max(1)).unwrap_or(1)
    } else {
        0
    })
}

async fn insert_notification(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: &str,
    kind: &str,
    issuer_alias: &str,
    reason_code: &str,
    now: i64,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO account_assistance_notifications
         (id, user_id, kind, issuer_alias, reason_code, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(kind)
    .bind(issuer_alias)
    .bind(reason_code)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_audit(
    tx: &mut Transaction<'_, Sqlite>,
    event_type: &str,
    assignment_id: Option<&str>,
    assignment_alias: Option<&str>,
    actor_user_id: Option<&str>,
    target_email_digest: Option<&[u8; 32]>,
    target_existed: Option<bool>,
    reason_code: Option<&str>,
    note_present: Option<bool>,
    now: i64,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO account_assistance_audit_events
         (id, event_type, assignment_id, assignment_alias, actor_user_id,
          target_email_digest, target_existed, reason_code, note_present, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(event_type)
    .bind(assignment_id)
    .bind(assignment_alias)
    .bind(actor_user_id)
    .bind(target_email_digest.map(|value| value.as_slice()))
    .bind(target_existed.map(i64::from))
    .bind(reason_code)
    .bind(note_present.map(i64::from))
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
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

fn validate_reason(value: &str) -> AppResult<&str> {
    match value {
        "in_person_accessibility" | "mailbox_unavailable" | "device_setup" => Ok(value),
        _ => Err(AppError::Validation(
            "invalid account assistance reason code".into(),
        )),
    }
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

fn keyed_digest(key: &[u8; 32], domain: &[u8], value: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("fixed HMAC key");
    mac.update(domain);
    mac.update(&[0]);
    mac.update(value);
    mac.finalize().into_bytes().into()
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn random_alias() -> String {
    let mut bytes = [0u8; 6];
    OsRng.fill_bytes(&mut bytes);
    format!("assistant-{}", URL_SAFE_NO_PAD.encode(bytes).to_lowercase())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use chrono::Utc;
    use serde_json::{json, Value};
    use sqlx::Row;
    use tower::ServiceExt;

    use super::ASSISTED_SESSION_SECONDS;
    use crate::{
        auth::build_auth, config::Config, db, passwordless::EMAIL_IDENTITY_ISSUER, routes, AppState,
    };

    async fn runtime() -> (axum::Router, crate::auth::AuthRuntime, sqlx::SqlitePool) {
        let config = Config::test();
        let pool = db::connect(&config.database_url).await.unwrap();
        let auth = build_auth(&config, &pool).unwrap();
        let app = routes::router(AppState {
            pool: pool.clone(),
            config: Arc::new(config),
            auth: auth.clone(),
        });
        (app, auth, pool)
    }

    async fn login(app: &axum::Router, name: &str) -> String {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/auth/dev-login")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"displayName": name}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        response
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    async fn json_body(response: axum::response::Response) -> Value {
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    async fn seed_assistant(pool: &sqlx::SqlitePool, user_id: &str) {
        sqlx::query(
            "INSERT INTO account_assistant_assignments
             (id, user_id, alias, created_at) VALUES ('assistant-assignment', ?, 'assistant-test', ?)",
        )
        .bind(user_id)
        .bind(Utc::now().timestamp())
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("UPDATE auth_sessions SET passkey_verified_at = ? WHERE user_id = ?")
            .bind(Utc::now().timestamp())
            .bind(user_id)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn issue(app: &axum::Router, cookie: &str, email: &str) -> axum::response::Response {
        app.clone()
            .oneshot(
                Request::post("/api/admin/account-assistance/otp")
                    .header("content-type", "application/json")
                    .header("origin", "http://localhost")
                    .header("x-forwarded-for", "203.0.113.8")
                    .header("cookie", cookie)
                    .body(Body::from(
                        json!({
                            "email": email,
                            "reasonCode": "in_person_accessibility",
                            "note": "<script>alert(1)</script>"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn issue_requires_separate_role_fresh_passkey_and_same_origin() {
        let (app, _, pool) = runtime().await;
        let cookie = login(&app, "Assistant").await;
        let user_id = "dev:assistant";

        let no_role = issue(&app, &cookie, "owner@example.test").await;
        assert_eq!(no_role.status(), StatusCode::NOT_FOUND);

        sqlx::query(
            "INSERT INTO capability_assignments
             (id, user_id, capability, created_at)
             VALUES ('moderator', ?, 'content_moderation', ?)",
        )
        .bind(user_id)
        .bind(Utc::now().timestamp())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE auth_sessions SET passkey_verified_at = ? WHERE user_id = ?")
            .bind(Utc::now().timestamp())
            .bind(user_id)
            .execute(&pool)
            .await
            .unwrap();
        let moderator_only = issue(&app, &cookie, "owner@example.test").await;
        assert_eq!(moderator_only.status(), StatusCode::NOT_FOUND);

        seed_assistant(&pool, user_id).await;
        sqlx::query("UPDATE auth_sessions SET passkey_verified_at = NULL WHERE user_id = ?")
            .bind(user_id)
            .execute(&pool)
            .await
            .unwrap();
        let stale = issue(&app, &cookie, "owner@example.test").await;
        assert_eq!(stale.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            json_body(stale).await["error"],
            "admin_passkey_reauth_required"
        );

        sqlx::query("UPDATE auth_sessions SET passkey_verified_at = ? WHERE user_id = ?")
            .bind(Utc::now().timestamp())
            .bind(user_id)
            .execute(&pool)
            .await
            .unwrap();
        let missing_origin = app
            .oneshot(
                Request::post("/api/admin/account-assistance/otp")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(Body::from(
                        json!({
                            "email": "owner@example.test",
                            "reasonCode": "in_person_accessibility"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_origin.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn assisted_otp_is_one_time_digest_only_and_upgrades_after_mailbox_verification() {
        let (app, auth, pool) = runtime().await;
        let cookie = login(&app, "Assistant").await;
        seed_assistant(&pool, "dev:assistant").await;

        let issued = issue(&app, &cookie, "elder@gmail.com").await;
        assert_eq!(issued.status(), StatusCode::OK);
        let issued = json_body(issued).await;
        let challenge = issued["challengeId"].as_str().unwrap();
        let code = issued["code"].as_str().unwrap();
        assert_eq!(code.len(), 8);
        assert!(code.bytes().all(|value| value.is_ascii_digit()));
        assert_eq!(issued["targetStatus"], "new");
        let stored: Vec<u8> =
            sqlx::query_scalar("SELECT code_digest FROM account_assistance_otp_challenges")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored.len(), 32);
        assert_ne!(stored, code.as_bytes());

        let verified = app
            .clone()
            .oneshot(
                Request::post("/api/auth/email/verify")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.9")
                    .body(Body::from(
                        json!({"challengeId": challenge, "code": code}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(verified.status(), StatusCode::OK);
        let assisted_cookie = verified
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let verified_body = json_body(verified).await;
        assert_eq!(
            verified_body["assistedSession"]["issuerAlias"],
            "assistant-test"
        );
        let provenance: String = sqlx::query_scalar(
            "SELECT verification_provenance FROM auth_identities
             WHERE issuer = ? AND subject = 'elder@gmail.com'",
        )
        .bind(EMAIL_IDENTITY_ISSUER)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(provenance, "admin_assisted");
        let session = sqlx::query(
            "SELECT auth_method, expires_at - authenticated_at AS lifetime
             FROM auth_sessions WHERE auth_method = 'admin_assisted_otp'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            session.get::<String, _>("auth_method"),
            "admin_assisted_otp"
        );
        assert_eq!(session.get::<i64, _>("lifetime"), ASSISTED_SESSION_SECONDS);
        let notification_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM account_assistance_notifications")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(notification_count, 2);

        let replay = app
            .clone()
            .oneshot(
                Request::post("/api/auth/email/verify")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.9")
                    .body(Body::from(
                        json!({"challengeId": challenge, "code": code}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);

        let restricted = app
            .clone()
            .oneshot(
                Request::post("/api/settings/security/email/request")
                    .header("content-type", "application/json")
                    .header("cookie", &assisted_cookie)
                    .body(Body::from(
                        json!({"email": "other@example.test"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(restricted.status(), StatusCode::FORBIDDEN);

        for request in [
            Request::post("/api/settings/security/passkeys/register/start")
                .header("content-type", "application/json")
                .header("cookie", &assisted_cookie)
                .body(Body::from(json!({"name": "temporary"}).to_string()))
                .unwrap(),
            Request::post("/api/settings/import-tokens")
                .header("content-type", "application/json")
                .header("cookie", &assisted_cookie)
                .body(Body::from(
                    json!({"name": "persistent", "preset": "manage"}).to_string(),
                ))
                .unwrap(),
            Request::post("/api/auth/logout-all")
                .header("content-type", "application/json")
                .header("cookie", &assisted_cookie)
                .body(Body::empty())
                .unwrap(),
        ] {
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        let locale = app
            .clone()
            .oneshot(
                Request::put("/api/settings/locale")
                    .header("content-type", "application/json")
                    .header("cookie", &assisted_cookie)
                    .body(Body::from(json!({"locale": "zh-CN"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(locale.status(), StatusCode::OK);
        let profile = app
            .clone()
            .oneshot(
                Request::put("/api/settings/profile")
                    .header("content-type", "application/json")
                    .header("cookie", &assisted_cookie)
                    .body(Body::from(json!({"nickname": "Elder"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(profile.status(), StatusCode::OK);

        let ordinary = app
            .clone()
            .oneshot(
                Request::post("/api/auth/email/request")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.10")
                    .body(Body::from(json!({"email": "elder@gmail.com"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let ordinary = json_body(ordinary).await;
        let ordinary_code = auth.passwordless.captured_code("elder@gmail.com").unwrap();
        let ordinary_verify = app
            .clone()
            .oneshot(
                Request::post("/api/auth/email/verify")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.10")
                    .body(Body::from(
                        json!({
                            "challengeId": ordinary["challengeId"],
                            "code": ordinary_code
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ordinary_verify.status(), StatusCode::OK);
        let upgraded: String = sqlx::query_scalar(
            "SELECT verification_provenance FROM auth_identities
             WHERE issuer = ? AND subject = 'elder@gmail.com'",
        )
        .bind(EMAIL_IDENTITY_ISSUER)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(upgraded, "mailbox_verified");

        sqlx::query(
            "UPDATE account_assistant_assignments SET revoked_at = ? WHERE id = 'assistant-assignment'",
        )
        .bind(Utc::now().timestamp())
        .execute(&pool)
        .await
        .unwrap();
        let revoked = app
            .oneshot(
                Request::get("/api/me")
                    .header("cookie", assisted_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn replacement_expiry_attempts_and_concurrent_consume_fail_closed() {
        let (app, _, pool) = runtime().await;
        let cookie = login(&app, "Assistant").await;
        seed_assistant(&pool, "dev:assistant").await;
        let first = json_body(issue(&app, &cookie, "target@example.test").await).await;
        let second = json_body(issue(&app, &cookie, "target@example.test").await).await;
        let invalidated = app
            .clone()
            .oneshot(
                Request::post("/api/auth/email/verify")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.11")
                    .body(Body::from(
                        json!({
                            "challengeId": first["challengeId"],
                            "code": first["code"]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalidated.status(), StatusCode::UNAUTHORIZED);

        for _ in 0..5 {
            let wrong = app
                .clone()
                .oneshot(
                    Request::post("/api/auth/email/verify")
                        .header("content-type", "application/json")
                        .header("x-forwarded-for", "203.0.113.11")
                        .body(Body::from(
                            json!({
                                "challengeId": second["challengeId"],
                                "code": "00000000"
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
        }

        let exhausted: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM account_assistance_otp_challenges")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(exhausted, 0);

        let expired = json_body(issue(&app, &cookie, "expired@example.test").await).await;
        sqlx::query("UPDATE account_assistance_otp_challenges SET expires_at = 0")
            .execute(&pool)
            .await
            .unwrap();
        let expired_response = app
            .clone()
            .oneshot(
                Request::post("/api/auth/email/verify")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.11")
                    .body(Body::from(
                        json!({
                            "challengeId": expired["challengeId"],
                            "code": expired["code"]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(expired_response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            json_body(expired_response).await["error"],
            "assisted_otp_invalid"
        );

        let concurrent = json_body(issue(&app, &cookie, "other@example.test").await).await;
        let request = || {
            Request::post("/api/auth/email/verify")
                .header("content-type", "application/json")
                .header("x-forwarded-for", "203.0.113.12")
                .body(Body::from(
                    json!({
                        "challengeId": concurrent["challengeId"],
                        "code": concurrent["code"]
                    })
                    .to_string(),
                ))
                .unwrap()
        };
        let (left, right) = tokio::join!(
            app.clone().oneshot(request()),
            app.clone().oneshot(request())
        );
        let statuses = [left.unwrap().status(), right.unwrap().status()];
        assert_eq!(
            statuses
                .iter()
                .filter(|status| **status == StatusCode::OK)
                .count(),
            1
        );
        assert_eq!(
            statuses
                .iter()
                .filter(|status| **status == StatusCode::UNAUTHORIZED)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn revoked_assignment_generation_cannot_revive_old_challenges_after_reclaim() {
        let (app, _, pool) = runtime().await;
        let cookie = login(&app, "Assistant").await;
        seed_assistant(&pool, "dev:assistant").await;
        let old = json_body(issue(&app, &cookie, "generation@example.test").await).await;

        super::revoke_assignment(&pool, "assistant-assignment", Utc::now().timestamp())
            .await
            .unwrap();
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        let (new_assignment_id, new_alias) =
            super::grant_assignment(&mut tx, "dev:assistant", Utc::now().timestamp())
                .await
                .unwrap();
        tx.commit().await.unwrap();
        assert_ne!(new_assignment_id, "assistant-assignment");
        assert_ne!(new_alias, "assistant-test");
        let outstanding: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM account_assistance_otp_challenges")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(outstanding, 0);

        let old_request = || {
            Request::post("/api/auth/email/verify")
                .header("content-type", "application/json")
                .header("x-forwarded-for", "203.0.113.31")
                .body(Body::from(
                    json!({
                        "challengeId": old["challengeId"],
                        "code": old["code"]
                    })
                    .to_string(),
                ))
                .unwrap()
        };
        let (left, right) = tokio::join!(
            app.clone().oneshot(old_request()),
            app.clone().oneshot(old_request())
        );
        assert_eq!(left.unwrap().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(right.unwrap().status(), StatusCode::UNAUTHORIZED);

        let new_cookie = login(&app, "Assistant").await;
        sqlx::query("UPDATE auth_sessions SET passkey_verified_at = ? WHERE user_id = ?")
            .bind(Utc::now().timestamp())
            .bind("dev:assistant")
            .execute(&pool)
            .await
            .unwrap();
        let new = json_body(issue(&app, &new_cookie, "generation@example.test").await).await;
        let verified = app
            .oneshot(
                Request::post("/api/auth/email/verify")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.32")
                    .body(Body::from(
                        json!({
                            "challengeId": new["challengeId"],
                            "code": new["code"]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(verified.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn assisted_user_projection_hides_both_admin_capabilities_on_every_user_response() {
        let (app, _, pool) = runtime().await;
        let target_cookie = login(&app, "Target").await;
        sqlx::query(
            "INSERT INTO auth_identities
             (issuer, subject, user_id, created_at, verified_email, verification_provenance)
             VALUES (?, 'target@example.test', 'dev:target', 1, 'target@example.test', 'mailbox_verified')",
        )
        .bind(EMAIL_IDENTITY_ISSUER)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO capability_assignments
             (id, user_id, capability, created_at)
             VALUES ('target-moderator', 'dev:target', 'content_moderation', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_assistant_assignments
             (id, user_id, alias, created_at)
             VALUES ('target-assistant', 'dev:target', 'target-admin', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let normal_me = app
            .clone()
            .oneshot(
                Request::get("/api/me")
                    .header("cookie", target_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let normal_capabilities = json_body(normal_me).await["capabilities"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(normal_capabilities.len(), 2);

        let assistant_cookie = login(&app, "Assistant").await;
        seed_assistant(&pool, "dev:assistant").await;
        let issued = json_body(issue(&app, &assistant_cookie, "target@example.test").await).await;
        let verified = app
            .clone()
            .oneshot(
                Request::post("/api/auth/email/verify")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.41")
                    .body(Body::from(
                        json!({
                            "challengeId": issued["challengeId"],
                            "code": issued["code"]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(verified.status(), StatusCode::OK);
        let assisted_cookie = verified
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let verified_body = json_body(verified).await;
        assert_eq!(verified_body["capabilities"], json!([]));

        let assisted_me = app
            .clone()
            .oneshot(
                Request::get("/api/me")
                    .header("cookie", &assisted_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(json_body(assisted_me).await["capabilities"], json!([]));

        let profile = app
            .clone()
            .oneshot(
                Request::put("/api/settings/profile")
                    .header("content-type", "application/json")
                    .header("cookie", &assisted_cookie)
                    .body(Body::from(json!({"nickname": "Target"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(profile.status(), StatusCode::OK);
        assert_eq!(json_body(profile).await["capabilities"], json!([]));

        let admin = app
            .oneshot(
                Request::get("/api/admin/moderation/queue")
                    .header("cookie", assisted_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(admin.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn existing_account_is_reused_and_limits_return_retry_after() {
        let (app, auth, pool) = runtime().await;
        let ordinary = app
            .clone()
            .oneshot(
                Request::post("/api/auth/email/request")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.20")
                    .body(Body::from(
                        json!({"email": "existing@example.test"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let ordinary = json_body(ordinary).await;
        let ordinary_code = auth
            .passwordless
            .captured_code("existing@example.test")
            .unwrap();
        let signed_in = app
            .clone()
            .oneshot(
                Request::post("/api/auth/email/verify")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.20")
                    .body(Body::from(
                        json!({
                            "challengeId": ordinary["challengeId"],
                            "code": ordinary_code
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let existing_cookie = signed_in
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let existing_user = json_body(signed_in).await["id"]
            .as_str()
            .unwrap()
            .to_string();

        let assistant_cookie = login(&app, "Assistant").await;
        seed_assistant(&pool, "dev:assistant").await;
        let assisted = issue(&app, &assistant_cookie, "existing@example.test").await;
        let assisted = json_body(assisted).await;
        assert_eq!(assisted["targetStatus"], "existing");
        let completed = app
            .clone()
            .oneshot(
                Request::post("/api/auth/email/verify")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.21")
                    .body(Body::from(
                        json!({
                            "challengeId": assisted["challengeId"],
                            "code": assisted["code"]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(json_body(completed).await["id"], existing_user);
        let existing_session = app
            .clone()
            .oneshot(
                Request::get("/api/me")
                    .header("cookie", existing_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(existing_session.status(), StatusCode::OK);

        for _ in 0..3 {
            assert_eq!(
                issue(&app, &assistant_cookie, "limited@example.test")
                    .await
                    .status(),
                StatusCode::OK
            );
        }
        let target_limited = issue(&app, &assistant_cookie, "limited@example.test").await;
        assert_eq!(target_limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(target_limited.headers().get("retry-after").is_some());

        let issuer_limited = issue(&app, &assistant_cookie, "sixth@example.test").await;
        assert_eq!(issuer_limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(issuer_limited.headers().get("retry-after").is_some());
    }
}
