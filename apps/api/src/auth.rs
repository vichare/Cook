use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_web_auth_core::{
    AttemptStore, CasOutcome, IdentityKey, IdentityMapper, LoginAttempt, LoginConfig,
    LoginRateLimiter, LoginService, RateLimitDecision, SessionConfig, SessionId, SessionRecord,
    SessionService, SessionStore, StorageError, UserId, VerifiedIdentity,
};
use rust_web_auth_google::GoogleOidcClient;
use rust_web_auth_http::CookiePolicy;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    config::{AuthMode, Config},
    error::{AppError, AppResult},
    models::User,
};

const RATE_LIMIT_WINDOW_SECONDS: i64 = 15 * 60;
const RATE_LIMIT_ATTEMPTS: i64 = 10;

#[derive(Clone)]
pub struct AuthRuntime {
    pub mode: AuthMode,
    pub login: Option<Arc<LoginService>>,
    pub sessions: Arc<SessionService>,
    pub session_cookie: CookiePolicy,
    pub state_cookie: CookiePolicy,
}

impl AuthRuntime {
    pub fn session_cookie_name(&self) -> &str {
        self.session_cookie.name()
    }

    pub fn state_cookie_name(&self) -> &str {
        self.state_cookie.name()
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
    Ok(AuthRuntime {
        mode: config.auth_mode.clone(),
        login,
        sessions,
        session_cookie: CookiePolicy::new(config.auth_environment(), "cook_session")
            .map_err(|error| AppError::configuration(error.to_string()))?,
        state_cookie: CookiePolicy::new(config.auth_environment(), "cook_oauth_state")
            .map_err(|error| AppError::configuration(error.to_string()))?,
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

pub async fn load_user(pool: &SqlitePool, user_id: &UserId) -> AppResult<User> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT id, COALESCE(profile_email, email), display_name FROM users WHERE id = ?",
    )
    .bind(user_id.as_str())
    .fetch_optional(pool)
    .await?;
    let (id, email, display_name) = row.ok_or(AppError::Unauthorized)?;
    Ok(User {
        id,
        email,
        display_name,
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
        let mut transaction = self.pool.begin().await.map_err(storage_error)?;
        if let Some((user_id,)) = sqlx::query_as::<_, (String,)>(
            "SELECT user_id FROM auth_identities WHERE issuer = ? AND subject = ?",
        )
        .bind(&identity.key.issuer)
        .bind(&identity.key.subject)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage_error)?
        {
            sqlx::query("UPDATE users SET profile_email = ?, display_name = ? WHERE id = ?")
                .bind(&identity.verified_email)
                .bind(
                    identity
                        .display_name
                        .as_deref()
                        .unwrap_or(&identity.verified_email),
                )
                .bind(&user_id)
                .execute(&mut *transaction)
                .await
                .map_err(storage_error)?;
            transaction.commit().await.map_err(storage_error)?;
            return UserId::new(user_id).map_err(|_| StorageError);
        }

        let candidate = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO users (id, email, profile_email, display_name) VALUES (?, ?, ?, ?)",
        )
        .bind(&candidate)
        .bind(format!("oidc-{candidate}@identity.invalid"))
        .bind(&identity.verified_email)
        .bind(
            identity
                .display_name
                .as_deref()
                .unwrap_or(&identity.verified_email),
        )
        .execute(&mut *transaction)
        .await
        .map_err(storage_error)?;
        let inserted = sqlx::query(
            "INSERT INTO auth_identities (issuer, subject, user_id, created_at)
             VALUES (?, ?, ?, ?) ON CONFLICT(issuer, subject) DO NOTHING",
        )
        .bind(&identity.key.issuer)
        .bind(&identity.key.subject)
        .bind(&candidate)
        .bind(Utc::now().timestamp())
        .execute(&mut *transaction)
        .await
        .map_err(storage_error)?;
        let user_id = if inserted.rows_affected() == 1 {
            candidate
        } else {
            sqlx::query("DELETE FROM users WHERE id = ?")
                .bind(&candidate)
                .execute(&mut *transaction)
                .await
                .map_err(storage_error)?;
            sqlx::query_scalar(
                "SELECT user_id FROM auth_identities WHERE issuer = ? AND subject = ?",
            )
            .bind(&identity.key.issuer)
            .bind(&identity.key.subject)
            .fetch_one(&mut *transaction)
            .await
            .map_err(storage_error)?
        };
        transaction.commit().await.map_err(storage_error)?;
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
        sqlx::query(
            "INSERT INTO auth_sessions
             (session_id, user_id, token_digest, created_at, last_activity_at, expires_at, revoked_at, version)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session.session_id.to_string())
        .bind(session.user_id.as_str())
        .bind(session.token_digest.as_slice())
        .bind(session.created_at.timestamp())
        .bind(session.last_activity_at.timestamp())
        .bind(session.expires_at.timestamp())
        .bind(session.revoked_at.map(|value| value.timestamp()))
        .bind(i64::try_from(session.version).map_err(|_| StorageError)?)
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
            "SELECT session_id, user_id, token_digest, created_at, last_activity_at,
                    expires_at, revoked_at, version
             FROM auth_sessions WHERE token_digest = ?",
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
            "SELECT session_id, user_id, token_digest, created_at, last_activity_at,
                    expires_at, revoked_at, version
             FROM auth_sessions WHERE session_id = ?",
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
            "UPDATE auth_sessions SET last_activity_at = ?, expires_at = ?, revoked_at = ?, version = ?
             WHERE session_id = ? AND version = ?",
        )
        .bind(replacement.last_activity_at.timestamp())
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
        let first = store
            .resolve_user(&VerifiedIdentity {
                key: IdentityKey {
                    issuer: "issuer-a".into(),
                    subject: "subject".into(),
                },
                verified_email: "first@example.test".into(),
                display_name: Some("First".into()),
            })
            .await
            .unwrap();
        let updated = store
            .resolve_user(&VerifiedIdentity {
                key: IdentityKey {
                    issuer: "issuer-a".into(),
                    subject: "subject".into(),
                },
                verified_email: "updated@example.test".into(),
                display_name: Some("Updated".into()),
            })
            .await
            .unwrap();
        assert_eq!(first, updated);
    }

    #[tokio::test]
    async fn distinct_identities_may_share_the_same_profile_email() {
        let store = store().await;
        let first = store
            .resolve_user(&VerifiedIdentity {
                key: IdentityKey {
                    issuer: "issuer-a".into(),
                    subject: "subject-a".into(),
                },
                verified_email: "shared@example.test".into(),
                display_name: None,
            })
            .await
            .unwrap();
        let second = store
            .resolve_user(&VerifiedIdentity {
                key: IdentityKey {
                    issuer: "issuer-a".into(),
                    subject: "subject-b".into(),
                },
                verified_email: "shared@example.test".into(),
                display_name: None,
            })
            .await
            .unwrap();
        assert_ne!(first, second);
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
