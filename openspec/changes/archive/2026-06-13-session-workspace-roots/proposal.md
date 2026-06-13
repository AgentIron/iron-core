## Why

AgentIron needs multiple tabs to share one live `IronRuntime` while each tab/session keeps a different working directory and workspace root set. Today `Config.workspace_roots` and builtin tool roots are construction-time runtime state, so changing directories requires recreating the agent and losing session history, MCP connections, plugin state, and skill catalog continuity.

## What Changes

- Add session-scoped workspace roots so each live session can have independent active roots.
- Add `AgentSession::set_workspace_roots(Vec<PathBuf>)` as the public API for changing a session's roots without recreating the runtime.
- Defer workspace-root changes requested during an active prompt until the session returns to idle.
- Ensure each prompt turn uses one consistent workspace-root snapshot for runtime context rendering and builtin tool path authorization.
- Update builtin filesystem, search, and shell tools to enforce the session's active workspace roots rather than a runtime-global root set.
- Automatically rescan project skills when a session's pending roots are applied, updating that session's available skill snapshot while preserving already-active skill instructions.
- Preserve runtime-level `Config.workspace_roots` as the initial/default roots for newly created sessions and as a fallback for existing callers.

## Capabilities

### New Capabilities

- `session-workspace-roots`: Session-scoped workspace root management, turn-boundary application semantics, prompt/tool root consistency, and project-skill refresh behavior.

### Modified Capabilities

- None.

## Impact

- **Core runtime**: `DurableSession`, `RuntimeSession`, `IronRuntime`, `AgentSession`, prompt lifecycle hooks.
- **Request building**: system prompt runtime context must receive session root snapshots instead of reading only `Config.workspace_roots`.
- **Builtin tools**: file operations, search tools, and shell tools must authorize paths against session-active roots for the prompt turn.
- **Skills**: project skill discovery must support session root snapshots and refresh available skills on applied root changes.
- **Public API**: new `AgentSession::set_workspace_roots(Vec<PathBuf>)` method; no runtime restart required.
- **Compatibility**: existing `Config::with_workspace_roots()` remains the construction-time default for sessions that do not set session-specific roots.
