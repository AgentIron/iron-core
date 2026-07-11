## Why

AgentIron needs a deterministic, non-interactive execution path for durable automation such as cron-driven reports. The current runtime can execute stored prompts interactively or as delegated child work, but it has no first-class automation-task identity or CLI that can safely reconstruct persisted runtime configuration and run one as a root session.

## What Changes

- Add a durable `AutomationTask` that names a stored prompt and records the expected outcome for a reusable, eventually schedulable automation.
- Add typed core CRUD for automation tasks, including validation and referential integrity with stored prompts.
- Block deletion of a stored prompt while automation tasks reference it; do not cascade-delete tasks.
- Add an `agent-iron run <task-id>` binary for non-interactive root-session execution.
- Require a workspace and timeout for every headless run, with command-line, environment, and process-working-directory resolution where applicable.
- Resolve profile, approval policy, tools, skills, MCP servers, provider, model, and credentials from persisted core configuration and snapshot the resolved execution inputs at run start.
- Require an explicitly `AutoApprove` profile for headless execution and fail closed for interactive approval policies, unresolved references, unavailable allow-listed tools, missing credentials, or interactive authentication requirements.
- Define stable text and versioned JSON output, run statuses, cancellation behavior, and process exit codes for automation consumers.
- Exclude task mutation commands, scheduling, persisted run history, arbitrary prompt/context overrides, web-search functionality, and WASM plugin discovery or bootstrap from this change.

## Capabilities

### New Capabilities

- `automation-tasks`: Durable automation-task identity, typed persistence, validation, referential integrity, and execution-input resolution.
- `headless-task-cli`: Non-interactive CLI startup, safety preflight, root execution, timeout and cancellation, output contracts, and exit behavior.

### Modified Capabilities

- `stored-prompts`: Prevent deletion of a stored prompt that is referenced by one or more automation tasks.

## Impact

- Adds an `agent-iron` binary target and CLI parsing/runtime bootstrap dependencies.
- Extends `ConfigStore` with an automation-task migration, typed records, CRUD, and reference-conflict errors.
- Refactors prompt execution so root CLI runs and delegated stored-prompt runs can share resolved execution inputs without manufacturing a parent session.
- Uses persisted provider/model settings, provider profiles, credential storage, MCP server settings, skill settings, agent profiles, and stored prompts during CLI bootstrap.
- Establishes automation-facing output and exit-code APIs that future schedulers and run-history persistence can consume.
- Does not persist or load WASM plugin inventory because plugin installation state is currently runtime-local rather than core-owned configuration.
