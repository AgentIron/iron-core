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

#### Scenario: Runtime default applies uniformly across configured servers
- **WHEN** a new session is created under a runtime whose MCP default policy is enabled or disabled
- **THEN** the session initializes each configured MCP server using that same runtime-level policy
- **THEN** per-server metadata does not override the single runtime-level default used for new-session MCP enablement

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

#### Scenario: Unavailable MCP execution reports a precise reason
- **WHEN** a caller attempts to execute an MCP-backed tool that is not executable for the current session
- **THEN** the runtime reports whether the tool is unavailable because the server is disabled for that session, because the server is unhealthy, or because the tool name is unknown for that server
- **THEN** the runtime does not report an ambiguous or session-inaccurate unavailable-tool reason

### Requirement: Runtime manages MCP connection and discovery lifecycle
The runtime SHALL manage MCP server connection lifecycle independently from per-session enablement, including transport establishment, protocol initialization, tool discovery, error reporting, and reconnect-driven rediscovery. Runtime-managed MCP background tasks SHALL also stop when the owning runtime shuts down.

#### Scenario: Runtime shutdown stops MCP background lifecycle
- **WHEN** the owning runtime shuts down
- **THEN** MCP reconnect monitoring and other background MCP lifecycle work started for that runtime stop as part of runtime shutdown
- **THEN** MCP background tasks do not outlive the logical runtime that created them

### Requirement: MCP tool calls reuse the standard runtime execution lifecycle
The runtime SHALL execute MCP-backed tool calls through the same validation, approval, durable recording, and result propagation lifecycle used for other runtime tools, while delegating the actual call to the connected MCP server. The canonical session-effective runtime path SHALL apply the configured runtime approval strategy rather than relying only on the tool's raw `requires_approval()` flag.

#### Scenario: Model-issued MCP tool call completes through the runtime
- **WHEN** the model issues a tool call for an MCP-backed tool that is visible for the current session
- **THEN** the runtime validates arguments against the discovered input schema
- **THEN** the runtime applies the standard approval and durable tool-record lifecycle
- **THEN** the runtime calls the backing MCP server tool and records the returned result or failure

#### Scenario: Child MCP tool call follows the same execution path
- **WHEN** a child tool execution path such as `python_exec` invokes a visible MCP-backed tool
- **THEN** the runtime executes that call through the same validation, approval, and durable-record flow used for non-child tool calls

#### Scenario: Approval strategy always forces approval for MCP tools
- **WHEN** the runtime approval strategy is configured as `Always` and a visible MCP-backed tool is invoked through the session-effective runtime path
- **THEN** the runtime requires explicit approval even if that tool's raw definition would not otherwise require it

#### Scenario: Approval strategy never skips approval for MCP tools
- **WHEN** the runtime approval strategy is configured as `Never` and a visible MCP-backed tool is invoked through the session-effective runtime path
- **THEN** the runtime does not request approval solely because the MCP-backed tool definition reports `requires_approval()`

### Requirement: Runtime supports concrete MCP transport clients
The runtime SHALL provide concrete transport support for configured MCP servers using the declared transport type, including stdio, HTTP, and HTTP+SSE. For stdio transports, the runtime SHALL spawn the subprocess with the parent process environment minus environment variables whose names match sensitive credential patterns, rather than a hardcoded allowlist. The runtime SHALL strip vars matching case-insensitive suffix patterns associated with secrets (`_API_KEY`, `_SECRET`, `_SECRET_KEY`, `_TOKEN`, `_PASSWORD`, `_CREDENTIALS`, `_AUTH_TOKEN`, `_ACCESS_KEY`, `_ACCESS_TOKEN`) and well-known credential var names (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, `AZURE_CLIENT_SECRET`, `GOOGLE_APPLICATION_CREDENTIALS`, `DATABASE_URL`, `GITHUB_TOKEN`, `GH_TOKEN`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`). The runtime SHALL log the names of stripped vars at debug level without logging their values. User-configured env vars from the MCP server config SHALL be merged after stripping and SHALL override any stripped or inherited values. For all MCP transports, the runtime SHALL encode outbound MCP protocol messages using the MCP camelCase wire format and SHALL decode inbound MCP protocol messages using the MCP camelCase wire format. This includes initialize, tool listing, tool calling, and related structured payloads whose wire field names differ from Rust snake_case naming. For MCP bootstrap, the runtime SHALL accept a successful `initialize` response whose JSON-RPC `id` is null or absent only when that response can be correlated unambiguously to the single in-flight bootstrap request. For MCP requests after bootstrap, the runtime SHALL continue to require valid request/response ID correlation and SHALL NOT accept ambiguous id-less responses as successful replies. HTTP and HTTP+SSE transports SHALL use a shared `HttpConfig` struct that carries the server URL and optional custom headers. The runtime SHALL send the `Accept: application/json, text/event-stream` header on all HTTP-based MCP requests and merge any configured custom headers.

#### Scenario: Stdio subprocess inherits non-sensitive parent environment vars
- **WHEN** a configured MCP server uses stdio transport and the parent process has environment variables that do not match sensitive patterns
- **THEN** the spawned subprocess inherits those non-sensitive vars
- **THEN** common toolchain vars like `PATH`, `HOME`, `APPDATA`, `XDG_CONFIG_HOME`, `CARGO_HOME`, `GOPATH`, `NODE_PATH` are available to the subprocess without requiring explicit MCP server config

#### Scenario: Stdio subprocess strips vars matching sensitive suffix patterns
- **WHEN** the parent process has environment variables whose names end in `_API_KEY`, `_SECRET`, `_TOKEN`, `_PASSWORD`, or similar sensitive suffixes
- **THEN** those vars are not present in the spawned subprocess environment
- **THEN** the runtime logs the names of stripped vars at debug level

#### Scenario: Stdio subprocess strips well-known credential vars
- **WHEN** the parent process has environment variables like `AWS_ACCESS_KEY_ID`, `GITHUB_TOKEN`, `ANTHROPIC_API_KEY`, or other well-known credential names
- **THEN** those vars are not present in the spawned subprocess environment

#### Scenario: User-configured env overrides stripped vars
- **WHEN** an MCP server config specifies an env var that would otherwise be stripped by the sensitive pattern matching
- **THEN** the user-configured value is present in the subprocess environment
- **THEN** the user config acts as an explicit override

#### Scenario: Sensitive pattern matching is case-insensitive
- **WHEN** the parent process has an environment variable whose name matches a sensitive pattern with different casing (e.g., `My_Api_Key` matching `_API_KEY`)
- **THEN** that var is still stripped

#### Scenario: Initialize request uses camelCase wire fields
- **WHEN** the runtime sends an MCP `initialize` request
- **THEN** the JSON payload uses `protocolVersion` and `clientInfo` field names
- **THEN** the runtime does not send snake_case field names like `protocol_version` or `client_info`

#### Scenario: Initialize response parses camelCase wire fields
- **WHEN** an MCP server returns an `initialize` response using `protocolVersion` and `serverInfo`
- **THEN** the runtime successfully parses the response into its internal protocol structs

#### Scenario: Tool list response parses camelCase pagination and schema fields
- **WHEN** an MCP server returns a `tools/list` response using camelCase fields such as `nextCursor` and `inputSchema`
- **THEN** the runtime successfully parses pagination state and tool schemas

#### Scenario: Tool call response parses camelCase error and resource metadata fields
- **WHEN** an MCP server returns a `tools/call` response using camelCase fields such as `isError` and `mimeType`
- **THEN** the runtime successfully parses the response content and error state

#### Scenario: SSE transport handles structured event responses
- **WHEN** a configured MCP server uses the HTTP+SSE transport
- **THEN** the runtime handles SSE framing explicitly rather than assuming the first data block is always the response payload
- **THEN** the runtime correlates the server response to the initiating MCP request sufficiently to avoid accepting unrelated stream events as a successful response

#### Scenario: HTTP bootstrap accepts an id-less initialize response in the safe case
- **WHEN** a configured MCP server using plain HTTP returns a successful `initialize` response with a null or absent `id`
- **THEN** the runtime accepts that response if it corresponds to the single in-flight bootstrap request
- **THEN** the runtime marks the server initialized rather than failing with an ID mismatch

#### Scenario: Stdio bootstrap does not drop an id-less initialize response before correlation
- **WHEN** a configured MCP server using stdio returns a successful `initialize` response with a null or absent `id`
- **THEN** the runtime does not discard that bootstrap response as a notification before evaluating bootstrap correlation
- **THEN** the runtime accepts that response if it corresponds to the single in-flight bootstrap request

#### Scenario: HTTP+SSE bootstrap does not drop an id-less initialize response before correlation
- **WHEN** a configured MCP server using HTTP+SSE returns a successful `initialize` response with a null or absent `id`
- **THEN** the runtime does not discard that bootstrap response solely because the `id` is missing before evaluating bootstrap correlation
- **THEN** the runtime accepts that response if it corresponds to the single in-flight bootstrap request

#### Scenario: Ordinary MCP traffic still requires valid response correlation
- **WHEN** an MCP server returns a response without a usable `id` after bootstrap or while multiple requests could be outstanding
- **THEN** the runtime does not treat that response as a successful reply to an ordinary request
- **THEN** the runtime preserves strict request/response correlation semantics for post-bootstrap MCP traffic

#### Scenario: HTTP transport uses HttpConfig for URL and headers
- **WHEN** a configured MCP server uses the HTTP transport
- **THEN** the transport reads its URL and optional custom headers from the `HttpConfig` struct
- **THEN** all requests include the default `Accept` header and any configured custom headers

#### Scenario: SSE transport uses HttpConfig for URL and headers
- **WHEN** a configured MCP server uses the HTTP+SSE transport
- **THEN** the transport reads its URL and optional custom headers from the `HttpConfig` struct
- **THEN** both the SSE bootstrap GET and JSON-RPC POST requests include the default `Accept` header and any configured custom headers

### Requirement: Client-visible MCP inspection and session control APIs
The system SHALL provide client-visible APIs to inspect runtime MCP inventory and to enable or disable configured MCP servers for a session. The system SHALL expose enough state for clients to distinguish runtime server health from session enablement intent. When the runtime renders prompt/runtime context for a model request, the displayed working directory and workspace roots SHALL reflect the configured builtin tool roots when such roots are available, rather than the process current directory alone.

#### Scenario: Public effective-tool inspection matches execution-visible tools
- **WHEN** a client requests the effective tools visible for a session
- **THEN** the returned tool definitions come from the same session-effective runtime tool surface used by prompt construction and tool execution
- **THEN** the inspection API does not diverge from actual MCP prompt visibility for that session

#### Scenario: Prompt runtime context uses configured primary root as working directory
- **WHEN** builtin tool roots are configured
- **THEN** the runtime context displays the first configured root as the working directory

#### Scenario: Prompt runtime context exposes all configured roots as workspace roots
- **WHEN** multiple builtin tool roots are configured
- **THEN** the runtime context includes all configured roots in its workspace root list

#### Scenario: Prompt runtime context falls back to process current directory when no roots are configured
- **WHEN** no builtin tool roots are configured
- **THEN** the runtime context uses the process current directory as the displayed working directory

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
