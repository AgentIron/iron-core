## ADDED Requirements

### Requirement: HTTP MCP requests include Accept header
All MCP HTTP requests (both plain HTTP and HTTP+SSE transports) SHALL include the header `Accept: application/json, text/event-stream` by default. This applies to both JSON-RPC POST requests and SSE bootstrap GET requests.

#### Scenario: HTTP transport sends Accept header on POST
- **WHEN** an MCP server is configured with the HTTP transport and a JSON-RPC request is sent
- **THEN** the HTTP POST request includes the header `Accept: application/json, text/event-stream`

#### Scenario: SSE transport sends Accept header on bootstrap GET
- **WHEN** an MCP server is configured with the HTTP+SSE transport and the SSE reader establishes a connection
- **THEN** the initial GET request includes the header `Accept: application/json, text/event-stream`

#### Scenario: SSE transport sends Accept header on POST
- **WHEN** an MCP server is configured with the HTTP+SSE transport and a JSON-RPC POST request is sent
- **THEN** the POST request includes the header `Accept: application/json, text/event-stream`

### Requirement: Custom headers can be configured per MCP server
The MCP transport configuration SHALL support optional custom HTTP headers for HTTP and HTTP+SSE transports. Custom headers SHALL be merged into every HTTP request made to that server, alongside the default `Accept` header.

#### Scenario: Custom headers are sent on HTTP requests
- **WHEN** an MCP server is configured with the HTTP transport and custom headers `{ "Authorization": "Bearer token123" }`
- **THEN** every HTTP POST request to that server includes both the default `Accept` header and the custom `Authorization` header

#### Scenario: Custom headers are sent on SSE requests
- **WHEN** an MCP server is configured with the HTTP+SSE transport and custom headers `{ "X-API-Key": "abc" }`
- **THEN** both the SSE bootstrap GET and JSON-RPC POST requests include the custom `X-API-Key` header alongside the default `Accept` header

#### Scenario: Custom Accept header overrides the default
- **WHEN** an MCP server is configured with a custom `Accept` header value
- **THEN** the custom value is used instead of the default `application/json, text/event-stream`

#### Scenario: No custom headers configured
- **WHEN** an MCP server is configured with the HTTP transport and no custom headers
- **THEN** requests include only the default `Accept: application/json, text/event-stream` header and no additional custom headers

### Requirement: HttpConfig struct shared by HTTP transport variants
A shared `HttpConfig` struct SHALL encapsulate the URL and optional headers for HTTP-based MCP transports. Both `McpTransport::Http` and `McpTransport::HttpSse` variants SHALL use this struct.

#### Scenario: HttpConfig carries URL and optional headers
- **WHEN** an `HttpConfig` is constructed with a URL and optional headers
- **THEN** the config is accessible from both `McpTransport::Http` and `McpTransport::HttpSse` variants

#### Scenario: HttpConfig deserializes without headers for backward compatibility
- **WHEN** an HTTP transport config is deserialized from a source that includes only a URL and no headers field
- **THEN** the `headers` field defaults to `None` and the transport functions correctly with only the default `Accept` header

### Requirement: Custom headers are excluded from serialization when not set
When no custom headers are configured, the `HttpConfig` SHALL serialize without an explicit `headers` field, keeping serialized output clean and backward-compatible.

#### Scenario: Serialization omits headers when None
- **WHEN** an `HttpConfig` with `headers: None` is serialized
- **THEN** the output does not include a `headers` key
