## Why

iron-core currently exposes only runtime-local tools and ACP client capability overrides, which leaves no first-class way to use vendor-provided MCP servers as part of an agent session. We need a session-scoped MCP model so users can configure integrations once, then enable or disable them per session to control context size without assuming those integrations exist across handoffs or remote runtimes.

## What Changes

- Add runtime-local MCP server inventory support so an iron-core runtime can track configured MCP servers, their transport type, connection health, discovered tools, and runtime admission decisions.
- Add session-scoped MCP enablement so each session can independently enable or disable configured MCP servers without affecting other sessions.
- Define effective tool visibility so MCP tools are exposed to the model only when the backing server is both session-enabled and currently connected; servers in error state are hidden from the tool surface.
- Add client-facing APIs for listing runtime MCP servers, inspecting their state, and enabling or disabling them for a session.
- Define a single runtime option controlling whether configured MCP servers are enabled by default for newly created sessions.
- Keep MCP enablement runtime-local: handoff/export continues to exclude enabled MCP state and does not assume the destination runtime has the same integrations available.

## Capabilities

### New Capabilities
- `session-scoped-mcp-support`: Runtime-local MCP server inventory, per-session MCP enablement, effective tool exposure rules, and client APIs for viewing and toggling MCP integrations.

### Modified Capabilities
None.

## Impact

- Affected code: runtime session state, tool exposure and execution plumbing, prompt/inference request assembly, session APIs, ACP/facade surfaces, active-context accounting, and embedded Python tool visibility.
- Affected systems: MCP transport/client layer for stdio and remote servers, runtime integration inventory management, and client UI consumers that need runtime/server/session state.
- Dependencies: an MCP client library or transport implementation for stdio and remote MCP connections.
