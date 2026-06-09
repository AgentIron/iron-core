/// Embedded migration SQL for the config store schema.
///
/// Migrations are applied in order. Each migration is idempotent where possible.
pub const MIGRATIONS: &[(i64, &str)] = &[(
    1,
    r#"
        CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY
        );

        CREATE TABLE IF NOT EXISTS profiles (
            id TEXT PRIMARY KEY,
            schema_version INTEGER NOT NULL,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS prompts (
            id TEXT PRIMARY KEY,
            schema_version INTEGER NOT NULL,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS schedule (
            id TEXT PRIMARY KEY,
            schema_version INTEGER NOT NULL,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS credentials (
            provider_slug TEXT PRIMARY KEY,
            credential_mode TEXT NOT NULL,
            encrypted_payload BLOB NOT NULL,
            nonce BLOB NOT NULL,
            encryption_metadata TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        INSERT OR IGNORE INTO schema_version (version) VALUES (1);
        "#,
)];

/// The current schema version.
#[allow(dead_code)]
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

/// Apply all pending migrations to the database.
pub async fn apply_migrations(pool: &sqlx::SqlitePool) -> Result<(), super::error::ConfigError> {
    // Create schema_version table if it doesn't exist
    sqlx::query("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .map_err(super::error::ConfigError::from)?;

    // Check current version
    let current_version: Option<i64> =
        sqlx::query_scalar("SELECT version FROM schema_version LIMIT 1")
            .fetch_optional(pool)
            .await
            .map_err(super::error::ConfigError::from)?;

    let current_version = current_version.unwrap_or(0);

    // Apply pending migrations
    for (version, sql) in MIGRATIONS {
        if *version > current_version {
            let mut tx = pool
                .begin()
                .await
                .map_err(|e| super::error::ConfigError::Migration(e.to_string()))?;

            sqlx::query(sql).execute(&mut *tx).await.map_err(|e| {
                super::error::ConfigError::Migration(format!("Migration {} failed: {}", version, e))
            })?;

            sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (?)")
                .bind(*version)
                .execute(&mut *tx)
                .await
                .map_err(|e| {
                    super::error::ConfigError::Migration(format!(
                        "Failed to update schema version to {}: {}",
                        version, e
                    ))
                })?;

            tx.commit().await.map_err(|e| {
                super::error::ConfigError::Migration(format!(
                    "Failed to commit migration {}: {}",
                    version, e
                ))
            })?;
        }
    }

    Ok(())
}
