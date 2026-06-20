# provider-profiles Specification

## Purpose
TBD - created by archiving change store-backed-provider-profiles. Update Purpose after archive.
## Requirements
### Requirement: Core SHALL build an effective provider registry from built-ins plus persisted profiles
`iron-core` SHALL provide a way to construct an effective provider registry by starting with the built-in `iron-providers` registry and applying persisted custom provider profiles and explicit provider profile overrides from the core config store.

Persisted provider profiles SHALL override built-in profiles with the same slug. Persisted provider profiles with new slugs SHALL be added alongside built-ins.

#### Scenario: Empty provider profile store preserves built-ins
- **WHEN** the effective provider registry is built with no persisted provider profile records
- **THEN** built-in provider profiles from `iron-providers` remain available
- **AND** no config-store rows are required for built-in providers

#### Scenario: Custom provider profile is added
- **WHEN** the config store contains a valid provider profile whose slug does not match a built-in provider
- **THEN** the effective provider registry includes that custom provider profile
- **AND** callers can resolve the custom provider by slug

#### Scenario: Persisted profile overrides built-in
- **WHEN** the config store contains a valid provider profile whose slug matches a built-in provider
- **THEN** the effective provider registry uses the persisted profile for that slug
- **AND** other built-in profiles remain available

### Requirement: Provider profile records SHALL be non-secret provider protocol metadata
Provider profile records SHALL contain provider protocol metadata needed to construct provider connections and prompt guidance. Provider profile records SHALL NOT contain API keys, OAuth tokens, refresh tokens, user credential state, starred models, or app-specific display preferences.

#### Scenario: Provider profile is stored
- **WHEN** a caller stores a provider profile record
- **THEN** the accepted payload contains only non-secret provider metadata
- **AND** provider credentials remain stored through provider credential APIs

#### Scenario: Provider profile payload contains credential material
- **WHEN** a caller imports or stores a provider profile payload containing credential secret material
- **THEN** the operation fails with an actionable validation error
- **AND** no provider profile record is persisted from that payload

### Requirement: Core SHALL support provider profile import and export
`iron-core` SHALL support importing and exporting provider profile JSON using the `iron-providers` profile schema supported by the compiled dependency version.

Imported provider profiles SHALL be validated before persistence. Exported provider profiles SHALL exclude credentials and runtime-only state.

#### Scenario: Valid provider profile JSON imports
- **WHEN** a caller imports valid provider profile JSON
- **THEN** the profile is stored as a custom profile or explicit override keyed by its slug
- **AND** the stored profile can be loaded into the effective provider registry

#### Scenario: Invalid provider profile JSON is rejected
- **WHEN** a caller imports malformed JSON or a payload that does not satisfy provider profile validation
- **THEN** the import operation returns a validation error
- **AND** no partial provider profile record is stored

#### Scenario: Provider profile exports
- **WHEN** a caller exports a persisted provider profile by slug
- **THEN** the exported JSON contains the stored provider profile metadata
- **AND** the exported JSON does not include provider credential material

### Requirement: Effective provider registry SHALL be usable without durable storage
Callers that do not configure durable provider profile storage SHALL retain built-in-only provider behavior.

#### Scenario: Runtime has no provider profile store
- **WHEN** a runtime or test constructs provider behavior without a config-backed provider profile store
- **THEN** built-in providers remain available through the default `iron-providers` registry
- **AND** no custom provider profiles or overrides are required

