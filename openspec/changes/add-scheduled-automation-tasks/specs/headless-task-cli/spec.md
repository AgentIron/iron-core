## MODIFIED Requirements

### Requirement: Every headless run SHALL have a validated workspace
Each automation task SHALL provide a persisted project root used as the default headless workspace. The CLI SHALL support `--workspace` and `AGENTIRON_WORKSPACE` as explicit operator overrides, resolving workspace in the order command line, environment, then automation task. Before runtime execution, it SHALL canonicalize the selected path, require it to be an existing directory, and use it as both the session workspace root and the allowed root for workspace-scoped built-in tools. It SHALL NOT use the process working directory as an implicit fallback.

#### Scenario: Explicit workspace is used
- **WHEN** a caller supplies an existing directory through `--workspace`
- **THEN** the CLI canonicalizes that directory
- **AND** uses it as the run's session workspace and built-in-tool allowed root

#### Scenario: Task supplies workspace
- **WHEN** neither `--workspace` nor `AGENTIRON_WORKSPACE` is supplied
- **THEN** the CLI uses the selected automation task's persisted project root

#### Scenario: Workspace is invalid
- **WHEN** the resolved workspace does not exist, is not a directory, or cannot be canonicalized
- **THEN** the CLI returns a configuration error
- **AND** no model request or tool execution begins

### Requirement: Every headless run SHALL have an execution timeout
Each automation task SHALL provide a persisted positive timeout used by default for headless execution. The CLI SHALL support `--timeout` and `AGENTIRON_TIMEOUT` as explicit operator overrides using documented duration values such as `30s`, `5m`, and `1h`, resolving timeout in the order command line, environment, then automation task. Timeout expiration SHALL request cancellation and produce the `timed_out` status even if cleanup continues afterward.

#### Scenario: Timeout is supplied on command line
- **WHEN** a caller supplies a valid positive `--timeout`
- **THEN** the CLI applies it to the complete task execution

#### Scenario: Task supplies timeout
- **WHEN** neither `--timeout` nor `AGENTIRON_TIMEOUT` is supplied
- **THEN** the CLI applies the selected automation task's persisted timeout

#### Scenario: Timeout is invalid
- **WHEN** the resolved timeout is absent, malformed, zero, negative, or exceeds the supported bound
- **THEN** the CLI reports a configuration error
- **AND** does not begin task execution

#### Scenario: Timeout expires
- **WHEN** execution exceeds the resolved timeout
- **THEN** the CLI requests cancellation of the run
- **AND** reports status `timed_out`
- **AND** exits with the timeout exit code
