## MODIFIED Requirements

### Requirement: Runtime supports concrete MCP transport clients
The runtime SHALL provide concrete transport support for configured MCP servers using the declared transport type, including stdio, HTTP, and HTTP+SSE. HTTP and HTTP+SSE transports SHALL use a shared `HttpConfig` struct that carries the server URL and optional custom headers. The runtime SHALL send the `Accept: application/json, text/event-stream` header on all HTTP-based MCP requests and merge any configured custom headers.

#### Scenario: SSE transport handles structured event responses
- **WHEN** a configured MCP server uses the HTTP+SSE transport
- **THEN** the runtime handles SSE framing explicitly rather than assuming the first data block is always the response payload
- **THEN** the runtime correlates the server response to the initiating MCP request sufficiently to avoid accepting unrelated stream events as a successful response

#### Scenario: HTTP transport uses HttpConfig for URL and headers
- **WHEN** a configured MCP server uses the HTTP transport
- **THEN** the transport reads its URL and optional custom headers from the `HttpConfig` struct
- **THEN** all requests include the default `Accept` header and any configured custom headers

#### Scenario: SSE transport uses HttpConfig for URL and headers
- **WHEN** a configured MCP server uses the HTTP+SSE transport
- **THEN** the transport reads its URL and optional custom headers from the `HttpConfig` struct
- **THEN** both the SSE bootstrap GET and JSON-RPC POST requests include the default `Accept` header and any configured custom headers
