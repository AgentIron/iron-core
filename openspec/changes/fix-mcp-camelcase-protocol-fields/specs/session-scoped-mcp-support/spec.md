## MODIFIED Requirements

### Requirement: Runtime supports concrete MCP transport clients
The runtime SHALL provide concrete transport support for configured MCP servers using the declared transport type, including stdio, HTTP, and HTTP+SSE. For all MCP transports, the runtime SHALL encode outbound MCP protocol messages using the MCP camelCase wire format and SHALL decode inbound MCP protocol messages using the MCP camelCase wire format. This includes initialize, tool listing, tool calling, and related structured payloads whose wire field names differ from Rust snake_case naming.

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
