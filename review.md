# MCP Audit — Resolved

> **Status: APPROVED** — all gaps identified in the original audit have been resolved.
> Updated April 11, 2026.

## Plan Compliance

- ✅ **Runtime-local MCP inventory** (`openspec/specs/session-scoped-mcp-support/spec.md:7-19`)
  - Implemented in `src/mcp/server.rs`, `src/mcp/connection.rs`, and exposed via `src/runtime.rs` / `src/facade.rs`.

- ✅ **Session-scoped MCP enablement uses one runtime default policy** (`.../session-scoped-mcp-support/spec.md:20-30`)
  - `create_session()` correctly seeds new sessions from `config.mcp.enabled_by_default`.
  - **Resolved:** imported/inserted sessions are re-initialized from the destination runtime policy. The session-effective path no longer falls back to per-server metadata.
  - **Resolved:** existing sessions get explicit runtime-default state for newly registered servers.

- ✅ **Effective MCP tool exposure depends on session enablement + health, and prompt/public inspection reuse the same session-effective surface** (`.../session-scoped-mcp-support/spec.md:31-56`, `86-93`)
  - Visible-tool composition is centralized in `src/mcp/session_catalog.rs` and used by `src/runtime.rs`, `src/prompt_runner.rs`, and `src/facade.rs`.

- ✅ **Runtime manages MCP connection/discovery lifecycle and shutdown** (`.../session-scoped-mcp-support/spec.md:57-63`)
  - Implemented in `src/mcp/connection.rs` with shutdown-aware monitor and runtime shutdown signaling.

- ✅ **MCP tool calls reuse the standard runtime execution lifecycle, including precise unavailable-tool reporting** (`.../session-scoped-mcp-support/spec.md:65-77`)
  - **Resolved:** model-issued MCP tool calls now delegate to `SessionToolCatalog::execute()` for precise disabled/unhealthy/unknown-tool diagnostics.

- ✅ **Embedded Python child-tool execution reuses the same session-effective MCP path** (`openspec/specs/python-tool-namespace/spec.md:36-56`)
  - **Resolved:** unavailable child MCP/plugin tools are rejected through the same precise session-effective execution path.

- ✅ **Concrete transport clients: stdio, HTTP, HTTP+SSE** (`.../session-scoped-mcp-support/spec.md:78-85`)
  - Implemented in `src/mcp/client.rs` with HTTP+SSE request/response correlation.

- ✅ **Handoff excludes MCP state and destination runtime decides availability** (`.../session-scoped-mcp-support/spec.md:94-104`)
  - **Resolved:** imported sessions are re-seeded from destination runtime default enablement.

- ✅ **Unambiguous MCP lookup and no fake public helper execution** (`.../session-scoped-mcp-support/spec.md:105-119`)
  - Longest-match lookup implemented. Legacy `McpTool::execute()` hard-fails clearly.

## Previously Identified Correctness Issues — All Resolved

1. ✅ **Imported sessions now adopt destination-runtime MCP policy**
   - On `insert_session()` (and when registering a new MCP server against existing sessions), explicit per-session enablement is materialized from the runtime default. Per-server fallback removed from the session-effective path.

2. ✅ **Prompt execution path uses precise MCP unavailable-tool diagnostics**
   - Model-issued absent tools are now routed through `SessionToolCatalog::execute()` which surfaces the precise disabled/unhealthy/unknown-tool reason.

3. ✅ **Embedded Python child-tool path uses canonical unavailable-tool diagnostics**
   - Missing child MCP/plugin tools are now rejected through the same precise session-effective path.

4. ✅ **MCP connection race eliminated**
   - Per-server single-flight connect guarding prevents duplicate connect/discovery attempts. No more `Response ID mismatch` flaky failures.

## Test Results

- ✅ `cargo test` (default features): **490 passed, 0 failed**

## Verdict

**approved**

MCP is **fully implemented** and production-ready. All previously identified gaps have been resolved:
- session-policy correctness for imported/missing session state,
- unified unavailable-tool execution diagnostics for both model-issued and Python child-issued calls,
- connection race/flakiness eliminated with single-flight guarding,
- transport hardening (stdio, HTTP, SSE),
- security hardening (environment, stderr, correlation, logging).
