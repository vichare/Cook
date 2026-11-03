use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Configuration(String),
    #[error("authentication required")]
    Unauthorized,
    #[error("too many login attempts")]
    RateLimited,
    #[error("authentication provider unavailable")]
    AuthUnavailable,
    #[error("resource not found")]
    NotFound,
    #[error("permission denied")]
    Forbidden,
    #[error("{0}")]
    PayloadTooLarge(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Data(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl AppError {
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::AuthUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "auth_unavailable"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"),
            Self::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Self::Validation(_) => (StatusCode::BAD_REQUEST, "validation_error"),
            Self::Configuration(_)
            | Self::Data(_)
            | Self::Database(_)
            | Self::Migration(_)
            | Self::Io(_) => {
                tracing::error!(error = %self, "request failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
        };
        let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
            "The server could not complete the request.".to_string()
        } else {
            self.to_string()
        };
        (status, Json(ErrorBody { error, message })).into_response()
    }
}
