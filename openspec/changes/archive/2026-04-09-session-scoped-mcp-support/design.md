## Context

iron-core is ACP-native on its client-facing side and currently exposes tools through a runtime-global `ToolRegistry`. That model works for built-in tools and embedded Python orchestration, but it does not provide a first-class way to incorporate vendor-provided MCP servers or to scope those integrations per session. The desired product behavior is to let users configure MCP servers once for a runtime, then enable or disable them per session to control tool-surface and context size.

This change also needs to preserve a key portability boundary that already exists in handoff: session continuity transfers conversation state, not runtime-local capabilities. That means enabled MCP servers cannot be serialized into handoff bundles or assumed to exist on a destination runtime.

## Goals / Non-Goals

**Goals:**
- Introduce a runtime-local inventory of configured MCP servers independent from any single client process.
- Let each session independently enable or disable configured MCP servers.
- Make MCP-backed tools appear and behave like normal runtime tools when they are effectively available.
- Hide tools from servers in error state so the model does not attempt to use broken integrations.
- Provide a client-visible API surface for listing runtime MCP servers, viewing status, and toggling session enablement.
- Support a single runtime-level default for whether configured MCP servers start enabled in new sessions.

**Non-Goals:**
- Expose iron-core itself as an MCP server.
- Define cross-runtime portability for MCP inventory or enabled state.
- Add per-tool or per-server default policies beyond the single new-session enabled/disabled default.
- Solve MCP prompts/resources as first-class runtime concepts in this change; the immediate scope is MCP tools.
- Define client UX, credential storage UX, or vendor-specific onboarding flows.

## Decisions

### Runtime owns configured MCP inventory
The runtime will own the authoritative set of configured MCP servers, their transport metadata, health, discovered tool catalog, and runtime admission state. This keeps MCP capability local to the iron-core instance rather than assuming parity with a client or another runtime.

Alternatives considered:
- Client-owned inventory only: rejected because remote or embedded runtimes may have different connectivity, policy, or available integrations.
- Session-owned configurations: rejected because configuration lifecycle is broader than a single conversation.

### Sessions own enablement intent, not connection state
Each session will store whether a configured server is enabled or disabled for that session. Session state expresses user intent only. Runtime server health remains global.

Alternatives considered:
- Session stores a richer tri-state like enabled/disabled/error: rejected because error is not session-owned and would duplicate runtime state.
- Runtime-global enablement: rejected because users need to manage context bloat per session.

### Effective MCP tool visibility is computed from session intent plus runtime health
An MCP tool is visible to the model only if its server is both enabled for the session and currently connected/usable in the runtime. If a server enters error state, its tools disappear from the effective tool surface. If it later reconnects, enabled sessions see the tools again without needing to re-enable them.

Alternatives considered:
- Keep errored tools visible with failing executions: rejected because it wastes context and encourages avoidable tool attempts.
- Require manual re-enable after reconnect: rejected because it adds state complexity without improving control.

### MCP tools should enter the same effective tool pipeline as local tools
The model, approval flow, durable tool records, and `python_exec` should all consume a session-specific effective tool set rather than a single runtime-global registry snapshot. This preserves existing semantics while allowing MCP tools to feel native when available.

Alternatives considered:
- Keep MCP execution outside the normal tool pipeline: rejected because it would split prompt exposure, approval, history, and Python orchestration semantics.

### New sessions use one runtime-level default for MCP enablement
The runtime will define one configuration knob controlling whether configured MCP servers are enabled by default for newly created sessions. The default product assumption is enabled, but deployments can invert it globally.

Alternatives considered:
- Per-server defaults: rejected as unnecessary policy complexity for the initial scope.
- Always disabled by default: rejected because the desired default product behavior is enabled unless configured otherwise.

### Handoff excludes MCP enablement and inventory
Enabled MCP servers and runtime MCP inventory remain runtime-local state and are not included in handoff export/import. This preserves the current handoff portability model and avoids assuming destination runtimes have matching integrations.

Alternatives considered:
- Export enabled server IDs in handoff: rejected because IDs and availability are runtime-specific.

## Risks / Trade-offs

- [Runtime-global inventory introduces more runtime state] -> Keep the model simple: configured servers, runtime health, discovered tool catalog, and session enablement intent only.
- [Tool visibility can change between turns when server health changes] -> Define this as expected behavior and recompute effective tools at prompt boundaries.
- [MCP tool name collisions across servers may confuse the model] -> Require a deterministic naming strategy in implementation and expose the resulting visible names through the client API.
- [Remote/server auth and transport setup can sprawl into product concerns] -> Keep credential UX and onboarding out of scope for this change and focus on runtime-facing integration semantics.
- [Context size remains affected by historical tool usage even after disablement] -> Document that enable/disable controls future tool exposure, not transcript compaction or history rewriting.

## Migration Plan

1. Introduce runtime-local MCP inventory and session enablement state without changing handoff behavior.
2. Move request building, prompt tool exposure, execution, and `python_exec` catalog generation to use a session-effective tool view.
3. Add client-facing APIs for runtime inventory inspection and session enable/disable operations.
4. Add tests covering default enablement, independent session toggles, error-state tool removal, reconnect behavior, and handoff exclusion.

Rollback strategy: remove MCP inventory/session state and return to the existing runtime-global tool surface. No persisted handoff format change is required because MCP state is not serialized.

## Open Questions

- Which MCP client library and remote transport variants should be adopted first for the runtime implementation.
- How much server/tool metadata should be exposed in the client-facing API beyond ids, labels, transport, health, and discovered tool summaries.
