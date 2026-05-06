## ADDED Requirements

### Requirement: Core SHALL support app-owned provider credential storage
`iron-core` SHALL define a provider credential storage boundary that lets clients provide persistence for provider OAuth credential material without making `iron-core` own the concrete storage backend.

#### Scenario: Client supplies provider credential storage
- **WHEN** a client configures managed provider credential orchestration
- **THEN** the client SHALL be able to provide a credential store implementation used by `iron-core` for provider OAuth credential lookup, update, and removal

#### Scenario: Storage backend remains client-owned
- **WHEN** AgentIron stores OAuth credentials in SQLite for V1
- **THEN** `iron-core` SHALL interact only through the credential store boundary
- **AND** `iron-core` SHALL NOT depend on SQLite, OS keyring, or a concrete secure-storage implementation

### Requirement: Core SHALL resolve provider credentials per prompt
`iron-core` SHALL resolve the active provider credential from app-supplied provider/model context for each managed provider prompt execution.

#### Scenario: Prompt specifies provider and model
- **WHEN** a client submits a managed provider prompt with provider and model context
- **THEN** `iron-core` SHALL resolve credentials for that provider before constructing the provider runtime configuration

#### Scenario: Existing injected provider path remains available
- **WHEN** a caller constructs and injects a provider directly
- **THEN** `iron-core` SHALL allow that provider to run without requiring provider credential orchestration

### Requirement: Core SHALL preserve API-key compatibility
Existing API-key provider behavior SHALL continue to work, and API-key credentials SHALL remain valid inputs for provider runtime construction.

#### Scenario: API-key credential constructs runtime config
- **WHEN** a managed provider prompt resolves an API-key credential
- **THEN** `iron-core` SHALL construct `iron_providers::RuntimeConfig` with an API-key `ProviderCredential`
- **AND** provider invocation SHALL use the same API-key auth behavior as before

#### Scenario: API key wins for dual-mode provider
- **WHEN** a provider supports both API-key and OAuth credentials
- **AND** both credential modes are available for that provider
- **THEN** `iron-core` SHALL select the API-key credential for provider invocation

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
`iron-core` SHALL provide a provider OAuth disconnect operation that removes OAuth credential material for the provider without removing API-key configuration.

#### Scenario: OAuth credential is disconnected
- **WHEN** a client disconnects OAuth for a provider
- **THEN** `iron-core` SHALL remove OAuth credential material for that provider through the credential store boundary
- **AND** SHALL NOT remove API-key configuration for that provider

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
