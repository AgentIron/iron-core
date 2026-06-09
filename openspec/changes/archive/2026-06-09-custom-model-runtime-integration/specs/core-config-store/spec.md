## MODIFIED Requirements

### Requirement: Config store supports custom model records

The system SHALL provide typed `ConfigStore` APIs for custom model records because custom models are shared runtime configuration required by app and headless provider/model selection. Custom model records SHALL be keyed by provider slug and model ID and SHALL include display, context window, output limit, capability, cost, streaming, reasoning effort, and store-maintained timestamp metadata.

#### Scenario: Custom model roundtrips
- **WHEN** a caller stores a custom model for a provider slug and model ID
- **THEN** the caller can retrieve the custom model by the same provider slug and model ID
- **AND** model metadata such as display name, context window, output limit, capabilities, costs, streaming support, and reasoning effort values roundtrips through typed fields

#### Scenario: Custom models list by provider
- **WHEN** a caller lists custom models for a provider slug
- **THEN** the system returns only custom models associated with that provider slug

#### Scenario: Starred models remain outside core config
- **WHEN** a caller needs app-specific starred model preferences
- **THEN** those preferences are not represented by the core custom model APIs
- **AND** the app remains responsible for starred model persistence

### Requirement: Config store exposes a runtime settings snapshot

The system SHALL provide a `ConfigStore` API that loads a validated runtime settings snapshot containing provider configs, custom models, default model selection, MCP server definitions, and skill settings. The snapshot SHALL centralize cross-record validation and interpretation needed by app and headless runtime startup. Default model validation SHALL consider both built-in model metadata and custom model records from ConfigStore.

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

#### Scenario: Snapshot validates default model against effective catalog
- **WHEN** a default model is set to a provider/model combination
- **THEN** validation checks the effective model catalog (built-in plus custom) rather than only custom model records

## ADDED Requirements

### Requirement: Config store SHALL validate custom model provider slugs against known providers

When storing a custom model record, the system SHALL validate that the `provider_slug` matches either a built-in provider slug known to `ProviderRegistry::default()` or a persisted `ProviderConfigRecord`. Unknown provider slugs SHALL result in a validation error.

#### Scenario: Custom model for built-in provider
- **WHEN** a caller stores a custom model with a provider slug matching a built-in provider (e.g., `openai`, `anthropic`)
- **THEN** validation passes

#### Scenario: Custom model for persisted provider config
- **WHEN** a caller stores a custom model with a provider slug matching a persisted `ProviderConfigRecord`
- **THEN** validation passes

#### Scenario: Custom model for unknown provider
- **WHEN** a caller stores a custom model with a provider slug that matches neither a built-in provider nor a persisted provider config
- **THEN** the API returns a `ConfigError` indicating the provider slug is not recognized

### Requirement: Custom model schema SHALL support streaming and reasoning effort metadata

The `custom_models` SQLite table and corresponding input/record types SHALL include `supports_streaming` and `reasoning_effort_values` fields. Migration v3 SHALL add these columns with defaults that preserve existing row semantics.

#### Scenario: Migration v3 adds streaming column
- **WHEN** a v2 config database is opened by the updated code
- **THEN** migration v3 adds `supports_streaming` with a default of 1 (true)
- **AND** existing custom model rows retain their effective behavior

#### Scenario: Migration v3 adds reasoning effort values column
- **WHEN** a v2 config database is opened by the updated code
- **THEN** migration v3 adds `reasoning_effort_values_json` as NULL by default
- **AND** existing custom model rows are not affected
