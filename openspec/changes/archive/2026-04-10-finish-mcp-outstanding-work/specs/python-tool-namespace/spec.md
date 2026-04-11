## MODIFIED Requirements

### Requirement: Embedded Python exposes the runtime tool catalog
The embedded Python runtime SHALL expose callable tool access for every tool visible in the session-effective runtime tool catalog at the start of a script run, rather than limiting the namespace to tools present in the static global `ToolRegistry`.

#### Scenario: Public and Python-visible session tool views stay aligned
- **WHEN** a tool is visible to embedded Python through the session-effective runtime tool catalog
- **THEN** the public effective-tool inspection for that same session reports the same tool as visible
- **THEN** embedded Python does not depend on a divergent MCP visibility implementation

### Requirement: Child-tool execution reuses the session-effective runtime path
The embedded Python runtime SHALL resolve child-tool validation, approval, and execution through the same session-effective runtime tool path used for ordinary model-issued tool calls.

#### Scenario: MCP child-tool lookup remains unambiguous
- **WHEN** a Python child-tool call targets an MCP-backed tool whose provider-visible name includes a server ID containing underscores or other separator-like characters
- **THEN** the runtime resolves the intended MCP tool using the same unambiguous lookup logic used for ordinary MCP tool execution
