//! Errors produced by durable configuration, migration, and encryption operations.

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

    /// Invalid or malformed credential encryption key.
    #[error("Invalid credential key: {0}")]
    InvalidKey(String),

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

    /// Effective model catalog error.
    #[error("Model catalog error: {0}")]
    Catalog(String),

    /// Automation task references a stored prompt that does not exist.
    #[error("Automation task references unknown stored prompt: {0}")]
    UnknownStoredPrompt(String),

    /// Stored prompt deletion blocked because automation tasks reference it.
    #[error("Stored prompt '{prompt_id}' is referenced by automation tasks: {task_ids:?}")]
    PromptReferencedByTasks {
        /// The prompt that was being deleted.
        prompt_id: String,
        /// IDs of tasks that reference the prompt.
        task_ids: Vec<String>,
    },

    /// Automation task normalized name collides with an existing task.
    #[error(
        "Task normalized name '{normalized_name}' collides with existing task '{existing_id}'"
    )]
    TaskNameConflict {
        /// The normalized name that collided.
        normalized_name: String,
        /// The ID of the existing task that already owns the name.
        existing_id: String,
    },

    /// Schedule references an automation task that does not exist.
    #[error("Schedule references unknown automation task: {0}")]
    UnknownAutomationTask(String),

    /// Automation task deletion blocked because schedules reference it.
    #[error("Automation task '{task_id}' is referenced by schedules: {schedule_ids:?}")]
    TaskReferencedBySchedules {
        /// The task that was being deleted.
        task_id: String,
        /// IDs of schedules that reference the task.
        schedule_ids: Vec<String>,
    },

    /// Stored prompt normalized name collides with an existing prompt.
    #[error(
        "Prompt normalized name '{normalized_name}' collides with existing prompt '{existing_id}'"
    )]
    PromptNameConflict {
        /// The normalized name that collided.
        normalized_name: String,
        /// The ID of the existing prompt that already owns the name.
        existing_id: String,
    },

    /// Profile deletion blocked because stored prompts reference it.
    #[error("Profile '{profile_id}' is referenced by prompts: {prompt_ids:?}")]
    ProfileReferencedByPrompts {
        /// The profile that was being deleted.
        profile_id: String,
        /// IDs of prompts that reference the profile.
        prompt_ids: Vec<String>,
    },

    /// Deletion cannot proceed because malformed records prevent reference checking.
    #[error("Cannot verify referential integrity due to malformed records: {details}")]
    IntegrityUnknown {
        /// Human-readable details about the unreadable records.
        details: String,
    },

    /// Policy-aware profile deletion would leave fewer valid persisted
    /// profiles than the caller requested.
    #[error(
        "Cannot delete profile: {remaining} valid profile(s) would remain, below the requested minimum of {minimum}"
    )]
    MinimumValidProfiles {
        /// The minimum valid-profile count the caller requested to remain.
        minimum: usize,
        /// The computed valid-profile count that would remain after deletion.
        remaining: usize,
    },
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
        use serde_json::error::Category;
        match err.classify() {
            Category::Io => ConfigError::Serialization(format!("IO error: {}", err)),
            Category::Syntax | Category::Data | Category::Eof => {
                ConfigError::Deserialization(err.to_string())
            }
        }
    }
}

impl From<super::effective_catalog::CatalogError> for ConfigError {
    fn from(err: super::effective_catalog::CatalogError) -> Self {
        ConfigError::Catalog(err.to_string())
    }
}
