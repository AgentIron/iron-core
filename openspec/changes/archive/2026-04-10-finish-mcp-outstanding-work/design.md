## Context

The MCP implementation now has a working runtime-owned session-effective tool surface, stdio-backed end-to-end execution, prompt exposure, and embedded Python child-tool parity. The remaining work is narrower and mostly architectural cleanup plus transport hardening.

The main gaps are:
- the `HTTP+SSE` path is still only lightly implemented and not validated by realistic transport tests
- the public effective-tools inspection path still uses the legacy MCP helper rather than the real runtime session-effective catalog
- the reconnect monitor is spawned independently of runtime-managed task ownership and shutdown
- MCP tool-name parsing still assumes the first underscore after `mcp_` separates server ID from tool name, which is fragile for server IDs containing underscores

This follow-up change should finish those edges without reopening the broader MCP design.

## Goals / Non-Goals

**Goals:**
- Make the declared `HTTP+SSE` transport support robust enough to justify the spec language.
- Make public MCP effective-tool inspection reflect the same source of truth as prompt construction and tool execution.
- Make MCP background lifecycle fully runtime-owned, including shutdown behavior.
- Remove name-parsing ambiguity in MCP tool lookup and error handling.
- Add focused tests for the remaining weak spots.

**Non-Goals:**
- Rebuilding the already-working stdio MCP path.
- Expanding MCP beyond tool support into prompts, resources, or sampling.
- Redesigning session-scoped enablement semantics.

## Decisions

### Unify public effective-tool inspection with `SessionToolCatalog`
The public inspection APIs should derive their session-visible tool definitions from the same session-effective runtime path used by prompt execution. The legacy `EffectiveToolView` can remain only as an internal compatibility helper if needed, but it should no longer be the canonical inspection source for MCP tool exposure.

This avoids divergence where public inspection says one thing while prompt construction and execution do another.

### Treat `HTTP+SSE` as a real protocol mode with explicit response handling
The SSE transport path should not rely on “first `data:` block wins” behavior. The implementation should explicitly model the stream semantics it expects, correlate responses to requests, and tolerate non-payload SSE events.

This does not require a fully general event framework, but it does require enough structure that the transport is meaningfully distinct from the plain HTTP path and behaves predictably with realistic servers.

### Make reconnect monitoring runtime-owned and shutdown-aware
The reconnect monitor should be started and stopped under runtime ownership rather than free-floating `tokio::spawn`. This can be done by either:
- moving the loop under `IronRuntime::spawn()` ownership, or
- explicitly storing and cancelling a connection-manager-owned task with a shutdown signal.

The key requirement is that a logical runtime shutdown must also stop MCP background activity.

### Stop reparsing MCP names with first-underscore splitting
The current string format is good enough for provider-visible names, but lookup should not depend on ambiguous first-underscore parsing. The runtime should either:
- keep a direct map from visible tool name to structured MCP handle, or
- parse by longest matching registered server ID instead of first underscore.

The simpler option is to reuse session-effective catalog mappings wherever possible and keep fallback parsing only for diagnostics.

## Risks / Trade-offs

- [Improving SSE handling may still not cover every possible server implementation] -> Scope the behavior clearly and validate against a realistic fake SSE server that exercises framing and correlation assumptions.
- [Unifying inspection paths may require mild refactoring around legacy helpers] -> Prefer redirecting public APIs to the session-effective catalog rather than deleting old helpers immediately.
- [Shutdown-aware background lifecycle introduces more bookkeeping] -> Keep ownership simple and explicit so runtime shutdown remains deterministic.

## Migration Plan

- Redirect public effective-tool inspection to the session-effective catalog.
- Refactor MCP background lifecycle so reconnect monitoring participates in runtime shutdown.
- Harden the SSE path and add dedicated transport tests.
- Replace ambiguous MCP name parsing with structured lookup or longest-match parsing.
- Re-run MCP and regression suites after each step.

## Open Questions

- Should `HTTP+SSE` tests use an in-process fake event-stream server, or is a narrowly-scoped test transport sufficient?
- Is `EffectiveToolView` still worth keeping after the public inspection path is redirected, or should it be retired in a later cleanup?
