## Context

`iron-core` currently persists typed stored prompts and agent profiles, runtime provider/model settings, provider profiles and credentials, MCP server definitions, and skill settings. Stored prompts may select a profile and are currently invoked through delegated child execution with an interactive parent. There is no durable automation-task record, root-oriented one-shot runner, or binary that reconstructs this state for unattended execution.

Approval behavior is a critical boundary. `AutoApprove` bypasses per-tool permission requests for every tool visible after the profile filter is applied, while `Inherit` can expand as the runtime inventory changes. The no-op client channel is not a safety mechanism because it can answer permission requests. Headless execution must therefore validate policy before any model or tool work.

WASM plugin inventory and auth state are runtime-local and are not represented in `RuntimeSettingsSnapshot` or ConfigStore. MCP servers and skill settings are persisted. The CLI can deterministically reconstruct the latter but cannot reconstruct plugins without broadening this change into plugin installation persistence.

## Goals / Non-Goals

**Goals:**

- Introduce a small durable automation identity suitable for GUI management and later scheduling.
- Run an existing automation task non-interactively as a root session.
- Reconstruct core-owned runtime configuration deterministically and fail closed on missing or interactive dependencies.
- Make workspace, timeout, output, cancellation, and exit semantics suitable for cron and other automation callers.
- Share task-resolution and prompt-composition logic between root and delegated execution where practical.

**Non-Goals:**

- CLI CRUD or interactive setup.
- Scheduling, overlap prevention, persisted run history, retries, or semantic outcome evaluation.
- Task-level profile/model/tool overrides, arbitrary prompts, or parameter substitution.
- Adding web search or any other integration.
- Persisting, discovering, installing, or loading WASM plugins.
- Changing the security behavior of built-in tools, network policy, credentials, MCP servers, or skills.

## Decisions

### Model automation as a reference, not a configuration aggregate

`AutomationTask` contains `id`, `name`, `stored_prompt_id`, `expected_outcome`, `created_at`, and `updated_at`. The name exists for GUI display. Instructions, skills, and the optional profile remain on `StoredPrompt`; provider/model, tool filters, and approval remain on `AgentProfile`.

This avoids duplicated policy and lets multiple tasks reuse one prompt with distinct expected outcomes. A task-level profile override was rejected because it creates competing ownership for security-sensitive configuration.

### Use a first-class relational task table

ConfigStore will add a dedicated schema-versioned automation-task table and typed `set/get/list/delete_automation_task` APIs. Task writes require the referenced prompt to exist. Prompt deletion is blocked while referenced, returns the referencing task IDs, and never cascades.

Encoding tasks as prompt or opaque schedule payloads was rejected because tasks have their own lifecycle and will become the target of future schedules. Allowing dangling references was rejected because it postpones a predictable configuration error until unattended execution.

### Resolve live dependencies once per run

At run start, a resolver loads the current task, prompt, profile, requested skills, effective tools, provider, model, and workspace into an immutable execution input. The expected outcome is appended to the model-visible user goal; profile identity remains in system prompt layers. Concurrent configuration edits affect only later runs.

The resolver should be reusable by root CLI execution and delegated prompt invocation, while the execution mechanism remains distinct: CLI runs are root sessions and delegated calls remain child sessions.

### Make the CLI run-only

The command surface is:

```text
agent-iron run <task-id> \
  [--config <path>] \
  [--workspace <path>] \
  --timeout <duration> \
  [--format text|json] \
  [--quiet]
```

Supported options may use corresponding `AGENTIRON_*` environment variables. Command line wins over environment. The task remains positional so cron definitions state exactly what they run. Configuration editing belongs in the GUI and typed core APIs, not the initial CLI.

Workspace resolves from CLI, environment, then process current directory and must canonicalize to an existing directory. Timeout must resolve from CLI or environment and applies to the complete execution. There is deliberately no unbounded default.

### Bootstrap only core-owned persisted configuration

Startup opens and migrates ConfigStore, loads runtime settings and effective provider profiles, resolves credentials, registers built-ins, registers enabled MCP definitions, discovers skills under persisted trust settings and the resolved workspace, then loads profiles, prompts, and the task. Existing built-in network and filesystem protections remain in force.

WASM plugins are excluded because no core-owned persisted inventory currently exists. The CLI does not scan directories or opportunistically install artifacts. A profile that explicitly allow-lists an absent plugin tool fails unavailable-tool preflight. Plugin support can be added after a separate change defines durable installation records and deterministic bootstrap.

### Give RuntimeDefault a persisted headless meaning

For the CLI, `RuntimeDefault` resolves the saved ConfigStore default provider/model through the effective provider profile and credential store. Missing, disabled, invalid, or unauthenticated defaults fail without selecting another provider or model.

Injecting a dummy provider and treating it as the default was rejected because it makes unattended behavior dependent on construction details rather than saved configuration.

### Treat profile approval and tools as the security boundary

Headless preflight requires the resolved profile to be explicitly `AutoApprove`; `PerTool` fails and is never upgraded. `Allow`, `Deny`, and `Inherit` retain their existing meanings. `Inherit` explicitly trusts every tool in the current reconstructed inventory, while `Allow` provides a constrained option. Explicitly allow-listed but unavailable tools fail preflight.

An omitted prompt profile follows existing default-profile resolution. Because the built-in default is interactive, it ordinarily fails headless preflight. This makes unattended privilege an intentional profile decision rather than an implicit CLI convenience.

Interactive provider or integration authentication also fails. Existing credentials and supported automatic refresh are allowed, but the CLI never waits for browser, device-code, permission, or other client-mediated input.

### Execute a root session with explicit terminal outcomes

After resolution and preflight, the CLI creates a root session using the resolved execution input. It does not manufacture a parent session to reuse delegated machinery. A generated run ID and terminal status (`completed`, `failed`, `cancelled`, or `timed_out`) form an ephemeral `AutomationRun` result; run persistence is deferred.

Timeout and signal handlers request cancellation. Timeout wins if it has already determined the terminal condition. Runtime completion means technical completion only; expected-outcome text is not independently evaluated.

### Keep stdout automation-safe

Text mode writes only final assistant text to stdout. JSON mode writes exactly one versioned terminal object containing run/task identity, status, output, expected outcome, resolved profile/provider/model/workspace, effective tools, timing, and structured error. Progress and warnings use stderr; quiet suppresses progress but not final or fatal output.

Stable exit codes separate usage (`2`), config/reference (`3`), unsafe policy (`4`), provider/credential initialization (`5`), execution (`6`), cancellation (`7`), and timeout (`8`) from completion (`0`). Recoverable tool errors do not override a technically completed run.

## Risks / Trade-offs

- [An `AutoApprove + Inherit` profile gains newly registered tools] -> Document this as explicit trust in the current inventory, report effective tools in JSON, and support `Allow` for constrained profiles.
- [Startup may partially initialize MCP servers before another dependency fails] -> Complete static reference and policy checks as early as possible, cancel initialized resources on failure, and emit a single terminal result.
- [Timeout cancellation may not stop an external operation immediately] -> Mark the terminal result `timed_out`, request cooperative cancellation, bound cleanup where possible, and avoid reporting completion afterward.
- [Local skill contents or MCP executables can change between runs] -> Snapshot the resolved run inventory at start and report identities; artifact pinning is outside this change.
- [Blocking prompt deletion changes an existing API's behavior] -> Return a typed conflict with task IDs so GUI callers can reassign or delete references deliberately.
- [Plugin-backed profiles cannot run through the initial CLI] -> Fail explicit missing-tool requirements and add plugin bootstrap only after plugin persistence has its own contract.

## Migration Plan

1. Add the automation-task migration and typed ConfigStore APIs without creating any task records automatically.
2. Add reference checks to prompt deletion and map database conflicts to typed domain errors.
3. Add shared execution resolution, root execution, and CLI bootstrap behind the new binary target.
4. Add CLI contract and integration tests using temporary ConfigStores and controlled providers/tools.
5. Existing profiles, prompts, schedules, and interactive execution continue unchanged; rollback removes the binary and code paths while leaving the additive task table harmless to older builds.

## Open Questions

None required for implementation. Scheduling, run persistence, plugin installation persistence, and semantic expected-outcome evaluation require separate proposals.
