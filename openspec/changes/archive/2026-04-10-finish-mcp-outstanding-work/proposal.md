## Why

The archived `fully-operational-mcp` change brought MCP much closer to working end-to-end, but review found a small set of remaining gaps that still prevent MCP from being considered fully complete. The main runtime path is now functional for stdio-backed MCP servers, but the SSE transport path is still thin and unverified, the public effective-tools inspection API still uses the legacy MCP helper rather than the real session-effective runtime path, and the connection monitor lifecycle is not fully tied to runtime shutdown.

These gaps are small compared to the work already completed, but they matter because the synced specs now require real support for `HTTP+SSE`, consistent session-effective behavior across public inspection and execution APIs, and proper runtime-owned lifecycle behavior.

## What Changes

- Complete the remaining MCP transport work so `HTTP+SSE` behaves like a real supported transport rather than a weak best-effort implementation.
- Align the public effective-tools inspection path with the same session-effective runtime tool surface used by prompt construction and execution.
- Tie MCP reconnect monitoring and background lifecycle to runtime shutdown so MCP tasks do not outlive a logical runtime.
- Harden MCP tool-name parsing and lookup so server IDs and tool names are handled unambiguously.
- Add targeted verification for the remaining unsupported or weakly-tested paths, especially non-stdio transports and public inspection behavior.

## Capabilities

### Modified Capabilities
- `session-scoped-mcp-support`: Finish the remaining transport, lifecycle, and inspection-path work so the implementation fully satisfies the synced MCP operational spec.
- `python-tool-namespace`: No new behavioral scope beyond the synced spec, but follow-up verification may touch this capability where MCP child-tool parity depends on the final transport and inspection cleanup.

## Impact

- Affects MCP transport and lifecycle code in `src/mcp/client.rs`, `src/mcp/connection.rs`, and related runtime plumbing.
- Affects the public session-effective inspection path exposed through `IronRuntime` and `IronAgent`.
- Adds targeted tests for the remaining MCP edge cases that are not yet proven by the current suite.
