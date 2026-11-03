use std::{path::Path, str::FromStr};

use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Row, SqlitePool,
};

use crate::{error::AppResult, ingredients::normalize_ingredient};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub async fn connect(database_url: &str) -> AppResult<SqlitePool> {
    if let Some(path) = database_url.strip_prefix("sqlite://") {
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(5));
    let max_connections = if database_url == "sqlite::memory:" {
        1
    } else {
        8
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await?;
    MIGRATOR.run(&pool).await?;
    normalize_existing_ingredients(&pool).await?;
    Ok(pool)
}

async fn normalize_existing_ingredients(pool: &SqlitePool) -> AppResult<()> {
    let mut transaction = pool.begin().await?;
    let already_applied: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM app_data_migrations WHERE name = 'ingredient_nfkc_v1'")
            .fetch_optional(&mut *transaction)
            .await?;
    if already_applied.is_some() {
        transaction.commit().await?;
        return Ok(());
    }
    let catalogs = sqlx::query("SELECT id, display_name, normalized_name FROM ingredient_catalog")
        .fetch_all(&mut *transaction)
        .await?;
    for row in catalogs {
        let id: String = row.get("id");
        let display_name: String = row.get("display_name");
        let stored_normalized: String = row.get("normalized_name");
        let normalized = normalize_ingredient(&display_name);
        if normalized.is_empty() || normalized == stored_normalized {
            continue;
        }
        let target_id: Option<String> =
            sqlx::query_scalar("SELECT id FROM ingredient_catalog WHERE normalized_name = ?")
                .bind(&normalized)
                .fetch_optional(&mut *transaction)
                .await?;
        if let Some(target_id) = target_id {
            merge_pantry_catalog_rows(&mut transaction, &id, &target_id).await?;
            sqlx::query(
                "UPDATE recipe_ingredients SET canonical_ingredient_id = ?
                 WHERE canonical_ingredient_id = ?",
            )
            .bind(&target_id)
            .bind(&id)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "UPDATE ingredient_aliases SET canonical_ingredient_id = ?
                 WHERE canonical_ingredient_id = ?",
            )
            .bind(&target_id)
            .bind(&id)
            .execute(&mut *transaction)
            .await?;
            sqlx::query("DELETE FROM ingredient_catalog WHERE id = ?")
                .bind(&id)
                .execute(&mut *transaction)
                .await?;
        } else {
            sqlx::query("UPDATE ingredient_catalog SET normalized_name = ? WHERE id = ?")
                .bind(normalized)
                .bind(id)
                .execute(&mut *transaction)
                .await?;
        }
    }
    let recipe_rows = sqlx::query("SELECT id, name FROM recipe_ingredients")
        .fetch_all(&mut *transaction)
        .await?;
    for row in recipe_rows {
        sqlx::query("UPDATE recipe_ingredients SET normalized_name = ? WHERE id = ?")
            .bind(normalize_ingredient(row.get::<String, _>("name").as_str()))
            .bind(row.get::<String, _>("id"))
            .execute(&mut *transaction)
            .await?;
    }
    let pantry_rows = sqlx::query("SELECT id, name FROM pantry_items")
        .fetch_all(&mut *transaction)
        .await?;
    for row in pantry_rows {
        sqlx::query("UPDATE pantry_items SET normalized_name = ? WHERE id = ?")
            .bind(normalize_ingredient(row.get::<String, _>("name").as_str()))
            .bind(row.get::<String, _>("id"))
            .execute(&mut *transaction)
            .await?;
    }
    sqlx::query("INSERT INTO app_data_migrations (name) VALUES ('ingredient_nfkc_v1')")
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn merge_pantry_catalog_rows(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    old_id: &str,
    target_id: &str,
) -> AppResult<()> {
    let duplicates = sqlx::query(
        "SELECT old.id AS old_row_id, current.id AS current_row_id,
                old.status AS old_status, current.status AS current_status,
                old.quantity_note AS old_note, current.quantity_note AS current_note,
                old.expires_on AS old_expiry, current.expires_on AS current_expiry
         FROM pantry_items old
         JOIN pantry_items current ON current.owner_id = old.owner_id
         WHERE old.canonical_ingredient_id = ?
           AND current.canonical_ingredient_id = ?",
    )
    .bind(old_id)
    .bind(target_id)
    .fetch_all(&mut **transaction)
    .await?;
    for row in duplicates {
        let old_status: String = row.get("old_status");
        let current_status: String = row.get("current_status");
        let old_note: String = row.get("old_note");
        let current_note: String = row.get("current_note");
        let note = [current_note, old_note]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        let old_expiry: Option<String> = row.get("old_expiry");
        let current_expiry: Option<String> = row.get("current_expiry");
        let expiry = [current_expiry, old_expiry].into_iter().flatten().min();
        sqlx::query(
            "UPDATE pantry_items
             SET status = ?, quantity_note = ?, expires_on = ?
             WHERE id = ?",
        )
        .bind(if old_status == current_status {
            current_status
        } else {
            "unknown".into()
        })
        .bind(note)
        .bind(expiry)
        .bind(row.get::<String, _>("current_row_id"))
        .execute(&mut **transaction)
        .await?;
        sqlx::query("DELETE FROM pantry_items WHERE id = ?")
            .bind(row.get::<String, _>("old_row_id"))
            .execute(&mut **transaction)
            .await?;
    }
    sqlx::query(
        "UPDATE pantry_items SET canonical_ingredient_id = ?
         WHERE canonical_ingredient_id = ?",
    )
    .bind(target_id)
    .bind(old_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_are_recorded_and_safe_to_reopen() {
        let path = std::env::temp_dir().join(format!("cook-{}.db", uuid::Uuid::new_v4()));
        let url = format!("sqlite://{}", path.display());
        let pool = connect(&url).await.unwrap();
        let applied: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(applied.0, 5);
        pool.close().await;

        let reopened = connect(&url).await.unwrap();
        reopened.close().await;
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn startup_normalization_merges_legacy_nfkc_duplicates_conservatively() {
        let pool = connect("sqlite::memory:").await.unwrap();
        sqlx::query("INSERT INTO users (id, email, display_name) VALUES ('u', 'u@test', 'U')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM app_data_migrations WHERE name = 'ingredient_nfkc_v1'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO ingredient_catalog (id, display_name, normalized_name, created_at)
             VALUES ('ascii', 'Milk', 'milk', 1), ('wide', 'Ｍｉｌｋ', 'ｍｉｌｋ', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO pantry_items
             (id, owner_id, name, normalized_name, status, quantity_note, canonical_ingredient_id)
             VALUES
             ('p1', 'u', 'Milk', 'milk', 'have', 'carton', 'ascii'),
             ('p2', 'u', 'Ｍｉｌｋ', 'ｍｉｌｋ', 'out', 'powder', 'wide')",
        )
        .execute(&pool)
        .await
        .unwrap();

        normalize_existing_ingredients(&pool).await.unwrap();

        let catalog_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ingredient_catalog")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(catalog_count, 1);
        let pantry: (String, String, String, String) = sqlx::query_as(
            "SELECT normalized_name, status, quantity_note, canonical_ingredient_id
             FROM pantry_items",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pantry.0, "milk");
        assert_eq!(pantry.1, "unknown");
        assert!(pantry.2.contains("carton"));
        assert!(pantry.2.contains("powder"));
        assert_eq!(pantry.3, "ascii");
    }
}
