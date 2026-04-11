# python-tool-namespace Specification

## Purpose
TBD - created by archiving change expose-python-tool-methods. Update Purpose after archive.
## Requirements
### Requirement: Embedded Python exposes the runtime tool catalog
The embedded Python runtime SHALL expose callable tool access for every tool visible in the session-effective runtime tool catalog at the start of a script run, rather than limiting the namespace to tools present in the static global `ToolRegistry`.

#### Scenario: Public and Python-visible session tool views stay aligned
- **WHEN** a tool is visible to embedded Python through the session-effective runtime tool catalog
- **THEN** the public effective-tool inspection for that same session reports the same tool as visible
- **THEN** embedded Python does not depend on a divergent MCP visibility implementation

### Requirement: Tool registry is the only canonical callable catalog
The embedded Python runtime SHALL derive its callable tool surface from the session-effective runtime tool catalog rather than directly from capability metadata or only from the static global `ToolRegistry`.

#### Scenario: Metadata-only capability does not become a Python method
- **WHEN** a capability exists only in capability metadata and is not present in the session-effective runtime tool catalog
- **THEN** the embedded Python environment does not expose it as a callable tool method

#### Scenario: Session-visible tool becomes callable
- **WHEN** a tool is present in the session-effective runtime tool catalog under a tool name
- **THEN** the embedded Python environment exposes that tool name as callable

### Requirement: Python tool access supports identifier-safe aliases and raw-name fallback
The embedded Python runtime SHALL provide ergonomic callable aliases for tool names that can be represented safely in Python, and SHALL provide a raw-name fallback for complete access to the visible tool catalog.

#### Scenario: Python-safe tool name is callable through alias
- **WHEN** a visible tool name can be represented safely as a Python identifier
- **THEN** the embedded Python environment exposes a callable alias for that tool

#### Scenario: Non-identifier tool name remains callable
- **WHEN** a visible tool name cannot be represented safely as a Python identifier
- **THEN** the embedded Python environment still allows the tool to be invoked through a raw-name fallback mechanism

### Requirement: Python tool calls reuse the standard runtime execution path
The embedded Python runtime SHALL execute namespace-exposed tool calls through the existing child-tool execution path.

#### Scenario: Python tool calls inherit validation and permission behavior
- **WHEN** a `python_exec` script invokes a visible tool through the embedded Python namespace
- **THEN** the runtime applies the same schema validation, permission checks, and durable tool-record lifecycle used for other child tool calls

### Requirement: Tool namespace is stable for a single script run
The embedded Python runtime SHALL expose a stable snapshot of the session-effective runtime tool catalog for the lifetime of one script execution.

#### Scenario: Mid-run effective tool change does not alter in-flight namespace
- **WHEN** the session-effective runtime tool catalog changes after a script has already started
- **THEN** the in-flight script continues to use the tool namespace snapshot captured at script start

### Requirement: Child-tool execution reuses the session-effective runtime path
The embedded Python runtime SHALL resolve child-tool validation, approval, and execution through the same session-effective runtime tool path used for ordinary model-issued tool calls.

#### Scenario: MCP child-tool lookup remains unambiguous
- **WHEN** a Python child-tool call targets an MCP-backed tool whose provider-visible name includes a server ID containing underscores or other separator-like characters
- **THEN** the runtime resolves the intended MCP tool using the same unambiguous lookup logic used for ordinary MCP tool execution

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

### Requirement: Unsupported direct OS access yields actionable recovery guidance
The embedded Python runtime SHALL return an actionable sandbox-violation failure when a script attempts unsupported direct OS or filesystem access, and the failure SHALL direct the caller to use visible runtime tools through the `tools` namespace.

#### Scenario: Direct filesystem access is rejected with tool guidance
- **WHEN** a `python_exec` script attempts direct OS or filesystem access that reaches Monty's `OsCall` boundary
- **THEN** the runtime fails the script with a sandbox-violation result
- **AND** the failure message tells the caller to use visible runtime tools through `tools.<alias>(payload)` or `tools.call(name, payload)`

