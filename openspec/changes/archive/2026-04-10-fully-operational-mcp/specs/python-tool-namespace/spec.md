## MODIFIED Requirements

### Requirement: Embedded Python exposes the runtime tool catalog
The embedded Python runtime SHALL expose callable tool access for every tool visible in the session-effective runtime tool catalog at the start of a script run, rather than limiting the namespace to tools present in the static global `ToolRegistry`.

#### Scenario: Built-in tool appears in Python namespace
- **WHEN** a built-in tool is visible in the session-effective runtime tool catalog and a `python_exec` script starts
- **THEN** the embedded Python environment exposes a callable entry for that tool

#### Scenario: Enabled MCP tool appears in Python namespace
- **WHEN** an MCP-backed tool is visible in the session-effective runtime tool catalog for the current session and a `python_exec` script starts
- **THEN** the embedded Python environment exposes a callable entry for that tool

#### Scenario: Disabled MCP tool is absent from Python namespace
- **WHEN** an MCP-backed tool is not visible in the session-effective runtime tool catalog because its server is disabled or unavailable for the current session
- **THEN** the embedded Python environment does not expose a callable entry for that tool

#### Scenario: Embedded Python feature build uses the session-effective catalog
- **WHEN** the project is built with the `embedded-python` feature enabled
- **THEN** the embedded Python runtime compiles using the session-effective tool catalog path rather than any removed or legacy registry-only constructor

### Requirement: Tool registry is the only canonical callable catalog
The embedded Python runtime SHALL derive its callable tool surface from the session-effective runtime tool catalog rather than directly from capability metadata or only from the static global `ToolRegistry`.

#### Scenario: Metadata-only capability does not become a Python method
- **WHEN** a capability exists only in capability metadata and is not present in the session-effective runtime tool catalog
- **THEN** the embedded Python environment does not expose it as a callable tool method

#### Scenario: Session-visible tool becomes callable
- **WHEN** a tool is present in the session-effective runtime tool catalog under a tool name
- **THEN** the embedded Python environment exposes that tool name as callable

### Requirement: Tool namespace is stable for a single script run
The embedded Python runtime SHALL expose a stable snapshot of the session-effective runtime tool catalog for the lifetime of one script execution.

#### Scenario: Mid-run effective tool change does not alter in-flight namespace
- **WHEN** the session-effective runtime tool catalog changes after a script has already started
- **THEN** the in-flight script continues to use the tool namespace snapshot captured at script start

### Requirement: Child-tool execution reuses the session-effective runtime path
The embedded Python runtime SHALL resolve child-tool validation, approval, and execution through the same session-effective runtime tool path used for ordinary model-issued tool calls.

#### Scenario: Child tool call rejects tools outside the session-effective surface
- **WHEN** a Python child-tool call targets a tool that is not visible in the session-effective runtime tool catalog
- **THEN** the runtime rejects the call as unavailable for that session
- **THEN** the failure reason is recorded through the normal durable child-tool lifecycle

#### Scenario: Child MCP tool call uses the same execution path as normal MCP calls
- **WHEN** a Python child-tool call targets a visible MCP-backed tool
- **THEN** the runtime validates arguments using the discovered MCP schema
- **THEN** the runtime applies the same approval rules used for ordinary MCP tool calls
- **THEN** the runtime executes the tool through the shared MCP connection manager
