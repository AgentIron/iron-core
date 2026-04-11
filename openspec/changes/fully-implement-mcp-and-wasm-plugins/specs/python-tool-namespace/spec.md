## MODIFIED Requirements

### Requirement: Python tool calls reuse the standard runtime execution path
The embedded Python runtime SHALL execute namespace-exposed tool calls through the existing child-tool execution path. That path SHALL honor the same runtime approval strategy, schema validation, durable tool-record lifecycle, and session-effective visibility rules used for ordinary model-issued tool calls.

#### Scenario: Python tool calls inherit validation and permission behavior
- **WHEN** a `python_exec` script invokes a visible tool through the embedded Python namespace
- **THEN** the runtime applies the same schema validation, permission checks, and durable tool-record lifecycle used for other child tool calls

#### Scenario: Python child-tool approval follows runtime approval strategy
- **WHEN** embedded Python invokes a visible tool and the runtime approval strategy is `Always`, `Never`, or `PerTool`
- **THEN** the runtime decides whether approval is required using that same runtime approval strategy rather than a divergent child-tool-specific rule

### Requirement: Child-tool execution reuses the session-effective runtime path
The embedded Python runtime SHALL resolve child-tool validation, approval, and execution through the same session-effective runtime tool path used for ordinary model-issued tool calls.

#### Scenario: MCP child-tool lookup remains unambiguous
- **WHEN** a Python child-tool call targets an MCP-backed tool whose provider-visible name includes a server ID containing underscores or other separator-like characters
- **THEN** the runtime resolves the intended MCP tool using the same unambiguous lookup logic used for ordinary MCP tool execution

#### Scenario: Python child-tool can reach visible plugin-backed tools
- **WHEN** a plugin-backed tool is present in the session-effective runtime tool catalog and visible to embedded Python at script start
- **THEN** embedded Python can invoke that plugin-backed tool through the same child-tool execution path used for local and MCP-backed tools

#### Scenario: Python child-tool rejects tools outside the session-effective surface
- **WHEN** a Python child-tool call targets a tool that is not visible in the session-effective runtime tool catalog for that session
- **THEN** the runtime rejects the call as unavailable for that session through the same session-effective execution path used for ordinary tool calls
