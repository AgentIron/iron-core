## Why

The current MCP implementation is close, but it still has correctness gaps that leave parts of the declared behavior unreliable. In particular, stdio connection setup can panic instead of surfacing runtime health errors, the HTTP+SSE transport is not safe for multiple concurrent in-flight requests, and the MCP regression suite is not fully green.

## What Changes

- Make stdio-backed MCP connection setup fail through normal runtime error handling instead of panicking during client construction.
- Strengthen HTTP+SSE transport behavior so one shared MCP server connection can safely correlate multiple concurrent requests without dropping another request's response.
- Bring MCP regression coverage back to a passing and trustworthy state by fixing broken integration tests and adding concurrency-focused SSE validation.
- Remove or deprecate the publicly exposed stub `McpTool` execution path so MCP public APIs do not advertise behavior that is not actually implemented.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `session-scoped-mcp-support`: tighten runtime lifecycle, transport correctness, and public MCP execution guarantees so failure handling and concurrent HTTP+SSE behavior match the declared MCP support contract.

## Impact

- Affected code: `src/mcp/client.rs`, `src/mcp/connection.rs`, `src/mcp/effective_tools.rs`, `src/mcp/mod.rs`, `src/lib.rs`, and MCP-related tests.
- Affected APIs: MCP transport behavior and any public MCP helper types that currently expose stub execution behavior.
- Affected systems: runtime MCP connection lifecycle, transport correlation, and regression verification.
