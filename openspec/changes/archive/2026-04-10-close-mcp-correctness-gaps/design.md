## Context

The MCP runtime already has working session-scoped visibility, stdio-backed end-to-end execution, reconnect monitoring, and embedded Python child-tool parity. The remaining issues are correctness and contract gaps rather than missing broad functionality.

The audit found four concrete problems:
- stdio transport setup can panic during client construction, which bypasses normal runtime health transitions and leaves server state inconsistent
- the HTTP+SSE transport uses one shared inbound stream but does not safely support multiple concurrent in-flight requests on that shared connection
- one MCP regression suite is broken and no longer verifies the behavior it claims to cover
- a legacy public `McpTool` type is still exported even though its `execute` path is a stub and is not the real runtime execution path

These gaps matter because the current spec claims runtime-managed lifecycle, concrete transport support, and standard execution behavior. The implementation is close, but those guarantees are not yet reliable in every path.

## Goals / Non-Goals

**Goals:**
- Ensure MCP connection setup failures surface as runtime errors and health transitions instead of panics.
- Make HTTP+SSE request/response handling safe for one shared server connection serving multiple concurrent tool calls.
- Restore MCP verification so the checked-in regression suite is green and exercises the required behavior.
- Remove or constrain public MCP helper APIs that currently advertise unimplemented execution behavior.

**Non-Goals:**
- Expanding MCP beyond tools into prompts, resources, or sampling.
- Redesigning session-scoped enablement semantics.
- Replacing the broader MCP architecture that is already working for stdio and session-effective visibility.

## Decisions

### Transport construction must return fallible errors
Client construction for stdio-backed servers should become fallible so subprocess spawn and pipe acquisition failures can be reported through `connect_server()` and translated into `McpServerHealth::Error` with `last_error` details.

This is preferable to preserving `expect`/`panic` behavior because the spec contract is about runtime-owned connection lifecycle, not process-aborting setup. A failed MCP server must degrade runtime inventory state, not crash the task.

### HTTP+SSE correlation must be dispatcher-based, not receiver-race-based
The shared SSE connection should use a request dispatcher keyed by JSON-RPC request id. The stream-reading task should remain the single consumer of SSE frames and should route each parsed response to the waiting request for that id.

This is preferable to the current shared receiver loop because independent callers otherwise compete to consume the same event stream and can accidentally discard each other's responses. A per-request waiter map preserves one shared transport connection while making concurrency safe.

### Non-payload SSE events should be explicitly filtered before dispatch
The SSE reader should ignore keepalive and non-response events, parse only JSON-RPC response payloads, and route only correlated responses into waiter channels.

This keeps the transport behavior aligned with the spec without introducing a broader event framework that is out of scope.

### Legacy public MCP helper exposure should not imply executable behavior
The runtime should either remove public export of `McpTool` or make its behavior explicitly non-executable in a way that cannot be mistaken for a successful tool call. The canonical executable surface must remain `SessionToolCatalog` plus `McpConnectionManager`.

This avoids a mismatch where the public API advertises a tool type whose execution path is not wired to the real runtime.

### Verification must cover concurrency and current public APIs
Regression coverage should be updated to compile against the current MCP types, and at least one transport test should exercise concurrent in-flight HTTP+SSE requests on the same shared client.

This is preferable to relying on single-request happy-path tests because the main remaining correctness risk is concurrency on the shared SSE stream.

## Risks / Trade-offs

- [Dispatcher-based SSE handling adds shared state and bookkeeping] -> Keep the design narrow: one reader task, one waiter map keyed by request id, and clear cleanup on timeout/close.
- [Making client construction fallible touches connection-manager flow] -> Contain the change to transport creation and error propagation so health-state transitions remain explicit.
- [Changing public MCP helper exposure may be observable to downstream callers] -> Prefer deprecation or a hard failure mode with clear messaging if immediate removal would be too disruptive.
- [Concurrency tests can become timing-sensitive] -> Use deterministic in-process fake SSE servers and assert correlation behavior instead of relying on sleeps alone.

## Migration Plan

- Convert stdio client setup and transport factory creation to return structured errors.
- Introduce request-id-based SSE dispatch for the shared HTTP+SSE client.
- Remove, deprecate, or hard-fail the stub public `McpTool` execution path.
- Repair MCP integration tests to match current runtime types and add concurrent SSE verification.
- Re-run MCP and embedded-Python suites after each step.

## Open Questions

- Should `McpTool` be removed from the public re-export set now, or first be kept as a deprecated compatibility type that always returns a hard execution error?
- Should HTTP+SSE support request cancellation cleanup for abandoned waiters in this change, or is timeout-based cleanup sufficient for the current scope?
