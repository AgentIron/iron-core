## Context

`render_task_xml` maps each expanded `(hour, minute)` pair to a Task Scheduler `CalendarTrigger`. Day-of-month restrictions use `ScheduleByMonth`, day-of-week restrictions use `ScheduleByWeek`, and combined restrictions use both triggers to preserve cron OR semantics. When neither day field is restricted, the renderer currently supplies no schedule body, but the Task Scheduler schema requires every `CalendarTrigger` to contain one schedule element.

The unrestricted-day path has two forms. A cron with unrestricted months is a true daily schedule. A cron with restricted months must still run every valid day in only those months. `ScheduleByDay` represents the former but cannot carry a month filter; `ScheduleByMonth` represents the latter when populated with every day-of-month and the selected months.

The renderer has a separate pre-existing problem for month-qualified weekday expressions: `ScheduleByWeek` does not permit a `Months` child in the Task Scheduler schema. Correctly representing that case requires `ScheduleByMonthDayOfWeek` or a typed unsupported-schedule result. That case is related to native XML validity but is not required to resolve issue #104's unrestricted-day failure.

## Goals / Non-Goals

**Goals:**

- Emit a schema-valid schedule body for every `CalendarTrigger` generated when both cron day fields are unrestricted.
- Preserve the occurrence set for unrestricted daily and month-restricted daily crons.
- Catch both semantic rendering regressions and XML rejected by the native Windows parser.

**Non-Goals:**

- Change the supported cron grammar or trigger expansion limit.
- Change day-of-month/day-of-week OR semantics.
- Change task ownership, invocation, persistence, or reconciliation behavior.
- Introduce an XML schema dependency on non-Windows platforms.
- Repair or redesign month-qualified weekday schedules.
- Clean up temporary XML files created by the production installation path.

## Decisions

### Use `ScheduleByDay` for unrestricted daily schedules

When day-of-month, day-of-week, and month are all unrestricted, render `ScheduleByDay` with `DaysInterval` set to `1`. This is the direct Task Scheduler representation of a daily cron and supplies the schedule element required by the schema.

The alternative of enumerating all days and months in `ScheduleByMonth` is more verbose and obscures the schedule's daily intent.

### Use `ScheduleByMonth` for month-restricted daily schedules

When both day fields are unrestricted but months are restricted, render `ScheduleByMonth` with days `1` through `31` and the selected months. Task Scheduler naturally skips nonexistent calendar dates, so this produces every real day in each selected month without occurrences in other months.

Using `ScheduleByDay` would drop the cron month constraint. Splitting the cron into many triggers would increase XML size without improving fidelity.

### Test structure and native acceptance separately

Unit tests will parse and expand the exact cron expressions `0 3 * * *` and `0 3 * 6 *` before rendering, then assert the resulting schedule-body shape. Starting from `CronExpression` avoids relying on manually constructed empty vectors that do not match production wildcard expansion.

A Windows-only native integration test will be inert unless `AGENTIRON_RUN_NATIVE_SCHEDULER_TESTS=1` enables scheduler mutation. The Windows CI job will set that variable and run the test. The test will write the same UTF-16LE representation used by production, register a disabled task with a collision-resistant name in the Task Scheduler root, and delete both the registered task and temporary XML before reporting success. Using the root isolates XML acceptance from the separate question of whether the AgentIron task folder exists on a fresh machine.

Native registration is preferred over adding an XML schema validator because `schtasks.exe` is the actual compatibility boundary and is already available in the Windows CI job. Registration proves that Windows accepts the XML and its encoding; deterministic structural tests remain responsible for schedule fidelity. Cleanup failure is a test failure, while cleanup is still attempted after registration or assertion failure.

## Risks / Trade-offs

- [Task cleanup can be interrupted by runner termination] -> Use a collision-resistant test prefix, register the task disabled, use a cleanup guard during normal return and unwinding, and make cleanup failure visible when it can be reported.
- [Native tests mutate the host scheduler] -> Gate the test behind an explicit environment variable set only by the Windows CI job or an intentional developer invocation.
- [Native tests can expose Windows environment variance] -> Register in the Task Scheduler root, keep deterministic unit tests as the semantic coverage, and limit native verification to XML registration and deletion.
- [Enumerating day 31 for short months appears broader than real dates] -> Task Scheduler cannot fire on nonexistent dates, so the resulting occurrence set remains faithful to cron wildcard day semantics.

## Migration Plan

No stored data migration is required. Desired schedules that previously failed installation remain in ConfigStore. After upgrading, callers must explicitly reconcile those schedules to install the corrected native task; read-only inspection will continue to report them as degraded until reconciliation succeeds.

Rollback requires no data conversion. A rollback can leave a reconciled daily task installed, but a later reconciliation through the older renderer may fail to replace it with invalid XML.
