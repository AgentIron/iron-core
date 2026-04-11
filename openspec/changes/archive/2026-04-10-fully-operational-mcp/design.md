## Context

`iron-core` already has the beginnings of MCP support in three places: a runtime-local server registry, session-scoped enablement state, and an MCP effective-tool view. That work established inventory and visibility rules, but the actual runtime still builds provider requests from the plain local `ToolRegistry`, executes tool calls only from that registry, and leaves MCP connection, discovery, and tool invocation as placeholders.

This change is cross-cutting because it affects prompt construction, tool dispatch, session-effective visibility, MCP transport lifecycle, and embedded Python child-tool execution. The implementation also has to preserve the distinction between runtime-owned MCP connectivity and session-owned enablement intent.

## Goals / Non-Goals

**Goals:**
- Make MCP-backed tools part of the real session-effective tool surface used for provider requests.
- Make MCP-backed tool names resolvable and executable through the same runtime path used for other tool calls.
- Implement runtime-managed MCP connection, initialization, tool discovery, error handling, and reconnect behavior.
- Preserve session-scoped MCP enablement and health-based visibility rules from the existing specification.
- Ensure embedded Python and child-tool execution see the same session-effective tool surface as normal turns.
- Add end-to-end tests that prove MCP works beyond inspection-only APIs.

**Non-Goals:**
- Redesigning the session-scoped enablement model or handoff semantics already established for MCP.
- Adding a new client UX surface for configuring servers beyond the existing runtime/session APIs.
- Expanding MCP into a generalized plugin framework; that remains distinct from WASM integration plugins.
- Solving every possible advanced MCP feature in v1, such as prompts/resources/sampling, if the current need is server-backed tool execution.

## Decisions

### Introduce one runtime-owned effective tool catalog for prompting and execution
The runtime will compute a session-effective tool catalog that contains both provider-facing `ToolDefinition`s and executable handles for each visible tool. Local tools will resolve to the existing `ToolRegistry`; MCP tools will resolve to MCP-backed executable handles rather than ephemeral wrapper objects.

This catalog will become the single source of truth for:
- provider request construction
- approval checks
- normal tool execution
- child-tool execution from embedded Python

This removes the current split where MCP visibility exists in helper APIs but the real prompt loop still uses only `ToolRegistry`.

Implementation note:
The session-effective tool catalog must borrow or otherwise reference the real runtime-owned executable registries and connection manager. It must not be built from cloned `ToolRegistry` state, because the current registry clone drops executable tool handlers and would silently break normal local tool execution.

Alternatives considered:
- Keep the current `ToolRegistry` as the only executable catalog and register synthetic MCP tools into it: rejected because MCP availability is session-dependent and runtime-dynamic, while `ToolRegistry` is global and static.
- Continue computing MCP definitions separately only for request building: rejected because execution, approval, and child-tool paths would still diverge.

### Resolve MCP tools by structured handles, not by reparsing provider names
The runtime will assign provider-visible names to MCP tools, but internally it will store structured metadata for each visible MCP tool, including server identity and remote tool name. Execution will use this stored handle instead of reparsing names like `mcp_<server>_<tool>` back into server/tool components.

This avoids brittle string parsing and keeps prompt exposure and execution bound to the same resolved object.

Alternatives considered:
- Keep parsing names on every call: rejected because it is fragile, duplicates lookup logic, and couples execution correctness to naming conventions.

### Separate runtime MCP connectivity from session enablement
MCP connectivity, initialization, discovery, and health are runtime concerns. Session enablement only controls whether tools from an already-known server are exposed and callable for that session. Registering an MCP server in an MCP-enabled runtime should therefore trigger runtime connection management independently from per-session enablement defaults.

This keeps runtime inventory truthful even when some sessions choose not to expose a server, and it prevents session-default policy from silently suppressing discovery.

Alternatives considered:
- Connect only when at least one session enables a server: rejected because it delays inventory/health visibility and adds lifecycle coupling between unrelated sessions.
- Continue tying connection behavior to `enabled_by_default`: rejected because that field represents new-session policy, not runtime transport availability.

Implementation note:
The runtime must own a single long-lived `McpConnectionManager` instance. Prompt execution and child-tool execution should consume that shared manager rather than constructing fresh per-session managers on demand, otherwise connection state, discovery state, and reconnect behavior will never become real runtime behavior.

### Provider request construction must use the session-effective catalog
Prompt construction must derive provider-visible tool definitions from the same session-effective catalog used for approval and execution. The runtime should use the MCP-aware request-builder path so the model sees enabled, healthy MCP-backed tools and does not see disabled or unhealthy ones.

Alternatives considered:
- Keep using the plain `ToolRegistry` for prompt construction and only switch execution: rejected because the model would never be offered MCP tools even if execution support existed.

### Embedded Python must consume the same session-effective catalog
The embedded Python runtime must build its namespace from the session-effective tool catalog and must route child-tool validation, approval, and execution through the same catalog. This includes compiling cleanly with the `embedded-python` feature enabled.

Alternatives considered:
- Leave the Python namespace on the old registry path and only fix normal prompt execution: rejected because it preserves inconsistent behavior and leaves MCP child-tool execution broken.

### Tests must exercise real MCP wiring, not registry mutation shortcuts
Integration coverage must validate runtime registration, connection startup, discovery, prompt exposure, and execution through real code paths. Tests that manually mutate MCP registry health and discovered-tool state are useful as unit coverage, but they are not sufficient evidence that MCP is operational end-to-end.

### Put protocol-specific transport behind a concrete MCP client adapter
The placeholder transport code will be replaced with a concrete adapter layer responsible for transport setup, MCP initialization, tool listing, and tool calling. The runtime-facing side will use a stable internal interface so tests can run against a fake/in-process MCP client without requiring external processes or live network services.

This keeps the runtime architecture testable while still making the actual implementation fully operational.

Alternatives considered:
- Put protocol logic directly in `McpConnectionManager`: rejected because it would make testing and transport evolution harder.
- Leave discovery and calls as manual registry mutation in tests: rejected because it does not validate the real feature.

### Embedded Python should snapshot the session-effective tool surface
The embedded Python tool namespace will be built from the same session-effective catalog captured at script start, not directly from the static `ToolRegistry`. This preserves the existing snapshot semantics while allowing enabled MCP-backed tools to appear in Python child execution when they are visible to that session.

Alternatives considered:
- Keep Python bound to `ToolRegistry` only: rejected because it would remain inconsistent with normal prompt execution once MCP is functional.

## Risks / Trade-offs

- [A unified effective tool catalog adds new runtime plumbing] -> Keep the representation minimal and use it consistently across request building, approval, and execution rather than introducing multiple parallel abstractions.
- [Real MCP transport support increases async and lifecycle complexity] -> Isolate protocol behavior behind a transport/client adapter and cover reconnect, discovery, and error transitions with integration tests.
- [Long-lived MCP connections may fail independently of sessions] -> Treat health as runtime-owned state, surface it in inventory APIs, and gate execution on both session enablement and current connection usability.
- [Dynamic remote tool schemas can drift from discovery time] -> Validate call arguments against the last discovered schema and surface actionable failures when a server rejects or invalidates a tool.
- [Python namespace changes can broaden visible tools] -> Preserve snapshot-at-start behavior and continue reusing the standard child-tool approval and validation flow.

## Migration Plan

- Introduce the runtime effective-tool catalog and switch prompt construction and execution paths to use it.
- Replace MCP placeholders with the concrete connection/discovery/call adapter while keeping existing session enablement APIs stable.
- Update embedded Python tool-catalog creation to use the session-effective snapshot.
- Add integration tests that cover the full MCP lifecycle and verify regression behavior for non-MCP tools.
- Roll back by disabling MCP in runtime config, which preserves the existing local-tool path without requiring handoff or session format changes.

## Open Questions

- Should runtime registration connect servers immediately or enqueue connection work on the runtime handle for non-blocking startup?
- What is the narrowest internal client adapter shape that can support stdio and HTTP/SSE transports without leaking protocol details into the prompt runner?
- Do we want a dedicated client-visible reconnect action, or is automatic reconnect on runtime-managed failures sufficient for the first implementation?
