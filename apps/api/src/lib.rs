pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod importer;
pub mod imports;
pub mod models;
pub mod passwordless;
pub mod recommendation;
pub mod routes;

use std::sync::Arc;

use axum::Router;
use config::Config;
use error::AppResult;
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
    pub auth: auth::AuthRuntime,
}

pub async fn build_app(config: Config) -> AppResult<Router> {
    config.validate()?;
    let pool = db::connect(&config.database_url).await?;
    let auth = auth::build_auth(&config, &pool)?;
    let state = AppState {
        pool,
        config: Arc::new(config),
        auth,
    };
    Ok(
        routes::router(state).layer(TraceLayer::new_for_http().make_span_with(
            |request: &axum::http::Request<_>| {
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    path = %request.uri().path()
                )
            },
        )),
    )
}
