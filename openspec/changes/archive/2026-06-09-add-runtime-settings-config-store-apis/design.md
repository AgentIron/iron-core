## Context

IC-1 established `ConfigStore` as the durable core-owned configuration abstraction for profiles, prompts, schedules, and encrypted provider credentials. Issue #67 covers the next layer of shared runtime setup currently owned by the AgentIron desktop app: provider config, default model selection, custom models, MCP servers, and skill settings.

The boundary is runtime setup, not every app preference. AgentIron-only preferences such as starred models stay in the app. Custom models move into `iron-core` because they affect provider/model selection and may be required for headless operation. Provider credentials stay in the encrypted credential API/store and must not be serialized into provider runtime config.

## Goals

- Add typed `ConfigStore` APIs for shared runtime settings needed by app and headless consumers.
- Keep the public abstraction named `ConfigStore`; the database is an implementation detail.
- Persist provider runtime config without credential fields or `iron-providers` protocol metadata.
- Persist provider display names copied from provider metadata for stable presentation, without user-editable naming semantics for now.
- Persist custom model records as core-owned provider/model metadata.
- Persist default model selection as typed `provider_slug` plus `model_id`.
- Persist MCP server definitions with `inherited_env_vars` for selected parent-env inheritance by name.
- Persist skill discovery/trust settings already represented by runtime `SkillConfig`.
- Provide a runtime settings snapshot API that validates cross-record relationships and projects durable settings into runtime startup inputs.
- Add persistence round-trip and missing/default behavior tests.

## Non-Goals

- Moving AgentIron `starredModels` into `iron-core`.
- Moving theme, autostart, shortcuts, user profile UI state, or other desktop-only preferences into `iron-core`.
- Persisting provider credentials in provider runtime config.
- Owning provider protocol metadata that belongs in `iron-providers`.
- Migrating existing AgentIron app settings rows; AgentIron/AgentIron#56 owns app migration.
- Letting frontends write SQLite directly.
- Normalizing MCP env/header values into a separate secret store in this change.
- Adding remote/cloud config sync.

## Public API Shape

`iron_core::config` should expose typed records and inputs for the runtime settings domains. Exact method names may be refined during implementation, but callers should not need SQL table names or app-owned JSON keys.

Representative API groups:

```rust
impl ConfigStore {
    async fn set_provider_config(&self, input: ProviderConfigInput) -> Result<ProviderConfigRecord, ConfigError>;
    async fn get_provider_config(&self, provider_slug: &str) -> Result<Option<ProviderConfigRecord>, ConfigError>;
    async fn list_provider_configs(&self) -> Result<Vec<ProviderConfigRecord>, ConfigError>;
    async fn remove_provider_config(&self, provider_slug: &str) -> Result<(), ConfigError>;

    async fn set_custom_model(&self, input: CustomModelInput) -> Result<CustomModelRecord, ConfigError>;
    async fn get_custom_model(&self, provider_slug: &str, model_id: &str) -> Result<Option<CustomModelRecord>, ConfigError>;
    async fn list_custom_models(&self, provider_slug: Option<&str>) -> Result<Vec<CustomModelRecord>, ConfigError>;
    async fn remove_custom_model(&self, provider_slug: &str, model_id: &str) -> Result<(), ConfigError>;

    async fn set_default_model(&self, input: DefaultModelInput) -> Result<DefaultModelRecord, ConfigError>;
    async fn get_default_model(&self) -> Result<Option<DefaultModelRecord>, ConfigError>;
    async fn clear_default_model(&self) -> Result<(), ConfigError>;

    async fn set_mcp_server(&self, input: McpServerConfigInput) -> Result<McpServerConfigRecord, ConfigError>;
    async fn get_mcp_server(&self, id: &str) -> Result<Option<McpServerConfigRecord>, ConfigError>;
    async fn list_mcp_servers(&self) -> Result<Vec<McpServerConfigRecord>, ConfigError>;
    async fn remove_mcp_server(&self, id: &str) -> Result<(), ConfigError>;

    async fn set_skill_settings(&self, input: SkillSettingsInput) -> Result<SkillSettingsRecord, ConfigError>;
    async fn get_skill_settings(&self) -> Result<SkillSettingsRecord, ConfigError>;

    async fn load_runtime_settings(&self) -> Result<RuntimeSettingsSnapshot, ConfigError>;
}
```

All methods should return `Result<T, ConfigError>`. Read-by-ID methods should return `Ok(None)` for missing optional records. Singleton reads may return a default record where the domain has clear defaults, such as skill settings defaulting to untrusted project skills and an empty additional directory list.

## Runtime Settings Types

Provider runtime config should include only non-secret, user/runtime fields:

- `provider_slug: String`
- `display_name: String`, copied from provider metadata when created or refreshed
- `enabled: bool`
- `base_url: Option<String>`
- store-maintained timestamps

Provider runtime config must not contain API keys, OAuth tokens, or auth mode. Provider credentials remain in the credential APIs and encrypted store.

Custom model config should be keyed by `(provider_slug, model_id)` and include display/capability/cost metadata needed for runtime selection:

- `provider_slug: String`
- `model_id: String`
- `display_name: String`
- `context_window: Option<u32>`
- `output_limit: Option<u32>`
- `supports_tool_calls: bool`
- `supports_reasoning: bool`
- `supports_vision: bool`
- `cost_input_per_million: Option<f64>`
- `cost_output_per_million: Option<f64>`
- store-maintained timestamps

Default model config should be a typed singleton with:

- `provider_slug: String`
- `model_id: String`
- store-maintained updated timestamp

MCP server config should align with the runtime server shape and add durable fields needed by the app:

- `id: String`
- `label: String`
- `description: Option<String>`
- `transport: Stdio | Http | HttpSse`
- stdio: `command`, `args`, `env`, `inherited_env_vars`
- http/http_sse: `url`, `headers`
- `working_dir: Option<PathBuf>`
- `enabled_by_default: bool`
- store-maintained timestamps

Skill settings should include:

- `trust_project_skills: bool`
- `additional_skill_dirs: Vec<PathBuf>`
- store-maintained updated timestamp

## Runtime Settings Snapshot

Fine-grained CRUD APIs are useful for settings UIs. Runtime startup and headless consumers need a single read path that applies consistent interpretation and validation. `load_runtime_settings()` should return a `RuntimeSettingsSnapshot` containing provider configs, custom models, default model selection, MCP server definitions, and skill settings.

The snapshot loader should validate core-owned invariants such as non-empty IDs/slugs, unique MCP server IDs, valid transport-specific field combinations, valid skill directory values, and default model shape. It should not require every built-in provider to have a persisted provider row because built-in provider metadata may come from `iron-providers`. Validation of whether a default model exists may consider built-in metadata plus persisted custom models where available.

The snapshot may provide helper methods to project durable settings into runtime `Config`, `McpServerConfig`, and `SkillConfig` inputs, but it should not replace existing runtime configuration APIs.

## SQLite Schema

Add a compiled-in migration v2. Keep the IC-1 v1 tables untouched.

Recommended table layout:

```sql
provider_configs (
  provider_slug TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  enabled INTEGER NOT NULL,
  base_url TEXT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
)

custom_models (
  provider_slug TEXT NOT NULL,
  model_id TEXT NOT NULL,
  display_name TEXT NOT NULL,
  context_window INTEGER NULL,
  output_limit INTEGER NULL,
  supports_tool_calls INTEGER NOT NULL DEFAULT 0,
  supports_reasoning INTEGER NOT NULL DEFAULT 0,
  supports_vision INTEGER NOT NULL DEFAULT 0,
  cost_input_per_million REAL NULL,
  cost_output_per_million REAL NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (provider_slug, model_id)
)

runtime_defaults (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  provider_slug TEXT NOT NULL,
  model_id TEXT NOT NULL,
  updated_at TEXT NOT NULL
)

mcp_servers (
  id TEXT PRIMARY KEY,
  label TEXT NOT NULL,
  description TEXT NULL,
  transport_kind TEXT NOT NULL,
  command TEXT NULL,
  args_json TEXT NULL,
  env_json TEXT NULL,
  inherited_env_vars_json TEXT NULL,
  url TEXT NULL,
  headers_json TEXT NULL,
  working_dir TEXT NULL,
  enabled_by_default INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
)

skill_settings (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  trust_project_skills INTEGER NOT NULL,
  additional_skill_dirs_json TEXT NOT NULL,
  updated_at TEXT NOT NULL
)
```

Do not add a SQL foreign key from `runtime_defaults.provider_slug` to `provider_configs` yet. Built-in providers may exist in `iron-providers` without a persisted provider row, and Rust-level validation can reconcile built-in provider metadata plus custom model rows more accurately than SQLite foreign keys can.

Use JSON columns for MCP arrays/maps and skill directory lists to avoid over-normalizing settings that are naturally read/written as whole records. Validate these JSON payloads at the API boundary.

## MCP Environment Handling

Stdio MCP servers should continue to start with the sanitized parent environment: non-sensitive parent vars are inherited, sensitive vars are stripped by pattern/name, and stripped names may be logged without values.

`inherited_env_vars` adds a second, explicit parent-env layer. For each configured name, if the parent process contains that variable, the runtime should copy the parent value into the subprocess environment even if the name matches a sensitive pattern. This lets users pass credentials from their shell or service manager without storing the secret in `ConfigStore`.

Merge precedence for stdio subprocess env should be:

1. sanitized parent environment
2. explicit inherited parent values named by `inherited_env_vars`
3. configured `env` map values

Configured `env` values win if the same variable appears in both `inherited_env_vars` and `env`.

## Validation

Validation should happen at the core API boundary before records are persisted and again where needed during snapshot loading.

Validation rules should include:

- provider slug, model ID, MCP server ID, MCP label, and stdio command are non-empty after trimming
- model numeric fields are positive when present
- cost fields are non-negative when present
- provider config has no credential fields
- default model stores provider/model as separate fields
- MCP transport kind has the required fields for its transport and rejects irrelevant field combinations where practical
- MCP `args`, `env`, `headers`, and `inherited_env_vars` deserialize to the expected collection types
- `inherited_env_vars` entries are names only, not `KEY=value` strings
- skill directory entries are non-empty paths

## Testing Strategy

Tests should cover:

- migration from v1 to v2 and opening an already-current v2 database
- in-memory store applying v2 migrations
- provider config CRUD and credential-field exclusion
- custom model CRUD, list by provider, and `(provider_slug, model_id)` uniqueness
- default model set/get/clear round-trips using typed provider/model fields
- MCP server CRUD for stdio, HTTP, and HTTP+SSE transports
- `inherited_env_vars` persistence and validation as names only
- skill settings defaults and round-trips
- runtime settings snapshot missing/default behavior
- snapshot validation for invalid default model, duplicate/invalid MCP definitions, and malformed persisted JSON where reachable
- confirmation that provider credentials are still read/written through credential APIs rather than provider config APIs

## Follow-Up Work

AgentIron/AgentIron#56 should migrate the desktop app from direct settings-table SQL to these typed `iron-core` APIs. That migration should move custom models into `iron-core`, leave starred models in AgentIron, and migrate provider API keys to the existing credential APIs rather than provider config JSON.
