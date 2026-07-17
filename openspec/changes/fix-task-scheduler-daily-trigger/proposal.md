## Why

The Windows Task Scheduler adapter emits a `CalendarTrigger` without a required schedule element for unrestricted daily crons such as `0 3 * * *`, causing `schtasks.exe` to reject the task. Issue [#104](https://github.com/AgentIron/iron-core/issues/104) also identifies the same invalid path for month-restricted schedules whose day fields are unrestricted.

## What Changes

- Generate schema-valid Task Scheduler calendar triggers for unrestricted daily schedules.
- Preserve month restrictions when both day-of-month and day-of-week are unrestricted, without adding or omitting cron occurrences.
- Add parse-to-render regression tests for plain daily and month-restricted daily cron expressions.
- Add explicitly gated Windows-native verification that temporarily registers disabled generated XML with `schtasks.exe` and strictly cleans up the disposable task and XML file.
- Leave month-qualified weekday schedules and production temporary-file cleanup to follow-up changes.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `scheduled-automation-tasks`: Clarify the valid Task Scheduler schedule elements and faithful month handling required when both cron day fields are unrestricted.

## Impact

- Affected code: `src/scheduled_task/platform/task_scheduler.rs` and its tests.
- Affected system: Windows Task Scheduler registration through `schtasks.exe`.
- CI: Windows coverage for native Task Scheduler XML acceptance.
- APIs and persisted data: no public API, schema, or desired-state changes.
- Operations: previously degraded desired schedules require explicit reconciliation after upgrading; inspection remains read-only.
