## 1. Runtime MCP Inventory

- [x] 1.1 Add MCP server configuration and runtime inventory state, including server ids, transport metadata, health, discovered tool summaries, and the runtime-level default for new-session enablement.
- [x] 1.2 Add MCP connection management for supported transports so the runtime can track connected and error states for configured servers.
- [x] 1.3 Ensure MCP inventory remains runtime-local and is not serialized into handoff export or import flows.

## 2. Session Enablement And Effective Tools

- [x] 2.1 Add session-scoped MCP enablement state so each session can independently enable or disable configured servers.
- [x] 2.2 Introduce a session-effective tool view that combines local tools with MCP-backed tools only when their server is enabled for the session and currently usable.
- [x] 2.3 Route provider request tool exposure, tool execution, approval flow, active-context accounting, and `python_exec` tool catalog generation through the session-effective tool view.

## 3. Client-Facing APIs

- [x] 3.1 Add runtime APIs for listing configured MCP servers and inspecting their runtime state and discovered tool summaries.
- [x] 3.2 Add session APIs for enabling and disabling configured MCP servers and for inspecting session enablement state.
- [x] 3.3 Surface the new MCP runtime and session controls through the facade and ACP-facing client interfaces needed by iron-core consumers.

## 4. Validation

- [x] 4.1 Add tests covering new-session default enablement, independent session toggles, and exclusion of MCP state from handoff.
- [x] 4.2 Add tests covering effective tool visibility for disabled, connected, errored, and reconnected MCP servers.
- [x] 4.3 Add tests covering client-visible runtime/session MCP inspection APIs and `python_exec` visibility of effective MCP tools.
