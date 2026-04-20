## Why

Remote MCP servers (e.g., Context7) require an `Accept: application/json, text/event-stream` header on HTTP requests. The current MCP HTTP and SSE clients send no explicit headers, causing initialization failures with a misleading "Response ID mismatch" error. Additionally, there is no mechanism to pass custom headers (e.g., `Authorization`, `CONTEXT7_API_KEY`) to remote MCP servers, even though the consuming application's UI already supports configuring them.

## What Changes

- Introduce an `HttpConfig` struct containing `url` and optional `headers`, shared by both `McpTransport::Http` and `McpTransport::HttpSse` variants
- Set `Accept: application/json, text/event-stream` as a default header on all MCP HTTP requests (both plain HTTP and SSE)
- Merge user-configured custom headers into requests, allowing per-server authentication and configuration
- Apply the `Accept` header to both the SSE bootstrap GET and JSON-RPC POST requests in `HttpSseMcpClient`

## Capabilities

### New Capabilities
- `mcp-http-headers`: Adds HTTP header support to MCP remote transports, including a required `Accept` header and user-configurable custom headers via a shared `HttpConfig` struct

### Modified Capabilities
- `session-scoped-mcp-support`: The `McpTransport` enum variants `Http` and `HttpSse` change from `{ url: String }` to `{ config: HttpConfig }`, which is a **BREAKING** change to the public API

## Impact

- **Public API**: `McpTransport::Http` and `McpTransport::HttpSse` variant shapes change — any code constructing these variants must update
- **Files**: `src/mcp/server.rs` (enum + new struct), `src/mcp/client.rs` (both HTTP client structs + dispatch function)
- **Tests**: ~46 `McpTransport::Http`/`HttpSse` instantiations across 5 test files need updating
- **Dependencies**: No new crate dependencies; uses existing `reqwest` and `HashMap`
- **Consumers**: Downstream crates (e.g., AgentIron app) that construct `McpTransport` variants will need to adapt to the new `HttpConfig` struct
