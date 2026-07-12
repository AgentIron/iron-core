## 1. Automation Execution Context

- [ ] 1.1 Add immutable ID, unique normalized name, display name, canonical project-root, and positive bounded timeout fields to automation-task domain and input types
- [ ] 1.2 Add normalized-name generation, case-insensitive lookup, collision rejection, and rename tests
- [ ] 1.3 Add a compiled ConfigStore migration and typed CRUD support for automation-task identity and execution context
- [ ] 1.4 Define migration behavior and diagnostics for existing automation tasks without normalized names or deterministic execution context
- [ ] 1.5 Resolve project root and timeout into immutable automation execution input and validate stale roots at run start
- [ ] 1.6 Update automation-task persistence, validation, reference-integrity, rename, and execution-resolution tests

## 2. Headless CLI Defaults

- [ ] 2.1 Change workspace precedence to CLI, environment, then persisted automation-task project root
- [ ] 2.2 Change timeout precedence to CLI, environment, then persisted automation-task timeout
- [ ] 2.3 Remove process-current-directory workspace fallback and preserve explicit override behavior
- [ ] 2.4 Add CLI unit and integration tests for task defaults, explicit overrides, invalid stored context, and scheduled-style invocation

## 3. Schedule Domain And Persistence

- [ ] 3.1 Define schema-versioned scheduled-task, input, status, diagnostic, host metadata, and error types
- [ ] 3.2 Implement five-field numeric cron parsing and validation for wildcards, values, ranges, lists, and steps
- [ ] 3.3 Add exhaustive cron boundary and unsupported-syntax tests, including day-of-month and day-of-week semantics
- [ ] 3.4 Add typed ConfigStore schedule CRUD over existing schedule records with deterministic listing and schema diagnostics
- [ ] 3.5 Enforce schedule-to-automation-task references and block automation-task deletion with referencing schedule IDs
- [ ] 3.6 Add persistence tests for reference integrity, replacement timestamps, malformed payloads, unsupported schema versions, and delete conflicts

## 4. Host Scheduler Contract

- [ ] 4.1 Define trusted scheduler installation context and core-generated runner invocation
- [ ] 4.2 Define async HostScheduler operations and observed host-entry types without arbitrary command inputs
- [ ] 4.3 Add injectable command-runner and filesystem boundaries with fake implementations for tests
- [ ] 4.4 Implement platform factory selection and unsupported-platform errors
- [ ] 4.5 Test that generated invocations use absolute runner and ConfigStore paths and expose no caller-controlled command surface

## 5. Schedule Manager And Reconciliation

- [ ] 5.1 Implement read-only inspection across desired schedules, automation-task references, headless preflight, and observed host entries
- [ ] 5.2 Implement compositional health, drift, corruption, unavailable-platform, missing-reference, unsafe-policy, and orphan diagnostics
- [ ] 5.3 Implement explicit idempotent reconciliation for install, replace, enable, disable, remove, and unchanged states
- [ ] 5.4 Define and implement explicit orphan-removal policy without mutation during inspection
- [ ] 5.5 Add fake-adapter tests for partial failures, retries, simultaneous diagnostics, runner-path drift, and optional host metadata

## 6. Linux Cron Adapter

- [ ] 6.1 Implement marker-delimited owned-block rendering for enabled and commented disabled tasks
- [ ] 6.2 Implement user-crontab parsing that preserves non-owned content and reports malformed or duplicate owned blocks
- [ ] 6.3 Implement cron install, replace, enable, disable, list, inspect, and remove through the command-runner boundary
- [ ] 6.4 Add mixed crontab fixtures covering unrelated entries, multiple owned blocks, malformed markers, disabled tasks, and exact-target removal
- [ ] 6.5 Verify Linux schedule compilation preserves common cron occurrence semantics without approximation

## 7. macOS Launchd Adapter

- [ ] 7.1 Define bounded faithful cron-to-StartCalendarInterval expansion and actionable unsupported cases
- [ ] 7.2 Implement owned user LaunchAgent plist rendering and parsing with `com.agentiron.task.` labels
- [ ] 7.3 Implement launchd install, replace, enable, disable, list, inspect, and remove through injectable boundaries
- [ ] 7.4 Add plist and command fixtures for multiple intervals, disabled state, drift, corruption, ownership filtering, and expansion limits

## 8. Windows Task Scheduler Adapter

- [ ] 8.1 Define bounded faithful cron-to-Task-Scheduler-trigger expansion and actionable unsupported cases
- [ ] 8.2 Implement owned Task Scheduler XML rendering and parsing under `\AgentIron\Tasks\<id>`
- [ ] 8.3 Implement `schtasks.exe` create, query, enable, disable, delete, list, and inspect operations through the command-runner boundary
- [ ] 8.4 Add mocked `schtasks.exe` and XML fixtures for multiple triggers, disabled state, drift, corruption, ownership filtering, and expansion limits

## 9. Cross-Platform Verification And Documentation

- [ ] 9.1 Add contract tests shared by all host adapters for ownership isolation, faithful-or-fail compilation, disabled retention, and idempotent reconciliation
- [ ] 9.2 Document local-system-timezone and native daylight-saving behavior plus platform capability failures
- [ ] 9.3 Document that scheduling accepts automation tasks only and that create-and-schedule UI flows remain two core operations
- [ ] 9.4 Run formatting, linting, unit tests, integration tests, and supported platform checks
- [ ] 9.5 File follow-up issues for durable run history, output retention, normalized host run metadata, missed-run policy, concurrent-run policy, credential-session limitations, and executable relocation repair
