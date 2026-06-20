## Purpose

Define the durable, core-owned AgentIron configuration store used for profiles, prompts, schedules, domain-scoped opaque settings, and encrypted provider credentials.
## Requirements
### Requirement: Core exposes a durable config module

The system SHALL provide a public `iron_core::config` module for durable AgentIron configuration owned by `iron-core`.

The durable store API SHALL integrate with the existing `iron_core::config` runtime-configuration module without replacing or renaming existing `Config` and `ConfigSource` APIs. Durable-store internals MAY live in nested implementation modules, but callers SHALL be able to use `iron_core::config::ConfigStore` and related public config-store types.

#### Scenario: Caller opens default config store
- **WHEN** a caller invokes the default config store constructor
- **THEN** the system opens the core-owned durable configuration store for the current OS user
- **AND** the caller does not need to know the concrete storage backend

#### Scenario: Existing runtime config remains available
- **WHEN** callers use existing runtime configuration APIs such as `iron_core::config::Config` or `iron_core::config::ConfigSource`
- **THEN** those APIs remain available in the `iron_core::config` module
- **AND** durable-store additions do not force callers to migrate existing runtime configuration code

#### Scenario: Caller opens explicit config path
- **WHEN** a caller invokes the explicit-path config store constructor with a database path
- **THEN** the system opens the durable configuration store at that path
- **AND** creates required parent directories when possible

#### Scenario: Caller uses in-memory config for tests
- **WHEN** tests create an in-memory config store
- **THEN** the store does not read or write user configuration files
- **AND** the store can run migrations and CRUD operations like the durable backend

### Requirement: ConfigStore abstracts storage backend details

The public config API SHALL expose a domain-oriented `ConfigStore` abstraction rather than requiring callers to use SQLite tables, SQL queries, SQLite connection strings, or a public `ConfigDb` type as the main API.

#### Scenario: Caller performs config operation
- **WHEN** a caller creates, reads, updates, deletes, or lists configuration records
- **THEN** the caller uses `iron_core::config` APIs
- **AND** does not need direct access to SQL handles or table definitions

#### Scenario: Backend evolves later
- **WHEN** `iron-core` later adds another config backend such as server-backed config or file-backed config
- **THEN** existing callers using `ConfigStore` remain insulated from SQLite-specific implementation details where possible

### Requirement: SQLite is the initial default config backend

The system SHALL use SQLite via `sqlx` as the initial default durable config backend and hard dependency for IC-1.

#### Scenario: Default store opens
- **WHEN** `ConfigStore::open()` or equivalent default constructor is called
- **THEN** the SQLite-backed store is used as the default implementation

#### Scenario: SQLite implementation is internal
- **WHEN** a caller uses the public config API
- **THEN** SQLite remains an implementation detail except for documented path and operational behavior

### Requirement: Platform default config paths

The system SHALL resolve platform-default configuration database paths for Linux, macOS, and Windows.

#### Scenario: Linux default path with XDG config home
- **WHEN** the default config store is opened on Linux
- **AND** `XDG_CONFIG_HOME` is set to a non-empty path
- **THEN** the database path is `$XDG_CONFIG_HOME/agentiron/config.db`

#### Scenario: Linux default path without XDG config home
- **WHEN** the default config store is opened on Linux
- **AND** `XDG_CONFIG_HOME` is unset or empty
- **THEN** the database path is `~/.config/agentiron/config.db`

#### Scenario: macOS default path
- **WHEN** the default config store is opened on macOS
- **THEN** the database path is `~/Library/Application Support/com.agentiron/iron-core/config.db`

#### Scenario: Windows default path
- **WHEN** the default config store is opened on Windows
- **THEN** the database path is `%APPDATA%/AgentIron/config.db`

### Requirement: Config schema is migrated on open

The system SHALL own and apply migrations for the config schema when a config store is opened.

Migrations SHALL be embedded in the compiled `iron-core` binary or otherwise compiled into crate code. The system SHALL NOT require external migration files to be packaged or discoverable at runtime.

#### Scenario: Empty database opens
- **WHEN** the config store opens an empty database
- **THEN** migrations create the initial `profiles`, `prompts`, `credentials`, and `schedule` tables

#### Scenario: Current database opens
- **WHEN** the config store opens a database already at the current schema version
- **THEN** opening succeeds without changing caller-visible data

#### Scenario: Older database opens
- **WHEN** the config store opens a database at an older supported schema version
- **THEN** pending migrations are applied before CRUD APIs are available

#### Scenario: Runtime migration files are absent
- **WHEN** the config store opens in an installed or packaged environment
- **AND** no migration directory exists next to the executable or crate sources
- **THEN** schema migration still succeeds using the compiled-in migrations

#### Scenario: In-memory store opens
- **WHEN** an in-memory config store is created for tests or embedders
- **THEN** the same compiled-in migrations are applied to the in-memory database before CRUD APIs are available

### Requirement: Config APIs return actionable errors

Config APIs SHALL return typed errors for expected failures and SHALL NOT panic for expected storage, migration, serialization, encryption, key-source, or concurrency failures.

Public config APIs SHALL use `Result<T, ConfigError>` for fallible operations. Read-by-ID methods SHALL return `Result<Option<T>, ConfigError>` unless a method is explicitly documented as requiring existence, in which case missing records SHALL return `ConfigError::NotFound`.

#### Scenario: Database cannot be opened
- **WHEN** the config store cannot open the configured path
- **THEN** the constructor returns an error describing the open failure category

#### Scenario: Migration fails
- **WHEN** schema migration fails
- **THEN** the constructor returns an error describing the migration failure category

#### Scenario: Record is missing
- **WHEN** a caller requests a record by ID and the record does not exist
- **THEN** the API returns either `None` or a typed not-found error according to the method contract

#### Scenario: Durable credential persistence fails
- **WHEN** provider credential storage backed by the config store encounters key-source, encryption, decryption, database, or busy-timeout failure
- **THEN** the failure is surfaced as a typed error rather than being collapsed into a missing credential or ignored write

#### Scenario: Write lock times out
- **WHEN** another `iron-core` process holds the write lock beyond the configured busy timeout
- **THEN** the write operation returns an actionable busy-timeout error

### Requirement: Config store supports opaque profile records

The system SHALL provide async CRUD storage for profile records using stable identifiers and versioned opaque payloads, without defining final `AgentProfile` semantics in IC-1.

Profile records SHALL use caller-provided non-empty string IDs, an integer `schema_version`, an opaque JSON payload, and created/updated timestamps maintained by the store.

#### Scenario: Profile record roundtrips
- **WHEN** a caller stores a profile record with an opaque payload and schema version
- **THEN** the caller can retrieve the same record by stable ID

#### Scenario: Profile semantics are deferred
- **WHEN** IC-1 stores a profile record
- **THEN** the store does not require fields or validation that belong to the future AgentProfile domain design

### Requirement: Config store supports opaque prompt records

The system SHALL provide async CRUD storage for prompt records using stable identifiers and versioned opaque payloads or bodies, without defining final stored-prompt semantics in IC-1.

Prompt records SHALL use caller-provided non-empty string IDs, an integer `schema_version`, an opaque JSON payload or body field, and created/updated timestamps maintained by the store.

#### Scenario: Prompt record roundtrips
- **WHEN** a caller stores a prompt record with an opaque payload or body and schema version
- **THEN** the caller can retrieve the same record by stable ID

#### Scenario: Prompt semantics are deferred
- **WHEN** IC-1 stores a prompt record
- **THEN** the store does not require template, variable, or `run_task` semantics that belong to IC-6

### Requirement: Config store supports opaque schedule records

The system SHALL provide async CRUD storage for schedule records using stable identifiers and versioned opaque payloads, without defining scheduler execution semantics in IC-1.

Schedule records SHALL use caller-provided non-empty string IDs, an integer `schema_version`, an opaque JSON payload, and created/updated timestamps maintained by the store.

#### Scenario: Schedule record roundtrips
- **WHEN** a caller stores a schedule record with an opaque payload and schema version
- **THEN** the caller can retrieve the same record by stable ID

#### Scenario: Scheduler semantics are deferred
- **WHEN** IC-1 stores a schedule record
- **THEN** the store does not require cron, recurrence, execution, or dispatch semantics that belong to IC-8

### Requirement: Opaque record writes use replace semantics

The profile, prompt, and schedule APIs SHALL support create/update operations that replace the stored opaque payload and schema version for a record ID while preserving the stable ID and updating the `updated_at` timestamp.

#### Scenario: Opaque record is updated
- **WHEN** a caller updates an existing profile, prompt, or schedule record by ID
- **THEN** the store replaces the record payload and schema version atomically
- **AND** the `updated_at` timestamp changes

#### Scenario: Opaque record is deleted
- **WHEN** a caller deletes a profile, prompt, or schedule record by ID
- **THEN** subsequent read-by-ID calls return `Ok(None)` unless the API method explicitly requires existence

### Requirement: Config writes are transactional

The SQLite-backed config store SHALL perform writes and migrations transactionally so partial writes are not committed as successful operations.

#### Scenario: Write succeeds
- **WHEN** a config write operation completes successfully
- **THEN** all changes for that operation are committed atomically

#### Scenario: Write fails
- **WHEN** a config write operation fails during the transaction
- **THEN** partial changes from that operation are not committed

### Requirement: Multiple core processes can share a config database

The SQLite-backed config store SHALL be configured for multiple `iron-core` instances on one system, allowing concurrent reads and serializing writes with a finite busy timeout.

#### Scenario: Multiple readers access config
- **WHEN** multiple `iron-core` instances read the same config database
- **THEN** the reads can proceed without requiring direct frontend coordination

#### Scenario: Concurrent writers contend
- **WHEN** multiple `iron-core` instances attempt to write the same config database
- **THEN** SQLite serializes the writes
- **AND** a writer that cannot acquire the lock before the busy timeout receives an actionable error

### Requirement: Frontends use core config APIs for durable state

Frontends and apps SHALL use `iron_core::config` APIs for durable AgentIron configuration rather than writing the config database directly.

#### Scenario: App needs durable AgentIron config
- **WHEN** an app or frontend needs to persist AgentIron configuration covered by the core config API
- **THEN** it calls the `iron_core::config` API
- **AND** does not write SQLite rows directly

#### Scenario: App needs unsupported durable config
- **WHEN** an app or frontend needs durable AgentIron configuration not yet covered by the core config API
- **THEN** the missing config domain should be added to `iron-core` rather than creating divergent frontend-owned persistence

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

### Requirement: Config store supports saved handoff bundles

The system SHALL provide typed `ConfigStore` APIs for saving, loading, listing, and deleting saved handoff bundles. Saved handoffs SHALL be stored by a stable caller-provided non-empty ID and non-empty human-readable name, and SHALL preserve the full serialized `HandoffBundle` as the authoritative saved artifact.

#### Scenario: Saved handoff roundtrips
- **WHEN** a caller saves a valid `HandoffBundle` with a stable ID and name
- **THEN** the caller can load the saved handoff by the same ID
- **AND** the loaded bundle matches the saved bundle exactly
- **AND** the loaded record includes saved handoff metadata

#### Scenario: Saved handoff is missing
- **WHEN** a caller loads a saved handoff ID that does not exist
- **THEN** the API returns `Ok(None)`

#### Scenario: Saved handoff is deleted
- **WHEN** a caller deletes a saved handoff by ID
- **THEN** subsequent load calls for that ID return `Ok(None)`

### Requirement: Config store lists saved handoff metadata

The system SHALL provide a `ConfigStore` API that lists saved handoff metadata without requiring callers to load every full handoff bundle. Saved handoff metadata SHALL include ID, name, bundle version, created timestamp, updated timestamp, source session ID when present, source model when present, source provider when present, and size estimate tokens.

#### Scenario: Saved handoff metadata is listed
- **WHEN** a caller lists saved handoffs after saving one or more valid handoff bundles
- **THEN** the returned list includes metadata for each saved handoff
- **AND** the metadata includes the caller-provided ID and name
- **AND** the metadata includes source session, source model, source provider, and size estimate values derived from the saved bundle when present
- **AND** the list response does not include full bundle JSON or hydrated session content

#### Scenario: Saved handoff metadata reflects updates
- **WHEN** a caller saves a new bundle using an existing saved handoff ID
- **THEN** the existing record is replaced atomically
- **AND** the original created timestamp is preserved
- **AND** the updated timestamp changes
- **AND** list metadata reflects the replacement bundle and name

### Requirement: Saved handoff persistence validates bundle compatibility

The system SHALL validate saved handoff IDs, names, serialized bundle shape, bundle version, and bundle metadata version at save and load boundaries. Validation SHALL reject unsupported or malformed saved handoff data with an actionable `ConfigError` rather than silently accepting, repairing, sanitizing, or reinterpreting the bundle.

#### Scenario: Invalid saved handoff input is rejected
- **WHEN** a caller attempts to save a handoff with an empty ID, empty name, unsupported bundle version, unsupported metadata version, or unserializable bundle data
- **THEN** the API returns a typed `ConfigError`
- **AND** no successful saved handoff record is committed for that invalid input

#### Scenario: Malformed stored bundle is rejected on load
- **WHEN** a saved handoff row contains malformed bundle JSON or an unsupported bundle version
- **THEN** loading that saved handoff returns a typed `ConfigError`
- **AND** the system does not return a partially hydrated, repaired, or sanitized bundle

#### Scenario: Saved handoff snapshot remains exact
- **WHEN** a valid saved handoff bundle contains fields that are sensitive, optional, runtime-derived, or unknown to the storage layer
- **THEN** the persistence API stores and loads the exact serialized bundle according to the current `HandoffBundle` schema
- **AND** the persistence API does not strip, redact, transform, or reinterpret bundle fields

### Requirement: Saved handoff schema is migrated on open

The SQLite-backed config store SHALL add a compiled-in migration for saved handoff storage. The migration SHALL create a `saved_handoffs` table containing the authoritative serialized bundle and metadata columns sufficient for listing saved handoffs.

#### Scenario: Existing config store opens after saved handoff migration
- **WHEN** a config store database created before saved handoff storage is opened
- **THEN** the system applies the saved handoff migration
- **AND** existing profile, prompt, schedule, credential, provider runtime, custom model, MCP, and skill settings records remain available
- **AND** saved handoff APIs are available

#### Scenario: In-memory store includes saved handoff schema
- **WHEN** an in-memory config store is created for tests or embedders
- **THEN** the same compiled-in saved handoff migration is applied
- **AND** saved handoff CRUD APIs are available without user configuration files

### Requirement: Config store SHALL support provider profile records
The config store SHALL provide typed async CRUD APIs for non-secret provider profile records keyed by provider slug. Provider profile records SHALL be stored separately from `AgentProfile` records and provider runtime configuration records.

Provider profile records SHALL include the provider slug, serialized provider profile payload, store-maintained timestamps, and optional source metadata useful for import diagnostics.

#### Scenario: Provider profile record roundtrips
- **WHEN** a caller stores a provider profile record with a valid slug and payload
- **THEN** the caller can retrieve the same provider profile record by slug
- **AND** the returned record includes store-maintained timestamps

#### Scenario: Provider profile records list deterministically
- **WHEN** a caller lists provider profile records
- **THEN** the system returns persisted provider profile records in deterministic order
- **AND** built-in provider profiles that were not explicitly persisted are not returned as config-store rows

#### Scenario: Provider profile record is deleted
- **WHEN** a caller deletes a provider profile record by slug
- **THEN** subsequent read-by-slug calls return `Ok(None)` unless the API explicitly requires existence
- **AND** deleting the row does not remove compiled-in built-in provider behavior

### Requirement: Config schema SHALL migrate provider profile storage
The config store schema migrations SHALL create durable storage for provider profile records when opening an older or empty database.

#### Scenario: Empty database opens with provider profile storage
- **WHEN** the config store opens an empty database
- **THEN** migrations create the provider profile storage required by the provider profile APIs
- **AND** no built-in provider profile rows are inserted by default

#### Scenario: Older database opens with provider profile migration
- **WHEN** the config store opens a database created before provider profile storage existed
- **THEN** pending migrations add provider profile storage without changing existing profile, prompt, credential, provider config, custom model, or schedule rows

