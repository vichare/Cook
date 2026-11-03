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

#[derive(Clone, Debug)]
pub struct Config {
    pub environment: AppEnvironment,
    pub auth_mode: AuthMode,
    pub bind: SocketAddr,
    pub database_url: String,
    pub public_origin: String,
    pub oauth_state_digest_key: [u8; 32],
    pub session_digest_key: [u8; 32],
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<String>,
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
            google_client_id: env::var("GOOGLE_CLIENT_ID").ok(),
            google_client_secret: env::var("GOOGLE_CLIENT_SECRET").ok(),
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
            google_client_id: None,
            google_client_secret: None,
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
            if self.oauth_state_digest_key == [0x42; 32]
                || self.session_digest_key == [0x24; 32]
                || self.oauth_state_digest_key == self.session_digest_key
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

fn require_secret(value: &Option<String>, name: &str) -> AppResult<()> {
    match value.as_deref() {
        Some(value) if !value.trim().is_empty() && !value.contains("replace") => Ok(()),
        _ => Err(AppError::configuration(format!(
            "production requires {name}"
        ))),
    }
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
        config.google_client_id = Some("client".into());
        config.google_client_secret = Some("secret".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn development_auth_cannot_bind_to_a_public_interface() {
        let mut config = Config::test();
        config.environment = AppEnvironment::Development;
        config.bind = "0.0.0.0:8080".parse().unwrap();
        assert!(config.validate().is_err());
    }
}
