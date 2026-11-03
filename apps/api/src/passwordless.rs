use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use lettre::{
    message::Mailbox, transport::smtp::authentication::Credentials, AsyncSmtpTransport,
    AsyncTransport, Message, Tokio1Executor,
};
use rust_web_auth_core::UserId;
use rust_web_auth_email_otp::{
    EmailAddress, EmailDeliveryError, EmailOtpAttempt, EmailOtpAttemptStore, EmailOtpConfig,
    EmailOtpCreateOutcome, EmailOtpError, EmailOtpMessage, EmailOtpPurpose,
    EmailOtpRateLimitDecision, EmailOtpRateLimiter, EmailOtpRequestRateLimitContext,
    EmailOtpService, EmailOtpStorageError, EmailOtpVerifyOutcome, EmailOtpVerifyRateLimitContext,
    EmailSender,
};
use rust_web_auth_http::CookiePolicy;
use rust_web_auth_passkey::{
    AuthenticationResult, CeremonyState, CeremonyStore, CeremonyTokenDigest, CredentialID, Passkey,
    PasskeyConfig, PasskeyCredentialStore, PasskeyError, PasskeyRegistrationOutcome,
    PasskeyService, UserPasskeys, WebauthnUserHandle,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::{
    config::{AppEnvironment, Config, SmtpConfig, SmtpTlsMode},
    error::{AppError, AppResult},
};

pub const EMAIL_IDENTITY_ISSUER: &str = "urn:cook:email-otp";
pub const GOOGLE_IDENTITY_ISSUER: &str = "https://accounts.google.com";
pub const EMAIL_SIGN_IN_PURPOSE: &str = "sign-in";
pub const EMAIL_LINK_PURPOSE: &str = "link-account";

const RATE_WINDOW_SECONDS: i64 = 15 * 60;
const REQUEST_EMAIL_LIMIT: i64 = 5;
const REQUEST_IP_LIMIT: i64 = 30;
const VERIFY_CHALLENGE_LIMIT: i64 = 10;
const VERIFY_IP_LIMIT: i64 = 60;

#[derive(Clone)]
pub struct PasswordlessRuntime {
    pub email_otp: Arc<EmailOtpService>,
    pub passkeys: Arc<PasskeyService>,
    pub store: Arc<SqlitePasswordlessStore>,
    pub passkey_registration_cookie: CookiePolicy,
    pub passkey_authentication_cookie: CookiePolicy,
    pending_registrations: Arc<Mutex<HashMap<[u8; 32], PendingRegistration>>>,
    #[cfg_attr(not(test), allow(dead_code))]
    capture_sender: Option<Arc<CaptureEmailSender>>,
}

#[derive(Clone)]
struct PendingRegistration {
    user_id: String,
    name: String,
    expires_at: DateTime<Utc>,
}

impl PasswordlessRuntime {
    pub fn remember_registration(
        &self,
        ceremony_token: &str,
        user_id: &str,
        name: String,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut pending = self
            .pending_registrations
            .lock()
            .map_err(|_| AppError::AuthUnavailable)?;
        pending.retain(|_, value| value.user_id != user_id && value.expires_at > now);
        pending.insert(
            request_digest(ceremony_token),
            PendingRegistration {
                user_id: user_id.to_string(),
                name,
                expires_at: now + Duration::minutes(5),
            },
        );
        Ok(())
    }

    pub fn consume_registration(
        &self,
        ceremony_token: &str,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<String> {
        let value = self
            .pending_registrations
            .lock()
            .map_err(|_| AppError::AuthUnavailable)?
            .remove(&request_digest(ceremony_token))
            .filter(|value| value.user_id == user_id && value.expires_at > now)
            .ok_or(AppError::Unauthorized)?;
        Ok(value.name)
    }

    #[cfg(test)]
    pub fn captured_code(&self, email: &str) -> Option<String> {
        self.capture_sender
            .as_ref()
            .and_then(|sender| sender.code_for(email))
    }
}

pub fn build_passwordless(config: &Config, pool: &SqlitePool) -> AppResult<PasswordlessRuntime> {
    let store = Arc::new(SqlitePasswordlessStore::new(pool.clone()));
    let (sender, capture_sender): (Arc<dyn EmailSender>, Option<Arc<CaptureEmailSender>>) =
        if let Some(smtp) = &config.smtp {
            (Arc::new(SmtpEmailSender::new(smtp)?), None)
        } else if config.environment != AppEnvironment::Production {
            let capture = Arc::new(CaptureEmailSender::default());
            (capture.clone(), Some(capture))
        } else {
            return Err(AppError::configuration(
                "production requires an SMTP email sender",
            ));
        };
    let email_config = EmailOtpConfig::new(
        config.email_otp_digest_key,
        ["/".to_string(), "/settings".to_string()],
    )
    .map_err(|error| AppError::configuration(error.to_string()))?;
    let email_otp = Arc::new(EmailOtpService::new(
        email_config,
        store.clone(),
        sender,
        store.clone(),
    ));
    let ceremonies = Arc::new(MemoryCeremonyStore::default());
    let passkey_config = PasskeyConfig::new(
        config.passkey_rp_id.clone(),
        &config.passkey_rp_origin,
        config.passkey_rp_name.clone(),
        config.passkey_ceremony_digest_key,
    )
    .map_err(|error| AppError::configuration(error.to_string()))?;
    let passkeys = Arc::new(
        PasskeyService::new(passkey_config, ceremonies.clone(), store.clone())
            .map_err(|error| AppError::configuration(error.to_string()))?,
    );
    let pending_registrations = Arc::new(Mutex::new(HashMap::new()));
    spawn_cleanup(
        email_otp.clone(),
        passkeys.clone(),
        pool.clone(),
        pending_registrations.clone(),
    );
    Ok(PasswordlessRuntime {
        email_otp,
        passkeys,
        store,
        passkey_registration_cookie: CookiePolicy::new(
            config.auth_environment(),
            "cook_passkey_registration",
        )
        .map_err(|error| AppError::configuration(error.to_string()))?,
        passkey_authentication_cookie: CookiePolicy::new(
            config.auth_environment(),
            "cook_passkey_authentication",
        )
        .map_err(|error| AppError::configuration(error.to_string()))?,
        pending_registrations,
        capture_sender,
    })
}

fn spawn_cleanup(
    email_otp: Arc<EmailOtpService>,
    passkeys: Arc<PasskeyService>,
    pool: SqlitePool,
    pending_registrations: Arc<Mutex<HashMap<[u8; 32], PendingRegistration>>>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15 * 60));
        interval.tick().await;
        loop {
            interval.tick().await;
            let now = Utc::now();
            if email_otp.cleanup(now).await.is_err() {
                tracing::error!("failed to clean expired email OTP attempts");
            }
            if passkeys.cleanup(now).await.is_err() {
                tracing::error!("failed to clean expired passkey ceremonies");
            }
            match pending_registrations.lock() {
                Ok(mut pending) => pending.retain(|_, value| value.expires_at > now),
                Err(_) => tracing::error!("failed to clean pending passkey registrations"),
            }
            if sqlx::query("DELETE FROM email_otp_link_targets WHERE expires_at <= ?")
                .bind(now.timestamp())
                .execute(&pool)
                .await
                .is_err()
            {
                tracing::error!("failed to clean expired email link targets");
            }
            if sqlx::query("DELETE FROM email_otp_rate_limits WHERE window_started_at <= ?")
                .bind(now.timestamp() - RATE_WINDOW_SECONDS)
                .execute(&pool)
                .await
                .is_err()
            {
                tracing::error!("failed to clean expired passwordless rate limits");
            }
        }
    });
}

#[derive(Clone)]
pub struct SqlitePasswordlessStore {
    pool: SqlitePool,
    pending_credential_names: Arc<Mutex<HashMap<String, String>>>,
    registered_credential_ids: Arc<Mutex<HashMap<String, String>>>,
}

impl SqlitePasswordlessStore {
    fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            pending_credential_names: Arc::new(Mutex::new(HashMap::new())),
            registered_credential_ids: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn set_pending_credential_name(&self, user_id: &str, name: String) -> AppResult<()> {
        self.registered_credential_ids
            .lock()
            .map_err(|_| AppError::AuthUnavailable)?
            .remove(user_id);
        self.pending_credential_names
            .lock()
            .map_err(|_| AppError::AuthUnavailable)?
            .insert(user_id.to_string(), name);
        Ok(())
    }

    pub fn take_registered_credential_id(&self, user_id: &str) -> AppResult<String> {
        self.registered_credential_ids
            .lock()
            .map_err(|_| AppError::AuthUnavailable)?
            .remove(user_id)
            .ok_or(AppError::AuthUnavailable)
    }

    pub fn clear_pending_credential_name(&self, user_id: &str) {
        if let Ok(mut names) = self.pending_credential_names.lock() {
            names.remove(user_id);
        }
    }

    pub async fn linked_emails(&self, user_id: &str) -> AppResult<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT subject FROM auth_identities
             WHERE issuer = ? AND user_id = ? ORDER BY subject",
        )
        .bind(EMAIL_IDENTITY_ISSUER)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn google_verified_emails(&self, user_id: &str) -> AppResult<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT verified_email FROM auth_identities
             WHERE issuer = ? AND user_id = ? AND verified_email IS NOT NULL
             ORDER BY created_at, subject",
        )
        .bind(GOOGLE_IDENTITY_ISSUER)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn find_user_by_email(&self, normalized_email: &str) -> AppResult<Option<UserId>> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT user_id FROM auth_identities WHERE issuer = ? AND subject = ?",
        )
        .bind(EMAIL_IDENTITY_ISSUER)
        .bind(normalized_email)
        .fetch_optional(&self.pool)
        .await?;
        value
            .map(UserId::new)
            .transpose()
            .map_err(|_| AppError::AuthUnavailable)
    }

    pub async fn check_passkey_start(
        &self,
        normalized_email: &str,
        ip_partition: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let email_key = URL_SAFE_NO_PAD.encode(Sha256::digest(normalized_email.as_bytes()));
        let email_allowed = increment_limit(&self.pool, "passkey-email", &email_key, 10, now)
            .await
            .map_err(|_| AppError::AuthUnavailable)?;
        let ip_allowed = increment_limit(
            &self.pool,
            "passkey-ip",
            ip_partition,
            REQUEST_IP_LIMIT,
            now,
        )
        .await
        .map_err(|_| AppError::AuthUnavailable)?;
        Ok(email_allowed && ip_allowed)
    }

    pub async fn resolve_email_sign_in(&self, normalized_email: &str) -> AppResult<UserId> {
        let mut transaction = self.pool.begin().await?;
        if let Some(existing) = identity_user(&mut transaction, normalized_email).await? {
            sqlx::query(
                "UPDATE auth_identities SET verified_email = ?
                 WHERE issuer = ? AND subject = ? AND user_id = ?",
            )
            .bind(normalized_email)
            .bind(EMAIL_IDENTITY_ISSUER)
            .bind(normalized_email)
            .bind(&existing)
            .execute(&mut *transaction)
            .await?;
            transaction.commit().await?;
            return UserId::new(existing).map_err(|_| AppError::AuthUnavailable);
        }
        let user_id = Uuid::new_v4().to_string();
        let display_name = normalized_email
            .split_once('@')
            .map(|(local, _)| local)
            .filter(|value| !value.is_empty())
            .unwrap_or("Cook user");
        sqlx::query(
            "INSERT INTO users (id, email, profile_email, display_name) VALUES (?, ?, ?, ?)",
        )
        .bind(&user_id)
        .bind(format!("email-{user_id}@identity.invalid"))
        .bind(normalized_email)
        .bind(display_name)
        .execute(&mut *transaction)
        .await?;
        let inserted = sqlx::query(
            "INSERT INTO auth_identities
             (issuer, subject, user_id, created_at, verified_email)
             VALUES (?, ?, ?, ?, ?) ON CONFLICT(issuer, subject) DO NOTHING",
        )
        .bind(EMAIL_IDENTITY_ISSUER)
        .bind(normalized_email)
        .bind(&user_id)
        .bind(Utc::now().timestamp())
        .bind(normalized_email)
        .execute(&mut *transaction)
        .await?;
        let resolved = if inserted.rows_affected() == 1 {
            user_id
        } else {
            sqlx::query("DELETE FROM users WHERE id = ?")
                .bind(&user_id)
                .execute(&mut *transaction)
                .await?;
            identity_user(&mut transaction, normalized_email)
                .await?
                .ok_or(AppError::AuthUnavailable)?
        };
        transaction.commit().await?;
        UserId::new(resolved).map_err(|_| AppError::AuthUnavailable)
    }

    pub async fn link_email(&self, user_id: &str, normalized_email: &str) -> AppResult<()> {
        let mut transaction = self.pool.begin().await?;
        let google_emails: Vec<String> = sqlx::query_scalar(
            "SELECT verified_email FROM auth_identities
             WHERE issuer = ? AND user_id = ? AND verified_email IS NOT NULL",
        )
        .bind(GOOGLE_IDENTITY_ISSUER)
        .bind(user_id)
        .fetch_all(&mut *transaction)
        .await?;
        if !google_emails.is_empty() && !google_emails.iter().any(|value| value == normalized_email)
        {
            return Err(AppError::Validation(
                "email must match the verified email on the linked Google identity".into(),
            ));
        }
        sqlx::query(
            "INSERT INTO auth_identities
             (issuer, subject, user_id, created_at, verified_email)
             VALUES (?, ?, ?, ?, ?) ON CONFLICT(issuer, subject) DO NOTHING",
        )
        .bind(EMAIL_IDENTITY_ISSUER)
        .bind(normalized_email)
        .bind(user_id)
        .bind(Utc::now().timestamp())
        .bind(normalized_email)
        .execute(&mut *transaction)
        .await?;
        let mapped = identity_user(&mut transaction, normalized_email)
            .await?
            .ok_or(AppError::AuthUnavailable)?;
        if mapped != user_id {
            return Err(AppError::Conflict(
                "this email sign-in identity is linked to another account".into(),
            ));
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn bind_link_target(
        &self,
        challenge_id: &str,
        user_id: &str,
        expires_at: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO email_otp_link_targets
             (request_digest, user_id, expires_at) VALUES (?, ?, ?)",
        )
        .bind(request_digest(challenge_id).as_slice())
        .bind(user_id)
        .bind(expires_at.timestamp())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn consume_link_target(
        &self,
        challenge_id: &str,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let result = sqlx::query(
            "DELETE FROM email_otp_link_targets
             WHERE request_digest = ? AND user_id = ? AND expires_at > ?",
        )
        .bind(request_digest(challenge_id).as_slice())
        .bind(user_id)
        .bind(now.timestamp())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AppError::Unauthorized);
        }
        Ok(())
    }

    pub async fn ensure_user_handle(&self, user_id: &str) -> AppResult<WebauthnUserHandle> {
        let mut transaction = self.pool.begin().await?;
        let existing: Option<String> =
            sqlx::query_scalar("SELECT passkey_user_handle FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_one(&mut *transaction)
                .await?;
        let value = if let Some(existing) = existing {
            existing
        } else {
            let candidate = Uuid::new_v4().to_string();
            sqlx::query(
                "UPDATE users SET passkey_user_handle = ?
                 WHERE id = ? AND passkey_user_handle IS NULL",
            )
            .bind(&candidate)
            .bind(user_id)
            .execute(&mut *transaction)
            .await?;
            sqlx::query_scalar("SELECT passkey_user_handle FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_one(&mut *transaction)
                .await?
        };
        transaction.commit().await?;
        let uuid = Uuid::parse_str(&value).map_err(|_| AppError::AuthUnavailable)?;
        Ok(WebauthnUserHandle::new(uuid))
    }

    pub async fn list_passkeys(&self, user_id: &str) -> AppResult<Vec<PasskeySummary>> {
        let rows = sqlx::query(
            "SELECT id, name, created_at, last_used_at
             FROM passkey_credentials
             WHERE user_id = ? AND revoked_at IS NULL
             ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(PasskeySummary {
                    id: row.get("id"),
                    name: row.get("name"),
                    created_at: timestamp_string(row.get("created_at"))?,
                    last_used_at: row
                        .get::<Option<i64>, _>("last_used_at")
                        .map(timestamp_string)
                        .transpose()?,
                })
            })
            .collect()
    }

    pub async fn passkey_summary(
        &self,
        user_id: &str,
        credential_row_id: &str,
    ) -> AppResult<PasskeySummary> {
        let row = sqlx::query(
            "SELECT id, name, created_at, last_used_at
             FROM passkey_credentials
             WHERE id = ? AND user_id = ? AND revoked_at IS NULL",
        )
        .bind(credential_row_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::AuthUnavailable)?;
        Ok(PasskeySummary {
            id: row.get("id"),
            name: row.get("name"),
            created_at: timestamp_string(row.get("created_at"))?,
            last_used_at: row
                .get::<Option<i64>, _>("last_used_at")
                .map(timestamp_string)
                .transpose()?,
        })
    }

    pub async fn credential_id_for_owner(
        &self,
        user_id: &str,
        id: &str,
    ) -> AppResult<CredentialID> {
        let value: Vec<u8> = sqlx::query_scalar(
            "SELECT credential_id FROM passkey_credentials
             WHERE id = ? AND user_id = ? AND revoked_at IS NULL",
        )
        .bind(id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::NotFound)?;
        Ok(value)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeySummary {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

#[async_trait]
impl EmailOtpAttemptStore for SqlitePasswordlessStore {
    async fn create_or_replace(
        &self,
        attempt: EmailOtpAttempt,
        now: DateTime<Utc>,
    ) -> Result<EmailOtpCreateOutcome, EmailOtpStorageError> {
        let stored = sqlx::query(
            "INSERT INTO email_otp_attempts
             (challenge_digest, email_digest, email, purpose, continue_path, code_digest,
              created_at, expires_at, resend_allowed_at, attempts_remaining)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(email_digest, purpose) DO UPDATE SET
               challenge_digest = excluded.challenge_digest,
               email = excluded.email,
               continue_path = excluded.continue_path,
               code_digest = excluded.code_digest,
               created_at = excluded.created_at,
               expires_at = excluded.expires_at,
               resend_allowed_at = excluded.resend_allowed_at,
               attempts_remaining = excluded.attempts_remaining
             WHERE email_otp_attempts.resend_allowed_at <= ?
             RETURNING challenge_digest",
        )
        .bind(attempt.challenge_digest.as_slice())
        .bind(attempt.email_digest.as_slice())
        .bind(attempt.email.as_str())
        .bind(attempt.purpose.as_str())
        .bind(attempt.continue_path)
        .bind(attempt.code_digest.as_slice())
        .bind(attempt.created_at.timestamp())
        .bind(attempt.expires_at.timestamp())
        .bind(attempt.resend_allowed_at.timestamp())
        .bind(i64::from(attempt.attempts_remaining))
        .bind(now.timestamp())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| EmailOtpStorageError)?;
        Ok(if stored.is_some() {
            EmailOtpCreateOutcome::Created
        } else {
            EmailOtpCreateOutcome::Cooldown
        })
    }

    async fn verify_and_consume(
        &self,
        challenge_digest: &[u8; 32],
        code_digest: &[u8; 32],
        now: DateTime<Utc>,
    ) -> Result<EmailOtpVerifyOutcome, EmailOtpStorageError> {
        if let Some(row) = sqlx::query(
            "DELETE FROM email_otp_attempts
             WHERE challenge_digest = ? AND code_digest = ?
               AND created_at <= ? AND expires_at > ?
             RETURNING challenge_digest, email_digest, email, purpose, continue_path,
               code_digest, created_at, expires_at, resend_allowed_at, attempts_remaining",
        )
        .bind(challenge_digest.as_slice())
        .bind(code_digest.as_slice())
        .bind(now.timestamp())
        .bind(now.timestamp())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| EmailOtpStorageError)?
        {
            return Ok(EmailOtpVerifyOutcome::Verified(Box::new(
                otp_attempt_from_row(row)?,
            )));
        }
        let decremented = sqlx::query(
            "UPDATE email_otp_attempts
             SET attempts_remaining = attempts_remaining - 1
             WHERE challenge_digest = ? AND code_digest != ?
               AND created_at <= ? AND expires_at > ? AND attempts_remaining > 1
             RETURNING attempts_remaining",
        )
        .bind(challenge_digest.as_slice())
        .bind(code_digest.as_slice())
        .bind(now.timestamp())
        .bind(now.timestamp())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| EmailOtpStorageError)?;
        if decremented.is_some() {
            return Ok(EmailOtpVerifyOutcome::Rejected);
        }
        let exhausted = sqlx::query(
            "DELETE FROM email_otp_attempts
             WHERE challenge_digest = ? AND code_digest != ?
               AND created_at <= ? AND expires_at > ? AND attempts_remaining = 1",
        )
        .bind(challenge_digest.as_slice())
        .bind(code_digest.as_slice())
        .bind(now.timestamp())
        .bind(now.timestamp())
        .execute(&self.pool)
        .await
        .map_err(|_| EmailOtpStorageError)?;
        sqlx::query(
            "DELETE FROM email_otp_attempts WHERE challenge_digest = ? AND expires_at <= ?",
        )
        .bind(challenge_digest.as_slice())
        .bind(now.timestamp())
        .execute(&self.pool)
        .await
        .map_err(|_| EmailOtpStorageError)?;
        Ok(if exhausted.rows_affected() == 1 {
            EmailOtpVerifyOutcome::Rejected
        } else {
            EmailOtpVerifyOutcome::Invalid
        })
    }

    async fn invalidate(&self, challenge_digest: &[u8; 32]) -> Result<(), EmailOtpStorageError> {
        sqlx::query("DELETE FROM email_otp_attempts WHERE challenge_digest = ?")
            .bind(challenge_digest.as_slice())
            .execute(&self.pool)
            .await
            .map_err(|_| EmailOtpStorageError)?;
        Ok(())
    }

    async fn cleanup(&self, now: DateTime<Utc>) -> Result<usize, EmailOtpStorageError> {
        let result = sqlx::query("DELETE FROM email_otp_attempts WHERE expires_at <= ?")
            .bind(now.timestamp())
            .execute(&self.pool)
            .await
            .map_err(|_| EmailOtpStorageError)?;
        usize::try_from(result.rows_affected()).map_err(|_| EmailOtpStorageError)
    }
}

#[async_trait]
impl EmailOtpRateLimiter for SqlitePasswordlessStore {
    async fn check_request(
        &self,
        context: &EmailOtpRequestRateLimitContext,
        now: DateTime<Utc>,
    ) -> Result<EmailOtpRateLimitDecision, EmailOtpStorageError> {
        let email_key = URL_SAFE_NO_PAD.encode(context.email_digest);
        let email_allowed = increment_limit(
            &self.pool,
            "request-email",
            &email_key,
            REQUEST_EMAIL_LIMIT,
            now,
        )
        .await?;
        let ip_allowed = increment_limit(
            &self.pool,
            "request-ip",
            &context.ip_partition,
            REQUEST_IP_LIMIT,
            now,
        )
        .await?;
        Ok(if email_allowed && ip_allowed {
            EmailOtpRateLimitDecision::Allowed
        } else {
            EmailOtpRateLimitDecision::Rejected
        })
    }

    async fn check_verify(
        &self,
        context: &EmailOtpVerifyRateLimitContext,
        now: DateTime<Utc>,
    ) -> Result<EmailOtpRateLimitDecision, EmailOtpStorageError> {
        let challenge_key = URL_SAFE_NO_PAD.encode(context.challenge_digest);
        let challenge_allowed = increment_limit(
            &self.pool,
            "verify-challenge",
            &challenge_key,
            VERIFY_CHALLENGE_LIMIT,
            now,
        )
        .await?;
        let ip_allowed = increment_limit(
            &self.pool,
            "verify-ip",
            &context.ip_partition,
            VERIFY_IP_LIMIT,
            now,
        )
        .await?;
        Ok(if challenge_allowed && ip_allowed {
            EmailOtpRateLimitDecision::Allowed
        } else {
            EmailOtpRateLimitDecision::Rejected
        })
    }
}

#[async_trait]
impl PasskeyCredentialStore for SqlitePasswordlessStore {
    async fn load(
        &self,
        user_id: &UserId,
    ) -> Result<Option<UserPasskeys>, rust_web_auth_passkey::PasskeyStorageError> {
        let handle: Option<String> =
            sqlx::query_scalar("SELECT passkey_user_handle FROM users WHERE id = ?")
                .bind(user_id.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?
                .flatten();
        let Some(handle) = handle else {
            return Ok(None);
        };
        let handle = Uuid::parse_str(&handle)
            .map(WebauthnUserHandle::new)
            .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        let values: Vec<String> = sqlx::query_scalar(
            "SELECT credential_json FROM passkey_credentials
             WHERE user_id = ? AND revoked_at IS NULL ORDER BY created_at",
        )
        .bind(user_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        let credentials = values
            .into_iter()
            .map(|value| {
                serde_json::from_str(&value).map_err(|_| rust_web_auth_passkey::PasskeyStorageError)
            })
            .collect::<Result<Vec<Passkey>, _>>()?;
        Ok(Some(UserPasskeys {
            user_handle: handle,
            credentials,
        }))
    }

    async fn save_registered(
        &self,
        user_id: &UserId,
        user_handle: WebauthnUserHandle,
        passkey: Passkey,
    ) -> Result<PasskeyRegistrationOutcome, rust_web_auth_passkey::PasskeyStorageError> {
        let name = self
            .pending_credential_names
            .lock()
            .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?
            .remove(user_id.as_str())
            .ok_or(rust_web_auth_passkey::PasskeyStorageError)?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        let stored_handle: Option<String> =
            sqlx::query_scalar("SELECT passkey_user_handle FROM users WHERE id = ?")
                .bind(user_id.as_str())
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        if stored_handle.as_deref() != Some(&user_handle.as_uuid().to_string()) {
            return Err(rust_web_auth_passkey::PasskeyStorageError);
        }
        let credential_id: &[u8] = passkey.cred_id().as_ref();
        let value = serde_json::to_string(&passkey)
            .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        let credential_row_id = Uuid::new_v4().to_string();
        let result = sqlx::query(
            "INSERT INTO passkey_credentials
             (id, user_id, credential_id, credential_json, name, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&credential_row_id)
        .bind(user_id.as_str())
        .bind(credential_id)
        .bind(value)
        .bind(name)
        .bind(Utc::now().timestamp())
        .execute(&mut *transaction)
        .await;
        match result {
            Ok(_) => {
                transaction
                    .commit()
                    .await
                    .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
                self.registered_credential_ids
                    .lock()
                    .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?
                    .insert(user_id.as_str().to_string(), credential_row_id);
                Ok(PasskeyRegistrationOutcome::Saved)
            }
            Err(error)
                if error
                    .as_database_error()
                    .is_some_and(|db| db.is_unique_violation()) =>
            {
                Ok(PasskeyRegistrationOutcome::CredentialConflict)
            }
            Err(_) => Err(rust_web_auth_passkey::PasskeyStorageError),
        }
    }

    async fn update_credential(
        &self,
        user_id: &UserId,
        user_handle: WebauthnUserHandle,
        result: &AuthenticationResult,
    ) -> Result<bool, rust_web_auth_passkey::PasskeyStorageError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        let handle: Option<String> =
            sqlx::query_scalar("SELECT passkey_user_handle FROM users WHERE id = ?")
                .bind(user_id.as_str())
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?
                .flatten();
        if handle.as_deref() != Some(&user_handle.as_uuid().to_string()) {
            return Ok(false);
        }
        let credential_id: &[u8] = result.cred_id().as_ref();
        let row = sqlx::query(
            "SELECT id, credential_json FROM passkey_credentials
             WHERE user_id = ? AND credential_id = ? AND revoked_at IS NULL",
        )
        .bind(user_id.as_str())
        .bind(credential_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        let Some(row) = row else {
            return Ok(false);
        };
        let mut passkey: Passkey = serde_json::from_str(row.get("credential_json"))
            .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        let _ = passkey.update_credential(result);
        let value = serde_json::to_string(&passkey)
            .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        sqlx::query(
            "UPDATE passkey_credentials SET credential_json = ?, last_used_at = ?
             WHERE id = ? AND user_id = ?",
        )
        .bind(value)
        .bind(Utc::now().timestamp())
        .bind(row.get::<String, _>("id"))
        .bind(user_id.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        transaction
            .commit()
            .await
            .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        Ok(true)
    }

    async fn revoke_credential(
        &self,
        user_id: &UserId,
        user_handle: WebauthnUserHandle,
        credential_id: &CredentialID,
    ) -> Result<bool, rust_web_auth_passkey::PasskeyStorageError> {
        let handle: Option<String> =
            sqlx::query_scalar("SELECT passkey_user_handle FROM users WHERE id = ?")
                .bind(user_id.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?
                .flatten();
        if handle.as_deref() != Some(&user_handle.as_uuid().to_string()) {
            return Ok(false);
        }
        let credential_id: &[u8] = credential_id.as_ref();
        let result = sqlx::query(
            "UPDATE passkey_credentials SET revoked_at = ?
             WHERE user_id = ? AND credential_id = ? AND revoked_at IS NULL",
        )
        .bind(Utc::now().timestamp())
        .bind(user_id.as_str())
        .bind(credential_id)
        .execute(&self.pool)
        .await
        .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        Ok(result.rows_affected() == 1)
    }
}

#[derive(Default)]
struct MemoryCeremonyStore {
    states: Mutex<HashMap<CeremonyTokenDigest, CeremonyState>>,
}

#[async_trait]
impl CeremonyStore for MemoryCeremonyStore {
    async fn create(
        &self,
        digest: CeremonyTokenDigest,
        state: CeremonyState,
    ) -> Result<(), rust_web_auth_passkey::PasskeyStorageError> {
        let mut states = self
            .states
            .lock()
            .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        if states.insert(digest, state).is_some() {
            return Err(rust_web_auth_passkey::PasskeyStorageError);
        }
        Ok(())
    }

    async fn consume_and_delete(
        &self,
        digest: &CeremonyTokenDigest,
        now: DateTime<Utc>,
    ) -> Result<Option<CeremonyState>, rust_web_auth_passkey::PasskeyStorageError> {
        let value = self
            .states
            .lock()
            .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?
            .remove(digest);
        Ok(value.filter(|state| ceremony_available(state, now)))
    }

    async fn cleanup(
        &self,
        now: DateTime<Utc>,
    ) -> Result<usize, rust_web_auth_passkey::PasskeyStorageError> {
        let mut states = self
            .states
            .lock()
            .map_err(|_| rust_web_auth_passkey::PasskeyStorageError)?;
        let before = states.len();
        states.retain(|_, state| ceremony_available(state, now));
        Ok(before - states.len())
    }
}

fn ceremony_available(state: &CeremonyState, now: DateTime<Utc>) -> bool {
    match state {
        CeremonyState::Registration {
            created_at,
            expires_at,
            ..
        }
        | CeremonyState::Authentication {
            created_at,
            expires_at,
            ..
        } => now >= *created_at && now < *expires_at,
        _ => false,
    }
}

struct SmtpEmailSender {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpEmailSender {
    fn new(config: &SmtpConfig) -> AppResult<Self> {
        let credentials = Credentials::new(config.username.clone(), config.password.clone());
        let builder = match config.tls_mode {
            SmtpTlsMode::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
            }
            SmtpTlsMode::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host),
        }
        .map_err(|_| AppError::configuration("invalid SMTP host or TLS configuration"))?;
        let from = config
            .from
            .parse()
            .map_err(|_| AppError::configuration("SMTP_FROM must be a valid mailbox"))?;
        Ok(Self {
            transport: builder.port(config.port).credentials(credentials).build(),
            from,
        })
    }
}

#[async_trait]
impl EmailSender for SmtpEmailSender {
    async fn send_email_otp(
        &self,
        recipient: &EmailAddress,
        message: EmailOtpMessage<'_>,
    ) -> Result<(), EmailDeliveryError> {
        let recipient: Mailbox = recipient.as_str().parse().map_err(|_| EmailDeliveryError)?;
        let subject = if message.purpose.as_str() == EMAIL_LINK_PURPOSE {
            "Link your email to Cook"
        } else {
            "Your Cook sign-in code"
        };
        let email = Message::builder()
            .from(self.from.clone())
            .to(recipient)
            .subject(subject)
            .body(format!(
                "Your Cook verification code is {}.\n\nIt expires at {}. If you did not request this, ignore this email.",
                message.code.expose_for_delivery(),
                message.expires_at.to_rfc3339()
            ))
            .map_err(|_| EmailDeliveryError)?;
        self.transport
            .send(email)
            .await
            .map_err(|_| EmailDeliveryError)?;
        Ok(())
    }
}

#[derive(Default)]
struct CaptureEmailSender {
    codes: Mutex<HashMap<String, String>>,
}

impl CaptureEmailSender {
    #[cfg(test)]
    fn code_for(&self, email: &str) -> Option<String> {
        self.codes
            .lock()
            .ok()
            .and_then(|codes| codes.get(&email.to_lowercase()).cloned())
    }
}

#[async_trait]
impl EmailSender for CaptureEmailSender {
    async fn send_email_otp(
        &self,
        recipient: &EmailAddress,
        message: EmailOtpMessage<'_>,
    ) -> Result<(), EmailDeliveryError> {
        self.codes.lock().map_err(|_| EmailDeliveryError)?.insert(
            recipient.as_str().to_lowercase(),
            message.code.expose_for_delivery().to_string(),
        );
        Ok(())
    }
}

pub fn normalize_email(value: &str) -> Result<String, EmailOtpError> {
    EmailAddress::new(value.trim().to_string()).map(|email| email.as_str().to_lowercase())
}

pub fn email_address(value: &str) -> Result<EmailAddress, EmailOtpError> {
    EmailAddress::new(normalize_email(value)?)
}

pub fn purpose(value: &str) -> Result<EmailOtpPurpose, EmailOtpError> {
    EmailOtpPurpose::new(value.to_string())
}

pub fn map_email_error(error: EmailOtpError) -> AppError {
    match error {
        EmailOtpError::RateLimited => AppError::RateLimited,
        EmailOtpError::Unavailable | EmailOtpError::InvalidConfiguration => {
            AppError::AuthUnavailable
        }
        EmailOtpError::InvalidRequest => AppError::Validation("invalid email OTP request".into()),
        EmailOtpError::VerificationFailed => AppError::Unauthorized,
    }
}

pub fn map_passkey_error(error: PasskeyError) -> AppError {
    match error {
        PasskeyError::Unavailable | PasskeyError::InvalidConfiguration => AppError::AuthUnavailable,
        PasskeyError::CeremonyFailed => AppError::Unauthorized,
    }
}

async fn identity_user(
    transaction: &mut Transaction<'_, Sqlite>,
    normalized_email: &str,
) -> AppResult<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT user_id FROM auth_identities WHERE issuer = ? AND subject = ?")
            .bind(EMAIL_IDENTITY_ISSUER)
            .bind(normalized_email)
            .fetch_optional(&mut **transaction)
            .await?,
    )
}

async fn increment_limit(
    pool: &SqlitePool,
    kind: &str,
    key: &str,
    maximum: i64,
    now: DateTime<Utc>,
) -> Result<bool, EmailOtpStorageError> {
    let cutoff = now.timestamp() - RATE_WINDOW_SECONDS;
    let count: i64 = sqlx::query_scalar(
        "INSERT INTO email_otp_rate_limits
         (limit_kind, rate_key, window_started_at, attempt_count)
         VALUES (?, ?, ?, 1)
         ON CONFLICT(limit_kind, rate_key) DO UPDATE SET
           window_started_at = CASE
             WHEN email_otp_rate_limits.window_started_at <= ? THEN excluded.window_started_at
             ELSE email_otp_rate_limits.window_started_at
           END,
           attempt_count = CASE
             WHEN email_otp_rate_limits.window_started_at <= ? THEN 1
             ELSE email_otp_rate_limits.attempt_count + 1
           END
         RETURNING attempt_count",
    )
    .bind(kind)
    .bind(key)
    .bind(now.timestamp())
    .bind(cutoff)
    .bind(cutoff)
    .fetch_one(pool)
    .await
    .map_err(|_| EmailOtpStorageError)?;
    Ok(count <= maximum)
}

fn otp_attempt_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<EmailOtpAttempt, EmailOtpStorageError> {
    Ok(EmailOtpAttempt {
        challenge_digest: array_32(row.get("challenge_digest"))?,
        email_digest: array_32(row.get("email_digest"))?,
        email: EmailAddress::new(row.get::<String, _>("email"))
            .map_err(|_| EmailOtpStorageError)?,
        purpose: EmailOtpPurpose::new(row.get::<String, _>("purpose"))
            .map_err(|_| EmailOtpStorageError)?,
        continue_path: row.get("continue_path"),
        code_digest: array_32(row.get("code_digest"))?,
        created_at: timestamp(row.get("created_at"))?,
        expires_at: timestamp(row.get("expires_at"))?,
        resend_allowed_at: timestamp(row.get("resend_allowed_at"))?,
        attempts_remaining: u8::try_from(row.get::<i64, _>("attempts_remaining"))
            .map_err(|_| EmailOtpStorageError)?,
    })
}

fn request_digest(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

fn timestamp(value: i64) -> Result<DateTime<Utc>, EmailOtpStorageError> {
    DateTime::from_timestamp(value, 0).ok_or(EmailOtpStorageError)
}

fn timestamp_string(value: i64) -> AppResult<String> {
    DateTime::from_timestamp(value, 0)
        .map(|value| value.to_rfc3339())
        .ok_or(AppError::AuthUnavailable)
}

fn array_32(value: Vec<u8>) -> Result<[u8; 32], EmailOtpStorageError> {
    value.try_into().map_err(|_| EmailOtpStorageError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_web_auth_email_otp::{EmailOtpRequest, EmailOtpVerification};

    async fn runtime() -> (SqlitePool, PasswordlessRuntime) {
        let pool = crate::db::connect("sqlite::memory:").await.unwrap();
        let runtime = build_passwordless(&Config::test(), &pool).unwrap();
        (pool, runtime)
    }

    async fn request(
        runtime: &PasswordlessRuntime,
        email: &str,
        purpose_value: &str,
    ) -> rust_web_auth_email_otp::EmailOtpChallenge {
        runtime
            .email_otp
            .request(
                EmailOtpRequest {
                    email: email_address(email).unwrap(),
                    purpose: purpose(purpose_value).unwrap(),
                    continue_path: if purpose_value == EMAIL_LINK_PURPOSE {
                        "/settings".into()
                    } else {
                        "/".into()
                    },
                    ip_partition: "test-ip".into(),
                },
                Utc::now(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn otp_attempts_expire_reject_replay_and_exhaust_five_codes() {
        let (pool, runtime) = runtime().await;
        let email = "owner@example.test";
        let first = request(&runtime, "OWNER@EXAMPLE.TEST", EMAIL_SIGN_IN_PURPOSE).await;
        let stored_email: String =
            sqlx::query_scalar("SELECT email FROM email_otp_attempts LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored_email, email);
        let delivered = runtime.captured_code(email).unwrap();
        let wrong_code = if delivered == "000000" {
            "000001"
        } else {
            "000000"
        };
        for _ in 0..5 {
            assert!(matches!(
                runtime
                    .email_otp
                    .verify(
                        EmailOtpVerification {
                            challenge_id: first.challenge_id.expose_for_client(),
                            code: wrong_code,
                            ip_partition: "test-ip",
                        },
                        Utc::now(),
                    )
                    .await,
                Err(EmailOtpError::VerificationFailed)
            ));
        }
        assert!(matches!(
            runtime
                .email_otp
                .verify(
                    EmailOtpVerification {
                        challenge_id: first.challenge_id.expose_for_client(),
                        code: &delivered,
                        ip_partition: "test-ip",
                    },
                    Utc::now(),
                )
                .await,
            Err(EmailOtpError::VerificationFailed)
        ));

        let second = request(&runtime, email, EMAIL_SIGN_IN_PURPOSE).await;
        let delivered = runtime.captured_code(email).unwrap();
        runtime
            .email_otp
            .verify(
                EmailOtpVerification {
                    challenge_id: second.challenge_id.expose_for_client(),
                    code: &delivered,
                    ip_partition: "test-ip",
                },
                Utc::now(),
            )
            .await
            .unwrap();
        assert!(matches!(
            runtime
                .email_otp
                .verify(
                    EmailOtpVerification {
                        challenge_id: second.challenge_id.expose_for_client(),
                        code: &delivered,
                        ip_partition: "test-ip",
                    },
                    Utc::now(),
                )
                .await,
            Err(EmailOtpError::VerificationFailed)
        ));

        let third = request(&runtime, "expired@example.test", EMAIL_SIGN_IN_PURPOSE).await;
        let expired_code = runtime.captured_code("expired@example.test").unwrap();
        sqlx::query("UPDATE email_otp_attempts SET expires_at = ?")
            .bind(Utc::now().timestamp() - 1)
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            runtime
                .email_otp
                .verify(
                    EmailOtpVerification {
                        challenge_id: third.challenge_id.expose_for_client(),
                        code: &expired_code,
                        ip_partition: "test-ip",
                    },
                    Utc::now(),
                )
                .await,
            Err(EmailOtpError::VerificationFailed)
        ));
    }

    #[tokio::test]
    async fn otp_rate_limits_are_persisted_and_layered() {
        let (_, runtime) = runtime().await;
        let now = Utc::now();
        let request_context = EmailOtpRequestRateLimitContext {
            email_digest: [8; 32],
            ip_partition: "request-ip".into(),
        };
        for _ in 0..REQUEST_EMAIL_LIMIT {
            assert_eq!(
                runtime
                    .store
                    .check_request(&request_context, now)
                    .await
                    .unwrap(),
                EmailOtpRateLimitDecision::Allowed
            );
        }
        assert_eq!(
            runtime
                .store
                .check_request(&request_context, now)
                .await
                .unwrap(),
            EmailOtpRateLimitDecision::Rejected
        );

        let verify_context = EmailOtpVerifyRateLimitContext {
            challenge_digest: [9; 32],
            ip_partition: "verify-ip".into(),
        };
        for _ in 0..VERIFY_CHALLENGE_LIMIT {
            assert_eq!(
                runtime
                    .store
                    .check_verify(&verify_context, now)
                    .await
                    .unwrap(),
                EmailOtpRateLimitDecision::Allowed
            );
        }
        assert_eq!(
            runtime
                .store
                .check_verify(&verify_context, now)
                .await
                .unwrap(),
            EmailOtpRateLimitDecision::Rejected
        );
    }

    #[tokio::test]
    async fn email_link_conflicts_and_passkey_rows_are_owner_scoped() {
        let (pool, runtime) = runtime().await;
        for user in ["alice", "bob"] {
            sqlx::query(
                "INSERT INTO users (id, email, profile_email, display_name)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(user)
            .bind(format!("{user}@identity.invalid"))
            .bind(format!("{user}@example.test"))
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
        }
        runtime
            .store
            .link_email("alice", "shared@example.test")
            .await
            .unwrap();
        assert!(matches!(
            runtime.store.link_email("bob", "shared@example.test").await,
            Err(AppError::Conflict(_))
        ));

        sqlx::query(
            "INSERT INTO passkey_credentials
             (id, user_id, credential_id, credential_json, name, created_at)
             VALUES ('credential', 'alice', X'010203', '{}', 'Alice key', ?)",
        )
        .bind(Utc::now().timestamp())
        .execute(&pool)
        .await
        .unwrap();
        assert!(runtime
            .store
            .credential_id_for_owner("alice", "credential")
            .await
            .is_ok());
        assert!(matches!(
            runtime
                .store
                .credential_id_for_owner("bob", "credential")
                .await,
            Err(AppError::NotFound)
        ));
    }

    #[tokio::test]
    async fn google_first_email_link_requires_the_exact_normalized_google_email() {
        let (pool, runtime) = runtime().await;
        sqlx::query(
            "INSERT INTO users (id, email, profile_email, display_name)
             VALUES ('google-user', 'google@identity.invalid', 'owner@example.com', 'Owner')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO auth_identities
             (issuer, subject, user_id, created_at, verified_email)
             VALUES ('https://accounts.google.com', 'subject', 'google-user', ?, 'owner@example.com')",
        )
        .bind(Utc::now().timestamp())
        .execute(&pool)
        .await
        .unwrap();

        runtime
            .store
            .link_email("google-user", "owner@example.com")
            .await
            .unwrap();
        assert!(matches!(
            runtime
                .store
                .link_email("google-user", "other@example.com")
                .await,
            Err(AppError::Validation(_))
        ));
        assert_eq!(
            runtime.store.linked_emails("google-user").await.unwrap(),
            vec!["owner@example.com"]
        );
    }
}
