use chrono::{DateTime, Utc};
use serde_json::Value;

/// A stored profile record.
#[derive(Debug, Clone)]
pub struct ProfileRecord {
    pub id: String,
    pub schema_version: i64,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating a profile.
#[derive(Debug, Clone)]
pub struct ProfileInput {
    pub id: String,
    pub schema_version: i64,
    pub payload: Value,
}

/// A stored prompt record.
#[derive(Debug, Clone)]
pub struct PromptRecord {
    pub id: String,
    pub schema_version: i64,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating a prompt.
#[derive(Debug, Clone)]
pub struct PromptInput {
    pub id: String,
    pub schema_version: i64,
    pub payload: Value,
}

/// A stored schedule record.
#[derive(Debug, Clone)]
pub struct ScheduleRecord {
    pub id: String,
    pub schema_version: i64,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating a schedule entry.
#[derive(Debug, Clone)]
pub struct ScheduleInput {
    pub id: String,
    pub schema_version: i64,
    pub payload: Value,
}

/// A stored provider credential record (metadata only, secrets encrypted).
#[derive(Debug, Clone)]
pub struct CredentialRecord {
    pub provider_slug: String,
    pub credential_mode: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
