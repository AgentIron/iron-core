## Why

WASM plugins currently fit into the runtime through the tool-call surface, which means their client-visible output is effectively constrained to tool definitions, approval state, status, and text-like result payloads. That is enough for simple automation, but it is not enough for a good client experience when a plugin wants to present richer read-only or replaceable state such as todo lists, status panels, tables, progress views, or structured dashboards.

Today a plugin can return text, but there is no runtime-standard way for a client to know that a plugin result should be rendered as something richer than transcript output. As a result, plugin UX is bottlenecked by plain-text rendering even when the plugin and client could support a better experience.

## What Changes

- Introduce a runtime-standard, client-visible presentation channel for WASM plugin tool calls that is distinct from transcript text.
- Define a structured plugin result envelope that carries plugin-authored transcript text plus optional structured presentation payloads.
- Expose plugin-originated UI payloads through the runtime/facade event surfaces so clients can render richer experiences without scraping tool-result text.
- Define capability boundaries so plugins can request structured presentation without directly owning client lifecycle, arbitrary frontend execution, user-choice prompting, or general runtime actions.
- Establish a small v1 set of supported presentation payload types and lifecycle semantics (`presentation_id` and presentation update mode) that clients can implement consistently.
- Preserve compatibility for text-only clients by requiring plugin-authored transcript text alongside any rich presentation payload.

## Capabilities

### New Capabilities
- `plugin-rich-output`: Allows WASM plugin tool calls to return plugin-authored transcript text plus structured client-renderable presentation payloads so clients can present richer plugin experiences than plain text alone.

### Modified Capabilities
- `wasm-integration-plugins`: Extends plugin execution and inspection semantics so plugin tool results can include structured presentation metadata and client-visible rendering payloads.

## Impact

- Affected code: likely `src/plugin/*`, `src/mcp/session_catalog.rs`, `src/events.rs`, `src/facade.rs`, runtime tool execution/result propagation, and plugin-related tests.
- Affected APIs: plugin tool result contracts, event/facade surfaces for tool results, and client-facing runtime inspection/rendering of plugin outputs.
- Client impact: clients can choose to render structured UI payloads when supported, while text-only clients continue to use fallback transcript text.
- Security and UX impact: the runtime must constrain what plugins can request so plugins describe presentation state declaratively rather than executing arbitrary frontend code or driving generalized interaction flows.
