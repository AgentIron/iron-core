# headless-task-cli Specification

## Purpose
Define deterministic, non-interactive execution of durable automation tasks through the `agent-iron` CLI.

## Requirements
### Requirement: The agent-iron CLI SHALL expose run-only task execution
The `agent-iron` binary SHALL provide `agent-iron run <task-id>` for non-interactive execution of an existing automation task. It SHALL NOT provide task, prompt, profile, credential, integration, or schedule mutation commands in this change, and SHALL NOT accept an arbitrary prompt, task-level profile override, or arbitrary context variables.

#### Scenario: Existing task is selected by ID
- **WHEN** a caller invokes `agent-iron run <task-id>` with valid required options
- **THEN** the CLI loads and executes that existing automation task

#### Scenario: Task ID is omitted
- **WHEN** a caller invokes the run command without a task ID
- **THEN** the CLI reports a usage error
- **AND** does not start runtime bootstrap

#### Scenario: Mutation is attempted
- **WHEN** a caller requests an unsupported creation, editing, or deletion command
- **THEN** the CLI reports a usage error
- **AND** does not mutate persisted configuration

### Requirement: Headless configuration SHALL use explicit deterministic precedence
The CLI SHALL resolve supported startup settings with command-line values taking precedence over corresponding `AGENTIRON_*` environment variables, which take precedence over documented defaults. The config location SHALL be selectable by `--config` or its environment equivalent. The task ID SHALL remain a required positional argument and SHALL NOT be sourced from an environment variable.

#### Scenario: Command-line setting overrides environment
- **WHEN** both a supported command-line option and its environment equivalent are present
- **THEN** the command-line value is used

#### Scenario: Environment supplies an omitted optional setting
- **WHEN** a supported command-line option is absent and its environment equivalent is valid
- **THEN** the environment value is used

#### Scenario: Environment does not select task
- **WHEN** an environment variable names a task but the positional task ID is absent
- **THEN** the CLI reports a usage error

### Requirement: Every headless run SHALL have a validated workspace
The CLI SHALL support `--workspace` and `AGENTIRON_WORKSPACE`, resolving workspace in the order command line, environment, then process working directory. Before runtime execution, it SHALL canonicalize the selected path, require it to be an existing directory, and use it as both the session workspace root and the allowed root for workspace-scoped built-in tools.

#### Scenario: Explicit workspace is used
- **WHEN** a caller supplies an existing directory through `--workspace`
- **THEN** the CLI canonicalizes that directory
- **AND** uses it as the run's session workspace and built-in-tool allowed root

#### Scenario: Process directory supplies workspace
- **WHEN** neither `--workspace` nor `AGENTIRON_WORKSPACE` is supplied
- **THEN** the CLI resolves the process working directory as the workspace

#### Scenario: Workspace is invalid
- **WHEN** the resolved workspace does not exist, is not a directory, or cannot be canonicalized
- **THEN** the CLI returns a configuration error
- **AND** no model request or tool execution begins

### Requirement: Every headless run SHALL have an execution timeout
The CLI SHALL support `--timeout` and `AGENTIRON_TIMEOUT` using documented duration values such as `30s`, `5m`, and `1h`. A run SHALL NOT start unless a valid positive timeout resolves from the command line or environment. Timeout expiration SHALL request cancellation and produce the `timed_out` status even if cleanup continues afterward.

#### Scenario: Timeout is supplied on command line
- **WHEN** a caller supplies a valid positive `--timeout`
- **THEN** the CLI applies it to the complete task execution

#### Scenario: Timeout is absent
- **WHEN** neither `--timeout` nor `AGENTIRON_TIMEOUT` is supplied
- **THEN** the CLI reports a usage or configuration error
- **AND** does not begin task execution

#### Scenario: Timeout expires
- **WHEN** execution exceeds the resolved timeout
- **THEN** the CLI requests cancellation of the run
- **AND** reports status `timed_out`
- **AND** exits with the timeout exit code

### Requirement: Headless bootstrap SHALL reconstruct core-owned runtime configuration
The CLI SHALL open and migrate ConfigStore, load persisted runtime settings, provider profiles, credentials, MCP server definitions, skill settings, agent profiles, stored prompts, and the selected automation task, then construct the runtime and root session from those values. It SHALL preserve existing built-in-tool and network protections. It SHALL NOT discover, install, register, or load WASM plugins because plugin inventory is not core-persisted in this change.

#### Scenario: Persisted integrations are reconstructed
- **WHEN** the selected task resolves successfully
- **THEN** enabled persisted MCP servers and configured skill sources are made available according to their saved settings and workspace trust policy
- **AND** core built-in tools are configured with the resolved workspace and existing safety settings

#### Scenario: WASM plugins are not bootstrapped
- **WHEN** runtime-local plugin artifacts or plugin directories are present
- **THEN** the CLI does not scan, register, install, or load them
- **AND** plugin tools are absent from the effective headless tool inventory

#### Scenario: Required plugin tool is unavailable
- **WHEN** the selected profile explicitly allow-lists a plugin tool that is not available in the reconstructed runtime
- **THEN** headless preflight fails with an actionable unavailable-tool error

### Requirement: RuntimeDefault SHALL resolve the saved default provider and model
For headless execution, a profile using `RuntimeDefault` SHALL resolve through ConfigStore's saved default provider/model selection, effective provider profile, and stored credential configuration. The CLI SHALL fail closed if that chain is missing, disabled, invalid, or unavailable and SHALL NOT substitute a different provider or model.

#### Scenario: Saved default resolves
- **WHEN** a task's effective profile uses `RuntimeDefault`
- **AND** ConfigStore contains a valid enabled default provider/model with usable credentials
- **THEN** the CLI uses that exact provider and model

#### Scenario: Saved default is unavailable
- **WHEN** the saved default provider/model or required credential cannot be resolved
- **THEN** bootstrap fails with an actionable provider or credential error
- **AND** no fallback provider or model is selected

### Requirement: Headless execution SHALL fail closed on interactive policy
Before starting a model request, the CLI SHALL require the effective agent profile to use `AutoApprove`. It SHALL accept `Inherit`, `Allow`, or `Deny` tool filters without changing them, SHALL reject `PerTool`, and SHALL never upgrade an approval policy automatically. An omitted stored-prompt profile SHALL resolve through the existing default-profile behavior and SHALL fail preflight if the resulting profile is not explicitly headless-safe.

#### Scenario: AutoApprove profile passes policy preflight
- **WHEN** the effective profile uses `AutoApprove`
- **AND** all explicitly allow-listed tools are available
- **THEN** policy preflight permits execution with the profile's effective tool inventory

#### Scenario: Inherit trusts current inventory
- **WHEN** an `AutoApprove` profile uses the `Inherit` tool filter
- **THEN** every tool in the reconstructed session-effective runtime inventory is eligible to execute without interactive approval

#### Scenario: Interactive approval is rejected
- **WHEN** the effective profile uses `PerTool` or otherwise requires client interaction
- **THEN** policy preflight fails with an unsafe-policy error
- **AND** no policy is silently changed

#### Scenario: Default profile is not headless-safe
- **WHEN** a stored prompt omits its profile
- **AND** the resolved default profile requires interactive approval
- **THEN** policy preflight fails before model or tool execution

### Requirement: Headless authentication SHALL not wait for user interaction
The CLI SHALL use already configured credentials and MAY perform supported automatic token refresh. If provider, MCP, or other capability initialization requires interactive login, browser authorization, a device code response, or another client-mediated auth interaction, the CLI SHALL fail actionably rather than wait for input.

#### Scenario: Existing credential is usable
- **WHEN** a required provider or integration credential is already usable or can refresh automatically
- **THEN** headless bootstrap proceeds without prompting

#### Scenario: Interactive authentication is required
- **WHEN** a required capability requests interactive authentication
- **THEN** the CLI reports a credential or initialization failure
- **AND** does not wait for terminal input or an absent client

### Requirement: Headless tasks SHALL execute as root sessions
The CLI SHALL create and execute a root session directly from the resolved automation execution input. It SHALL NOT manufacture an interactive parent session or route root execution through delegated child-session APIs. Root and delegated stored-prompt execution SHALL share resolution and prompt-composition machinery where their semantics overlap.

#### Scenario: CLI task starts root execution
- **WHEN** bootstrap and preflight succeed
- **THEN** the CLI creates a root session using the resolved profile, provider, model, tools, skills, workspace, instructions, and expected outcome
- **AND** execution does not require a parent client or parent session

### Requirement: Headless execution SHALL support signal cancellation
The CLI SHALL handle supported interrupt and termination signals by requesting runtime cancellation and reporting `cancelled` unless timeout had already determined the terminal status.

#### Scenario: Interrupt requests cancellation
- **WHEN** the process receives a supported interrupt or termination signal during execution
- **THEN** it requests cancellation of the active run
- **AND** reports status `cancelled`
- **AND** exits with the cancellation exit code

### Requirement: Headless output SHALL separate machine data from diagnostics
The CLI SHALL support `text` and `json` formats and a quiet mode. Standard output SHALL contain only the selected final result contract. Progress, warnings, and human-readable diagnostics SHALL use standard error. Quiet mode SHALL suppress progress but SHALL NOT suppress the final result or fatal errors.

#### Scenario: Text run completes
- **WHEN** a run completes in text format
- **THEN** standard output contains only the final assistant text
- **AND** progress or warnings, if emitted, use standard error

#### Scenario: Quiet mode completes
- **WHEN** quiet mode is enabled
- **THEN** progress output is suppressed
- **AND** final output and fatal errors remain observable

### Requirement: JSON output SHALL be one versioned terminal object
For JSON format, the CLI SHALL emit exactly one JSON object on standard output whenever JSON mode is requested through arguments or environment, including usage errors detected before argument parsing completes. The object SHALL include a schema version, run ID, task ID and name, status, output, expected outcome, resolved profile/provider/model/workspace, effective tool names, start/end timing, duration, and a structured nullable error. Terminal statuses SHALL be `completed`, `failed`, `cancelled`, or `timed_out`.

#### Scenario: JSON run completes
- **WHEN** execution completes in JSON format
- **THEN** standard output contains exactly one valid terminal JSON object
- **AND** its status is `completed`
- **AND** its error is null

#### Scenario: Post-parse JSON run fails
- **WHEN** configuration, policy, initialization, execution, cancellation, or timeout fails after JSON mode is established
- **THEN** standard output still contains exactly one valid terminal JSON object
- **AND** its non-completed status and structured error describe the failure
- **AND** the process exits nonzero

#### Scenario: Pre-parse usage error honors JSON
- **WHEN** a caller requests JSON format through arguments or environment but supplies arguments that fail parsing (missing task ID, unknown option, missing value, or missing subcommand)
- **THEN** standard output still contains exactly one valid terminal JSON object describing the usage error
- **AND** the process exits with the usage exit code

### Requirement: Headless exit codes SHALL be stable
The CLI SHALL use exit code `0` for `completed`, `2` for usage errors, `3` for configuration or reference errors, `4` for unsafe policy, `5` for provider or credential initialization, `6` for execution failure, `7` for cancellation, and `8` for timeout. A tool error that the model handles successfully SHALL NOT by itself force a nonzero exit.

#### Scenario: Completed run exits successfully
- **WHEN** the terminal run status is `completed`
- **THEN** the process exits with code `0`

#### Scenario: Failure category maps to stable code
- **WHEN** the run terminates in a defined non-completed category
- **THEN** the process exits with that category's documented nonzero code

#### Scenario: Model recovers from tool error
- **WHEN** a tool invocation fails but the model handles the error and runtime execution completes
- **THEN** the run may report `completed`
- **AND** the tool error alone does not force exit code `6`
