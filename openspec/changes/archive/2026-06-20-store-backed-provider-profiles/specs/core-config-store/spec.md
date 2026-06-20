## ADDED Requirements

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
