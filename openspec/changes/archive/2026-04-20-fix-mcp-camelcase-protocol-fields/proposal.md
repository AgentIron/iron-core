## Why

The MCP protocol message structs in `src/mcp/protocol.rs` currently serialize and deserialize Rust snake_case field names directly on the wire. MCP requires camelCase JSON field names, so spec-conforming servers reject `initialize` requests and the client may also fail to parse valid camelCase responses. Real servers including filesystem, firecrawl, and context7 are failing, so this is a protocol correctness bug rather than a server-specific interoperability issue.

## What Changes

- Add camelCase serde field mapping to MCP protocol message structs so outbound requests use MCP-compliant field names.
- Update inbound deserialization for MCP protocol structs so valid camelCase responses from servers parse correctly.
- Audit all MCP protocol message structs for wire-shape correctness, including initialize, tool listing, tool calling, and embedded content/resource structures.
- Add serialization and integration tests that prove MCP wire messages use camelCase and round-trip correctly against expected JSON shapes.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `session-scoped-mcp-support`: refine MCP protocol message encoding/decoding so the runtime speaks MCP-compliant camelCase JSON across stdio, HTTP, and HTTP+SSE transports.

## Impact

- Affected code: `src/mcp/protocol.rs`, MCP transport clients in `src/mcp/client.rs`, and MCP transport/integration tests.
- Affected systems: all MCP transports and all MCP server initialization/tool exchange flows.
- API/protocol impact: no public Rust API changes, but wire-format behavior changes to align with the MCP specification.
