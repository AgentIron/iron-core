## ADDED Requirements

### Requirement: Core SHALL define durable automation tasks
The system SHALL define a typed `AutomationTask` with a stable ID, user-facing name, stored-prompt ID, expected-outcome text, creation timestamp, and update timestamp. The task SHALL reference its profile, instructions, skills, provider, model, tools, and approval policy indirectly through its stored prompt and that prompt's optional profile.

#### Scenario: Automation task captures durable automation identity
- **WHEN** a caller creates an automation task
- **THEN** it includes a stable task ID
- **AND** includes a user-facing name suitable for GUI display
- **AND** references a stored prompt by ID
- **AND** includes required expected-outcome text

#### Scenario: Multiple tasks reuse one stored prompt
- **WHEN** multiple automation tasks reference the same stored prompt
- **THEN** each task retains its own identity, name, and expected outcome
- **AND** the stored prompt's instructions, skills, and optional profile are not duplicated into either task

### Requirement: Core SHALL validate automation-task identity and content
The system SHALL trim the task ID, name, stored-prompt ID, and expected outcome, and SHALL reject a task when any resulting value is empty or when a textual identifier contains control characters. Task IDs SHALL be case-sensitive and SHALL remain stable across updates.

#### Scenario: Valid automation task is accepted
- **WHEN** a caller supplies non-empty valid fields and an existing stored-prompt ID
- **THEN** the system accepts the automation task
- **AND** preserves the normalized task ID exactly after validation

#### Scenario: Invalid automation task is rejected
- **WHEN** a required task field is empty after trimming or a textual identifier contains control characters
- **THEN** the system returns a typed validation error
- **AND** does not persist a partial task

### Requirement: ConfigStore SHALL provide typed automation-task CRUD
ConfigStore SHALL provide typed set, get, list, and delete operations for schema-versioned automation tasks in a first-class task table. Set SHALL create or replace a task atomically, preserve its original creation timestamp on replacement, and update its modification timestamp. List SHALL return tasks in deterministic order.

#### Scenario: Task is created and retrieved
- **WHEN** a caller stores a valid automation task
- **THEN** the caller can retrieve the typed task by ID
- **AND** its persisted fields and timestamps are available

#### Scenario: Existing task is replaced
- **WHEN** a caller stores a valid task using an existing task ID
- **THEN** the new task fields replace the previous fields atomically
- **AND** the original creation timestamp is preserved
- **AND** the update timestamp advances

#### Scenario: Tasks are listed deterministically
- **WHEN** a caller lists automation tasks repeatedly without intervening writes
- **THEN** each result uses the same ordering

#### Scenario: Task is deleted without deleting its prompt
- **WHEN** a caller deletes an automation task
- **THEN** later retrieval reports that the task does not exist
- **AND** its referenced stored prompt remains unchanged

### Requirement: Automation tasks SHALL preserve stored-prompt referential integrity
ConfigStore SHALL require an automation task's stored-prompt ID to identify an existing stored prompt at write time. It SHALL prevent dangling task references without cascading deletion in either direction.

#### Scenario: Missing stored prompt rejects task write
- **WHEN** a caller attempts to store a task whose stored-prompt ID does not exist
- **THEN** ConfigStore returns a typed reference error
- **AND** does not persist the task

#### Scenario: Task deletion preserves stored prompt
- **WHEN** a caller deletes the last task referencing a stored prompt
- **THEN** ConfigStore does not delete or modify the stored prompt

### Requirement: Core SHALL resolve an immutable execution input for a task run
At run start, the system SHALL resolve the automation task, stored prompt, selected agent profile, expected outcome, requested skills, effective tools, provider, model, and workspace into an execution input used for that run. Resolution SHALL use current persisted values, and later configuration edits SHALL not alter the in-progress run.

#### Scenario: Run resolves current dependencies
- **WHEN** execution of an automation task begins
- **THEN** the system resolves the task's current stored prompt and profile references
- **AND** combines stored instructions with the task's expected outcome as the model-visible user goal
- **AND** keeps profile identity instructions in the system prompt layers

#### Scenario: Concurrent edit does not mutate active run
- **WHEN** a referenced task, prompt, or profile changes after run resolution completes
- **THEN** the active run continues with its resolved execution input
- **AND** a later run observes the updated configuration

#### Scenario: Missing named dependency fails resolution
- **WHEN** a task references a missing or malformed stored prompt or named profile
- **THEN** resolution fails with an actionable typed error
- **AND** no model request or tool execution begins

### Requirement: Automation run outcomes SHALL distinguish technical completion from expected-outcome verification
An automation run SHALL treat the task's expected outcome as model-visible goal text and SHALL NOT claim to independently verify semantic success. A `completed` run status SHALL mean runtime execution completed successfully, not that an external evaluator proved the expected outcome.

#### Scenario: Technically completed run
- **WHEN** the runtime finishes the root session without an unhandled execution error
- **THEN** the run status is `completed`
- **AND** the result retains the expected-outcome text
- **AND** the result does not assert independent semantic verification
