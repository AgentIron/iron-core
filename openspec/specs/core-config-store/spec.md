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
