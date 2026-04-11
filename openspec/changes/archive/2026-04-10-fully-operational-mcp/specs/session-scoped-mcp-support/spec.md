## ADDED Requirements

### Requirement: Runtime manages MCP connection and discovery lifecycle
The runtime SHALL manage MCP server connection lifecycle independently from per-session enablement, including transport establishment, protocol initialization, tool discovery, error reporting, and reconnect-driven rediscovery.

#### Scenario: Registered server connects and discovers tools
- **WHEN** an MCP-enabled runtime registers a reachable MCP server
- **THEN** the runtime establishes the MCP connection for that server
- **THEN** the runtime initializes the MCP session and records the server as usable only after successful initialization
- **THEN** the runtime stores the discovered MCP tools in the runtime inventory

#### Scenario: Registration uses runtime-owned shared connection state
- **WHEN** the runtime registers an MCP server
- **THEN** the runtime uses its shared MCP connection manager to connect, initialize, discover, and reconnect that server
- **THEN** later prompt execution and child-tool execution observe the same connection and discovery state

#### Scenario: Failed server initialization marks server unavailable
- **WHEN** a registered MCP server fails to connect, initialize, or list tools
- **THEN** the runtime marks that server as unavailable or errored in runtime health state
- **THEN** the runtime does not expose that server's tools in the effective tool surface until discovery succeeds again

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

#### Scenario: Stdio server is initialized through subprocess transport
- **WHEN** a configured MCP server uses the stdio transport
- **THEN** the runtime spawns the configured subprocess
- **THEN** the runtime performs MCP initialization and tool operations over that subprocess transport

#### Scenario: HTTP server is initialized through HTTP transport
- **WHEN** a configured MCP server uses the HTTP transport
- **THEN** the runtime performs MCP initialization and tool operations using the configured HTTP endpoint

#### Scenario: SSE server is initialized through HTTP+SSE transport
- **WHEN** a configured MCP server uses the HTTP+SSE transport
- **THEN** the runtime performs MCP initialization and tool operations using the configured SSE-capable transport path rather than treating it as plain HTTP by name only

## MODIFIED Requirements

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

### Requirement: MCP integration must not regress local tool execution
The session-effective runtime tool surface SHALL preserve execution of ordinary local tools while adding MCP-backed tools.

#### Scenario: Local tool remains executable after MCP integration
- **WHEN** a session executes a built-in or custom local tool after MCP support is enabled
- **THEN** the runtime resolves and executes that local tool successfully through the same session-effective execution path
- **THEN** MCP support does not require local tool handlers to be re-registered or reconstructed from degraded cloned state
