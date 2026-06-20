## Purpose

Define how `iron-core` resolves, persists, refreshes, reports, and disconnects provider credentials while preserving provider-safe boundaries and durable credential storage semantics.
## Requirements
### Requirement: Core SHALL support core-owned provider credential storage

`iron-core` SHALL define and provide provider credential storage for provider OAuth/API-key credential material through the core config store, while preserving the existing credential store boundary for tests, in-memory operation, and future alternate backends.

The credential store boundary SHALL be fallible so durable implementations can surface database, migration, key-source, encryption, decryption, serialization, and busy-timeout failures as typed errors. In-memory and null stores MAY return successful no-op or missing-credential results, but the durable store SHALL NOT silently ignore failed writes or collapse storage failures into missing credentials.

#### Scenario: Core durable credential store is available
- **WHEN** a caller opens the core config store
- **THEN** the caller can obtain or use a provider credential store implementation backed by that config store
- **AND** provider credential lookup, update, removal, and listing can be persisted without frontend-owned storage

#### Scenario: Existing custom store remains available
- **WHEN** tests or alternate embedders supply a custom provider credential store implementation
- **THEN** `iron-core` continues to support that store through the existing credential store boundary

#### Scenario: Durable credential operation fails
- **WHEN** the core config backed credential store cannot read, write, encrypt, decrypt, or acquire the database lock for a credential operation
- **THEN** the operation returns a typed credential-store/config error to provider credential orchestration
- **AND** orchestration surfaces an actionable failure instead of treating the provider as simply not configured

#### Scenario: Frontend does not own provider credential persistence
- **WHEN** AgentIron or another frontend needs durable provider credential persistence
- **THEN** it uses the core-provided config/credential APIs
- **AND** does not need to implement its own SQLite credential backend

### Requirement: Core MAY depend on concrete durable storage for the default provider credential store

`iron-core` MAY depend on SQLite, OS keyring support, and credential encryption primitives to provide the default durable provider credential store. Provider credential orchestration itself SHALL still interact through the credential store boundary rather than direct SQL.

#### Scenario: Durable store uses SQLite
- **WHEN** provider credentials are persisted by the default core config store
- **THEN** `iron-core` uses its SQLite-backed config backend as the durable storage implementation

#### Scenario: Orchestration resolves credentials
- **WHEN** provider credential orchestration looks up credentials for a prompt
- **THEN** it uses the credential store boundary
- **AND** does not require prompt execution code to issue SQL queries directly

### Requirement: Core SHALL store at most one credential per provider

The core durable provider credential store SHALL store no more than one credential for each provider slug. Storing a new credential for a provider SHALL replace any previous stored credential for that provider, regardless of credential mode.

This one-credential rule applies to stored durable credentials. Prompt-supplied or runtime-supplied API keys remain outside the durable store and SHALL continue to take precedence over stored OAuth credentials when resolving credentials for a dual-mode provider.

#### Scenario: API key replaces OAuth
- **WHEN** an OAuth credential is stored for a provider
- **AND** an API-key credential is later stored for the same provider
- **THEN** the API-key credential replaces the OAuth credential
- **AND** listing or lookup for the provider returns only the API-key credential

#### Scenario: OAuth replaces API key
- **WHEN** an API-key credential is stored for a provider
- **AND** an OAuth credential is later stored for the same provider
- **THEN** the OAuth credential replaces the API-key credential
- **AND** listing or lookup for the provider returns only the OAuth credential

#### Scenario: Remove credential
- **WHEN** a stored credential is removed for a provider
- **THEN** no credential remains for that provider in the durable core store

#### Scenario: Prompt API key wins over stored OAuth
- **WHEN** a prompt supplies an API key for a provider that supports API keys
- **AND** the durable core store contains an OAuth credential for the same provider
- **THEN** provider credential resolution uses the prompt-supplied API key
- **AND** the stored OAuth credential remains unchanged unless a separate store operation replaces or removes it

#### Scenario: OAuth disconnect with stored OAuth
- **WHEN** OAuth disconnect is requested for a provider whose durable stored credential is OAuth
- **THEN** the durable stored credential is removed

#### Scenario: OAuth disconnect with stored API key
- **WHEN** OAuth disconnect is requested for a provider whose durable stored credential is an API key
- **THEN** the stored API-key credential remains unchanged

#### Scenario: OAuth replaced a stored API key before disconnect
- **WHEN** a stored API-key credential was previously replaced by a stored OAuth credential for the same provider
- **AND** OAuth disconnect is later requested for that provider
- **THEN** disconnect removes the stored OAuth credential
- **AND** the previously replaced stored API key is not restored by the durable store

### Requirement: Core SHALL preserve API-key compatibility

Existing API-key provider behavior SHALL continue to work, and API-key credentials SHALL remain valid inputs for provider runtime construction. For durable storage, the core store SHALL hold at most one stored credential per provider; API-key precedence for dual-mode providers applies to prompt-supplied or runtime-supplied API keys over stored OAuth credentials, not to two simultaneous durable credentials for the same provider.

#### Scenario: API-key credential constructs runtime config
- **WHEN** a managed provider prompt resolves an API-key credential
- **THEN** `iron-core` SHALL construct `iron_providers::RuntimeConfig` with an API-key `ProviderCredential`
- **AND** provider invocation SHALL use the same API-key auth behavior as before

#### Scenario: Prompt API key wins for dual-mode provider
- **WHEN** a provider supports both API-key and OAuth credentials
- **AND** a prompt or runtime context supplies an API key for that provider
- **AND** the durable store contains an OAuth credential for that provider
- **THEN** `iron-core` SHALL select the supplied API-key credential for provider invocation
- **AND** SHALL NOT require the durable store to retain both credential modes for that provider

### Requirement: Core SHALL encrypt durable provider credential payloads at rest

Provider credential secret payloads persisted by the core config store SHALL be encrypted before being written to durable storage and decrypted only when returning `StoredCredential` values through authorized core APIs.

#### Scenario: Credential is written
- **WHEN** a provider credential is stored in the core config store
- **THEN** the durable row stores encrypted payload data rather than plaintext API keys, access tokens, refresh tokens, or ID tokens

#### Scenario: Credential is read through core API
- **WHEN** provider credential orchestration reads a stored credential through the credential store boundary
- **THEN** the core config store decrypts the payload and returns the corresponding `StoredCredential`

#### Scenario: Credentials are listed
- **WHEN** configured provider slugs or credential metadata are listed
- **THEN** the API does not expose decrypted credential secret material

### Requirement: Core SHALL support OS-keyring-backed credential encryption keys

The default credential encryption mode SHALL use an OS-keyring-backed key source to store or retrieve local master key material used to encrypt provider credential payloads in SQLite.

#### Scenario: OS keyring key exists
- **WHEN** the default key source opens and a valid AgentIron key exists in the OS keyring
- **THEN** that key is used to decrypt and encrypt credential payloads

#### Scenario: OS keyring key is missing
- **WHEN** the default key source opens and no AgentIron key exists in the OS keyring
- **THEN** the system creates and stores a new key when the platform keyring is available

#### Scenario: OS keyring unavailable
- **WHEN** the default key source requires OS keyring access and the OS keyring is unavailable
- **THEN** credential read/write operations fail with an actionable key-unavailable error unless another configured key source is active

### Requirement: Core SHALL support environment-variable credential encryption keys

The system SHALL support an explicit environment-variable key mode for credential encryption so headless Linux and cron-style execution can use the durable provider credential store without a desktop OS keyring.

#### Scenario: Environment key is configured
- **WHEN** the environment-variable key mode is enabled and the configured environment variable contains valid key material
- **THEN** the system uses that key material to encrypt and decrypt provider credential payloads

#### Scenario: Environment key is missing
- **WHEN** the environment-variable key mode is enabled but the configured environment variable is absent
- **THEN** credential operations fail with an actionable key-unavailable error

#### Scenario: Environment key is invalid
- **WHEN** the environment-variable key mode is enabled but the configured environment variable contains invalid key material
- **THEN** credential operations fail with an actionable invalid-key error
- **AND** the error does not reveal the key value

#### Scenario: Headless credential operation
- **WHEN** `agent-iron` or another headless client runs without OS keyring access but with a valid environment key
- **THEN** provider credential read/write operations can succeed using the environment key mode

### Requirement: Core SHALL avoid plaintext credential fallback

The system SHALL NOT silently fall back to storing provider credential payloads in plaintext when no valid credential encryption key source is available.

#### Scenario: No key source available
- **WHEN** a provider credential write is requested and no valid encryption key source is available
- **THEN** the write fails before storing the credential payload
- **AND** no plaintext credential payload is persisted

#### Scenario: Non-secret config without key source
- **WHEN** no valid credential encryption key source is available
- **THEN** non-secret config APIs such as opaque profile, prompt, or schedule storage can still operate when they do not require credential encryption

### Requirement: Core SHALL resolve provider credentials per prompt

`iron-core` SHALL resolve the active provider credential from app-supplied provider/model context for each managed provider prompt execution.

#### Scenario: Prompt specifies provider and model
- **WHEN** a client submits a managed provider prompt with provider and model context
- **THEN** `iron-core` SHALL resolve credentials for that provider before constructing the provider runtime configuration

#### Scenario: Existing injected provider path remains available
- **WHEN** a caller constructs and injects a provider directly
- **THEN** `iron-core` SHALL allow that provider to run without requiring provider credential orchestration

### Requirement: Core SHALL refresh OAuth credentials before provider invocation

`iron-core` SHALL refresh OAuth credentials before constructing a provider when the access token is expired or within the configured refresh margin.

#### Scenario: OAuth credential is near expiry
- **WHEN** a managed provider prompt resolves an OAuth credential that expires within five minutes
- **THEN** `iron-core` SHALL refresh the credential before provider construction
- **AND** SHALL persist the refreshed OAuth credential through the credential store boundary

#### Scenario: OAuth refresh fails
- **WHEN** OAuth refresh fails before provider invocation
- **THEN** `iron-core` SHALL NOT invoke the provider with the stale access token
- **AND** SHALL return an actionable refresh failure status or error to the client

### Requirement: Core SHALL pass only provider-safe OAuth credential material to iron-providers

`iron-core` SHALL pass only the current OAuth access token, optional expiry, and optional ID token to `iron-providers` for OAuth-backed provider invocation.

#### Scenario: OAuth bearer runtime config is constructed
- **WHEN** OAuth credential resolution succeeds for a provider prompt
- **THEN** `iron-core` SHALL construct `iron_providers::ProviderCredential::OAuthBearer` with the current access token, expiry, and optional ID token
- **AND** SHALL NOT pass refresh tokens to `iron-providers`

#### Scenario: Codex ID token is available
- **WHEN** Codex OAuth credential material includes an ID token
- **THEN** `iron-core` SHALL preserve and pass the ID token in the OAuth bearer credential supplied to `iron-providers`

### Requirement: Core SHALL expose provider auth status to clients

`iron-core` SHALL expose structured provider auth status so clients can render provider credential state without inspecting secret material.

#### Scenario: Provider credential is not configured
- **WHEN** no API-key or OAuth credential is available for a provider
- **THEN** `iron-core` SHALL report `NotConfigured` for that provider

#### Scenario: Provider uses API key
- **WHEN** an API-key credential is available and selected for a provider
- **THEN** `iron-core` SHALL report `ConfiguredApiKey` for that provider

#### Scenario: Provider uses OAuth
- **WHEN** an OAuth credential is available and valid for a provider
- **THEN** `iron-core` SHALL report `ConnectedOAuth` with the credential expiry when known

#### Scenario: Provider credential cannot be used
- **WHEN** credential resolution or refresh cannot produce a usable provider credential
- **THEN** `iron-core` SHALL report one of `Expired`, `RefreshFailed`, `Revoked`, or `UnsupportedCredential` with actionable context

### Requirement: Core SHALL support OAuth disconnect without removing API keys

`iron-core` SHALL provide a provider OAuth disconnect operation that removes OAuth credential material for the provider without removing API-key configuration that is currently stored or supplied outside the durable OAuth credential. Because the core durable credential store keeps at most one stored credential per provider, disconnect SHALL NOT restore a previously replaced stored API-key credential.

#### Scenario: OAuth credential is disconnected
- **WHEN** a client disconnects OAuth for a provider whose durable stored credential is OAuth
- **THEN** `iron-core` SHALL remove OAuth credential material for that provider through the credential store boundary

#### Scenario: Stored API-key credential is present during disconnect
- **WHEN** a client disconnects OAuth for a provider whose durable stored credential is an API key
- **THEN** `iron-core` SHALL NOT remove the stored API-key credential

#### Scenario: Supplied API-key configuration exists during disconnect
- **WHEN** a client disconnects OAuth for a provider
- **AND** API-key configuration is supplied by prompt context, runtime context, environment, or another non-durable-store path
- **THEN** `iron-core` SHALL NOT remove that API-key configuration

### Requirement: Core SHALL support device-code OAuth metadata and exchange helpers for V1 OAuth providers

`iron-core` SHALL provide V1 OAuth device-code metadata and token exchange helpers for `kimi-code` and `codex` while keeping presentation of the login flow client-owned.

#### Scenario: Client starts Kimi Code OAuth interaction
- **WHEN** a client asks to start OAuth for `kimi-code`
- **THEN** `iron-core` SHALL provide device-code interaction data using Kimi Code OAuth metadata
- **AND** the client SHALL remain responsible for rendering the interaction to the user

#### Scenario: Client starts Codex OAuth interaction
- **WHEN** a client asks to start OAuth for `codex`
- **THEN** `iron-core` SHALL provide device-code interaction data using OpenAI OAuth metadata for Codex
- **AND** the client SHALL remain responsible for rendering the interaction to the user

### Requirement: Core SHALL retry safe OAuth provider auth failures once

`iron-core` SHALL coordinate one forced-refresh retry after an OAuth-backed provider auth failure when retrying is safe.

#### Scenario: OAuth provider auth failure before streamed output
- **WHEN** an OAuth-backed provider invocation fails with an auth error before any stream output is emitted
- **THEN** `iron-core` SHALL force-refresh the OAuth credential and retry the provider invocation at most once

#### Scenario: OAuth provider auth failure after streamed output
- **WHEN** an OAuth-backed provider invocation fails after stream output has been emitted
- **THEN** `iron-core` SHALL NOT silently retry the provider invocation
- **AND** SHALL surface the auth failure to the client

#### Scenario: API-key provider auth failure
- **WHEN** an API-key-backed provider invocation fails with an auth error
- **THEN** `iron-core` SHALL NOT attempt OAuth refresh retry for that invocation

### Requirement: Provider credential support SHALL use effective provider profile metadata
Provider credential orchestration SHALL derive API-key, OAuth bearer, and no-auth support from the effective provider profile metadata when a profile is available for the provider slug.

Provider-specific OAuth device-code metadata MAY remain specialized, but generic credential-mode support used for provider invocation SHALL not rely solely on hardcoded built-in slug lists.

#### Scenario: Custom profile supports API-key credentials
- **WHEN** credential orchestration resolves credentials for a custom provider profile that supports API-key authentication
- **THEN** API-key credentials are accepted for that provider
- **AND** provider invocation uses the auth strategy declared by the effective provider profile

#### Scenario: Effective profile supports OAuth bearer credentials
- **WHEN** credential orchestration resolves credentials for an effective provider profile that supports OAuth bearer authentication
- **THEN** stored OAuth bearer credentials are considered usable for that provider
- **AND** unsupported credential modes are rejected with an actionable unsupported-credential error

#### Scenario: Effective profile supports no-auth invocation
- **WHEN** credential orchestration resolves credentials for an effective provider profile that supports no-auth invocation
- **THEN** the provider can be constructed without requiring API-key or OAuth credential material

#### Scenario: No effective profile metadata is available
- **WHEN** credential orchestration cannot find effective provider profile metadata for a provider slug
- **THEN** credential resolution returns an actionable unsupported-provider or unsupported-credential outcome
- **AND** it does not silently assume all credential modes are supported

