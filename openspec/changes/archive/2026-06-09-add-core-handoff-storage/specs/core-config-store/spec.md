## ADDED Requirements

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
