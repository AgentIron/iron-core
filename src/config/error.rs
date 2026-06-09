use thiserror::Error;

/// Errors that can occur when interacting with the config store.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Failed to resolve or create the config directory path.
    #[error("Config path error: {0}")]
    Path(String),

    /// Failed to open the database.
    #[error("Database open failed: {0}")]
    DatabaseOpen(String),

    /// Schema migration failed.
    #[error("Migration failed: {0}")]
    Migration(String),

    /// Database query failed.
    #[error("Query failed: {0}")]
    Query(String),

    /// Serialization failed.
    #[error("Serialization failed: {0}")]
    Serialization(String),

    /// Deserialization failed.
    #[error("Deserialization failed: {0}")]
    Deserialization(String),

    /// Record not found.
    #[error("Record not found: {0}")]
    NotFound(String),

    /// Unique constraint violation.
    #[error("Conflict: {0}")]
    Conflict(String),

    /// Credential encryption key not available.
    #[error("Credential key unavailable: {0}")]
    KeyUnavailable(String),

    /// Encryption failed.
    #[error("Encryption failed: {0}")]
    Encryption(String),

    /// Decryption failed.
    #[error("Decryption failed: {0}")]
    Decryption(String),

    /// Database busy timeout exceeded.
    #[error("Database busy: {0}")]
    BusyTimeout(String),

    /// Input validation failed.
    #[error("Validation error: {0}")]
    Validation(String),
}

impl From<sqlx::Error> for ConfigError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => ConfigError::NotFound("record not found".to_string()),
            sqlx::Error::Database(db_err) => {
                if db_err.message().contains("UNIQUE constraint failed") {
                    ConfigError::Conflict(db_err.message().to_string())
                } else if db_err.message().contains("busy")
                    || db_err.message().contains("database is locked")
                {
                    ConfigError::BusyTimeout(db_err.message().to_string())
                } else {
                    ConfigError::Query(db_err.message().to_string())
                }
            }
            _ => ConfigError::Query(err.to_string()),
        }
    }
}

impl From<std::io::Error> for ConfigError {
    fn from(err: std::io::Error) -> Self {
        ConfigError::Path(err.to_string())
    }
}

impl From<serde_json::Error> for ConfigError {
    fn from(err: serde_json::Error) -> Self {
        ConfigError::Serialization(err.to_string())
    }
}
