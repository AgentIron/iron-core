## Why

Some MCP servers return a successful `initialize` response with `id: null` or with no `id` field. `iron-core` currently handles this inconsistently across transports: plain HTTP rejects the response as an ID mismatch, while stdio and HTTP+SSE drop the response before correlation and eventually time out. This prevents interoperability with real-world MCP servers during bootstrap even when the rest of the response is usable.

## What Changes

- Add a narrowly scoped interoperability rule for MCP bootstrap so `initialize` responses with null or missing IDs can be accepted only when correlation remains unambiguous.
- Make stdio and HTTP+SSE transport dispatchers preserve id-less bootstrap responses long enough for the client to apply the bootstrap correlation rule instead of dropping them as notifications.
- Keep strict JSON-RPC request/response ID validation for normal post-bootstrap MCP traffic.
- Add transport-level tests that cover tolerant bootstrap handling and continued strictness for ambiguous or concurrent non-bootstrap cases.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `session-scoped-mcp-support`: refine MCP transport behavior so bootstrap initialization tolerates null or missing response IDs only in narrowly safe cases and does so consistently across stdio, HTTP, and HTTP+SSE transports.

## Impact

- Affected code: `src/mcp/client.rs`, MCP transport tests, and any runtime code that surfaces MCP connection errors.
- Affected systems: MCP connection bootstrap for stdio, HTTP, and HTTP+SSE transports.
- API/protocol impact: no public API additions, but MCP bootstrap becomes more interoperable while preserving strict correlation for ordinary requests.
