## 1. Runtime Failure Handling

- [x] 1.1 Make stdio MCP client construction and transport factory creation return structured errors instead of panicking
- [x] 1.2 Update MCP connection setup to translate transport construction failures into server error state with actionable `last_error` details
- [x] 1.3 Add regression coverage for failed stdio spawn or pipe setup without runtime task panic

## 2. HTTP+SSE Concurrency Safety

- [x] 2.1 Replace shared receiver race behavior with request-id-based response dispatch for HTTP+SSE transport
- [x] 2.2 Filter keepalive and other non-payload SSE events before request completion and clean up per-request waiters on completion or timeout
- [x] 2.3 Add transport tests that exercise multiple concurrent in-flight HTTP+SSE requests on one shared client connection

## 3. Public API Cleanup

- [x] 3.1 Remove, deprecate, or hard-fail the stub public `McpTool` execution path so it cannot return fabricated success-shaped results
- [x] 3.2 Update MCP public re-exports and any dependent tests to reflect the canonical runtime execution path

## 4. Verification

- [x] 4.1 Repair `tests/mcp_integration_tests.rs` to match the current MCP runtime APIs and compile cleanly
- [x] 4.2 Re-run MCP regression suites, including visibility, e2e, outstanding, and integration tests
- [x] 4.3 Re-run embedded-Python MCP coverage to verify child-tool access still works with the updated MCP runtime path
