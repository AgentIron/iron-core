## MODIFIED Requirements

### Requirement: Windows scheduling SHALL use owned Task Scheduler XML definitions
The Windows adapter SHALL manage tasks under `\AgentIron\Tasks\<id>`, generate Task Scheduler XML with one or more triggers as needed, and use `schtasks.exe` for lifecycle operations. When both cron day fields are unrestricted, every generated `CalendarTrigger` SHALL contain exactly one valid Task Scheduler schedule element and SHALL preserve any cron month restriction without adding or omitting occurrences. The adapter SHALL NOT mutate tasks outside the AgentIron task folder during application lifecycle operations.

#### Scenario: Windows task is installed
- **WHEN** a schedule can be compiled faithfully for Task Scheduler
- **THEN** reconciliation registers generated XML under the owned task path
- **AND** the action contains only the core-generated task invocation

#### Scenario: Unrestricted daily schedule is installed
- **WHEN** the Windows adapter compiles `0 3 * * *`
- **THEN** each generated calendar trigger contains a daily schedule element with a one-day interval
- **AND** Task Scheduler accepts the generated XML

#### Scenario: Month-restricted daily schedule is installed
- **WHEN** the Windows adapter compiles `0 3 * 6 *`
- **THEN** each generated calendar trigger contains a monthly schedule element covering every possible day in only the selected months
- **AND** Task Scheduler accepts the generated XML without introducing occurrences outside the selected months

#### Scenario: Failed daily installation is retried after upgrade
- **WHEN** a desired daily schedule remains degraded after an earlier installation failure
- **AND** a caller explicitly reconciles it using the corrected Windows adapter
- **THEN** the adapter retries registration using valid generated XML
- **AND** read-only inspection alone does not install or replace the task

#### Scenario: Windows task is disabled
- **WHEN** desired state is disabled
- **THEN** the owned task remains registered in disabled state

#### Scenario: Unowned Windows task exists
- **WHEN** another task exists outside `\AgentIron\Tasks\`
- **THEN** AgentIron listing, reconciliation, and removal leave it unchanged
