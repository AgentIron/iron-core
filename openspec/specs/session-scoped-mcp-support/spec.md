# session-scoped-mcp-support Specification

## Purpose

This specification defines the session-scoped MCP (Model Context Protocol) support for iron-core, enabling runtime-local MCP server inventory, per-session enablement, and effective tool exposure rules.
## Requirements
### Requirement: Runtime-local MCP server inventory
The runtime SHALL maintain a runtime-local inventory of configured MCP servers. Each configured server SHALL have a stable runtime identity and SHALL expose client-visible metadata including transport type, connection health, and discovered tool summaries when available. The runtime SHALL be authoritative for which MCP servers are available to it, independent from any specific client process. The runtime SHALL keep this inventory current as MCP servers connect, disconnect, fail, recover, and rediscover tools.

#### Scenario: Client lists configured runtime MCP servers
- **WHEN** a client requests the runtime MCP server inventory
- **THEN** the runtime returns all configured MCP servers known to that runtime
- **THEN** each returned server includes its runtime identity, transport type, and current health state
- **THEN** each returned server includes discovered tool summaries when discovery has succeeded

#### Scenario: Runtime inventory is local to the current runtime
- **WHEN** a session is handed off or imported into another runtime
- **THEN** the destination runtime does not assume the source runtime's MCP server inventory exists locally

### Requirement: Session-scoped MCP enablement
Each session SHALL independently track whether each configured MCP server is enabled or disabled for that session. Enabling or disabling an MCP server for one session SHALL NOT change the enablement state of any other session. New sessions SHALL initialize their MCP enablement state from a single runtime-level default that can be configured as enabled-by-default or disabled-by-default.

#### Scenario: New session uses runtime default enablement
- **WHEN** a new session is created
- **THEN** the session initializes MCP server enablement according to the runtime's configured default for new sessions

#### Scenario: Session toggle does not affect another session
- **WHEN** one session disables a configured MCP server
- **THEN** another session retains its own independent enablement state for that server

### Requirement: Effective MCP tool exposure depends on enablement and server health
The system SHALL expose MCP-backed tools to the model only when the backing MCP server is both enabled for the current session and currently connected or otherwise usable in the runtime. MCP-backed tools from servers in error or unavailable states SHALL be excluded from the effective tool surface presented to the model. The same enablement and health gates SHALL apply to runtime tool execution so MCP-backed tools that are not visible for a session are also not executable for that session.

#### Scenario: Enabled usable server contributes tools to prompt construction
- **WHEN** a configured MCP server is enabled for the current session and is currently usable in the runtime
- **THEN** tools from that server are included in the effective tool surface used to build the provider request for that session

#### Scenario: Prompt construction uses the session-effective tool surface
- **WHEN** the runtime builds a provider request for a session
- **THEN** the provider-visible tool definitions come from the same session-effective tool surface used for approval and execution
- **THEN** enabled, healthy MCP-backed tools are visible to the model
- **THEN** disabled or unhealthy MCP-backed tools are not visible to the model

#### Scenario: Disabled server tools are hidden
- **WHEN** a configured MCP server is disabled for the current session
- **THEN** tools from that server are excluded from the effective tool surface for that session
- **THEN** tool calls for that server are rejected as unavailable for that session

#### Scenario: Errored server tools are hidden
- **WHEN** a configured MCP server enters an error state
- **THEN** tools from that server are excluded from the effective tool surface for every session until the server becomes usable again

#### Scenario: Reconnected server tools return for enabled sessions
- **WHEN** a configured MCP server becomes usable again after an error and rediscovery succeeds
- **THEN** its tools are restored to the effective tool surface for sessions where that server remains enabled

### Requirement: Runtime manages MCP connection and discovery lifecycle
The runtime SHALL manage MCP server connection lifecycle independently from per-session enablement, including transport establishment, protocol initialization, tool discovery, error reporting, and reconnect-driven rediscovery. Runtime-managed MCP background tasks SHALL also stop when the owning runtime shuts down.

#### Scenario: Runtime shutdown stops MCP background lifecycle
- **WHEN** the owning runtime shuts down
- **THEN** MCP reconnect monitoring and other background MCP lifecycle work started for that runtime stop as part of runtime shutdown
- **THEN** MCP background tasks do not outlive the logical runtime that created them

### Requirement: MCP tool calls reuse the standard runtime execution lifecycle
The runtime SHALL execute MCP-backed tool calls through the same validation, approval, durable recording, and result propagation lifecycle used for other runtime tools, while delegating the actual call to the connected MCP server.

#### Scenario: Model-issued MCP tool call completes through the runtime
- **WHEN** the model issues a tool call for an MCP-backed tool that is visible for the current session
- **THEN** the runtime validates arguments against the discovered input schema
- **THEN** the runtime applies the standard approval and durable tool-record lifecycle
- **THEN** the runtime calls the backing MCP server tool and records the returned result or failure

#### Scenario: Child MCP tool call follows the same execution path
- **WHEN** a child tool execution path such as `python_exec` invokes a visible MCP-backed tool
- **THEN** the runtime executes that call through the same validation, approval, and durable-record flow used for non-child tool calls

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

### Requirement: MCP state is excluded from handoff portability
Handoff export and import SHALL exclude runtime MCP inventory and session MCP enablement state. Importing a handoff bundle SHALL NOT automatically enable, disable, configure, or assume availability of MCP servers on the destination runtime.

#### Scenario: Handoff does not carry enabled MCP servers
- **WHEN** a session with one or more enabled MCP servers is exported and later imported
- **THEN** the imported session does not inherit enabled MCP server state from the source runtime

#### Scenario: Destination runtime decides its own MCP availability
- **WHEN** a handoff bundle is imported into a runtime with a different MCP configuration
- **THEN** the destination runtime determines available MCP servers using its own runtime-local inventory and policy

### Requirement: MCP integration must not regress local tool execution
The session-effective runtime tool surface SHALL preserve execution of ordinary local tools while adding MCP-backed tools.

#### Scenario: MCP lookup remains unambiguous for provider-visible names
- **WHEN** the runtime resolves an MCP-backed tool from its provider-visible tool name
- **THEN** the resolution logic does not depend on ambiguous first-underscore splitting that can misidentify server IDs containing underscores
- **THEN** the runtime resolves the intended MCP server/tool pair or fails with a precise unavailable-tool error

### Requirement: Public MCP helper APIs must not advertise stub execution behavior
Publicly exposed MCP helper types and APIs SHALL either execute through the real runtime MCP path or fail clearly as unsupported. The system MUST NOT expose a public MCP helper execution path that returns a synthetic success-shaped payload while bypassing the runtime-owned MCP connection manager.

#### Scenario: Public MCP helper cannot report fake execution success
- **WHEN** a caller uses a public MCP helper type that is not wired to the runtime-owned execution path
- **THEN** the helper is either not publicly exposed for execution or returns a clear unsupported error
- **THEN** the helper does not return a fabricated tool result that could be mistaken for a real MCP server response

