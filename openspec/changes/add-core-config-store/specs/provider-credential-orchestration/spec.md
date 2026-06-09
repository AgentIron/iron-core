## MODIFIED Requirements

### Requirement: Core SHALL support core-owned provider credential storage

`iron-core` SHALL define and provide provider credential storage for provider OAuth/API-key credential material through the core config store, while preserving the existing credential store boundary for tests, in-memory operation, and future alternate backends.

#### Scenario: Core durable credential store is available
- **WHEN** a caller opens the core config store
- **THEN** the caller can obtain or use a provider credential store implementation backed by that config store
- **AND** provider credential lookup, update, removal, and listing can be persisted without frontend-owned storage

#### Scenario: Existing custom store remains available
- **WHEN** tests or alternate embedders supply a custom provider credential store implementation
- **THEN** `iron-core` continues to support that store through the existing credential store boundary

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
