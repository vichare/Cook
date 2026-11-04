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
    #[error("fresh Passkey verification is required")]
    AdminReauthRequired,
    #[error("{0}")]
    PayloadTooLarge(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{message}")]
    ModerationConflict {
        message: String,
        recipe_version: Option<i64>,
        moderation_version: Option<i64>,
        case_version: Option<i64>,
        appeal_version: Option<i64>,
    },
    #[error("selected canonical ingredient is not available to this account")]
    CanonicalIngredientsUnavailable {
        canonical_ingredient_ids: Vec<String>,
    },
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
#[serde(rename_all = "camelCase")]
struct ErrorBody {
    error: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    recipe_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    moderation_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    case_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    appeal_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    canonical_ingredient_ids: Option<Vec<String>>,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let self_ = match self {
            Self::ModerationConflict {
                message,
                recipe_version,
                moderation_version,
                case_version,
                appeal_version,
            } => {
                return (
                    StatusCode::CONFLICT,
                    Json(ErrorBody {
                        error: "conflict",
                        message,
                        recipe_version,
                        moderation_version,
                        case_version,
                        appeal_version,
                        canonical_ingredient_ids: None,
                    }),
                )
                    .into_response();
            }
            Self::CanonicalIngredientsUnavailable {
                canonical_ingredient_ids,
            } => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorBody {
                        error: "canonical_ingredient_unavailable",
                        message: "selected canonical ingredient is not available to this account"
                            .to_string(),
                        recipe_version: None,
                        moderation_version: None,
                        case_version: None,
                        appeal_version: None,
                        canonical_ingredient_ids: Some(canonical_ingredient_ids),
                    }),
                )
                    .into_response();
            }
            other => other,
        };
        let (status, error) = match self_ {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::AuthUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "auth_unavailable"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::AdminReauthRequired => (StatusCode::FORBIDDEN, "admin_passkey_reauth_required"),
            Self::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"),
            Self::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Self::ModerationConflict { .. } => unreachable!(),
            Self::CanonicalIngredientsUnavailable { .. } => unreachable!(),
            Self::Validation(_) => (StatusCode::BAD_REQUEST, "validation_error"),
            Self::Configuration(_)
            | Self::Data(_)
            | Self::Database(_)
            | Self::Migration(_)
            | Self::Io(_) => {
                tracing::error!(error = %self_, "request failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
        };
        let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
            "The server could not complete the request.".to_string()
        } else {
            self_.to_string()
        };
        (
            status,
            Json(ErrorBody {
                error,
                message,
                recipe_version: None,
                moderation_version: None,
                case_version: None,
                appeal_version: None,
                canonical_ingredient_ids: None,
            }),
        )
            .into_response()
    }
}
