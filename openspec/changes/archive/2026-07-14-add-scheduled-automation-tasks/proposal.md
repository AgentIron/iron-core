## Why

AgentIron can execute durable automation tasks non-interactively, but it cannot yet persist when those tasks should run or reconcile them with the host operating system's scheduler. Scheduling must build on the existing automation-task boundary so convenience workflows create an automation task first and never turn the scheduler into an arbitrary command or prompt runner.

## What Changes

- Add typed scheduled-task definitions that reference immutable automation-task IDs and contain only a five-field cron expression and enabled state.
- Give automation tasks unique case-insensitive normalized names for user-facing lookup while schedules and host ownership continue to use immutable task IDs.
- Treat ConfigStore schedule records as desired state and expose separate inspection and reconciliation operations against host scheduler state.
- Add rich status and diagnostics for missing references, unsafe headless policy, absent or drifted host entries, corrupt owned entries, orphaned entries, unsupported schedules, and unavailable platform services.
- Add a host-scheduler abstraction with Linux cron, macOS launchd, and Windows Task Scheduler implementations.
- Install only core-generated `agent-iron run <task-id>` invocations; callers cannot supply executables, arguments, shell fragments, environment variables, or command templates as task payloads.
- Compile a common numeric five-field cron language where a platform can represent it faithfully and reject unsupported expressions without approximation.
- Use the host's local timezone and native daylight-saving behavior.
- Preserve disabled tasks as disabled host entries, including marker-delimited commented blocks in user crontabs.
- Extend automation tasks with a normalized user-facing name plus deterministic project-root and timeout execution settings needed for unattended execution.
- Allow the headless CLI to derive project root and timeout from the selected automation task while retaining explicit invocation overrides for direct operators.
- Add best-effort host run metadata to status while deferring durable run history and normalized cross-platform observability to follow-up work.

## Capabilities

### New Capabilities
- `scheduled-automation-tasks`: Typed scheduled-task persistence, validation, host ownership, platform compilation, inspection, reconciliation, and status reporting.

### Modified Capabilities
- `automation-tasks`: Make each automation task self-contained for unattended execution by persisting its project root and timeout.
- `headless-task-cli`: Permit scheduled invocations to derive deterministic project root and timeout from the automation task instead of requiring scheduler-owned command customization.

## Impact

- Adds scheduler domain and manager modules plus platform-specific cron, launchd, and Windows Task Scheduler adapters.
- Adds typed interpretation of existing ConfigStore schedule records and automation-task schema evolution.
- Changes automation-task inputs and persisted records to include project root and timeout.
- Changes headless CLI resolution precedence for workspace and timeout while preserving task-only execution and non-interactive safety checks.
- Introduces command-runner and filesystem boundaries so host integrations can be tested without mutating a developer's scheduler.
- Requires follow-up issues for durable scheduled-run history, normalized last/next-run reporting, output retention, missed-run policy, and executable relocation or upgrade repair.
