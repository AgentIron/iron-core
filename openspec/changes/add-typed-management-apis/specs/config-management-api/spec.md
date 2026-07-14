## ADDED Requirements

### Requirement: Core SHALL expose a typed configuration management service
Core SHALL provide an application-facing `ConfigManagementService` with typed list, get, save, and delete operations for agent profiles, stored prompts, automation tasks, and optionally attached scheduled automation tasks. The service SHALL accept and return domain types rather than opaque schema-versioned JSON records and SHALL preserve ConfigStore as the durable source of truth.

#### Scenario: UI saves typed configuration
- **WHEN** a caller saves a profile, stored prompt, automation task, or scheduled task through the management service
- **THEN** core validates and persists the corresponding typed domain input
- **AND** the caller does not construct an opaque ConfigStore payload or select a schema version

#### Scenario: UI lists typed configuration
- **WHEN** a caller lists a managed configuration kind
- **THEN** core returns typed entries in deterministic order
- **AND** malformed or unsupported opaque records are not returned as valid domain values

### Requirement: Management reads SHALL report per-record diagnostics
Single and bulk management reads SHALL return each discovered record as either a ready typed entry or a needs-attention result containing its stable ID, optional decoded typed value, and diagnostics. Diagnostics SHALL cover malformed payloads, unsupported schema versions, invalid fields, unavailable references, and repair-required identities. One invalid opaque record SHALL NOT prevent other valid records from being listed unless the underlying store operation fails.

#### Scenario: One persisted record is malformed
- **WHEN** a managed record set contains valid records and one malformed or unsupported record
- **THEN** the result includes the valid typed entries
- **AND** includes a needs-attention result identifying the invalid record and issue category

#### Scenario: One persisted record is fetched and malformed
- **WHEN** a caller gets an existing record whose payload cannot be decoded
- **THEN** core returns a needs-attention result with the stable ID, no decoded value, and typed diagnostics
- **AND** does not return the malformed record as a ready domain value

#### Scenario: Store access fails
- **WHEN** ConfigStore cannot complete the list or read operation
- **THEN** the service returns a typed fatal storage error
- **AND** does not misrepresent the failure as an empty record set

### Requirement: Core SHALL expose schema-aware dependency impact
Core SHALL provide typed dependency-impact queries for a profile ID, provider credential slug, stored-prompt ID, and automation-task ID. Results SHALL distinguish direct references from transitive dependents and SHALL identify relationship paths among profiles, prompts, automation tasks, and schedules.

#### Scenario: Profile impact is queried
- **WHEN** a stored prompt directly references a profile and automation tasks reference that prompt
- **THEN** the profile impact identifies the prompt as a direct dependent
- **AND** identifies the automation tasks and their schedules as transitive dependents with their relationship paths

#### Scenario: Credential impact is queried
- **WHEN** profiles select a provider credential slug and prompts or tasks depend on those profiles
- **THEN** the credential impact identifies the profiles as direct dependents
- **AND** identifies dependent prompts, tasks, and schedules transitively without exposing credential material

### Requirement: Structural deletes SHALL preserve referential integrity
Deleting a profile referenced by stored prompts, a stored prompt referenced by automation tasks, or an automation task referenced by schedules SHALL fail with a typed conflict that identifies the direct referencing records. If malformed or unsupported records prevent core from proving that no reference exists, deletion SHALL fail with a typed integrity-unknown error identifying those records. Core SHALL NOT cascade these structural deletes. Removing a provider credential SHALL remain permitted because an unconfigured credential is a valid provider state, but callers SHALL be able to query its dependency impact first.

#### Scenario: Referenced profile deletion is blocked
- **WHEN** a caller deletes a profile referenced by one or more stored prompts
- **THEN** core returns a typed conflict containing the referencing prompt IDs
- **AND** preserves the profile and dependent records

#### Scenario: Credential is removed despite dependencies
- **WHEN** a caller removes a provider credential used by one or more profiles
- **THEN** core removes the credential through the typed credential operation
- **AND** subsequent status reports the provider as not configured
- **AND** profile, prompt, task, and schedule definitions remain unchanged

#### Scenario: Malformed record obscures dependency safety
- **WHEN** a malformed or unsupported record could contain a reference relevant to a structural delete
- **THEN** core returns a typed integrity-unknown error identifying the unreadable record
- **AND** preserves the target and dependent records

### Requirement: Scheduled-task management SHALL compose desired and observed state
When a schedule manager is attached, `ConfigManagementService` SHALL expose typed scheduled-task desired-state CRUD and existing scheduler inspection and reconciliation operations. Calls requiring scheduler behavior without an attachment SHALL return a typed scheduler-unavailable error. Read operations SHALL NOT mutate host scheduler state, and all installed actions SHALL remain core-generated automation-task invocations.

#### Scenario: UI refreshes scheduled tasks
- **WHEN** a caller lists scheduled-task definitions and status
- **THEN** core returns typed desired definitions and compositional observed status
- **AND** performs no host scheduler mutation

#### Scenario: UI explicitly reconciles a schedule
- **WHEN** a caller requests reconciliation through the management service
- **THEN** core delegates to the typed schedule manager
- **AND** does not accept an executable, command, argument list, shell fragment, environment map, or stored-prompt execution target

#### Scenario: Scheduler is not attached
- **WHEN** a caller invokes schedule inspection, reconciliation, or combined deletion without an attached schedule manager
- **THEN** core returns a typed scheduler-unavailable error
- **AND** non-scheduler configuration management remains available

### Requirement: Combined schedule deletion SHALL prefer execution safety
Combined schedule deletion SHALL remove or disable the owned host entry before deleting desired ConfigStore state. A host-removal failure SHALL preserve desired state. A desired-state deletion failure after host removal SHALL return a typed partial outcome describing both effects and resulting drift.

#### Scenario: Host removal fails
- **WHEN** combined deletion cannot remove or disable the owned host entry
- **THEN** core preserves the desired scheduled-task record
- **AND** returns the host failure without claiming deletion succeeded

#### Scenario: Desired deletion fails after host removal
- **WHEN** host removal succeeds but deleting desired ConfigStore state fails
- **THEN** core reports `host_removed` as true and `desired_deleted` as false
- **AND** returns diagnostics describing the remaining desired-state drift

### Requirement: Successful writes SHALL update attached registries
Profile and stored-prompt writes SHALL persist durable state before updating attached registries. If durable persistence succeeds and registry synchronization fails, core SHALL return a typed partial-operation outcome. A persistence failure SHALL leave the attached registry unchanged.

#### Scenario: Durable write succeeds
- **WHEN** a profile or prompt is persisted successfully and registry synchronization succeeds
- **THEN** the attached registry reflects the new durable value before the management operation returns success

#### Scenario: Registry synchronization fails
- **WHEN** durable persistence succeeds but the attached registry cannot be updated
- **THEN** core returns a partial-operation outcome identifying durable success and registry failure
- **AND** does not roll back or misreport the committed durable write

### Requirement: Preview and durable scheduled-run history SHALL remain separate capabilities
This management service SHALL NOT introduce direct stored-prompt scheduling, a second non-interactive stored-prompt runner, or an in-memory session view presented as durable scheduled-run history. Interactive prompt preview and durable scheduled-run/session history SHALL require focused follow-up contracts.

#### Scenario: Caller manages a stored prompt
- **WHEN** a caller creates or updates a stored prompt
- **THEN** the management API does not create a host schedule or invoke unattended execution

#### Scenario: Process restarts after a scheduled run
- **WHEN** no durable scheduled-run history capability has been implemented
- **THEN** the management service does not claim that active or in-memory child-session APIs provide historical run records
