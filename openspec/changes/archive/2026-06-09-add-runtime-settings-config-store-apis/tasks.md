## 1. Public Runtime Settings Types

- [x] 1.1 Add public provider runtime config input/record types with provider slug, copied metadata display name, enabled flag, optional base URL, and timestamps.
- [x] 1.2 Add public custom model input/record types keyed by provider slug and model ID with display, capability, limit, and cost metadata.
- [x] 1.3 Add public default model input/record types using separate `provider_slug` and `model_id` fields.
- [x] 1.4 Add public MCP server config input/record types covering stdio, HTTP, HTTP+SSE, optional description, working directory, enabled-by-default, env/header maps, and `inherited_env_vars`.
- [x] 1.5 Add public skill settings input/record types for `trust_project_skills` and `additional_skill_dirs`.
- [x] 1.6 Add `RuntimeSettingsSnapshot` type containing provider configs, custom models, default model, MCP servers, and skill settings.

## 2. ConfigStore APIs

- [x] 2.1 Add provider config CRUD/list APIs to `ConfigStore`.
- [x] 2.2 Add custom model CRUD/list APIs to `ConfigStore`, including list-by-provider support.
- [x] 2.3 Add default model set/get/clear APIs to `ConfigStore`.
- [x] 2.4 Add MCP server CRUD/list APIs to `ConfigStore`.
- [x] 2.5 Add skill settings get/set APIs to `ConfigStore`, including missing-record defaults.
- [x] 2.6 Add `ConfigStore::load_runtime_settings()` or equivalent snapshot loader.
- [x] 2.7 Ensure all APIs return `Result<T, ConfigError>` and use `Result<Option<T>, ConfigError>` for optional read-by-ID records.

## 3. Schema and Migrations

- [x] 3.1 Add compiled-in migration v2 for `provider_configs`.
- [x] 3.2 Add compiled-in migration v2 for `custom_models` with primary key `(provider_slug, model_id)`.
- [x] 3.3 Add compiled-in migration v2 for singleton `runtime_defaults`.
- [x] 3.4 Add compiled-in migration v2 for `mcp_servers` using JSON columns for args, env, inherited env vars, and headers.
- [x] 3.5 Add compiled-in migration v2 for singleton `skill_settings`.
- [x] 3.6 Update schema version handling so v1 databases migrate to v2 and current v2 databases open without caller-visible changes.
- [x] 3.7 Keep existing v1 profile, prompt, schedule, and credential tables unchanged.

## 4. Validation and Snapshot Behavior

- [x] 4.1 Validate non-empty provider slugs, model IDs, MCP server IDs, MCP labels, stdio commands, and skill directory paths.
- [x] 4.2 Validate optional numeric model metadata is positive or non-negative as appropriate.
- [x] 4.3 Validate provider config cannot contain or expose credential fields.
- [x] 4.4 Validate MCP transport-specific field combinations before persistence and during snapshot loading.
- [x] 4.5 Validate `inherited_env_vars` entries are environment variable names, not `KEY=value` pairs.
- [x] 4.6 Validate default model selection as typed provider/model fields and reconcile persisted custom models where possible.
- [x] 4.7 Ensure snapshot loading returns clear `ConfigError` values for malformed persisted JSON or invalid runtime settings.
- [x] 4.8 Ensure snapshot loading applies missing/default behavior for absent skill settings and absent optional runtime settings.

## 5. MCP Inherited Environment Variables

- [x] 5.1 Add `inherited_env_vars` to durable MCP server config records and runtime MCP stdio config as needed.
- [x] 5.2 Preserve sanitized parent-env behavior for stdio MCP subprocesses.
- [x] 5.3 Reintroduce parent env values named by `inherited_env_vars` after sanitization, including sensitive names when explicitly requested.
- [x] 5.4 Apply configured `env` map values after `inherited_env_vars` so explicit config wins on conflicts.
- [x] 5.5 Ensure logs never include inherited or configured env values.

## 6. Tests

- [x] 6.1 Add migration tests for empty, v1, and current v2 config databases.
- [x] 6.2 Add provider config persistence round-trip and list/delete tests.
- [x] 6.3 Add custom model persistence round-trip, list-by-provider, uniqueness, and delete tests.
- [x] 6.4 Add default model set/get/clear and typed field round-trip tests.
- [x] 6.5 Add MCP server persistence round-trip tests for stdio, HTTP, and HTTP+SSE.
- [x] 6.6 Add MCP `inherited_env_vars` validation and stdio env merge precedence tests.
- [x] 6.7 Add skill settings default and round-trip tests.
- [x] 6.8 Add runtime settings snapshot tests for complete settings, missing/default settings, and invalid persisted records.
- [x] 6.9 Add tests proving provider credentials are not persisted or exposed through provider config APIs.

## 7. Documentation and Verification

- [x] 7.1 Document the runtime settings APIs and their distinction from runtime `Config` snapshots.
- [x] 7.2 Document the provider config credential boundary and point callers to credential APIs.
- [x] 7.3 Document custom models as core-owned runtime settings and starred models as app-owned preferences.
- [x] 7.4 Document MCP `inherited_env_vars` behavior and env merge precedence.
- [x] 7.5 Run focused config store tests.
- [x] 7.6 Run focused MCP stdio env tests.
- [x] 7.7 Run `cargo check --manifest-path src-tauri/Cargo.toml` only if this repository contains that manifest for the changed Rust code path; otherwise run the narrowest crate-level Rust check available.
- [x] 7.8 Run `openspec status --change add-runtime-settings-config-store-apis` or the repo-supported OpenSpec validation command.
