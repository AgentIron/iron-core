## MODIFIED Requirements

### Requirement: Core SHALL define durable automation tasks
The system SHALL define a typed `AutomationTask` with an immutable internal ID, unique normalized name, user-facing display name, stored-prompt ID, expected-outcome text, canonical project root, positive execution timeout, creation timestamp, and update timestamp. Normalized names SHALL be case-insensitive for lookup and SHALL reject collisions rather than silently generating suffixes. The task SHALL reference its profile, instructions, skills, provider, model, tools, and approval policy indirectly through its stored prompt and that prompt's optional profile.

#### Scenario: Automation task captures durable automation identity
- **WHEN** a caller creates an automation task
- **THEN** it includes an immutable task ID
- **AND** includes a unique normalized name suitable for convenient lookup
- **AND** includes a user-facing display name suitable for GUI display
- **AND** references a stored prompt by ID
- **AND** includes required expected-outcome text
- **AND** includes the project root and timeout required for deterministic unattended execution

#### Scenario: Multiple tasks reuse one stored prompt
- **WHEN** multiple automation tasks reference the same stored prompt
- **THEN** each task retains its own ID, normalized name, display name, expected outcome, project root, and timeout
- **AND** the stored prompt's instructions, skills, and optional profile are not duplicated into either task

#### Scenario: Display name changes without breaking schedules
- **WHEN** a caller renames an automation task
- **THEN** its immutable internal ID remains unchanged
- **AND** schedules continue to reference the same task without rewriting their references

### Requirement: Core SHALL validate automation-task identity and content
The system SHALL validate the immutable task ID, normalize the task name into the supported user-facing form, trim the display name, stored-prompt ID, and expected outcome, and reject a task when any required resulting value is empty or when a textual identifier contains control characters. Core SHALL canonicalize the project root, require it to be an existing directory, and require a positive bounded execution timeout. Task IDs SHALL remain stable across updates, and normalized-name uniqueness SHALL be enforced case-insensitively.

#### Scenario: Valid automation task is accepted
- **WHEN** a caller supplies valid identity and content fields, an existing stored-prompt ID, an existing project-root directory, and a positive supported timeout
- **THEN** the system accepts the automation task
- **AND** preserves its immutable task ID
- **AND** stores its normalized lookup name
- **AND** persists the canonical project root and timeout

#### Scenario: Invalid automation task is rejected
- **WHEN** a required task field is empty after normalization, a textual identifier contains control characters, the project root is invalid, or the timeout is invalid
- **THEN** the system returns a typed validation error
- **AND** does not persist a partial task

#### Scenario: Normalized name collides case-insensitively
- **WHEN** a caller creates or renames a task to a normalized name already used by another task regardless of case
- **THEN** the system returns a typed conflict
- **AND** does not silently suffix or replace either task

### Requirement: Core SHALL resolve an immutable execution input for a task run
At run start, the system SHALL resolve the automation task, stored prompt, selected agent profile, expected outcome, requested skills, effective tools, provider, model, canonical project root, and timeout into an execution input used for that run. Resolution SHALL use current persisted values, and later configuration edits SHALL not alter the in-progress run.

#### Scenario: Run resolves current dependencies
- **WHEN** execution of an automation task begins
- **THEN** the system resolves the task's current stored prompt and profile references
- **AND** combines stored instructions with the task's expected outcome as the model-visible user goal
- **AND** resolves the task's canonical project root as the session workspace
- **AND** resolves the task's timeout for complete execution
- **AND** keeps profile identity instructions in the system prompt layers

#### Scenario: Concurrent edit does not mutate active run
- **WHEN** a referenced task, prompt, profile, project root, or timeout changes after run resolution completes
- **THEN** the active run continues with its resolved execution input
- **AND** a later run observes the updated configuration

#### Scenario: Missing named dependency fails resolution
- **WHEN** a task references a missing or malformed stored prompt or named profile
- **THEN** resolution fails with an actionable typed error
- **AND** no model request or tool execution begins

#### Scenario: Stored execution context becomes invalid
- **WHEN** a task's persisted project root no longer exists or its timeout cannot be decoded
- **THEN** resolution fails with an actionable configuration error
- **AND** no model request or tool execution begins
