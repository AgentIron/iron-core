# python-tool-namespace Specification

## Purpose
TBD - created by archiving change expose-python-tool-methods. Update Purpose after archive.
## Requirements
### Requirement: Embedded Python exposes the runtime tool catalog
The embedded Python runtime SHALL expose callable tool access for every tool currently visible in `ToolRegistry` at the start of a script run.

#### Scenario: Built-in tool appears in Python namespace
- **WHEN** a built-in tool is registered in `ToolRegistry` and a `python_exec` script starts
- **THEN** the embedded Python environment exposes a callable entry for that tool

#### Scenario: Custom tool appears in Python namespace
- **WHEN** a custom tool is registered in `ToolRegistry` and a `python_exec` script starts
- **THEN** the embedded Python environment exposes a callable entry for that tool

### Requirement: Tool registry is the only canonical callable catalog
The embedded Python runtime SHALL derive its callable tool surface from `ToolRegistry` rather than directly from capability metadata.

#### Scenario: Metadata-only capability does not become a Python method
- **WHEN** a capability exists only in capability metadata and is not represented as a registered tool
- **THEN** the embedded Python environment does not expose it as a callable tool method

#### Scenario: Capability-backed tool is callable when registered as a tool
- **WHEN** a capability-backed implementation is materialized in `ToolRegistry` under a tool name
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
The embedded Python runtime SHALL expose a stable snapshot of the visible tool catalog for the lifetime of one script execution.

#### Scenario: Mid-run registry mutation does not change in-flight script namespace
- **WHEN** the runtime tool catalog changes after a script has already started
- **THEN** the in-flight script continues to use the tool namespace snapshot captured at script start

### Requirement: Python tool discovery is documented and inspectable
The embedded Python runtime SHALL document the supported tool namespace and SHALL provide enough discovery support for scripts and models to identify available tools.

#### Scenario: Runtime guidance describes Python namespace model
- **WHEN** embedded Python is enabled
- **THEN** the runtime guidance explains that Python tool access follows the visible runtime tool catalog rather than only `iron_call(name, args)`

