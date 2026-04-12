## Context

`iron-core` currently enforces JSON-RPC response ID correlation differently across MCP transports. Plain HTTP parses the response and rejects a missing or null `id` as a mismatch. Stdio and HTTP+SSE use reader/dispatcher tasks that require an `id` before routing a response to the waiting caller, so an id-less `initialize` response is dropped and the bootstrap call times out. The result is inconsistent behavior for the same server defect and unnecessary interoperability failures during MCP connection startup.

The design constraint is that request IDs are still the primary safety mechanism for correlating responses. Any tolerance for malformed responses must therefore be narrowly scoped so that it does not weaken ordinary multi-request traffic or future concurrency.

## Goals / Non-Goals

**Goals:**
- Make stdio, HTTP, and HTTP+SSE behave consistently when an MCP server returns a successful `initialize` response with a null or missing `id`.
- Preserve strict request/response ID validation for ordinary post-bootstrap MCP traffic.
- Ensure transport dispatchers do not discard a bootstrap response before the bootstrap correlation rule can be applied.
- Add tests that prove tolerant bootstrap handling and continued rejection of ambiguous id-less responses.

**Non-Goals:**
- Broadly accepting missing response IDs for all MCP requests.
- Changing public MCP APIs or relaxing JSON-RPC validation outside narrowly safe bootstrap cases.
- Introducing speculative request matching heuristics once multiple requests may be outstanding.

## Decisions

### Treat missing response IDs as a bootstrap-only interoperability exception
The client should accept a null or missing response ID only for `initialize`, and only when that response can be matched unambiguously to the single in-flight bootstrap request.

This is preferable to a global rule such as "accept any response whose ID is absent" because request ID correlation is still required for safety once normal traffic begins.

### Move tolerance to the transport correlation boundary, not only the final validation check
For stdio and HTTP+SSE, the critical bug is not just the final `response.id == Some(id)` check. Their reader tasks currently drop id-less responses before the waiting caller ever sees them. The design therefore requires transport-specific routing support for the bootstrap exception so all transports can apply the same policy.

This is preferable to patching only the HTTP validation path because it yields consistent behavior across transports.

### Require unambiguous correlation before accepting an id-less bootstrap response
If the client cannot prove that an id-less response belongs to the single bootstrap request, it must continue to reject or ignore that response. In practice, this means the exception applies only while exactly one bootstrap waiter is outstanding and before post-bootstrap concurrency begins.

This is preferable to looser heuristics because it preserves the integrity of the waiter map and avoids misrouting responses.

### Keep diagnostics transport-specific but policy-consistent
The user-visible failure shape should reflect the real failure mode for each transport, but the policy should be shared: tolerate id-less `initialize` only in the narrowly safe case; otherwise fail clearly. Tests should assert both acceptance and rejection behavior explicitly.

This is preferable to hiding all malformed-response cases behind a generic timeout because it keeps the system debuggable.

## Risks / Trade-offs

- [Bootstrap-only tolerance may still exclude some non-conforming servers that omit IDs for later requests] → Keep the initial scope narrow; if broader interop is needed later, evaluate it as a separate, explicitly justified change.
- [Dispatcher changes in stdio and HTTP+SSE could accidentally weaken notification handling] → Limit the exception to `initialize` and add regression tests proving ordinary notifications remain non-correlated.
- ["Exactly one pending request" logic can become subtle around timeouts or reader shutdown] → Keep the rule small, centralize the acceptance condition, and test timeout/error paths alongside the success case.
- [Transport-consistent behavior may still produce different error messages] → Standardize the policy and acceptance criteria, but allow transport-specific diagnostics where they help identify the failing layer.

## Migration Plan

- Update transport correlation logic so id-less bootstrap responses can be surfaced to the waiting `initialize` caller only in the safe case.
- Apply the same bootstrap correlation rule in plain HTTP validation.
- Add targeted tests for stdio, HTTP, and HTTP+SSE covering null/missing IDs during `initialize`.
- Add regression tests proving id-less ordinary responses remain rejected, ignored, or timed out when correlation is ambiguous.

## Open Questions

- Should diagnostics explicitly distinguish `"id": null` from a completely absent `id` field, or is treating both as the same interoperability class sufficient for now?
