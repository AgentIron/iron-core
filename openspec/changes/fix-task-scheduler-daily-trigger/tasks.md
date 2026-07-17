## 1. Task Scheduler Rendering

- [x] 1.1 Add a `ScheduleByDay` renderer and use a one-day interval for triggers whose day and month fields are unrestricted
- [x] 1.2 Render month-restricted daily triggers as `ScheduleByMonth` with days 1 through 31 and only the selected months
- [x] 1.3 Ensure both unrestricted-day construction paths supply exactly one non-empty schedule body without changing weekday-restricted rendering
- [x] 1.4 Replace non-schema `Settings` elements (`AllowStartIfOnBatteries`, `DontStopIfGoingOnBatteries`) with schema-valid equivalents (`DisallowStartIfOnBatteries`, `StopIfGoingOnBatteries`) in documented order

## 2. Regression Coverage

- [x] 2.1 Add a parse-to-render test for `0 3 * * *` that requires `ScheduleByDay` with `DaysInterval` 1
- [x] 2.2 Add a parse-to-render test for `0 3 * 6 *` that verifies days 1 through 31 and only June are represented by `ScheduleByMonth`
- [x] 2.3 Add a Windows-native test gated by `AGENTIRON_RUN_NATIVE_SCHEDULER_TESTS=1` that registers generated XML as a uniquely named disabled root task and uses a cleanup guard for the task and temporary XML
- [x] 2.4 Make native-test registration or cleanup failures fail with enough command output to diagnose the failed phase

## 3. Verification

- [x] 3.1 Run formatting and the Task Scheduler adapter unit tests
- [x] 3.2 Run clippy with all targets and features, treating warnings as errors
- [x] 3.3 Update the Windows CI scheduler-adapter job to set `AGENTIRON_RUN_NATIVE_SCHEDULER_TESTS=1` and execute the native registration test while ordinary local test runs remain inert
- [ ] 3.4 Confirm Windows CI passes both exact-cron structural tests and native XML registration with no disposable task or XML left behind
