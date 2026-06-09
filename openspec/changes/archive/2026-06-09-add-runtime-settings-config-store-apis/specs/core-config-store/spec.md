## ADDED Requirements

### Requirement: Config store supports provider runtime configuration

The system SHALL provide typed `ConfigStore` APIs for provider runtime configuration used by app and headless consumers. Provider runtime configuration SHALL include provider slug, display name copied from provider metadata, enabled flag, optional base URL, and store-maintained timestamps. Provider runtime configuration SHALL NOT include API keys, OAuth tokens, credential mode, or provider protocol metadata owned by `iron-providers`.

#### Scenario: Provider config roundtrips
- **WHEN** a caller stores provider runtime configuration with a provider slug, display name, enabled flag, and optional base URL
- **THEN** the caller can retrieve the same non-secret provider configuration by provider slug
- **AND** the returned record includes store-maintained timestamps

#### Scenario: Provider config excludes credentials
- **WHEN** a caller uses provider runtime configuration APIs
- **THEN** the API does not accept or return provider API keys, OAuth tokens, credential mode, or decrypted credential payloads
- **AND** provider credentials remain accessible only through the credential APIs/store

#### Scenario: Provider display name is copied metadata
- **WHEN** provider runtime configuration is stored
- **THEN** the display name represents provider metadata copied into the config record
- **AND** the config API does not define user-editable provider naming semantics

### Requirement: Config store supports custom model records

The system SHALL provide typed `ConfigStore` APIs for custom model records because custom models are shared runtime configuration required by app and headless provider/model selection. Custom model records SHALL be keyed by provider slug and model ID and SHALL include display, context window, output limit, capability, cost, and store-maintained timestamp metadata.

#### Scenario: Custom model roundtrips
- **WHEN** a caller stores a custom model for a provider slug and model ID
- **THEN** the caller can retrieve the custom model by the same provider slug and model ID
- **AND** model metadata such as display name, context window, output limit, capabilities, and costs roundtrips through typed fields

#### Scenario: Custom models list by provider
- **WHEN** a caller lists custom models for a provider slug
- **THEN** the system returns only custom models associated with that provider slug

#### Scenario: Starred models remain outside core config
- **WHEN** a caller needs app-specific starred model preferences
- **THEN** those preferences are not represented by the core custom model APIs
- **AND** the app remains responsible for starred model persistence

### Requirement: Config store supports typed default model selection

The system SHALL provide typed `ConfigStore` APIs for the default model selection. The default model SHALL be stored as separate `provider_slug` and `model_id` fields rather than an app-specific encoded provider/model string.

#### Scenario: Default model roundtrips
- **WHEN** a caller stores a default model with provider slug `openai` and model ID `gpt-4o`
- **THEN** the caller can retrieve provider slug `openai` and model ID `gpt-4o` as separate typed fields

#### Scenario: Default model is absent
- **WHEN** no default model has been stored
- **THEN** the default model read API returns an explicit missing/default result according to its method contract
- **AND** callers do not need to parse a missing app-specific encoded string

### Requirement: Config store supports durable MCP server definitions

The system SHALL provide typed `ConfigStore` APIs for durable MCP server definitions used to initialize runtime MCP inventory. MCP server records SHALL support stable ID, label, optional description, transport type, transport-specific fields for stdio/HTTP/HTTP+SSE, optional working directory, enabled-by-default flag, configured env/header maps, explicit `inherited_env_vars` names for stdio transports, and store-maintained timestamps.

#### Scenario: Stdio MCP server roundtrips
- **WHEN** a caller stores a stdio MCP server with command, args, configured env, inherited env var names, working directory, and enabled-by-default state
- **THEN** the caller can retrieve the same server definition through typed MCP server APIs
- **AND** `inherited_env_vars` contains environment variable names rather than secret values

#### Scenario: HTTP MCP server roundtrips
- **WHEN** a caller stores an HTTP or HTTP+SSE MCP server with URL and headers
- **THEN** the caller can retrieve the same transport kind, URL, headers, and enabled-by-default state through typed MCP server APIs

#### Scenario: Runtime snapshot includes MCP servers
- **WHEN** a caller loads runtime settings from the config store
- **THEN** the snapshot includes the durable MCP server definitions needed to initialize runtime-local MCP inventory

### Requirement: Config store supports skill settings

The system SHALL provide typed `ConfigStore` APIs for skill settings used by runtime skill discovery. Skill settings SHALL include `trust_project_skills`, `additional_skill_dirs`, and store-maintained updated timestamp metadata.

#### Scenario: Skill settings roundtrip
- **WHEN** a caller stores skill settings with project skill trust and additional skill directories
- **THEN** the caller can retrieve the same settings through typed skill settings APIs

#### Scenario: Missing skill settings use safe defaults
- **WHEN** no skill settings record exists
- **THEN** the read API or runtime settings snapshot uses safe defaults
- **AND** project skills are not trusted by default
- **AND** additional skill directories default to an empty list

### Requirement: Config store exposes a runtime settings snapshot

The system SHALL provide a `ConfigStore` API that loads a validated runtime settings snapshot containing provider configs, custom models, default model selection, MCP server definitions, and skill settings. The snapshot SHALL centralize cross-record validation and interpretation needed by app and headless runtime startup.

#### Scenario: Headless runtime loads shared settings
- **WHEN** a headless consumer opens the config store and loads the runtime settings snapshot
- **THEN** it receives the same shared runtime settings available to the app through typed `iron_core::config` APIs
- **AND** it does not read SQLite tables or app-owned settings JSON directly

#### Scenario: Snapshot validates cross-record settings
- **WHEN** persisted runtime settings contain invalid IDs, malformed transport fields, malformed JSON collections, or invalid default model fields
- **THEN** loading the runtime settings snapshot returns an actionable `ConfigError`
- **AND** invalid settings are not silently projected into runtime startup

#### Scenario: Snapshot supports missing/default config behavior
- **WHEN** optional runtime settings have not yet been persisted
- **THEN** snapshot loading applies documented missing/default behavior instead of requiring callers to know table-level details

### Requirement: Runtime settings schema is migrated on open

The SQLite-backed config store SHALL add compiled-in migrations for provider configs, custom models, runtime defaults, MCP servers, and skill settings. The migration SHALL preserve existing IC-1 profile, prompt, schedule, and credential data.

#### Scenario: Existing v1 config store opens after runtime settings migration
- **WHEN** a config store database created with the IC-1 schema is opened
- **THEN** the system applies the runtime-settings migration
- **AND** existing profile, prompt, schedule, and credential records remain available

#### Scenario: In-memory store includes runtime settings schema
- **WHEN** an in-memory config store is created for tests or embedders
- **THEN** the same compiled-in runtime-settings migrations are applied
- **AND** runtime settings CRUD APIs are available without user configuration files
