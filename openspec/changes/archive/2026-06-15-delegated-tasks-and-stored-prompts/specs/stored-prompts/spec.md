## ADDED Requirements

### Requirement: Core SHALL define typed stored prompts

The system SHALL define a typed `StoredPrompt` payload for reusable prompt tasks stored in ConfigStore prompt records.

#### Scenario: Stored prompt captures reusable task instructions
- **WHEN** a caller constructs a `StoredPrompt`
- **THEN** the prompt includes `instructions` text
- **AND** includes a list of requested `skills`
- **AND** may include an optional profile ID

#### Scenario: Stored prompt omits credential material
- **WHEN** a stored prompt selects a profile
- **THEN** it references the profile by ID
- **AND** does not store provider API keys or credential secret material

### Requirement: Core SHALL register and list stored prompts

The system SHALL provide APIs to register, unregister, retrieve, and list stored prompts by stable prompt name or ID for the lifetime of the runtime or agent instance.

#### Scenario: Stored prompt is registered
- **WHEN** a caller registers a stored prompt with a valid name or ID
- **THEN** the prompt can be retrieved by that name or ID
- **AND** list operations include it with deterministic ordering

#### Scenario: Stored prompt is replaced
- **WHEN** a caller registers a stored prompt using an existing name or ID
- **THEN** the new prompt replaces the previous prompt if validation succeeds

#### Scenario: Stored prompt is unregistered
- **WHEN** a caller unregisters a stored prompt by name or ID
- **THEN** later retrieval by that name or ID reports that no prompt is registered

### Requirement: Core SHALL load typed stored prompts from ConfigStore

The system SHALL load typed stored prompts from existing ConfigStore prompt records by decoding supported schema-versioned payloads and reporting per-prompt diagnostics for invalid records.

#### Scenario: Stored prompt loads successfully
- **WHEN** ConfigStore contains a prompt record with a supported stored-prompt schema version and valid payload
- **THEN** the system registers the decoded stored prompt under the prompt record ID

#### Scenario: Invalid stored prompt is skipped
- **WHEN** ConfigStore contains an invalid stored-prompt payload
- **THEN** loading skips that prompt
- **AND** returns a diagnostic for the invalid prompt record
- **AND** continues loading other valid prompt records

#### Scenario: Unsupported stored prompt schema is skipped
- **WHEN** ConfigStore contains a prompt record with an unsupported schema version
- **THEN** loading skips that prompt
- **AND** reports an unsupported-schema diagnostic for that prompt record

### Requirement: Stored prompt invocation SHALL use delegated execution machinery

The system SHALL invoke stored prompts through the same delegated child-session machinery used by delegated task execution rather than creating a separate prompt execution path.

#### Scenario: Stored prompt is invoked within a session
- **WHEN** a session-bound caller invokes a stored prompt by name with optional extra context
- **THEN** the system composes the stored instructions and extra context into a delegated child run
- **AND** uses the stored prompt's profile when present or the default profile when absent

#### Scenario: Stored prompt skills respect profile boundary
- **WHEN** a stored prompt requests skills
- **AND** the selected profile's `SkillFilter` excludes one or more requested skills
- **THEN** excluded skills are not activated for the child run
- **AND** the invocation reports diagnostics for excluded requested skills

#### Scenario: Stored prompt result includes child session reference
- **WHEN** a stored prompt invocation completes
- **THEN** the invocation result includes the delegated child session ID
- **AND** includes the final child outcome

### Requirement: Stored prompt invocation SHALL support multiple invocation surfaces

The system SHALL define stored prompt invocation so it can be used by session-bound slash commands initially and by scheduled tasks or CLI one-shot execution in future changes.

#### Scenario: Session slash command can invoke stored prompt
- **WHEN** frontend code handles a session slash command that references a stored prompt
- **THEN** it can call the stored-prompt invocation API with the current parent session
- **AND** child approvals can use the parent/main UI approval workflow

#### Scenario: Future non-session invocation remains compatible
- **WHEN** a future scheduler or CLI one-shot surface invokes a stored prompt without an interactive parent session
- **THEN** it can reuse the same stored-prompt registry and invocation model
- **AND** must supply or derive explicit approval behavior for non-interactive execution
