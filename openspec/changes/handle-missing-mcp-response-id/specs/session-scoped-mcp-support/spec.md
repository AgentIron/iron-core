## MODIFIED Requirements

### Requirement: Runtime supports concrete MCP transport clients
The runtime SHALL provide concrete transport support for configured MCP servers using the declared transport type, including stdio, HTTP, and HTTP+SSE. For MCP bootstrap, the runtime SHALL accept a successful `initialize` response whose JSON-RPC `id` is null or absent only when that response can be correlated unambiguously to the single in-flight bootstrap request. For MCP requests after bootstrap, the runtime SHALL continue to require valid request/response ID correlation and SHALL NOT accept ambiguous id-less responses as successful replies.

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
