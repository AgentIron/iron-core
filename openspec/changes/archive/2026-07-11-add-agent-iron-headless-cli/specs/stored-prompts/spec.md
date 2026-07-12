## MODIFIED Requirements

### Requirement: Core SHALL register and list stored prompts

The system SHALL provide APIs to register, unregister, retrieve, and list stored prompts by stable prompt name or ID for the lifetime of the runtime or agent instance. Unregistering or deleting a stored prompt SHALL fail with a typed conflict while one or more automation tasks reference it and SHALL NOT cascade-delete those tasks.

#### Scenario: Stored prompt is registered
- **WHEN** a caller registers a stored prompt with a valid name or ID
- **THEN** the prompt can be retrieved by that name or ID
- **AND** list operations include it with deterministic ordering

#### Scenario: Stored prompt is replaced
- **WHEN** a caller registers a stored prompt using an existing name or ID
- **THEN** the new prompt replaces the previous prompt if validation succeeds

#### Scenario: Unreferenced stored prompt is unregistered
- **WHEN** a caller unregisters a stored prompt by name or ID
- **AND** no automation task references that prompt
- **THEN** later retrieval by that name or ID reports that no prompt is registered

#### Scenario: Referenced stored prompt deletion is blocked
- **WHEN** a caller unregisters or deletes a stored prompt referenced by one or more automation tasks
- **THEN** the operation returns a typed conflict identifying the referencing task IDs
- **AND** the stored prompt remains registered
- **AND** the referencing tasks remain unchanged
