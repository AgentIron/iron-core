## MODIFIED Requirements

### Requirement: Runtime supports concrete MCP transport clients

The runtime SHALL provide concrete transport support for configured MCP servers using the declared transport type, including stdio, HTTP, and HTTP+SSE. For stdio transports, the runtime SHALL spawn the subprocess with the parent process environment minus environment variables whose names match sensitive credential patterns, rather than a hardcoded allowlist. The runtime SHALL strip vars matching case-insensitive suffix patterns associated with secrets (`_API_KEY`, `_SECRET`, `_SECRET_KEY`, `_TOKEN`, `_PASSWORD`, `_CREDENTIALS`, `_AUTH_TOKEN`, `_ACCESS_KEY`, `_ACCESS_TOKEN`) and well-known credential var names (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, `AZURE_CLIENT_SECRET`, `GOOGLE_APPLICATION_CREDENTIALS`, `DATABASE_URL`, `GITHUB_TOKEN`, `GH_TOKEN`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`). The runtime SHALL log the names of stripped vars at debug level without logging their values. Stdio MCP server config MAY include `inherited_env_vars`, a list of parent environment variable names to copy into the subprocess environment after sanitization without persisting the variable values. User-configured env vars from the MCP server config SHALL be merged after inherited env vars and SHALL override any stripped, inherited, or explicitly inherited values. For all MCP transports, the runtime SHALL encode outbound MCP protocol messages using the MCP camelCase wire format and SHALL decode inbound MCP protocol messages using the MCP camelCase wire format. This includes initialize, tool listing, tool calling, and related structured payloads whose wire field names differ from Rust snake_case naming. For MCP bootstrap, the runtime SHALL accept a successful `initialize` response whose JSON-RPC `id` is null or absent only when that response can be correlated unambiguously to the single in-flight bootstrap request. For MCP requests after bootstrap, the runtime SHALL continue to require valid request/response ID correlation and SHALL NOT accept ambiguous id-less responses as successful replies. HTTP and HTTP+SSE transports SHALL use a shared `HttpConfig` struct that carries the server URL and optional custom headers. The runtime SHALL send the `Accept: application/json, text/event-stream` header by default for HTTP and HTTP+SSE transports and SHALL include configured custom headers when present.

#### Scenario: Stdio subprocess inherits non-sensitive parent environment vars
- **WHEN** a configured MCP server uses stdio transport and the parent process has environment variables that do not match sensitive patterns
- **THEN** the spawned subprocess inherits those non-sensitive vars
- **THEN** common toolchain vars like `PATH`, `HOME`, `APPDATA`, `XDG_CONFIG_HOME`, `CARGO_HOME`, `GOPATH`, `NODE_PATH` are available to the subprocess without requiring explicit MCP server config

#### Scenario: Stdio subprocess strips vars matching sensitive suffix patterns
- **WHEN** the parent process has environment variables whose names end in `_API_KEY`, `_SECRET`, `_TOKEN`, `_PASSWORD`, or similar sensitive suffixes
- **THEN** those vars are not present in the spawned subprocess environment after the sanitization layer
- **THEN** the runtime logs the names of stripped vars at debug level
- **AND** the runtime does not log their values

#### Scenario: Stdio subprocess strips well-known credential vars
- **WHEN** the parent process has environment variables like `AWS_ACCESS_KEY_ID`, `GITHUB_TOKEN`, `ANTHROPIC_API_KEY`, or other well-known credential names
- **THEN** those vars are not present in the spawned subprocess environment after the sanitization layer

#### Scenario: Explicit inherited env vars reintroduce selected parent values
- **WHEN** an MCP stdio server config lists an environment variable name in `inherited_env_vars`
- **AND** the parent process contains that environment variable
- **THEN** the subprocess environment includes the parent value for that named variable after sanitization
- **AND** the config store persists only the variable name, not the variable value

#### Scenario: Explicit inherited env var is absent from parent
- **WHEN** an MCP stdio server config lists an environment variable name in `inherited_env_vars`
- **AND** the parent process does not contain that environment variable
- **THEN** the runtime does not synthesize a value for that variable
- **AND** MCP startup proceeds unless the server fails because the variable is required by the external command

#### Scenario: User-configured env overrides inherited vars
- **WHEN** an MCP server config specifies an env var that is also listed in `inherited_env_vars`
- **THEN** the user-configured value is present in the subprocess environment
- **AND** the user-configured env map has final precedence

#### Scenario: User-configured env overrides stripped vars
- **WHEN** an MCP server config specifies an env var that would otherwise be stripped by the sensitive pattern matching
- **THEN** the user-configured value is present in the subprocess environment
- **THEN** the user config acts as an explicit override

#### Scenario: Sensitive pattern matching is case-insensitive
- **WHEN** the parent process has an environment variable whose name matches a sensitive pattern with different casing (e.g., `My_Api_Key` matching `_API_KEY`)
- **THEN** that var is still stripped by the sanitization layer unless explicitly reintroduced through `inherited_env_vars` or configured env

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
