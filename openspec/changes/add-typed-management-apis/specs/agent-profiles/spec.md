## ADDED Requirements

### Requirement: Core SHALL persist profiles through typed management operations
Core SHALL provide typed profile save, get, list, and delete operations using `AgentProfile`, `AgentProfileId`, and `PROFILE_SCHEMA_VERSION`. Typed writes SHALL validate profile IDs, protected default identity, unique profile names, provider context, and user-facing approval values before persisting any change.

#### Scenario: Valid profile is saved
- **WHEN** a caller saves a valid typed profile under a non-default stable profile ID
- **THEN** core persists it using the current profile schema version
- **AND** a typed get operation returns the same normalized profile without exposing opaque JSON

#### Scenario: Unsupported approval value is loaded
- **WHEN** a persisted profile payload contains `ReadOnly` or `RequireApproval`
- **THEN** the profile is omitted from valid management entries
- **AND** its diagnostic identifies the rejected approval value

### Requirement: Profile renames SHALL preserve stable references
Updating a profile's user-facing name SHALL preserve its stable `AgentProfileId`. Name validation and collision checks SHALL complete before the durable record or in-memory registry is changed.

#### Scenario: Referenced profile is renamed
- **WHEN** a caller changes the name of a profile referenced by stored prompts
- **THEN** the profile retains its stable ID
- **AND** prompt references remain valid without rewriting those prompts
