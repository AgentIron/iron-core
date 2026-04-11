## Why

`iron-core` currently has partial MCP support: it can register MCP servers, track per-session enablement, and compute an inspection-oriented effective tool view, but MCP-backed tools are not wired into the real prompt or tool-execution path. This leaves MCP exposed as a feature surface that appears present in the API but cannot yet be used end-to-end.

## What Changes

- Wire MCP-backed tools into the real session-effective tool surface used for provider requests and runtime tool dispatch.
- Add runtime-managed MCP connection lifecycle behavior so configured servers can connect, discover tools, surface health, and recover from reconnectable failures.
- Implement executable MCP tool calls rather than placeholder wrappers, including argument validation, result propagation, and failure reporting.
- Align child-tool and embedded-Python execution paths with the same effective session tool surface so enabled MCP tools behave consistently everywhere the runtime can invoke tools.
- Add end-to-end tests for MCP registration, connection, discovery, prompt visibility, execution, and recovery behavior.

## Capabilities

### New Capabilities

### Modified Capabilities
- `session-scoped-mcp-support`: Extend MCP behavior from inventory and visibility-only state to actual runtime connection, effective prompt exposure, and executable tool calls.
- `python-tool-namespace`: Update the Python tool namespace to derive from the session-effective tool surface so enabled MCP-backed tools are available through the same runtime execution path as other visible tools.

## Impact

- Affects the MCP subsystem in `src/mcp/`, including connection management, discovered tool state, and tool invocation.
- Affects prompt construction and tool execution in `src/prompt_runner.rs`, `src/request_builder.rs`, `src/runtime.rs`, and related runtime plumbing.
- Affects embedded Python child-tool access so it reflects the same visible tool surface as normal prompt execution.
- Adds or updates integration tests around MCP end-to-end behavior and regression coverage for reconnect and error states.
