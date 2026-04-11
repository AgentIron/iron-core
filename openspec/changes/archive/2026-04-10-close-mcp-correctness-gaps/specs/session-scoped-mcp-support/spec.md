## ADDED Requirements

### Requirement: Public MCP helper APIs must not advertise stub execution behavior
Publicly exposed MCP helper types and APIs SHALL either execute through the real runtime MCP path or fail clearly as unsupported. The system MUST NOT expose a public MCP helper execution path that returns a synthetic success-shaped payload while bypassing the runtime-owned MCP connection manager.

#### Scenario: Public MCP helper cannot report fake execution success
- **WHEN** a caller uses a public MCP helper type that is not wired to the runtime-owned execution path
- **THEN** the helper is either not publicly exposed for execution or returns a clear unsupported error
- **THEN** the helper does not return a fabricated tool result that could be mistaken for a real MCP server response

## MODIFIED Requirements

### Requirement: Runtime manages MCP connection and discovery lifecycle
The runtime SHALL manage MCP server connection lifecycle independently from per-session enablement, including transport establishment, protocol initialization, tool discovery, error reporting, and reconnect-driven rediscovery. Failures during transport creation, connection setup, initialization, or discovery SHALL be reported through runtime health state rather than panicking the runtime-owned task.

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

#### Scenario: Failed stdio process spawn is surfaced as runtime error state
- **WHEN** a configured stdio MCP server cannot be spawned or its transport pipes cannot be established
- **THEN** the runtime does not panic during MCP client construction
- **THEN** the runtime records the server in an unavailable or errored health state with an actionable last error

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

#### Scenario: Shared HTTP+SSE connection correlates concurrent responses by request id
- **WHEN** multiple MCP requests are in flight concurrently over one shared HTTP+SSE server connection
- **THEN** the runtime correlates each SSE-delivered JSON-RPC response to the matching request id
- **THEN** one caller does not consume or discard another caller's response

#### Scenario: Shared HTTP+SSE connection ignores non-payload events
- **WHEN** the HTTP+SSE server emits keepalive, ping, or other non-response SSE events
- **THEN** the runtime ignores those events for request completion purposes
- **THEN** only correlated response payload events are used to complete MCP requests
