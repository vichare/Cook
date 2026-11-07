use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_web_auth_core::{
    AttemptStore, CasOutcome, IdentityKey, IdentityMapper, LoginAttempt, LoginCallback,
    LoginCompletion, LoginConfig, LoginError, LoginRateLimiter, LoginService, RateLimitDecision,
    SessionConfig, SessionId, SessionRecord, SessionService, SessionStore, StorageError, UserId,
    VerifiedIdentity,
};
use rust_web_auth_google::GoogleOidcClient;
use rust_web_auth_http::CookiePolicy;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    config::{AuthMode, Config},
    error::{AppError, AppResult},
    models::User,
    passwordless::{
        build_passwordless, normalize_email, PasswordlessRuntime, EMAIL_IDENTITY_ISSUER,
    },
};

const RATE_LIMIT_WINDOW_SECONDS: i64 = 15 * 60;
const RATE_LIMIT_ATTEMPTS: i64 = 10;

tokio::task_local! {
    static GOOGLE_LOGIN_CONTEXT: Arc<Mutex<GoogleLoginContext>>;
}

#[derive(Default)]
struct GoogleLoginContext {
    pending: Option<PendingGoogleCompletion>,
    rejection: Option<GoogleLoginRejection>,
}

#[derive(Clone)]
struct PendingGoogleCompletion {
    identity: VerifiedIdentity,
    normalized_email: String,
    candidate_user_id: String,
}

#[derive(Clone, Copy)]
enum GoogleLoginRejection {
    NonGmailEmailAccount,
    SplitIdentity,
}

impl GoogleLoginRejection {
    fn event_type(self) -> &'static str {
        match self {
            Self::NonGmailEmailAccount => "google_link_non_gmail_unsupported",
            Self::SplitIdentity => "google_identity_split_conflict",
        }
    }
}

pub enum GoogleCompletionError {
    Login(LoginError),
    UnsafeAccountLink,
}

#[derive(Clone)]
pub struct AuthRuntime {
    pub mode: AuthMode,
    pub login: Option<Arc<LoginService>>,
    pub sessions: Arc<SessionService>,
    pub session_cookie: CookiePolicy,
    pub state_cookie: CookiePolicy,
    pub admin_reauth_cookie: CookiePolicy,
    pub passwordless: PasswordlessRuntime,
    store: Arc<SqliteAuthStore>,
}

impl AuthRuntime {
    pub fn session_cookie_name(&self) -> &str {
        self.session_cookie.name()
    }

    pub fn state_cookie_name(&self) -> &str {
        self.state_cookie.name()
    }

    pub async fn complete_google(
        &self,
        callback: LoginCallback<'_>,
        now: DateTime<Utc>,
    ) -> Result<LoginCompletion, GoogleCompletionError> {
        let login = self.login.as_ref().ok_or(GoogleCompletionError::Login(
            LoginError::InvalidConfiguration,
        ))?;
        let context = Arc::new(Mutex::new(GoogleLoginContext::default()));
        let result = GOOGLE_LOGIN_CONTEXT
            .scope(context.clone(), login.complete(callback, now))
            .await;
        let rejection = context.lock().ok().and_then(|value| value.rejection);
        if let Some(rejection) = rejection {
            self.store.record_google_rejection(rejection).await;
            return Err(GoogleCompletionError::UnsafeAccountLink);
        }
        result.map_err(GoogleCompletionError::Login)
    }
}

#[derive(Clone)]
struct SqliteAuthStore {
    pool: SqlitePool,
}

pub fn build_auth(config: &Config, pool: &SqlitePool) -> AppResult<AuthRuntime> {
    let store = Arc::new(SqliteAuthStore { pool: pool.clone() });
    let sessions = Arc::new(
        SessionService::new(
            SessionConfig::default(),
            store.clone(),
            config.session_digest_key,
        )
        .map_err(|error| AppError::configuration(error.to_string()))?,
    );
    let login = if config.auth_mode == AuthMode::Google {
        let provider = GoogleOidcClient::new(
            config.google_client_id.clone().unwrap_or_default(),
            config.google_client_secret.clone().unwrap_or_default(),
        )
        .map_err(|error| AppError::configuration(error.to_string()))?;
        let login = Arc::new(LoginService::new(
            LoginConfig::new(config.google_callback_url(), config.oauth_state_digest_key)
                .map_err(|error| AppError::configuration(error.to_string()))?,
            Arc::new(provider),
            store.clone(),
            store.clone(),
            store.clone(),
            sessions.clone(),
        ));
        spawn_attempt_cleanup(login.clone());
        Some(login)
    } else {
        None
    };
    let passwordless = build_passwordless(config, pool)?;
    Ok(AuthRuntime {
        mode: config.auth_mode.clone(),
        login,
        sessions,
        session_cookie: CookiePolicy::new(config.auth_environment(), "cook_session")
            .map_err(|error| AppError::configuration(error.to_string()))?,
        state_cookie: CookiePolicy::new(config.auth_environment(), "cook_oauth_state")
            .map_err(|error| AppError::configuration(error.to_string()))?,
        admin_reauth_cookie: CookiePolicy::new(config.auth_environment(), "cook_admin_reauth")
            .map_err(|error| AppError::configuration(error.to_string()))?,
        passwordless,
        store,
    })
}

fn spawn_attempt_cleanup(login: Arc<LoginService>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15 * 60));
        interval.tick().await;
        loop {
            interval.tick().await;
            if login.cleanup_attempts(Utc::now()).await.is_err() {
                tracing::error!("failed to clean expired authentication attempts");
            }
        }
    });
}

pub async fn upsert_development_user(pool: &SqlitePool, display_name: &str) -> AppResult<User> {
    let display_name = display_name.trim();
    if display_name.is_empty() {
        return Err(AppError::Validation("development user is required".into()));
    }
    let slug = display_name.to_lowercase().replace([' ', '@'], "-");
    let user = User {
        id: format!("dev:{slug}"),
        email: format!("{slug}@example.test"),
        display_name: display_name.to_string(),
        nickname: None,
        onboarding_completed: false,
        locale: None,
        capabilities: Vec::new(),
        assisted_session: None,
        last_assisted_login_at: None,
    };
    sqlx::query(
        "INSERT INTO users (id, email, profile_email, display_name) VALUES (?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET profile_email = excluded.profile_email,
           display_name = excluded.display_name",
    )
    .bind(&user.id)
    .bind(&user.email)
    .bind(&user.email)
    .bind(&user.display_name)
    .execute(pool)
    .await?;
    Ok(user)
}

type UserRow = (String, String, String, Option<String>, bool, Option<String>);

pub async fn load_user(pool: &SqlitePool, user_id: &UserId) -> AppResult<User> {
    let row: Option<UserRow> = sqlx::query_as(
        "SELECT id, COALESCE(profile_email, email), display_name, public_nickname,
           profile_onboarding_completed, locale
         FROM users WHERE id = ?",
    )
    .bind(user_id.as_str())
    .fetch_optional(pool)
    .await?;
    let (id, email, display_name, nickname, onboarding_completed, locale) =
        row.ok_or(AppError::Unauthorized)?;
    let mut capabilities = if sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(
           SELECT 1 FROM capability_assignments
           WHERE user_id = ? AND capability = 'content_moderation' AND revoked_at IS NULL
         )",
    )
    .bind(&id)
    .fetch_one(pool)
    .await?
        != 0
    {
        vec!["content_moderation".into()]
    } else {
        Vec::new()
    };
    if sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(
           SELECT 1 FROM account_assistant_assignments
           WHERE user_id = ? AND revoked_at IS NULL
         )",
    )
    .bind(&id)
    .fetch_one(pool)
    .await?
        != 0
    {
        capabilities.push("account_assistance".into());
    }
    Ok(User {
        id,
        email,
        display_name,
        nickname,
        onboarding_completed,
        locale,
        capabilities,
        assisted_session: None,
        last_assisted_login_at: None,
    })
}

#[async_trait]
impl AttemptStore for SqliteAuthStore {
    async fn create(&self, attempt: LoginAttempt) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO auth_attempts
             (state_digest, pkce_verifier, nonce, redirect_uri, created_at, expires_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(attempt.state_digest.as_slice())
        .bind(attempt.pkce_verifier)
        .bind(attempt.nonce)
        .bind(attempt.redirect_uri)
        .bind(attempt.created_at.timestamp())
        .bind(attempt.expires_at.timestamp())
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    async fn consume_and_delete(
        &self,
        state_digest: &[u8; 32],
        now: DateTime<Utc>,
    ) -> Result<Option<LoginAttempt>, StorageError> {
        let row = sqlx::query(
            "DELETE FROM auth_attempts
             WHERE state_digest = ? AND created_at <= ? AND expires_at > ?
             RETURNING state_digest, pkce_verifier, nonce, redirect_uri, created_at, expires_at",
        )
        .bind(state_digest.as_slice())
        .bind(now.timestamp())
        .bind(now.timestamp())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        row.map(login_attempt_from_row).transpose()
    }

    async fn cleanup(&self, now: DateTime<Utc>) -> Result<usize, StorageError> {
        let result = sqlx::query("DELETE FROM auth_attempts WHERE expires_at <= ?")
            .bind(now.timestamp())
            .execute(&self.pool)
            .await
            .map_err(storage_error)?;
        usize::try_from(result.rows_affected()).map_err(|_| StorageError)
    }
}

#[async_trait]
impl IdentityMapper for SqliteAuthStore {
    async fn resolve_user(&self, identity: &VerifiedIdentity) -> Result<UserId, StorageError> {
        let normalized_email = normalize_email(&identity.verified_email).map_err(storage_error)?;
        let existing_google_user: Option<String> = sqlx::query_scalar(
            "SELECT user_id FROM auth_identities WHERE issuer = ? AND subject = ?",
        )
        .bind(&identity.key.issuer)
        .bind(&identity.key.subject)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        let email_user: Option<String> = sqlx::query_scalar(
            "SELECT user_id FROM auth_identities
             WHERE issuer = ? AND subject = ? AND verification_provenance = 'mailbox_verified'",
        )
        .bind(EMAIL_IDENTITY_ISSUER)
        .bind(&normalized_email)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;

        let user_id = match (existing_google_user, email_user) {
            (Some(google_user), Some(email_user)) if google_user != email_user => {
                set_google_rejection(GoogleLoginRejection::SplitIdentity);
                return Err(StorageError);
            }
            (Some(google_user), _) => google_user,
            (None, Some(email_user)) if is_gmail_address(&normalized_email) => email_user,
            (None, Some(_)) => {
                set_google_rejection(GoogleLoginRejection::NonGmailEmailAccount);
                return Err(StorageError);
            }
            (None, None) => Uuid::new_v4().to_string(),
        };

        GOOGLE_LOGIN_CONTEXT
            .try_with(|context| {
                let mut context = context.lock().map_err(|_| StorageError)?;
                context.pending = Some(PendingGoogleCompletion {
                    identity: identity.clone(),
                    normalized_email,
                    candidate_user_id: user_id.clone(),
                });
                Ok::<(), StorageError>(())
            })
            .map_err(|_| StorageError)??;
        UserId::new(user_id).map_err(|_| StorageError)
    }

    async fn find_user(&self, key: &IdentityKey) -> Result<Option<UserId>, StorageError> {
        let user_id: Option<String> = sqlx::query_scalar(
            "SELECT user_id FROM auth_identities WHERE issuer = ? AND subject = ?",
        )
        .bind(&key.issuer)
        .bind(&key.subject)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        user_id
            .map(UserId::new)
            .transpose()
            .map_err(|_| StorageError)
    }
}

#[async_trait]
impl SessionStore for SqliteAuthStore {
    async fn create(&self, session: SessionRecord) -> Result<(), StorageError> {
        let pending = GOOGLE_LOGIN_CONTEXT
            .try_with(|context| {
                context
                    .lock()
                    .ok()
                    .and_then(|mut value| value.pending.take())
            })
            .ok()
            .flatten();
        if let Some(pending) = pending {
            return self.create_google_session(session, pending).await;
        }
        sqlx::query(
            "INSERT INTO auth_sessions
             (session_id, user_id, token_digest, created_at, last_activity_at, expires_at,
              revoked_at, version, authorization_version)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, (
               SELECT authorization_version FROM users WHERE id = ?
             ))",
        )
        .bind(session.session_id.to_string())
        .bind(session.user_id.as_str())
        .bind(session.token_digest.as_slice())
        .bind(session.created_at.timestamp())
        .bind(session.last_activity_at.timestamp())
        .bind(session.expires_at.timestamp())
        .bind(session.revoked_at.map(|value| value.timestamp()))
        .bind(i64::try_from(session.version).map_err(|_| StorageError)?)
        .bind(session.user_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    async fn find_by_token_digest(
        &self,
        token_digest: &[u8; 32],
    ) -> Result<Option<SessionRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT s.session_id, s.user_id, s.token_digest, s.created_at, s.last_activity_at,
                    s.expires_at, s.revoked_at, s.version
             FROM auth_sessions s
             JOIN users u ON u.id = s.user_id
             WHERE s.token_digest = ? AND s.authorization_version = u.authorization_version
               AND (
                 s.auth_method != 'admin_assisted_otp'
                 OR EXISTS (
                   SELECT 1 FROM account_assistant_assignments a
                   WHERE a.id = s.issuer_assignment_id AND a.revoked_at IS NULL
                 )
               )",
        )
        .bind(token_digest.as_slice())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        row.map(session_from_row).transpose()
    }

    async fn find_by_id(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT s.session_id, s.user_id, s.token_digest, s.created_at, s.last_activity_at,
                    s.expires_at, s.revoked_at, s.version
             FROM auth_sessions s
             JOIN users u ON u.id = s.user_id
             WHERE s.session_id = ? AND s.authorization_version = u.authorization_version
               AND (
                 s.auth_method != 'admin_assisted_otp'
                 OR EXISTS (
                   SELECT 1 FROM account_assistant_assignments a
                   WHERE a.id = s.issuer_assignment_id AND a.revoked_at IS NULL
                 )
               )",
        )
        .bind(session_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        row.map(session_from_row).transpose()
    }

    async fn replace_if_unchanged(
        &self,
        expected: &SessionRecord,
        replacement: SessionRecord,
    ) -> Result<CasOutcome, StorageError> {
        let result = sqlx::query(
            "UPDATE auth_sessions SET last_activity_at = ?,
               expires_at = CASE
                 WHEN auth_method = 'admin_assisted_otp'
                 THEN MIN(?, authenticated_at + 3600)
                 ELSE ? END,
               revoked_at = ?, version = ?
             WHERE session_id = ? AND version = ?",
        )
        .bind(replacement.last_activity_at.timestamp())
        .bind(replacement.expires_at.timestamp())
        .bind(replacement.expires_at.timestamp())
        .bind(replacement.revoked_at.map(|value| value.timestamp()))
        .bind(i64::try_from(replacement.version).map_err(|_| StorageError)?)
        .bind(expected.session_id.to_string())
        .bind(i64::try_from(expected.version).map_err(|_| StorageError)?)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(if result.rows_affected() == 1 {
            CasOutcome::Updated
        } else {
            CasOutcome::Stale
        })
    }

    async fn revoke_all_for_user(
        &self,
        user_id: &UserId,
        revoked_at: DateTime<Utc>,
    ) -> Result<usize, StorageError> {
        let result = sqlx::query(
            "UPDATE auth_sessions SET revoked_at = ?, version = version + 1
             WHERE user_id = ? AND revoked_at IS NULL",
        )
        .bind(revoked_at.timestamp())
        .bind(user_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        usize::try_from(result.rows_affected()).map_err(|_| StorageError)
    }
}

impl SqliteAuthStore {
    async fn create_google_session(
        &self,
        session: SessionRecord,
        pending: PendingGoogleCompletion,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await.map_err(storage_error)?;
        let google_user: Option<String> = sqlx::query_scalar(
            "SELECT user_id FROM auth_identities WHERE issuer = ? AND subject = ?",
        )
        .bind(&pending.identity.key.issuer)
        .bind(&pending.identity.key.subject)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage_error)?;
        let email_user: Option<String> = sqlx::query_scalar(
            "SELECT user_id FROM auth_identities
             WHERE issuer = ? AND subject = ? AND verification_provenance = 'mailbox_verified'",
        )
        .bind(EMAIL_IDENTITY_ISSUER)
        .bind(&pending.normalized_email)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage_error)?;

        let user_id = match (google_user, email_user) {
            (Some(google_user), Some(email_user)) if google_user != email_user => {
                set_google_rejection(GoogleLoginRejection::SplitIdentity);
                return Err(StorageError);
            }
            (Some(google_user), _) => google_user,
            (None, Some(email_user)) if is_gmail_address(&pending.normalized_email) => email_user,
            (None, Some(_)) => {
                set_google_rejection(GoogleLoginRejection::NonGmailEmailAccount);
                return Err(StorageError);
            }
            (None, None) => {
                sqlx::query(
                    "INSERT INTO users (id, email, profile_email, display_name)
                     VALUES (?, ?, ?, ?)",
                )
                .bind(&pending.candidate_user_id)
                .bind(format!(
                    "oidc-{}@identity.invalid",
                    pending.candidate_user_id
                ))
                .bind(&pending.normalized_email)
                .bind(
                    pending
                        .identity
                        .display_name
                        .as_deref()
                        .unwrap_or(&pending.normalized_email),
                )
                .execute(&mut *transaction)
                .await
                .map_err(storage_error)?;
                pending.candidate_user_id.clone()
            }
        };

        sqlx::query(
            "INSERT INTO auth_identities
             (issuer, subject, user_id, created_at, verified_email, verification_provenance)
             VALUES (?, ?, ?, ?, ?, 'provider_verified')
             ON CONFLICT(issuer, subject) DO NOTHING",
        )
        .bind(&pending.identity.key.issuer)
        .bind(&pending.identity.key.subject)
        .bind(&user_id)
        .bind(session.created_at.timestamp())
        .bind(&pending.normalized_email)
        .execute(&mut *transaction)
        .await
        .map_err(storage_error)?;
        let mapped_user: String = sqlx::query_scalar(
            "SELECT user_id FROM auth_identities WHERE issuer = ? AND subject = ?",
        )
        .bind(&pending.identity.key.issuer)
        .bind(&pending.identity.key.subject)
        .fetch_one(&mut *transaction)
        .await
        .map_err(storage_error)?;
        if mapped_user != user_id {
            set_google_rejection(GoogleLoginRejection::SplitIdentity);
            return Err(StorageError);
        }
        sqlx::query(
            "UPDATE auth_identities
             SET verified_email = ?, verification_provenance = 'provider_verified'
             WHERE issuer = ? AND subject = ? AND user_id = ?",
        )
        .bind(&pending.normalized_email)
        .bind(&pending.identity.key.issuer)
        .bind(&pending.identity.key.subject)
        .bind(&user_id)
        .execute(&mut *transaction)
        .await
        .map_err(storage_error)?;
        sqlx::query("UPDATE users SET profile_email = ?, display_name = ? WHERE id = ?")
            .bind(&pending.normalized_email)
            .bind(
                pending
                    .identity
                    .display_name
                    .as_deref()
                    .unwrap_or(&pending.normalized_email),
            )
            .bind(&user_id)
            .execute(&mut *transaction)
            .await
            .map_err(storage_error)?;
        insert_session(&mut transaction, &session, &user_id).await?;
        transaction.commit().await.map_err(storage_error)
    }

    async fn record_google_rejection(&self, rejection: GoogleLoginRejection) {
        let event_type = rejection.event_type();
        if sqlx::query(
            "INSERT INTO auth_identity_audit_events (id, event_type, created_at)
             VALUES (?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(event_type)
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .is_err()
        {
            tracing::error!("failed to persist authentication identity audit event");
        }
        tracing::warn!(event_type, "Google identity linking was rejected");
    }
}

async fn insert_session(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session: &SessionRecord,
    user_id: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO auth_sessions
         (session_id, user_id, token_digest, created_at, last_activity_at, expires_at,
          revoked_at, version, authorization_version)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, (
           SELECT authorization_version FROM users WHERE id = ?
         ))",
    )
    .bind(session.session_id.to_string())
    .bind(user_id)
    .bind(session.token_digest.as_slice())
    .bind(session.created_at.timestamp())
    .bind(session.last_activity_at.timestamp())
    .bind(session.expires_at.timestamp())
    .bind(session.revoked_at.map(|value| value.timestamp()))
    .bind(i64::try_from(session.version).map_err(|_| StorageError)?)
    .bind(user_id)
    .execute(&mut **transaction)
    .await
    .map_err(storage_error)?;
    Ok(())
}

fn set_google_rejection(rejection: GoogleLoginRejection) {
    let _ = GOOGLE_LOGIN_CONTEXT.try_with(|context| {
        if let Ok(mut context) = context.lock() {
            context.rejection = Some(rejection);
        }
    });
}

fn is_gmail_address(normalized_email: &str) -> bool {
    normalized_email
        .rsplit_once('@')
        .is_some_and(|(_, domain)| domain == "gmail.com")
}

#[async_trait]
impl LoginRateLimiter for SqliteAuthStore {
    async fn check(
        &self,
        key: &str,
        now: DateTime<Utc>,
    ) -> Result<RateLimitDecision, StorageError> {
        let window_cutoff = now.timestamp() - RATE_LIMIT_WINDOW_SECONDS;
        let count: i64 = sqlx::query_scalar(
            "INSERT INTO auth_rate_limits (rate_key, window_started_at, attempt_count)
             VALUES (?, ?, 1)
             ON CONFLICT(rate_key) DO UPDATE SET
               window_started_at = CASE
                 WHEN auth_rate_limits.window_started_at <= ? THEN excluded.window_started_at
                 ELSE auth_rate_limits.window_started_at
               END,
               attempt_count = CASE
                 WHEN auth_rate_limits.window_started_at <= ? THEN 1
                 ELSE auth_rate_limits.attempt_count + 1
               END
             RETURNING attempt_count",
        )
        .bind(key)
        .bind(now.timestamp())
        .bind(window_cutoff)
        .bind(window_cutoff)
        .fetch_one(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(if count <= RATE_LIMIT_ATTEMPTS {
            RateLimitDecision::Allowed
        } else {
            RateLimitDecision::Rejected
        })
    }
}

fn login_attempt_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LoginAttempt, StorageError> {
    Ok(LoginAttempt {
        state_digest: array_32(row.try_get("state_digest").map_err(storage_error)?)?,
        pkce_verifier: row.try_get("pkce_verifier").map_err(storage_error)?,
        nonce: row.try_get("nonce").map_err(storage_error)?,
        redirect_uri: row.try_get("redirect_uri").map_err(storage_error)?,
        created_at: timestamp(row.try_get("created_at").map_err(storage_error)?)?,
        expires_at: timestamp(row.try_get("expires_at").map_err(storage_error)?)?,
    })
}

fn session_from_row(row: sqlx::sqlite::SqliteRow) -> Result<SessionRecord, StorageError> {
    let session_id: String = row.try_get("session_id").map_err(storage_error)?;
    let user_id: String = row.try_get("user_id").map_err(storage_error)?;
    let version: i64 = row.try_get("version").map_err(storage_error)?;
    Ok(SessionRecord {
        session_id: session_id.parse().map_err(|_| StorageError)?,
        user_id: UserId::new(user_id).map_err(|_| StorageError)?,
        token_digest: array_32(row.try_get("token_digest").map_err(storage_error)?)?,
        created_at: timestamp(row.try_get("created_at").map_err(storage_error)?)?,
        last_activity_at: timestamp(row.try_get("last_activity_at").map_err(storage_error)?)?,
        expires_at: timestamp(row.try_get("expires_at").map_err(storage_error)?)?,
        revoked_at: row
            .try_get::<Option<i64>, _>("revoked_at")
            .map_err(storage_error)?
            .map(timestamp)
            .transpose()?,
        version: u64::try_from(version).map_err(|_| StorageError)?,
    })
}

fn timestamp(value: i64) -> Result<DateTime<Utc>, StorageError> {
    DateTime::from_timestamp(value, 0).ok_or(StorageError)
}

fn array_32(value: Vec<u8>) -> Result<[u8; 32], StorageError> {
    value.try_into().map_err(|_| StorageError)
}

fn storage_error<T>(_: T) -> StorageError {
    StorageError
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn store() -> SqliteAuthStore {
        SqliteAuthStore {
            pool: db::connect("sqlite::memory:").await.unwrap(),
        }
    }

    async fn complete_google_identity(
        store: &SqliteAuthStore,
        identity: VerifiedIdentity,
    ) -> Result<UserId, StorageError> {
        let sessions =
            SessionService::new(SessionConfig::default(), Arc::new(store.clone()), [23; 32])
                .unwrap();
        let context = Arc::new(Mutex::new(GoogleLoginContext::default()));
        GOOGLE_LOGIN_CONTEXT
            .scope(context, async {
                let user_id = store.resolve_user(&identity).await?;
                sessions
                    .issue(user_id, Utc::now())
                    .await
                    .map_err(|_| StorageError)?;
                let mapped = store.find_user(&identity.key).await?.ok_or(StorageError)?;
                Ok(mapped)
            })
            .await
    }

    async fn seed_email_user(store: &SqliteAuthStore, user_id: &str, email: &str) {
        sqlx::query(
            "INSERT INTO users (id, email, profile_email, display_name, passkey_user_handle)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(format!("{user_id}@identity.invalid"))
        .bind(email)
        .bind(user_id)
        .bind(format!("handle-{user_id}"))
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO auth_identities
             (issuer, subject, user_id, created_at, verified_email)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(EMAIL_IDENTITY_ISSUER)
        .bind(email)
        .bind(user_id)
        .bind(Utc::now().timestamp())
        .bind(email)
        .execute(&store.pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn attempt_is_consumed_exactly_once() {
        let store = store().await;
        let now = Utc::now();
        let attempt = LoginAttempt {
            state_digest: [1; 32],
            pkce_verifier: "verifier".into(),
            nonce: "nonce".into(),
            redirect_uri: "http://localhost/callback".into(),
            created_at: now,
            expires_at: now + chrono::Duration::minutes(10),
        };
        AttemptStore::create(&store, attempt).await.unwrap();
        assert!(store
            .consume_and_delete(&[1; 32], now)
            .await
            .unwrap()
            .is_some());
        assert!(store
            .consume_and_delete(&[1; 32], now)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn identity_mapping_uses_issuer_and_subject_not_email() {
        let store = store().await;
        let first = complete_google_identity(
            &store,
            VerifiedIdentity {
                key: IdentityKey {
                    issuer: "issuer-a".into(),
                    subject: "subject".into(),
                },
                verified_email: "first@example.test".into(),
                display_name: Some("First".into()),
            },
        )
        .await
        .unwrap();
        let updated = complete_google_identity(
            &store,
            VerifiedIdentity {
                key: IdentityKey {
                    issuer: "issuer-a".into(),
                    subject: "subject".into(),
                },
                verified_email: "updated@example.test".into(),
                display_name: Some("Updated".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(first, updated);
    }

    #[tokio::test]
    async fn distinct_identities_may_share_the_same_profile_email() {
        let store = store().await;
        let first = complete_google_identity(
            &store,
            VerifiedIdentity {
                key: IdentityKey {
                    issuer: "issuer-a".into(),
                    subject: "subject-a".into(),
                },
                verified_email: "shared@example.test".into(),
                display_name: None,
            },
        )
        .await
        .unwrap();
        let second = complete_google_identity(
            &store,
            VerifiedIdentity {
                key: IdentityKey {
                    issuer: "issuer-a".into(),
                    subject: "subject-b".into(),
                },
                verified_email: "shared@example.test".into(),
                display_name: None,
            },
        )
        .await
        .unwrap();
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn gmail_email_identity_is_atomically_reused_for_google_login() {
        let store = store().await;
        seed_email_user(&store, "email-user", "user@gmail.com").await;
        sqlx::query(
            "INSERT INTO passkey_credentials
             (id, user_id, credential_id, credential_json, name, created_at)
             VALUES ('existing-passkey', 'email-user', X'010203', '{}', 'Existing key', ?)",
        )
        .bind(Utc::now().timestamp())
        .execute(&store.pool)
        .await
        .unwrap();
        let resolved = complete_google_identity(
            &store,
            VerifiedIdentity {
                key: IdentityKey {
                    issuer: "https://accounts.google.com".into(),
                    subject: "google-subject".into(),
                },
                verified_email: " USER@GMAIL.COM ".into(),
                display_name: Some("Google User".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(resolved.as_str(), "email-user");
        let handle: String =
            sqlx::query_scalar("SELECT passkey_user_handle FROM users WHERE id = 'email-user'")
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert_eq!(handle, "handle-email-user");
        let credential_owner: String = sqlx::query_scalar(
            "SELECT user_id FROM passkey_credentials WHERE id = 'existing-passkey'",
        )
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(credential_owner, "email-user");
        let session_owner: String = sqlx::query_scalar("SELECT user_id FROM auth_sessions LIMIT 1")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(session_owner, "email-user");
    }

    #[tokio::test]
    async fn admin_assisted_gmail_identity_is_not_auto_linked_to_google() {
        let store = store().await;
        seed_email_user(&store, "assisted-user", "assisted@gmail.com").await;
        sqlx::query(
            "UPDATE auth_identities
             SET verification_provenance = 'admin_assisted', verified_email = NULL
             WHERE issuer = ? AND subject = 'assisted@gmail.com'",
        )
        .bind(EMAIL_IDENTITY_ISSUER)
        .execute(&store.pool)
        .await
        .unwrap();
        let resolved = complete_google_identity(
            &store,
            VerifiedIdentity {
                key: IdentityKey {
                    issuer: "https://accounts.google.com".into(),
                    subject: "assisted-google".into(),
                },
                verified_email: "assisted@gmail.com".into(),
                display_name: None,
            },
        )
        .await
        .unwrap();
        assert_ne!(resolved.as_str(), "assisted-user");
        let assisted_owner: String = sqlx::query_scalar(
            "SELECT user_id FROM auth_identities WHERE issuer = ? AND subject = 'assisted@gmail.com'",
        )
        .bind(EMAIL_IDENTITY_ISSUER)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(assisted_owner, "assisted-user");
    }

    #[tokio::test]
    async fn non_gmail_email_identity_rejects_google_linking_without_creating_a_user() {
        let store = store().await;
        seed_email_user(&store, "email-user", "user@example.com").await;
        let context = Arc::new(Mutex::new(GoogleLoginContext::default()));
        let result = GOOGLE_LOGIN_CONTEXT
            .scope(context.clone(), async {
                store
                    .resolve_user(&VerifiedIdentity {
                        key: IdentityKey {
                            issuer: "https://accounts.google.com".into(),
                            subject: "google-subject".into(),
                        },
                        verified_email: "user@example.com".into(),
                        display_name: None,
                    })
                    .await
            })
            .await;
        assert!(result.is_err());
        assert!(matches!(
            context.lock().unwrap().rejection,
            Some(GoogleLoginRejection::NonGmailEmailAccount)
        ));
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM auth_identities WHERE issuer != ?")
                .bind(EMAIL_IDENTITY_ISSUER)
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn gmail_dot_and_plus_variants_are_not_equivalent() {
        let store = store().await;
        seed_email_user(&store, "email-user", "username@gmail.com").await;
        let dotted = complete_google_identity(
            &store,
            VerifiedIdentity {
                key: IdentityKey {
                    issuer: "https://accounts.google.com".into(),
                    subject: "dotted".into(),
                },
                verified_email: "user.name@gmail.com".into(),
                display_name: None,
            },
        )
        .await
        .unwrap();
        let tagged = complete_google_identity(
            &store,
            VerifiedIdentity {
                key: IdentityKey {
                    issuer: "https://accounts.google.com".into(),
                    subject: "tagged".into(),
                },
                verified_email: "username+tag@gmail.com".into(),
                display_name: None,
            },
        )
        .await
        .unwrap();
        assert_ne!(dotted.as_str(), "email-user");
        assert_ne!(tagged.as_str(), "email-user");
        assert_ne!(dotted, tagged);
    }

    #[tokio::test]
    async fn preexisting_split_identity_is_rejected_without_merging() {
        let store = store().await;
        seed_email_user(&store, "email-user", "user@gmail.com").await;
        sqlx::query(
            "INSERT INTO users (id, email, profile_email, display_name)
             VALUES ('google-user', 'google@identity.invalid', 'user@gmail.com', 'Google')",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO auth_identities
             (issuer, subject, user_id, created_at, verified_email)
             VALUES (?, 'subject', 'google-user', ?, 'user@gmail.com')",
        )
        .bind(crate::passwordless::GOOGLE_IDENTITY_ISSUER)
        .bind(Utc::now().timestamp())
        .execute(&store.pool)
        .await
        .unwrap();
        let context = Arc::new(Mutex::new(GoogleLoginContext::default()));
        let result = GOOGLE_LOGIN_CONTEXT
            .scope(context.clone(), async {
                store
                    .resolve_user(&VerifiedIdentity {
                        key: IdentityKey {
                            issuer: "https://accounts.google.com".into(),
                            subject: "subject".into(),
                        },
                        verified_email: "user@gmail.com".into(),
                        display_name: None,
                    })
                    .await
            })
            .await;
        assert!(result.is_err());
        assert!(matches!(
            context.lock().unwrap().rejection,
            Some(GoogleLoginRejection::SplitIdentity)
        ));
    }

    #[tokio::test]
    async fn concurrent_google_callbacks_converge_on_one_identity_and_user() {
        let store = store().await;
        seed_email_user(&store, "email-user", "user@gmail.com").await;
        let identity = VerifiedIdentity {
            key: IdentityKey {
                issuer: "https://accounts.google.com".into(),
                subject: "concurrent-subject".into(),
            },
            verified_email: "user@gmail.com".into(),
            display_name: Some("Concurrent User".into()),
        };
        let first_store = store.clone();
        let first_identity = identity.clone();
        let second_store = store.clone();
        let second_identity = identity.clone();
        let (first, second) = tokio::join!(
            complete_google_identity(&first_store, first_identity),
            complete_google_identity(&second_store, second_identity)
        );
        assert_eq!(first.unwrap().as_str(), "email-user");
        assert_eq!(second.unwrap().as_str(), "email-user");
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM auth_identities
             WHERE issuer = ? AND subject = ? AND user_id = 'email-user'",
        )
        .bind(&identity.key.issuer)
        .bind(&identity.key.subject)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
        let sessions: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM auth_sessions WHERE user_id = 'email-user'")
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert_eq!(sessions, 2);
    }

    #[tokio::test]
    async fn rate_limiter_rejects_after_ten_attempts() {
        let store = store().await;
        let now = Utc::now();
        for _ in 0..RATE_LIMIT_ATTEMPTS {
            assert_eq!(
                store.check("client", now).await.unwrap(),
                RateLimitDecision::Allowed
            );
        }
        assert_eq!(
            store.check("client", now).await.unwrap(),
            RateLimitDecision::Rejected
        );
    }
}
