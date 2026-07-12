## Context

The repository already has durable `AutomationTask` records, stored-prompt references, immutable execution resolution, headless policy checks, and `agent-iron run <task-id>`. ConfigStore also has an opaque `schedule` table, but there is no typed schedule model or host integration.

Issue 64 originally proposed scheduling saved prompts directly. That would bypass the automation-task aggregate and conflict with the shipped headless CLI. This design instead schedules existing automation tasks. A UI may combine task creation and scheduling for convenience, but core performs and reports them as distinct operations.

Host schedulers are external mutable systems with different recurrence semantics. ConfigStore and a host scheduler cannot participate in one transaction, so the API must distinguish desired state, observed state, read-only inspection, and mutating reconciliation.

## Goals / Non-Goals

**Goals:**

- Persist typed schedules that reference existing automation tasks.
- Prevent arbitrary commands, prompts, executable paths, arguments, and environment payloads from entering scheduled-task definitions.
- Compile numeric five-field cron expressions faithfully on Linux, macOS, and Windows or fail actionably.
- Preserve non-AgentIron host entries and identify AgentIron entries through platform-specific ownership markers.
- Provide UI-safe status reports that combine ConfigStore, automation-task validity, headless preflight, and host state.
- Make automation tasks deterministic when invoked without an interactive caller by storing project root and timeout on the task.
- Keep platform code testable through injected command and filesystem boundaries.

**Non-Goals:**

- General-purpose cron, launchd, or Task Scheduler management.
- Direct scheduling of stored prompts.
- Arbitrary command scheduling or user-controlled command templates.
- Cross-platform emulation of schedules a host cannot represent exactly.
- Timezone selection or normalization of daylight-saving behavior.
- Durable run history, output retention, missed-run recovery, or normalized last/next-run reporting.
- Windows COM integration in the first implementation.
- Scheduler UI implementation.

## Decisions

### Scheduled tasks reference automation tasks

`ScheduledTask` contains a stable schedule ID, an `automation_task_id`, a validated cron expression, enabled state, and store-maintained timestamps. It does not duplicate prompt, profile, workspace, timeout, provider, model, tool, or approval settings.

Automation tasks have three identity concerns: an immutable internal ID, a unique case-insensitive normalized name used for convenient lookup, and mutable display text. The referenced automation-task ID is authoritative for schedules and host ownership. Renaming does not rewrite schedule references. Normalized-name collisions are rejected rather than silently suffixed.

Alternative considered: reference stored prompts directly. Rejected because stored prompts are reusable instruction definitions rather than complete unattended executions, and the existing CLI already executes automation tasks.

### Automation tasks own project root and timeout

An automation task gains a canonical project root and positive execution timeout. These values are resolved with the task so manual and scheduled executions mean the same thing. CLI workspace and timeout options remain explicit operator overrides; when omitted, the task values are used.

Alternative considered: persist workspace and timeout on each schedule. Rejected because two schedules for one task would execute the nominally same task with different hidden semantics.

### ConfigStore is desired state

Typed schedule records in ConfigStore are the desired state. Host entries are observed state. `inspect` compares both without mutation. `reconcile` attempts to move owned host state toward desired state and returns the resulting report.

Saving desired state and reconciling are not presented as one atomic operation. A host failure leaves the schedule persisted with an actionable desired-but-not-installed or drifted status that can be retried.

Orphaned owned host entries are reported during inspection. They are removed only by an explicit reconciliation policy or removal operation, never as a side effect of status reads.

### Separate application and host layers

The public shape is two-layered:

```text
ScheduleManager
  - typed ConfigStore CRUD
  - automation-task and headless preflight validation
  - inspect and reconcile
  - status synthesis

HostScheduler
  - install or replace one owned entry
  - disable or enable one owned entry
  - remove one owned entry
  - list and inspect all owned entries
  - report platform capability failures
```

`HostScheduler` receives a compiled, core-owned invocation rather than arbitrary caller input. A factory selects the host implementation for the compilation target and returns an unsupported-platform error elsewhere.

### Installation context is manager-owned

`ScheduleManager` is constructed with trusted installation context containing an absolute `agent-iron` runner path and ConfigStore path. The desktop application or embedding process supplies the companion runner location; core does not assume `current_exe()` is the runner and host entries do not rely on `PATH`.

The installed action is derived exclusively by core from installation context and the referenced immutable automation-task ID. Task payloads cannot override the executable, config path, arguments, working directory, shell, or environment.

Executable relocation and upgrade repair are deferred, but drift inspection exposes a stale generated invocation so reconciliation can replace it when the application supplies updated installation context.

### Cron has language validity and platform validity

Core parses exactly five numeric fields: minute, hour, day of month, month, and day of week. Fields support wildcard, numeric value, numeric range, numeric list, and steps over wildcards or ranges. Seconds, macros, names, and non-cron schedule kinds are rejected.

Parsing establishes language validity. Each adapter separately establishes whether it can represent the parsed expression faithfully. Unsupported combinations return a typed platform-capability error; adapters never broaden, narrow, or approximate occurrence semantics.

Schedules use the host scheduler's local system timezone and native daylight-saving behavior. No timezone is persisted.

### Platform ownership and rendering

- Linux uses marker-delimited blocks in the user crontab. Enabled blocks contain generated cron lines. Disabled blocks retain the same generated lines as comments. Rewrites preserve all text outside valid AgentIron-owned blocks byte-for-byte where practical. Malformed and duplicate owned blocks are reported rather than guessed at.
- macOS uses user LaunchAgent plists with labels prefixed by `com.agentiron.task.` in the user LaunchAgents directory. Expressions may expand to multiple calendar intervals only when expansion is finite and semantically faithful.
- Windows uses tasks under `\AgentIron\Tasks\<id>`. Core generates Task Scheduler XML and invokes `schtasks.exe` to create, query, enable, disable, and delete definitions. XML allows one owned task to contain multiple triggers without adding a COM dependency.

Ownership identifies the namespace core may mutate; it is not a security assertion because a local user can forge markers or files.

### Disabled definitions remain installed

Disabling retains an owned host entry. Cron represents it as a commented marker-delimited block; launchd and Windows use host disabled state. This preserves discoverability and allows disabled desired state to reconcile as healthy.

### Status is compositional

A report contains an overall health summary plus independent desired, reference, execution, and host states and zero or more diagnostics. This permits simultaneous findings, such as an invalid automation-task reference and a corrupt owned host entry.

Optional host metadata may include last run time, next run time, or result when cheaply available. Missing metadata is not an error and fields remain optional.

## Risks / Trade-offs

- [Cron semantics differ between platforms] -> Require faithful adapter compilation and reject unsupported expressions with platform-specific diagnostics.
- [ConfigStore and host state diverge after partial failure] -> Persist desired state, expose drift, and make reconciliation explicitly retryable.
- [Crontab rewriting damages unrelated entries] -> Mutate only well-formed marker-delimited owned blocks, preserve outside content, and test round trips with mixed fixtures.
- [Runner paths become stale after upgrades] -> Include generated invocation in drift comparison and defer automated relocation repair to a follow-up issue.
- [Scheduled processes cannot access desktop-session credentials] -> Run normal headless preflight where possible and surface initialization failures; document credential-session limitations.
- [Trigger expansion becomes excessive] -> Bound expansion and reject expressions that exceed the platform adapter's safe trigger limit.
- [Local timezone or DST produces surprising runs] -> Document that native host behavior is authoritative and do not claim identical edge behavior across platforms.
- [Task schema evolution breaks existing records] -> Add a compiled migration with explicit defaults or migration validation and preserve old records transactionally.
- [Concurrent reconciliation races] -> Make host replacement idempotent, use stable ownership identities, and return observed post-operation state.

## Migration Plan

1. Add project-root and timeout fields to automation-task persistence with a compiled ConfigStore migration. Existing tasks that cannot receive safe deterministic values remain readable for management but report incomplete execution configuration until updated.
2. Add typed scheduled-task payload decoding over the existing opaque schedule table. Unsupported or malformed existing payloads remain stored and surface diagnostics rather than being silently rewritten.
3. Add the fake host adapter and manager reconciliation tests before enabling concrete platform factories.
4. Add platform adapters independently, with unsupported targets returning typed errors.
5. Existing headless CLI invocations with explicit workspace and timeout continue to work. New fallback behavior applies only when those options are absent.

Rollback removes host entries through the version that created them before reverting binaries. ConfigStore migrations are forward-only; persisted schedule records remain opaque to older versions, and automation-task schema rollback requires restoring a pre-migration database backup.

## Open Questions

- What bounded trigger-expansion limit should launchd and Windows adapters enforce?
- Should reconciliation of all tasks remove orphaned owned entries by default, or require an explicit `remove_orphans` policy flag?
- What conservative concurrent-run policy can all three host schedulers represent faithfully? This may need a focused follow-up before platform installation is enabled.
- Which follow-up issue should own durable scheduled-run history, output retention, and normalized last/next-run reporting?
