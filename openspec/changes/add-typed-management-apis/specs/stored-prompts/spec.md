## ADDED Requirements

### Requirement: Stored prompts SHALL separate stable IDs from user-facing identity
Core SHALL generate an immutable unique ConfigStore record ID when creating a stored prompt. Each prompt SHALL have a mutable display name and a required normalized lookup handle stored in a uniquely indexed column and derived from the display name. Normalized handles SHALL be compared case-insensitively and normalization collisions SHALL be rejected rather than resolved with generated user-facing suffixes.

#### Scenario: Prompt name is normalized
- **WHEN** a caller saves a prompt with display name `Check Email`
- **THEN** core derives the normalized handle `check-email`
- **AND** retains a distinct immutable prompt ID for references

#### Scenario: Prompt is created
- **WHEN** a caller creates a stored prompt without an existing immutable ID
- **THEN** core generates a unique stable prompt ID
- **AND** returns that ID with the persisted typed prompt

#### Scenario: Normalized prompt name collides
- **WHEN** another prompt's display name normalizes to a handle already owned by a different prompt ID
- **THEN** core returns a typed name conflict
- **AND** does not alter either prompt

### Requirement: Stored-prompt lookup SHALL be explicit and case-insensitive
Core SHALL provide lookup by immutable prompt ID and separate lookup by normalized handle. Handle lookup SHALL normalize user input and match without user-visible case sensitivity so IDs and handles cannot be ambiguously interpreted by one operation.

#### Scenario: User enters a differently cased name
- **WHEN** a caller looks up `CHECK EMAIL` by handle
- **THEN** core resolves the prompt whose normalized handle is `check-email`

#### Scenario: Caller looks up an immutable ID
- **WHEN** a caller requests a prompt by stable ID
- **THEN** core performs ID lookup without treating the value as a display name or handle

### Requirement: Stored-prompt handles SHALL use canonical ASCII kebab case
Core SHALL normalize prompt handles to lowercase ASCII alphanumeric segments separated only by single hyphens. Whitespace, underscores, and existing hyphens SHALL normalize to hyphens; repeated separators SHALL collapse; unsupported punctuation and non-ASCII characters SHALL be removed; and empty normalized results SHALL be rejected.

#### Scenario: Equivalent separators normalize identically
- **WHEN** callers save `Check Email`, `Check_Email`, or `CHECK-EMAIL` as display names
- **THEN** each candidate normalizes to `check-email`
- **AND** only one prompt may own that indexed handle

#### Scenario: Name has no supported characters
- **WHEN** a display name contains no ASCII alphanumeric characters after normalization
- **THEN** core returns a typed validation error
- **AND** does not persist the prompt

### Requirement: Stored-prompt rename SHALL preserve references
Changing a stored prompt's display name SHALL update its normalized handle while preserving its immutable prompt ID. Existing automation-task references SHALL remain unchanged and valid.

#### Scenario: Referenced prompt is renamed
- **WHEN** a prompt referenced by automation tasks receives a non-conflicting new display name
- **THEN** its display name and normalized handle change
- **AND** its immutable ID and all automation-task references remain unchanged

### Requirement: Typed stored-prompt management SHALL validate content and references
Typed stored-prompt writes SHALL reject blank instructions, empty or malformed requested skill identifiers, duplicate requested skills, requested skills unavailable to the selected profile in the creation-time availability snapshot, and missing non-default profile references. Creation-time availability SHALL be best-effort and SHALL NOT guarantee later execution availability. Management reads SHALL preserve records and report diagnostics when previously requested skills are no longer available.

#### Scenario: Prompt references a missing profile
- **WHEN** a caller saves a prompt with a non-default profile ID that does not exist
- **THEN** core returns a typed reference error
- **AND** does not persist the invalid prompt

#### Scenario: Persisted prompt requests an unavailable skill
- **WHEN** a stored prompt is structurally valid but a requested skill is not in the supplied management skill inventory
- **THEN** the prompt load result reports an unavailable-skill diagnostic naming the prompt and skill
- **AND** does not silently remove the requested skill from the payload

#### Scenario: New prompt requests a currently unavailable skill
- **WHEN** a caller creates a prompt requesting a skill unavailable to its selected profile in the supplied creation-time snapshot
- **THEN** core returns a typed validation error
- **AND** does not persist the prompt

### Requirement: Legacy stored prompts SHALL remain readable during identity migration
Core SHALL recognize the previously supported stored-prompt schema while introducing display and normalized identity. A valid legacy prompt SHALL derive a title-cased display name from its kebab-case stable record ID and normalized identity from that display name. It SHALL be written using the current schema when saved through typed management.

#### Scenario: Legacy prompt is listed
- **WHEN** ConfigStore contains a valid legacy stored-prompt payload
- **THEN** typed management can list it with identity derived deterministically from its record ID
- **AND** automation-task references to that ID remain valid

#### Scenario: Legacy prompt is updated
- **WHEN** a caller saves a listed legacy prompt through typed management
- **THEN** core persists the current stored-prompt schema with explicit display and normalized identity
- **AND** preserves the original stable prompt ID

#### Scenario: Legacy handles collide during migration
- **WHEN** multiple legacy prompt IDs derive the same normalized handle
- **THEN** migration assigns each colliding record a deterministic reserved repair handle satisfying the unique index
- **AND** marks each record as needing rename
- **AND** keeps each record retrievable by immutable ID without exposing the repair handle as a normal user-facing lookup handle
