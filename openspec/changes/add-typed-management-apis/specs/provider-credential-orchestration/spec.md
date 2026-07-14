## ADDED Requirements

### Requirement: Credential management SHALL return only secret-safe summaries
Core SHALL provide a typed credential-management list operation over configured credential rows only. It SHALL return provider slug, credential mode, metadata, and persisted-state auth status without returning API keys, OAuth access tokens, refresh tokens, ID tokens, encrypted payloads, decrypted secret material, or transient resolver-only status.

#### Scenario: Configured credentials are listed
- **WHEN** a caller lists credential management summaries
- **THEN** each result contains only non-secret metadata and status
- **AND** serialization or debug output of the result cannot reveal stored credential material

#### Scenario: Provider has no configured credential row
- **WHEN** a known provider has no persisted credential
- **THEN** credential listing omits that provider

#### Scenario: Runtime refresh is in progress
- **WHEN** a runtime resolver has transient refresh state that is not persisted
- **THEN** credential listing reports only persisted-state metadata and status
- **AND** does not synthesize the transient runtime state

### Requirement: API-key credentials SHALL use typed mutation operations
Core SHALL provide a typed add-or-replace operation for API-key credentials. The operation SHALL construct `StoredCredential::ApiKey` inside core, and callers SHALL NOT submit an arbitrary serialized credential payload through `ConfigManagementService`.

#### Scenario: API key is replaced
- **WHEN** a caller supplies a provider slug and non-empty API key to the typed replacement operation
- **THEN** core replaces whichever API-key or OAuth credential is currently stored through the existing protected credential store
- **AND** returns a secret-safe configured status rather than the key

#### Scenario: Empty API key is rejected
- **WHEN** a caller supplies an empty or all-whitespace API key
- **THEN** core returns a typed validation error
- **AND** preserves any existing credential

### Requirement: Typed API-key management SHALL preserve OAuth lifecycle semantics
Adding API-key management operations SHALL NOT expose OAuth token material or replace existing OAuth initiation, polling, refresh, revocation, and status behavior with generic raw credential writes.

#### Scenario: OAuth credential is present
- **WHEN** a caller lists or inspects a provider configured through OAuth
- **THEN** the management result reports OAuth mode and existing auth status semantics
- **AND** API-key mutation inputs and outputs do not contain OAuth token fields

### Requirement: Credential deletion SHALL remove the configured mode
The typed credential deletion operation SHALL remove whichever API-key or OAuth credential mode is currently configured for the provider slug without changing dependent profile, prompt, task, or schedule definitions.

#### Scenario: Configured credential is deleted
- **WHEN** a caller deletes a provider credential configured with either supported mode
- **THEN** core removes that persisted credential
- **AND** subsequent configured-credential listing omits the provider
- **AND** dependent configuration definitions remain unchanged
