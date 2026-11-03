use std::{env, net::SocketAddr};

use rust_web_auth_http::Environment;

use crate::error::{AppError, AppResult};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppEnvironment {
    Development,
    Test,
    Production,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthMode {
    Development,
    Google,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SmtpTlsMode {
    StartTls,
    Tls,
}

#[derive(Clone, Debug)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub tls_mode: SmtpTlsMode,
    pub username: String,
    pub password: String,
    pub from: String,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub environment: AppEnvironment,
    pub auth_mode: AuthMode,
    pub bind: SocketAddr,
    pub database_url: String,
    pub public_origin: String,
    pub oauth_state_digest_key: [u8; 32],
    pub session_digest_key: [u8; 32],
    pub email_otp_digest_key: [u8; 32],
    pub passkey_ceremony_digest_key: [u8; 32],
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<String>,
    pub smtp: Option<SmtpConfig>,
    pub passkey_rp_id: String,
    pub passkey_rp_origin: String,
    pub passkey_rp_name: String,
}

impl Config {
    pub fn from_env() -> AppResult<Self> {
        let environment = match env::var("APP_ENV")
            .unwrap_or_else(|_| "development".into())
            .as_str()
        {
            "development" => AppEnvironment::Development,
            "test" => AppEnvironment::Test,
            "production" => AppEnvironment::Production,
            value => {
                return Err(AppError::configuration(format!(
                    "unsupported APP_ENV: {value}"
                )))
            }
        };
        let auth_mode = match env::var("AUTH_MODE")
            .unwrap_or_else(|_| "dev".into())
            .as_str()
        {
            "dev" => AuthMode::Development,
            "google" => AuthMode::Google,
            value => {
                return Err(AppError::configuration(format!(
                    "unsupported AUTH_MODE: {value}"
                )))
            }
        };
        let development_key = [0x42; 32];
        let config = Self {
            environment,
            auth_mode,
            bind: env::var("API_BIND")
                .unwrap_or_else(|_| "127.0.0.1:8080".into())
                .parse()
                .map_err(|error| AppError::configuration(format!("invalid API_BIND: {error}")))?,
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://data/cook.db".into()),
            public_origin: env::var("PUBLIC_ORIGIN")
                .unwrap_or_else(|_| "http://localhost:5173".into()),
            oauth_state_digest_key: optional_key("OAUTH_STATE_DIGEST_KEY")?
                .unwrap_or(development_key),
            session_digest_key: optional_key("APP_SESSION_DIGEST_KEY")?.unwrap_or([0x24; 32]),
            email_otp_digest_key: optional_key("EMAIL_OTP_DIGEST_KEY")?.unwrap_or([0x31; 32]),
            passkey_ceremony_digest_key: optional_key("PASSKEY_CEREMONY_DIGEST_KEY")?
                .unwrap_or([0x57; 32]),
            google_client_id: env::var("GOOGLE_CLIENT_ID").ok(),
            google_client_secret: env::var("GOOGLE_CLIENT_SECRET").ok(),
            smtp: smtp_from_env()?,
            passkey_rp_id: env::var("PASSKEY_RP_ID").unwrap_or_else(|_| "localhost".into()),
            passkey_rp_origin: env::var("PASSKEY_RP_ORIGIN")
                .unwrap_or_else(|_| "http://localhost:5173".into()),
            passkey_rp_name: env::var("PASSKEY_RP_NAME").unwrap_or_else(|_| "Cook".into()),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn test() -> Self {
        Self {
            environment: AppEnvironment::Test,
            auth_mode: AuthMode::Development,
            bind: "127.0.0.1:0".parse().expect("test bind"),
            database_url: "sqlite::memory:".into(),
            public_origin: "http://localhost".into(),
            oauth_state_digest_key: [0x42; 32],
            session_digest_key: [0x24; 32],
            email_otp_digest_key: [0x31; 32],
            passkey_ceremony_digest_key: [0x57; 32],
            google_client_id: None,
            google_client_secret: None,
            smtp: None,
            passkey_rp_id: "localhost".into(),
            passkey_rp_origin: "http://localhost".into(),
            passkey_rp_name: "Cook Test".into(),
        }
    }

    pub fn validate(&self) -> AppResult<()> {
        if self.auth_mode == AuthMode::Development && !self.bind.ip().is_loopback() {
            return Err(AppError::configuration(
                "development authentication may only bind to a loopback address",
            ));
        }
        if self.environment == AppEnvironment::Production {
            if self.auth_mode != AuthMode::Google {
                return Err(AppError::configuration(
                    "production requires AUTH_MODE=google",
                ));
            }
            if self.public_origin != "https://cook.vichare.net" {
                return Err(AppError::configuration(
                    "production PUBLIC_ORIGIN must be https://cook.vichare.net",
                ));
            }
            require_secret(&self.google_client_id, "GOOGLE_CLIENT_ID")?;
            require_secret(&self.google_client_secret, "GOOGLE_CLIENT_SECRET")?;
            let smtp = self
                .smtp
                .as_ref()
                .ok_or_else(|| AppError::configuration("production requires SMTP configuration"))?;
            require_value(&smtp.host, "SMTP_HOST")?;
            require_value(&smtp.username, "SMTP_USERNAME")?;
            require_value(&smtp.password, "SMTP_PASSWORD")?;
            require_value(&smtp.from, "SMTP_FROM")?;
            if self.passkey_rp_id != "cook.vichare.net"
                || self.passkey_rp_origin != "https://cook.vichare.net"
            {
                return Err(AppError::configuration(
                    "production passkey RP must be cook.vichare.net with exact HTTPS origin",
                ));
            }
            if self.oauth_state_digest_key == [0x42; 32]
                || self.session_digest_key == [0x24; 32]
                || self.email_otp_digest_key == [0x31; 32]
                || self.passkey_ceremony_digest_key == [0x57; 32]
                || !all_keys_distinct(&[
                    self.oauth_state_digest_key,
                    self.session_digest_key,
                    self.email_otp_digest_key,
                    self.passkey_ceremony_digest_key,
                ])
            {
                return Err(AppError::configuration(
                    "production requires independent random auth digest keys",
                ));
            }
        }
        Ok(())
    }

    pub fn auth_environment(&self) -> Environment {
        if self.environment == AppEnvironment::Production {
            Environment::Production
        } else {
            Environment::Development
        }
    }

    pub fn google_callback_url(&self) -> String {
        format!("{}/api/auth/google/callback", self.public_origin)
    }
}

fn optional_key(name: &str) -> AppResult<Option<[u8; 32]>> {
    let Ok(value) = env::var(name) else {
        return Ok(None);
    };
    if value.len() != 64 {
        return Err(AppError::configuration(format!(
            "{name} must contain exactly 64 hexadecimal characters"
        )));
    }
    let mut key = [0_u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| AppError::configuration(format!("{name} must be hexadecimal")))?;
    }
    Ok(Some(key))
}

fn smtp_from_env() -> AppResult<Option<SmtpConfig>> {
    let names = [
        "SMTP_HOST",
        "SMTP_PORT",
        "SMTP_TLS",
        "SMTP_USERNAME",
        "SMTP_PASSWORD",
        "SMTP_FROM",
    ];
    if !names.iter().any(|name| env::var(name).is_ok()) {
        return Ok(None);
    }
    let required = |name: &str| {
        env::var(name).map_err(|_| AppError::configuration(format!("{name} is required")))
    };
    let tls_mode = match required("SMTP_TLS")?.as_str() {
        "starttls" => SmtpTlsMode::StartTls,
        "tls" => SmtpTlsMode::Tls,
        _ => return Err(AppError::configuration("SMTP_TLS must be starttls or tls")),
    };
    let port = required("SMTP_PORT")?
        .parse()
        .map_err(|_| AppError::configuration("SMTP_PORT must be a valid TCP port"))?;
    if port == 0 {
        return Err(AppError::configuration(
            "SMTP_PORT must be greater than zero",
        ));
    }
    Ok(Some(SmtpConfig {
        host: required("SMTP_HOST")?,
        port,
        tls_mode,
        username: required("SMTP_USERNAME")?,
        password: required("SMTP_PASSWORD")?,
        from: required("SMTP_FROM")?,
    }))
}

fn require_secret(value: &Option<String>, name: &str) -> AppResult<()> {
    match value.as_deref() {
        Some(value) if !value.trim().is_empty() && !value.contains("replace") => Ok(()),
        _ => Err(AppError::configuration(format!(
            "production requires {name}"
        ))),
    }
}

fn require_value(value: &str, name: &str) -> AppResult<()> {
    if value.trim().is_empty() || value.contains("replace") {
        return Err(AppError::configuration(format!(
            "production requires {name}"
        )));
    }
    Ok(())
}

fn all_keys_distinct(keys: &[[u8; 32]]) -> bool {
    keys.iter()
        .enumerate()
        .all(|(index, key)| keys[index + 1..].iter().all(|other| other != key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_never_accepts_development_auth() {
        let mut config = Config::test();
        config.environment = AppEnvironment::Production;
        config.public_origin = "https://cook.vichare.net".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn production_fails_without_google_configuration() {
        let mut config = Config::test();
        config.environment = AppEnvironment::Production;
        config.auth_mode = AuthMode::Google;
        config.public_origin = "https://cook.vichare.net".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn production_accepts_complete_independent_auth_configuration() {
        let mut config = Config::test();
        config.environment = AppEnvironment::Production;
        config.auth_mode = AuthMode::Google;
        config.public_origin = "https://cook.vichare.net".into();
        config.oauth_state_digest_key = [1; 32];
        config.session_digest_key = [2; 32];
        config.email_otp_digest_key = [3; 32];
        config.passkey_ceremony_digest_key = [4; 32];
        config.google_client_id = Some("client".into());
        config.google_client_secret = Some("secret".into());
        config.smtp = Some(SmtpConfig {
            host: "smtp.example.test".into(),
            port: 465,
            tls_mode: SmtpTlsMode::Tls,
            username: "user".into(),
            password: "password".into(),
            from: "Cook <cook@example.test>".into(),
        });
        config.passkey_rp_id = "cook.vichare.net".into();
        config.passkey_rp_origin = "https://cook.vichare.net".into();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn production_requires_otp_smtp_and_passkey_secrets() {
        let mut config = Config::test();
        config.environment = AppEnvironment::Production;
        config.auth_mode = AuthMode::Google;
        config.public_origin = "https://cook.vichare.net".into();
        config.oauth_state_digest_key = [1; 32];
        config.session_digest_key = [2; 32];
        config.google_client_id = Some("client".into());
        config.google_client_secret = Some("secret".into());
        config.passkey_rp_id = "cook.vichare.net".into();
        config.passkey_rp_origin = "https://cook.vichare.net".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn development_auth_cannot_bind_to_a_public_interface() {
        let mut config = Config::test();
        config.environment = AppEnvironment::Development;
        config.bind = "0.0.0.0:8080".parse().unwrap();
        assert!(config.validate().is_err());
    }
}
