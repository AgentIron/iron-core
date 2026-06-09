## Why

AgentIron/iron-core#67 extends the IC-1 config-store foundation so runtime setup can be shared by the AgentIron desktop app and headless/CLI consumers. Today the desktop app persists provider configuration, default model selection, custom models, MCP server definitions, and skill settings in an app-owned settings table. That blocks shared runtime setup because headless workflows cannot read the same settings through typed `iron-core` APIs.

This change makes `ConfigStore` the owner of shared runtime settings while preserving the existing boundary that provider credentials are stored through the encrypted credential API, not in provider config JSON. App-only preferences such as starred models remain owned by AgentIron.

## What Changes

- Add typed `ConfigStore` APIs for provider runtime configuration: provider slug, metadata display name, enabled flag, and optional base URL.
- Keep provider credentials out of provider runtime configuration; credentials remain in the existing encrypted credentials API/store.
- Add typed `ConfigStore` APIs for custom model records because custom models are needed for core provider/model selection in app and headless flows.
- Add a typed default model selection record using `provider_slug` and `model_id` rather than app-specific encoded strings.
- Add typed `ConfigStore` APIs for durable MCP server definitions, including stdio/http/http_sse transports, enabled-by-default behavior, optional description, working directory, env/header maps, and `inherited_env_vars` names.
- Add typed `ConfigStore` APIs for skill settings: `trust_project_skills` and `additional_skill_dirs`.
- Add a runtime settings snapshot read API that loads and validates the effective provider/default-model/custom-model/MCP/skill settings needed to start a runtime.
- Add migration v2 for the runtime-settings tables while preserving the existing IC-1 schema and compiled-in migration strategy.
- Update MCP stdio environment behavior so selected sensitive parent env vars can be reintroduced by name through `inherited_env_vars`, with explicit configured env values taking final precedence.

## Capabilities

### Modified Capabilities

- `core-config-store`: Adds typed runtime settings APIs and schema ownership for provider config, custom models, default model selection, MCP server definitions, skill settings, and runtime settings snapshots.
- `session-scoped-mcp-support`: Adds explicit inherited environment variable names to MCP stdio server configuration and defines environment merge precedence.

## Impact

- **Core API**: `iron_core::config::ConfigStore` gains runtime-settings record/input types, CRUD APIs, and a snapshot loader.
- **Storage**: SQLite migration v2 adds runtime-settings tables. SQLite remains an implementation detail behind `ConfigStore`.
- **Runtime setup**: AgentIron and headless/CLI consumers can use the same typed core APIs to construct provider/model/MCP/skill runtime configuration.
- **Credential boundary**: provider credentials remain in the encrypted credential store; provider config stores only non-secret runtime fields.
- **Model settings boundary**: custom models move to `iron-core`; starred models remain AgentIron-owned UI preference.
- **MCP secrets**: MCP server config can store raw env/header maps as supplied and can store `inherited_env_vars` names to pass selected parent env values without persisting those secret values.
- **Compatibility**: existing runtime `Config`, `McpConfig`, `SkillConfig`, and IC-1 profile/prompt/schedule/credential APIs remain available.
- **Out of scope**: migrating AgentIron app settings data, storing starred models in `iron-core`, provider protocol metadata owned by `iron-providers`, credential migration from existing app JSON, cloud sync, direct frontend SQL access, and user-editable provider display names.
