## MODIFIED Requirements

### Requirement: Runtime manages MCP connection and discovery lifecycle
The runtime SHALL manage MCP server connection lifecycle independently from per-session enablement, including transport establishment, protocol initialization, tool discovery, error reporting, and reconnect-driven rediscovery. Runtime-managed MCP background tasks SHALL also stop when the owning runtime shuts down.

#### Scenario: Runtime shutdown stops MCP background lifecycle
- **WHEN** the owning runtime shuts down
- **THEN** MCP reconnect monitoring and other background MCP lifecycle work started for that runtime stop as part of runtime shutdown
- **THEN** MCP background tasks do not outlive the logical runtime that created them

### Requirement: Runtime supports concrete MCP transport clients
The runtime SHALL provide concrete transport support for configured MCP servers using the declared transport type, including stdio, HTTP, and HTTP+SSE.

#### Scenario: SSE transport handles structured event responses
- **WHEN** a configured MCP server uses the HTTP+SSE transport
- **THEN** the runtime handles SSE framing explicitly rather than assuming the first data block is always the response payload
- **THEN** the runtime correlates the server response to the initiating MCP request sufficiently to avoid accepting unrelated stream events as a successful response

### Requirement: Client-visible MCP inspection and session control APIs
The system SHALL provide client-visible APIs to inspect runtime MCP inventory and to enable or disable configured MCP servers for a session. The system SHALL expose enough state for clients to distinguish runtime server health from session enablement intent.

#### Scenario: Public effective-tool inspection matches execution-visible tools
- **WHEN** a client requests the effective tools visible for a session
- **THEN** the returned tool definitions come from the same session-effective runtime tool surface used by prompt construction and tool execution
- **THEN** the inspection API does not diverge from actual MCP prompt visibility for that session

### Requirement: MCP integration must not regress local tool execution
The session-effective runtime tool surface SHALL preserve execution of ordinary local tools while adding MCP-backed tools.

#### Scenario: MCP lookup remains unambiguous for provider-visible names
- **WHEN** the runtime resolves an MCP-backed tool from its provider-visible tool name
- **THEN** the resolution logic does not depend on ambiguous first-underscore splitting that can misidentify server IDs containing underscores
- **THEN** the runtime resolves the intended MCP server/tool pair or fails with a precise unavailable-tool error
