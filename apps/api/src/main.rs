use std::env;

use chrono::Utc;
use cook_api::{
    build_app,
    config::Config,
    db,
    error::{AppError, AppResult},
};
use sha2::{Digest, Sha256};
use sqlx::Row;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

#[tokio::main]
async fn main() -> AppResult<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if !args.is_empty() {
        return run_host_command(&args).await;
    }
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cook_api=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    let config = Config::from_env()?;
    let bind = config.bind;
    let app = build_app(config).await?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "Cook API listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn run_host_command(args: &[String]) -> AppResult<()> {
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://data/cook.db".into());
    let pool = db::connect(&database_url).await?;
    match args {
        [moderation, claim_code, create]
            if moderation == "moderation" && claim_code == "claim-code" && create == "create" =>
        {
            let code = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
            let digest = Sha256::digest(code.as_bytes());
            let now = Utc::now().timestamp();
            sqlx::query(
                "INSERT INTO capability_claim_codes
                 (id, capability, code_digest, created_at, expires_at)
                 VALUES (?, 'content_moderation', ?, ?, ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(digest.as_slice())
            .bind(now)
            .bind(now + 15 * 60)
            .execute(&pool)
            .await?;
            println!("{code}");
        }
        [moderation, assignment, revoke, assignment_id]
            if moderation == "moderation" && assignment == "assignment" && revoke == "revoke" =>
        {
            Uuid::parse_str(assignment_id)
                .map_err(|_| AppError::configuration("assignment UUID is invalid"))?;
            let now = Utc::now().timestamp();
            let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
            let row = sqlx::query(
                "SELECT user_id FROM capability_assignments
                 WHERE id = ? AND capability = 'content_moderation' AND revoked_at IS NULL",
            )
            .bind(assignment_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;
            let user_id: String = row.get("user_id");
            sqlx::query("UPDATE capability_assignments SET revoked_at = ? WHERE id = ?")
                .bind(now)
                .bind(assignment_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE users SET authorization_version = authorization_version + 1 WHERE id = ?",
            )
            .bind(&user_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM auth_sessions WHERE user_id = ?")
                .bind(&user_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO moderation_audit_events
                 (id, event_type, assignment_id, created_at)
                 VALUES (?, 'capability_assignment_revoked', ?, ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(assignment_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        }
        _ => {
            return Err(AppError::configuration(
                "usage: cook-api moderation claim-code create | \
                 cook-api moderation assignment revoke <assignment-uuid>",
            ));
        }
    }
    pool.close().await;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
