use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::PathBuf;

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

// ============================================================================
// Runtime Settings Records (Issue #67)
// ============================================================================

/// A stored provider runtime configuration record.
#[derive(Debug, Clone)]
pub struct ProviderConfigRecord {
    pub provider_slug: String,
    pub display_name: String,
    pub enabled: bool,
    pub base_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating a provider runtime configuration.
#[derive(Debug, Clone)]
pub struct ProviderConfigInput {
    pub provider_slug: String,
    pub display_name: String,
    pub enabled: bool,
    pub base_url: Option<String>,
}

/// A stored custom model record.
#[derive(Debug, Clone)]
pub struct CustomModelRecord {
    pub provider_slug: String,
    pub model_id: String,
    pub display_name: String,
    pub context_window: Option<u32>,
    pub output_limit: Option<u32>,
    pub supports_tool_calls: bool,
    pub supports_reasoning: bool,
    pub supports_vision: bool,
    pub supports_streaming: bool,
    pub reasoning_effort_values: Vec<String>,
    pub cost_input_per_million: Option<f64>,
    pub cost_output_per_million: Option<f64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating a custom model.
#[derive(Debug, Clone)]
pub struct CustomModelInput {
    pub provider_slug: String,
    pub model_id: String,
    pub display_name: String,
    pub context_window: Option<u32>,
    pub output_limit: Option<u32>,
    pub supports_tool_calls: bool,
    pub supports_reasoning: bool,
    pub supports_vision: bool,
    pub supports_streaming: bool,
    pub reasoning_effort_values: Vec<String>,
    pub cost_input_per_million: Option<f64>,
    pub cost_output_per_million: Option<f64>,
}

/// A stored default model selection record.
#[derive(Debug, Clone)]
pub struct DefaultModelRecord {
    pub provider_slug: String,
    pub model_id: String,
    pub updated_at: DateTime<Utc>,
}

/// Input for setting the default model.
#[derive(Debug, Clone)]
pub struct DefaultModelInput {
    pub provider_slug: String,
    pub model_id: String,
}

/// A stored MCP server configuration record.
#[derive(Debug, Clone)]
pub struct McpServerConfigRecord {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub transport: crate::mcp::server::McpTransport,
    pub working_dir: Option<PathBuf>,
    pub enabled_by_default: bool,
    pub inherited_env_vars: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating an MCP server configuration.
#[derive(Debug, Clone)]
pub struct McpServerConfigInput {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub transport: crate::mcp::server::McpTransport,
    pub working_dir: Option<PathBuf>,
    pub enabled_by_default: bool,
    pub inherited_env_vars: Vec<String>,
}

/// A stored skill settings record.
#[derive(Debug, Clone)]
pub struct SkillSettingsRecord {
    pub trust_project_skills: bool,
    pub additional_skill_dirs: Vec<PathBuf>,
    pub updated_at: DateTime<Utc>,
}

/// Input for setting skill settings.
#[derive(Debug, Clone)]
pub struct SkillSettingsInput {
    pub trust_project_skills: bool,
    pub additional_skill_dirs: Vec<PathBuf>,
}

/// Validated runtime settings snapshot loaded from the config store.
#[derive(Debug, Clone)]
pub struct RuntimeSettingsSnapshot {
    pub provider_configs: Vec<ProviderConfigRecord>,
    pub custom_models: Vec<CustomModelRecord>,
    pub default_model: Option<DefaultModelRecord>,
    pub mcp_servers: Vec<McpServerConfigRecord>,
    pub skill_settings: SkillSettingsRecord,
}
