use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;
use std::time::Duration;

/// Create a SQLite connection pool with WAL mode and busy timeout.
pub async fn create_pool(path: &Path) -> Result<SqlitePool, super::error::ConfigError> {
    create_pool_with_timeout(path, Duration::from_secs(5)).await
}

/// Create a SQLite connection pool with WAL mode and a custom busy timeout.
///
/// Foreign-key enforcement is enabled explicitly on every connection (sqlx
/// also enables it by default) so schema-level `FOREIGN KEY` constraints are
/// always active. See [`migrations`](super::migrations) for declared
/// constraints.
pub async fn create_pool_with_timeout(
    path: &Path,
    busy_timeout: Duration,
) -> Result<SqlitePool, super::error::ConfigError> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(busy_timeout)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .map_err(|e| super::error::ConfigError::DatabaseOpen(e.to_string()))?;

    Ok(pool)
}

/// Create an in-memory SQLite pool for testing.
///
/// Foreign-key enforcement is enabled to keep test behavior consistent with
/// the file-backed pool.
pub async fn create_memory_pool() -> Result<SqlitePool, super::error::ConfigError> {
    let options = SqliteConnectOptions::new()
        .filename(":memory:")
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|e| super::error::ConfigError::DatabaseOpen(e.to_string()))?;

    Ok(pool)
}
