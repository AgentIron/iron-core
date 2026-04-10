## ADDED Requirements

### Requirement: Unsupported direct OS access yields actionable recovery guidance
The embedded Python runtime SHALL return an actionable sandbox-violation failure when a script attempts unsupported direct OS or filesystem access, and the failure SHALL direct the caller to use visible runtime tools through the `tools` namespace.

#### Scenario: Direct filesystem access is rejected with tool guidance
- **WHEN** a `python_exec` script attempts direct OS or filesystem access that reaches Monty's `OsCall` boundary
- **THEN** the runtime fails the script with a sandbox-violation result
- **AND** the failure message tells the caller to use visible runtime tools through `tools.<alias>(payload)` or `tools.call(name, payload)`

## MODIFIED Requirements

### Requirement: Python tool discovery is documented and inspectable
The embedded Python runtime SHALL document the supported tool namespace, SHALL describe `python_exec` as a sandboxed orchestration environment, and SHALL explain that filesystem, shell, network, and other host interactions from embedded Python MUST use visible runtime tools rather than direct Python OS APIs.

#### Scenario: Runtime guidance describes Python namespace model
- **WHEN** embedded Python is enabled
- **THEN** the runtime guidance explains that Python tool access follows the visible runtime tool catalog rather than only `iron_call(name, args)`

#### Scenario: Runtime guidance describes sandbox boundary
- **WHEN** embedded Python is enabled
- **THEN** the runtime guidance states that direct OS, filesystem, or environment access from Python is unavailable inside `python_exec`
- **AND** the runtime guidance tells the caller to use `tools.<alias>(payload)` or `tools.call(name, payload)` for host interactions

#### Scenario: Tool description reinforces supported host access path
- **WHEN** `python_exec` is visible to the model
- **THEN** its tool description identifies the runtime as sandboxed orchestration rather than a general host Python environment
- **AND** its tool description instructs the model to use the `tools` namespace instead of direct APIs such as `pathlib`, `open`, or `os` for host access
