## 1. Automation Task Domain

- [x] 1.1 Add typed `AutomationTask`, input, ID, validation, schema-version, and error types with unit tests for normalization and invalid fields
- [x] 1.2 Add the ConfigStore migration for the first-class automation-task table, timestamps, deterministic indexing, and stored-prompt foreign-key relationship
- [x] 1.3 Implement typed set, get, list, and delete automation-task APIs with atomic replacement and timestamp semantics
- [x] 1.4 Add ConfigStore tests for CRUD, deterministic ordering, replacement, unsupported or malformed records, and missing-prompt references
- [x] 1.5 Change stored-prompt deletion to return a typed conflict with referencing task IDs and add tests proving neither prompts nor tasks cascade-delete

## 2. Shared Execution Resolution

- [x] 2.1 Define the immutable resolved execution input and ephemeral automation-run result types, including terminal statuses, timing, resolved identities, effective tools, and structured errors
- [x] 2.2 Implement task, stored-prompt, profile, skill, tool, workspace, provider, and model resolution with actionable typed failures
- [x] 2.3 Compose stored instructions and expected outcome as the user goal while preserving profile identity in system prompt layers
- [x] 2.4 Refactor delegated stored-prompt execution to share applicable resolution and prompt-composition code without changing its child-session behavior
- [x] 2.5 Add tests proving dependency snapshots are stable during a run and subsequent runs observe updated records

## 3. Headless Safety And Runtime Bootstrap

- [x] 3.1 Implement headless policy preflight requiring `AutoApprove`, preserving `Inherit`/`Allow`/`Deny`, rejecting interactive approval, and checking explicitly allow-listed tool availability
- [x] 3.2 Implement saved `RuntimeDefault` resolution through the persisted default provider/model, effective provider profile, and credential configuration without fallback
- [x] 3.3 Implement non-interactive credential behavior that permits existing credentials and automatic refresh but fails client-mediated authentication requests
- [x] 3.4 Build runtime bootstrap from ConfigStore settings, built-in protections, persisted MCP definitions, skill settings, profiles, prompts, and automation tasks
- [x] 3.5 Ensure CLI bootstrap neither scans for nor registers or installs WASM plugins and reports explicitly required plugin tools as unavailable
- [x] 3.6 Add bootstrap and preflight tests for valid configuration, missing references, unsafe profiles, unavailable tools, missing defaults, credential failures, MCP/skill loading, and plugin exclusion

## 4. Root Automation Execution

- [x] 4.1 Add a root-session execution entry point that consumes the resolved execution input without creating a synthetic parent session
- [x] 4.2 Apply the canonical workspace to the session and workspace-scoped built-in tool roots while preserving existing network and filesystem restrictions
- [x] 4.3 Implement whole-run timeout and cooperative cancellation with distinct timeout and signal-cancellation terminal outcomes
- [x] 4.4 Add tests for root execution, technical completion semantics, model-recovered tool errors, execution failures, cancellation, and timeout

## 5. agent-iron Binary And Contracts

- [x] 5.1 Add the `agent-iron` binary target and minimal CLI parsing dependency or module consistent with repository conventions
- [x] 5.2 Implement `run <task-id>` parsing for config, workspace, required timeout, text/JSON format, and quiet options with CLI-over-environment precedence
- [x] 5.3 Validate and canonicalize workspace selection from CLI, environment, or process current directory and validate positive duration syntax
- [x] 5.4 Implement text stdout, diagnostic stderr, and quiet-mode behavior
- [x] 5.5 Implement the single versioned terminal JSON object and stable exit-code mapping for all post-parse outcomes
- [x] 5.6 Handle supported interrupt and termination signals by cancelling the active root run and preserving timeout precedence

## 6. End-To-End Verification

- [x] 6.1 Add binary integration fixtures that create temporary ConfigStores with controlled profiles, prompts, tasks, providers, credentials, MCP definitions, and skills
- [x] 6.2 Add command-level tests for usage failures, precedence, invalid workspace and timeout, successful text output, quiet mode, and each stable exit category
- [x] 6.3 Add JSON contract tests asserting exactly one stdout object, required versioned fields, clean stdout, and structured failure results
- [x] 6.4 Add end-to-end tests proving task runs use saved default provider/model, execute as root sessions, enforce AutoApprove preflight, and exclude runtime-local plugins
- [x] 6.5 Run `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets --all-features -- -D warnings`, resolving any regressions introduced by the change
