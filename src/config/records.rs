//! Typed inputs and records for the durable configuration schema.
//!
//! Record types mirror values read from SQLite, including durable timestamps.
//! Input types contain only caller-controlled columns; the store assigns and
//! preserves timestamps when it inserts or updates rows.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::PathBuf;

/// A stored profile record.
#[derive(Debug, Clone)]
pub struct ProfileRecord {
    /// Stable primary key of the profile row.
    pub id: String,
    /// Version that determines how to decode [`Self::payload`].
    pub schema_version: i64,
    /// Serialized profile document retained as JSON for version-aware decoding.
    pub payload: Value,
    /// Time at which the row was first inserted.
    pub created_at: DateTime<Utc>,
    /// Time at which the row was last replaced.
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating a profile.
#[derive(Debug, Clone)]
pub struct ProfileInput {
    /// Stable primary key to insert or replace.
    pub id: String,
    /// Schema version of [`Self::payload`].
    pub schema_version: i64,
    /// Profile document to serialize into the row.
    pub payload: Value,
}

/// A stored prompt record.
#[derive(Debug, Clone)]
pub struct PromptRecord {
    /// Immutable primary key of the prompt row.
    pub id: String,
    /// Version that determines how to decode [`Self::payload`].
    pub schema_version: i64,
    /// Serialized stored-prompt document.
    pub payload: Value,
    /// User-facing prompt name duplicated from the typed payload for identity management.
    pub display_name: String,
    /// Canonical lookup handle protected by a unique database index.
    pub normalized_name: String,
    /// `"ready"` or `"needs_rename"`.
    pub identity_state: String,
    /// Time at which the row was first inserted.
    pub created_at: DateTime<Utc>,
    /// Time at which the row was last replaced or renamed.
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating a prompt.
#[derive(Debug, Clone)]
pub struct PromptInput {
    /// Immutable primary key to insert or replace.
    pub id: String,
    /// Schema version of [`Self::payload`].
    pub schema_version: i64,
    /// Stored-prompt document to serialize into the row.
    pub payload: Value,
    /// User-facing name, which must agree with the typed payload for current schemas.
    pub display_name: String,
    /// Canonical handle, which must agree with the typed payload for current schemas.
    pub normalized_name: String,
}

/// A stored schedule record.
#[derive(Debug, Clone)]
pub struct ScheduleRecord {
    /// Stable primary key of the desired schedule row.
    pub id: String,
    /// Version that determines how to decode [`Self::payload`].
    pub schema_version: i64,
    /// Serialized desired scheduled-task state.
    pub payload: Value,
    /// Time at which the desired schedule was first inserted.
    pub created_at: DateTime<Utc>,
    /// Time at which the desired schedule was last replaced.
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating a schedule entry.
#[derive(Debug, Clone)]
pub struct ScheduleInput {
    /// Stable primary key to insert or replace.
    pub id: String,
    /// Schema version of [`Self::payload`].
    pub schema_version: i64,
    /// Desired schedule document to serialize into the row.
    pub payload: Value,
}

/// A stored provider credential record (metadata only, secrets encrypted).
#[derive(Debug, Clone)]
pub struct CredentialRecord {
    /// Provider slug that uniquely owns the credential row.
    pub provider_slug: String,
    /// Persisted mode discriminator, such as `api_key` or `oauth_bearer`.
    pub credential_mode: String,
    /// Time at which a credential was first stored for the provider.
    pub created_at: DateTime<Utc>,
    /// Time at which the provider's credential was last replaced or refreshed.
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Runtime Settings Records (Issue #67)
// ============================================================================

/// A stored provider runtime configuration record.
#[derive(Debug, Clone)]
pub struct ProviderConfigRecord {
    /// Provider slug used as the row's primary key.
    pub provider_slug: String,
    /// Human-readable provider name.
    pub display_name: String,
    /// Whether the provider is enabled for runtime use.
    pub enabled: bool,
    /// Optional API base URL override.
    pub base_url: Option<String>,
    /// Time at which the configuration was first inserted.
    pub created_at: DateTime<Utc>,
    /// Time at which the configuration was last replaced.
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating a provider runtime configuration.
#[derive(Debug, Clone)]
pub struct ProviderConfigInput {
    /// Provider slug to insert or replace.
    pub provider_slug: String,
    /// Human-readable provider name.
    pub display_name: String,
    /// Whether the provider should be enabled for runtime use.
    pub enabled: bool,
    /// Optional API base URL override.
    pub base_url: Option<String>,
}

/// A stored custom model record.
#[derive(Debug, Clone)]
pub struct CustomModelRecord {
    /// Provider that exposes the model.
    pub provider_slug: String,
    /// Provider-specific model identifier; unique with [`Self::provider_slug`].
    pub model_id: String,
    /// Human-readable model name.
    pub display_name: String,
    /// Maximum input context in tokens, when known.
    pub context_window: Option<u32>,
    /// Maximum generated output in tokens, when known.
    pub output_limit: Option<u32>,
    /// Whether the model accepts tool definitions and calls.
    pub supports_tool_calls: bool,
    /// Whether the model exposes reasoning controls.
    pub supports_reasoning: bool,
    /// Whether the model accepts image input.
    pub supports_vision: bool,
    /// Whether the model can stream responses.
    pub supports_streaming: bool,
    /// Provider-accepted reasoning effort values.
    pub reasoning_effort_values: Vec<String>,
    /// Input-token price per million tokens, when configured.
    pub cost_input_per_million: Option<f64>,
    /// Output-token price per million tokens, when configured.
    pub cost_output_per_million: Option<f64>,
    /// Time at which the custom model was first inserted.
    pub created_at: DateTime<Utc>,
    /// Time at which the custom model was last replaced.
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating a custom model.
#[derive(Debug, Clone)]
pub struct CustomModelInput {
    /// Provider that exposes the model.
    pub provider_slug: String,
    /// Provider-specific model identifier.
    pub model_id: String,
    /// Human-readable model name.
    pub display_name: String,
    /// Maximum input context in tokens, when known.
    pub context_window: Option<u32>,
    /// Maximum generated output in tokens, when known.
    pub output_limit: Option<u32>,
    /// Whether the model accepts tool definitions and calls.
    pub supports_tool_calls: bool,
    /// Whether the model exposes reasoning controls.
    pub supports_reasoning: bool,
    /// Whether the model accepts image input.
    pub supports_vision: bool,
    /// Whether the model can stream responses.
    pub supports_streaming: bool,
    /// Provider-accepted reasoning effort values.
    pub reasoning_effort_values: Vec<String>,
    /// Input-token price per million tokens; must be finite and non-negative.
    pub cost_input_per_million: Option<f64>,
    /// Output-token price per million tokens; must be finite and non-negative.
    pub cost_output_per_million: Option<f64>,
}

/// A stored default model selection record.
#[derive(Debug, Clone)]
pub struct DefaultModelRecord {
    /// Provider of the selected default model.
    pub provider_slug: String,
    /// Provider-specific identifier of the selected model.
    pub model_id: String,
    /// Time at which the singleton selection was last changed.
    pub updated_at: DateTime<Utc>,
}

/// Input for setting the default model.
#[derive(Debug, Clone)]
pub struct DefaultModelInput {
    /// Provider of the requested default model.
    pub provider_slug: String,
    /// Provider-specific identifier of the requested model.
    pub model_id: String,
}

/// A stored MCP server configuration record.
#[derive(Debug, Clone)]
pub struct McpServerConfigRecord {
    /// Stable server configuration identifier.
    pub id: String,
    /// Human-readable server label.
    pub label: String,
    /// Optional user-facing description.
    pub description: Option<String>,
    /// Process or HTTP transport used to connect to the server.
    pub transport: crate::mcp::server::McpTransport,
    /// Optional working directory for process-based transports.
    pub working_dir: Option<PathBuf>,
    /// Whether new sessions enable this server automatically.
    pub enabled_by_default: bool,
    /// Environment variable names inherited from the host process.
    pub inherited_env_vars: Vec<String>,
    /// Time at which the server configuration was first inserted.
    pub created_at: DateTime<Utc>,
    /// Time at which the server configuration was last replaced.
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating an MCP server configuration.
#[derive(Debug, Clone)]
pub struct McpServerConfigInput {
    /// Stable server configuration identifier.
    pub id: String,
    /// Human-readable server label.
    pub label: String,
    /// Optional user-facing description.
    pub description: Option<String>,
    /// Process or HTTP transport used to connect to the server.
    pub transport: crate::mcp::server::McpTransport,
    /// Optional working directory for process-based transports.
    pub working_dir: Option<PathBuf>,
    /// Whether new sessions should enable this server automatically.
    pub enabled_by_default: bool,
    /// Environment variable names to inherit from the host process.
    pub inherited_env_vars: Vec<String>,
}

/// A stored skill settings record.
#[derive(Debug, Clone)]
pub struct SkillSettingsRecord {
    /// Whether project-local skill definitions are trusted.
    pub trust_project_skills: bool,
    /// Additional directories included in skill discovery.
    pub additional_skill_dirs: Vec<PathBuf>,
    /// Time at which the singleton settings row was last changed.
    pub updated_at: DateTime<Utc>,
}

/// Input for setting skill settings.
#[derive(Debug, Clone)]
pub struct SkillSettingsInput {
    /// Whether project-local skill definitions should be trusted.
    pub trust_project_skills: bool,
    /// Additional directories to include in skill discovery.
    pub additional_skill_dirs: Vec<PathBuf>,
}

/// Validated runtime settings snapshot loaded from the config store.
#[derive(Debug, Clone)]
pub struct RuntimeSettingsSnapshot {
    /// Persisted provider runtime configurations.
    pub provider_configs: Vec<ProviderConfigRecord>,
    /// Persisted custom model definitions.
    pub custom_models: Vec<CustomModelRecord>,
    /// Selected default model, if configured.
    pub default_model: Option<DefaultModelRecord>,
    /// Persisted MCP server configurations.
    pub mcp_servers: Vec<McpServerConfigRecord>,
    /// Effective persisted skill settings, including defaults when no row exists.
    pub skill_settings: SkillSettingsRecord,
}

/// A stored bootstrap metadata record.
#[derive(Debug, Clone)]
pub struct BootstrapMetadataRecord {
    /// Namespace that prevents unrelated bootstrap processes from colliding.
    pub domain: String,
    /// Metadata key within [`Self::domain`].
    pub key: String,
    /// Opaque marker or version value.
    pub value: String,
    /// Time at which the value was last written.
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating bootstrap metadata.
#[derive(Debug, Clone)]
pub struct BootstrapMetadataInput {
    /// Namespace for the bootstrap metadata.
    pub domain: String,
    /// Metadata key within [`Self::domain`].
    pub key: String,
    /// Opaque marker or version value to persist.
    pub value: String,
}

/// Metadata for a saved handoff bundle.
#[derive(Debug, Clone)]
pub struct SavedHandoffMetadata {
    /// Stable primary key of the saved handoff.
    pub id: String,
    /// Human-readable handoff name.
    pub name: String,
    /// Serialized handoff bundle format version.
    pub bundle_version: String,
    /// Session from which the handoff originated, when recorded.
    pub source_session_id: Option<String>,
    /// Model active when the handoff was created, when recorded.
    pub source_model: Option<String>,
    /// Provider active when the handoff was created, when recorded.
    pub source_provider: Option<String>,
    /// Estimated token size stored with the bundle metadata.
    pub size_estimate_tokens: usize,
    /// Time at which the handoff was first saved.
    pub created_at: DateTime<Utc>,
    /// Time at which the handoff was last replaced.
    pub updated_at: DateTime<Utc>,
}

/// A saved handoff record with the full bundle.
#[derive(Debug, Clone)]
pub struct SavedHandoffRecord {
    /// Indexed metadata read from the handoff row.
    pub metadata: SavedHandoffMetadata,
    /// Fully decoded durable handoff bundle.
    pub bundle: crate::context::handoff::HandoffBundle,
}

/// Input for saving a handoff bundle.
#[derive(Debug, Clone)]
pub struct SavedHandoffInput {
    /// Stable primary key to insert or replace.
    pub id: String,
    /// Human-readable handoff name.
    pub name: String,
    /// Durable bundle whose version and metadata are validated before storage.
    pub bundle: crate::context::handoff::HandoffBundle,
}

/// A stored provider profile record (non-secret provider protocol metadata).
#[derive(Debug, Clone)]
pub struct ProviderProfileRecord {
    /// Provider slug used as the row's primary key.
    pub slug: String,
    /// Validated non-secret provider protocol metadata encoded as JSON.
    pub profile_json: String,
    /// Optional provenance label for the imported profile.
    pub source: Option<String>,
    /// Time at which the profile was first inserted.
    pub created_at: DateTime<Utc>,
    /// Time at which the profile was last replaced.
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating a stored provider profile.
#[derive(Debug, Clone)]
pub struct ProviderProfileInput {
    /// Provider slug to insert or replace; must match the JSON payload's slug.
    pub slug: String,
    /// Non-secret provider profile JSON to validate and persist.
    pub profile_json: String,
    /// Optional provenance label for the imported profile.
    pub source: Option<String>,
}
