## Why

WASM plugins currently fit into the runtime through the tool-call surface, which means their client-visible output is effectively constrained to tool definitions, approval state, status, and opaque result payloads. That is enough for simple automation, but it is not enough for a good client experience when a plugin wants to present richer read-only or replaceable state such as todo lists, status panels, or progress views.

Today a plugin can return text, but there is no runtime-standard way for a client to know that a plugin result should be rendered as something richer than transcript output. As a result, plugin UX is bottlenecked by plain-text rendering even when the plugin and client could support a better experience.

## What Changes

- Introduce a runtime-standard, client-visible rich-output channel for WASM plugin tool calls that is distinct from transcript text.
- Define a normalized plugin result envelope that carries plugin-authored transcript text plus an optional structured `view` payload.
- Expose transcript text and rich `view` fields distinctly through runtime/facade result surfaces so clients can render richer experiences without scraping transcript text or reverse-engineering arbitrary JSON.
- Define capability boundaries so plugins can describe structured view state without directly owning client lifecycle, arbitrary frontend execution, user-choice prompting, or general runtime actions.
- Establish a small v1 set of supported view kinds and lifecycle semantics (`view_id` and `view_mode`) that clients can implement consistently.
- Preserve compatibility for text-only clients by requiring transcript-safe plugin-authored text whenever a rich `view` payload is emitted.

## Capabilities

### New Capabilities
- `plugin-rich-output`: Allows WASM plugin tool calls to return plugin-authored transcript text plus structured client-renderable `view` payloads so clients can present richer plugin experiences than plain text alone.

### Modified Capabilities
- `wasm-integration-plugins`: Extends plugin execution and inspection semantics so plugin tool results can include structured `view` metadata and client-visible rendering payloads.

## Impact

- Affected code: likely `src/plugin/*`, `src/mcp/session_catalog.rs`, `src/events.rs`, `src/facade.rs`, runtime tool execution/result propagation, and plugin-related tests.
- Affected APIs: plugin tool result contracts, event/facade surfaces for tool results, and client-facing runtime inspection/rendering of plugin outputs.
- Client impact: rich-capable clients can render normalized view payloads when supported, while text-only clients continue to use fallback transcript text.
- Security and UX impact: the runtime must constrain what plugins can request so plugins describe view state declaratively rather than executing arbitrary frontend code or driving generalized interaction flows.

## Decisions Captured In This Proposal

- Result-level rich output uses the term `view` to avoid confusion with existing plugin manifest `presentation` metadata.
- Rich results normalize into a stable envelope with transcript text plus an optional session-scoped `view`.
- The first implementation will standardize only `todo_list`, `status`, and `progress` view kinds.
- `view_mode` semantics will be explicit for `replace`, `append`, and `transient`.
