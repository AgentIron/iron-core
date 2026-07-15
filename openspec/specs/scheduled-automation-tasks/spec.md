# scheduled-automation-tasks Specification

## Purpose
TBD - created by archiving change add-scheduled-automation-tasks. Update Purpose after archive.

## Requirements
### Requirement: Core SHALL define typed scheduled automation tasks
The system SHALL define a schema-versioned `ScheduledTask` with a stable schedule ID, an existing automation-task ID, a numeric five-field cron expression, enabled state, and store-maintained creation and update timestamps. A scheduled task SHALL NOT contain prompt text, a stored-prompt reference as its execution target, an executable, arguments, shell text, environment variables, or a user-provided command template.

#### Scenario: Existing automation task is scheduled
- **WHEN** a caller creates a schedule with a valid ID, existing automation-task ID, valid cron expression, and enabled state
- **THEN** core persists the typed scheduled task
- **AND** the schedule references the automation task by its stable ID

#### Scenario: Arbitrary execution payload is not representable
- **WHEN** a caller constructs a scheduled-task input
- **THEN** the typed input provides no field for an arbitrary command, prompt, executable, argument list, shell fragment, environment map, or stored-prompt execution target

### Requirement: Scheduled tasks SHALL preserve automation-task referential integrity
Core SHALL require the referenced automation task to exist when a scheduled task is created or updated. Deleting an automation task referenced by one or more desired schedules SHALL fail with a typed conflict that identifies the referencing schedule IDs.

#### Scenario: Missing automation task rejects schedule
- **WHEN** a caller saves a scheduled task whose automation-task ID does not exist
- **THEN** core returns a typed reference error
- **AND** does not persist a partial scheduled task

#### Scenario: Referenced automation task cannot be deleted
- **WHEN** a caller deletes an automation task referenced by a desired schedule
- **THEN** core returns a typed conflict identifying the schedule
- **AND** preserves both records

### Requirement: Core SHALL validate the common cron language
Core SHALL accept exactly five fields in minute, hour, day-of-month, month, and day-of-week order. Fields SHALL support wildcard, numeric values, numeric ranges, numeric lists, wildcard steps, and range steps within the field's valid numeric bounds. Core SHALL reject seconds fields, macros, named months or weekdays, malformed lists or ranges, zero steps, and non-cron schedule kinds.

#### Scenario: Numeric stepped expression is accepted
- **WHEN** a caller supplies `*/15 9-17/2 * 1,6,12 1-5`
- **THEN** common-language validation succeeds

#### Scenario: Unsupported syntax is rejected
- **WHEN** a caller supplies a macro, named weekday, seconds field, malformed range, out-of-bounds value, or zero step
- **THEN** validation returns an actionable field-specific error
- **AND** no host entry is installed

### Requirement: Host adapters SHALL compile schedules faithfully or reject them
Each host adapter SHALL either represent every occurrence of a valid common cron expression without introducing additional occurrences or omitting required occurrences, or return a typed unsupported-schedule error. Adapters SHALL NOT approximate an unsupported schedule.

#### Scenario: Platform supports expression
- **WHEN** the selected host adapter can represent a parsed expression faithfully
- **THEN** it compiles the expression into one or more native triggers belonging to one owned task

#### Scenario: Platform cannot preserve semantics
- **WHEN** the selected host adapter cannot represent a parsed expression faithfully or within its bounded expansion limit
- **THEN** installation fails with an unsupported-schedule diagnostic
- **AND** desired ConfigStore state remains available for inspection and retry

### Requirement: Schedules SHALL use native local-time behavior
Host entries SHALL use the host scheduler's current local system timezone and native daylight-saving behavior. Scheduled-task records SHALL NOT persist or emulate a separate timezone.

#### Scenario: System timezone changes
- **WHEN** the host system timezone changes after installation
- **THEN** future occurrences follow the host scheduler's native local-time interpretation
- **AND** core does not translate the schedule back to the former timezone

#### Scenario: Daylight-saving transition occurs
- **WHEN** local time skips or repeats during a daylight-saving transition
- **THEN** execution follows the native host scheduler behavior

### Requirement: ConfigStore SHALL be desired schedule state
Typed scheduled-task records in ConfigStore SHALL represent desired state, while owned host entries SHALL represent observed state. Saving desired state SHALL remain successful when subsequent host reconciliation fails, and the failure SHALL be visible through status.

#### Scenario: Host installation fails after save
- **WHEN** a valid scheduled task is persisted and host installation fails
- **THEN** the desired scheduled task remains persisted
- **AND** status reports that the desired task is not installed with the host failure diagnostic

### Requirement: Inspection SHALL be read-only and reconciliation SHALL be explicit
The application-facing scheduler API SHALL provide a read-only inspection operation and a distinct mutating reconciliation operation. Inspection SHALL NOT install, replace, disable, enable, or remove host entries. Reconciliation SHALL operate only on AgentIron-owned host entries.

#### Scenario: UI refresh inspects status
- **WHEN** a caller requests schedule status through inspection
- **THEN** core compares desired and observed state
- **AND** performs no host mutation

#### Scenario: Caller requests reconciliation
- **WHEN** a caller explicitly reconciles a valid desired schedule
- **THEN** core installs, replaces, enables, or disables its owned host entry as needed
- **AND** returns the observed post-operation status

### Requirement: Status reports SHALL compose desired, reference, execution, and host state
Core SHALL return a status report with an overall health summary, desired-state status, automation-task reference status, headless-execution readiness, host-entry status, diagnostics, and optional host run metadata. The report SHALL support multiple simultaneous diagnostics.

#### Scenario: Healthy task is reported
- **WHEN** desired state is valid, its automation task is headless-safe, and a matching enabled or disabled owned host entry exists
- **THEN** status reports healthy with the corresponding enabled state

#### Scenario: Multiple invariant failures exist
- **WHEN** a desired record references an invalid task and an owned host entry is corrupt
- **THEN** one report describes both failures
- **AND** does not collapse them into an ambiguous boolean

#### Scenario: Host metadata is unavailable
- **WHEN** a host adapter cannot report last run, next run, or last result
- **THEN** those fields are absent
- **AND** their absence alone does not make an otherwise matching entry unhealthy

### Requirement: Scheduled invocation SHALL be generated exclusively by core
The schedule manager SHALL derive an absolute runner invocation from trusted installation context and the referenced automation-task ID. Host entries SHALL invoke `agent-iron run <task-id>` against the intended ConfigStore and SHALL NOT rely on `PATH`, process working directory, or caller-provided command payloads.

#### Scenario: Owned invocation is installed
- **WHEN** a valid desired schedule is reconciled
- **THEN** the host entry uses the manager's absolute `agent-iron` runner path
- **AND** selects the intended ConfigStore
- **AND** names only the referenced automation task as user-controlled execution identity

#### Scenario: Runner path changes
- **WHEN** the trusted installation context contains a different runner path than an existing owned entry
- **THEN** inspection reports drift
- **AND** explicit reconciliation replaces the generated invocation

### Requirement: HostScheduler factory SHALL select the current platform
Core SHALL provide a factory that selects the Linux cron adapter, macOS launchd adapter, or Windows Task Scheduler adapter for supported compilation targets and returns a typed unsupported-platform error for other targets.

#### Scenario: Supported platform creates adapter
- **WHEN** the factory runs on Linux, macOS, or Windows
- **THEN** it returns the corresponding host adapter

#### Scenario: Unsupported platform requests scheduler
- **WHEN** the factory runs on another target
- **THEN** it returns an unsupported-platform error without host mutation

### Requirement: Linux scheduling SHALL preserve non-owned crontab content
The Linux adapter SHALL manage only marker-delimited AgentIron blocks in the current user's crontab. Enabled tasks SHALL contain generated active lines, disabled tasks SHALL retain generated lines as comments, and content outside valid owned blocks SHALL be preserved. Malformed or duplicate owned blocks SHALL produce diagnostics rather than guessed mutation.

#### Scenario: Task is installed beside user entries
- **WHEN** reconciliation installs a task into a crontab containing non-AgentIron entries
- **THEN** the adapter adds or replaces only the task's owned marker-delimited block
- **AND** preserves non-owned entries

#### Scenario: Cron task is disabled
- **WHEN** desired state changes from enabled to disabled
- **THEN** reconciliation retains the owned block with generated cron lines commented
- **AND** later inspection recognizes it as installed and disabled

#### Scenario: Owned markers are malformed
- **WHEN** inspection finds unbalanced or duplicate markers for an AgentIron task
- **THEN** status reports a corrupt owned entry
- **AND** reconciliation does not remove unrelated or ambiguously bounded text

### Requirement: macOS scheduling SHALL use owned user LaunchAgents
The macOS adapter SHALL manage user-level LaunchAgent plists whose labels use the `com.agentiron.task.` prefix. It SHALL mutate only plists in the expected user LaunchAgents location with valid owned labels and definitions.

#### Scenario: LaunchAgent is installed
- **WHEN** a schedule can be compiled faithfully for launchd
- **THEN** reconciliation writes a user LaunchAgent with an owned label and core-generated invocation

#### Scenario: Unowned plist is present
- **WHEN** the LaunchAgents directory contains a plist without a valid AgentIron task label
- **THEN** listing, reconciliation, and removal leave it unchanged

### Requirement: Windows scheduling SHALL use owned Task Scheduler XML definitions
The Windows adapter SHALL manage tasks under `\AgentIron\Tasks\<id>`, generate Task Scheduler XML with one or more triggers as needed, and use `schtasks.exe` for lifecycle operations. It SHALL NOT mutate tasks outside the AgentIron task folder.

#### Scenario: Windows task is installed
- **WHEN** a schedule can be compiled faithfully for Task Scheduler
- **THEN** reconciliation registers generated XML under the owned task path
- **AND** the action contains only the core-generated task invocation

#### Scenario: Windows task is disabled
- **WHEN** desired state is disabled
- **THEN** the owned task remains registered in disabled state

#### Scenario: Unowned Windows task exists
- **WHEN** another task exists outside `\AgentIron\Tasks\`
- **THEN** AgentIron listing, reconciliation, and removal leave it unchanged

### Requirement: Host integrations SHALL be testable without real scheduler mutation
Platform adapters SHALL use injectable command-runner and filesystem boundaries sufficient to test rendering, parsing, command construction, ownership filtering, disabled state, and failure handling without changing the test user's real scheduler.

#### Scenario: Adapter test installs a task
- **WHEN** a test reconciles through mocked command and filesystem boundaries
- **THEN** the test can assert generated native definitions and commands
- **AND** no real crontab, LaunchAgent, or Windows task is changed

### Requirement: Orphaned owned entries SHALL remain observable
Inspection SHALL report an owned host entry whose schedule ID has no desired ConfigStore record. Status reads SHALL NOT remove orphaned entries, and removal SHALL require explicit reconciliation policy or an explicit remove operation.

#### Scenario: ConfigStore record is missing
- **WHEN** an owned host entry exists without a corresponding desired schedule
- **THEN** inspection reports the entry as orphaned
- **AND** leaves it installed
