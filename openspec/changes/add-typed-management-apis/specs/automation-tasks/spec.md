## ADDED Requirements

### Requirement: Automation tasks SHALL be available through typed management
`ConfigManagementService` SHALL expose existing typed automation-task create-or-replace, get, list, normalized-handle lookup, and delete behavior without requiring callers to access automation-task tables or reconstruct schema validation.

#### Scenario: Automation task is saved through management
- **WHEN** a caller submits a valid `AutomationTaskInput`
- **THEN** the service persists and returns the typed automation task using existing normalization and reference validation
- **AND** does not accept raw automation-task columns or schema versions

### Requirement: Automation-task impacts SHALL include transitive configuration dependencies
Dependency queries SHALL identify an automation task's direct stored-prompt dependency, the prompt's optional profile dependency, the profile's optional provider credential dependency, and schedules that directly reference the task. The result SHALL distinguish dependencies from dependents.

#### Scenario: Task dependency impact is queried
- **WHEN** a task references a prompt that selects a managed profile and that profile selects a provider slug
- **THEN** the result identifies the prompt as a direct dependency
- **AND** identifies the profile and credential slug as transitive dependencies
- **AND** identifies referencing schedules as direct dependents
