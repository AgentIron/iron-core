## Context

`iron-core` already supports WASM-backed plugins as executable tool providers, and plugin tools already return structured JSON values through the runtime. The gap is not that plugins cannot return structure; the gap is that the runtime and clients do not yet share a standard way to treat some plugin results as **rich view state** rather than opaque generic values.

This matters most for plugin outputs that are useful to view visually but do not require user interaction through the plugin itself, such as:

- todo/task state owned by the model or plugin
- status panels
- progress views
- tables and dashboards

The goal of this change is to define a clean rich-output protocol for plugin results. It is **not** to define model-originated user choice prompts or a generalized plugin action framework.

## Goals / Non-Goals

**Goals**
- Allow plugin tool calls to return plugin-authored transcript text plus optional structured `view` payloads.
- Expose `view` payloads through runtime/facade result surfaces so clients can render them richly when supported.
- Provide stable `view` identity and update semantics so rich clients can replace or append UI surfaces without transcript scraping.
- Preserve compatibility for chat-only clients by requiring plugin-authored transcript text.

**Non-Goals**
- Generalized plugin-defined user actions.
- Model-originated user choice/disambiguation prompts.
- Runtime-owned auth workflows.
- Arbitrary executable frontend code.
- A full widget/layout framework.

## Decisions

### Plugins author transcript text and rich view payloads separately
Plugin results should contain a transcript channel and an optional rich `view` channel.

The transcript text is authored by the plugin and is required whenever a rich `view` payload is emitted. This is preferable to runtime-generated markdown/text because `iron-core` should not make domain assumptions or attempt generic diffing of rich state.

### Result-level rich output uses `view`, not `presentation`
The repo already uses `presentation` in plugin manifest metadata (`PresentationMetadata`) to describe the plugin itself. The result-level rich-output channel should therefore use the term `view`.

This is preferable to overloading `presentation` for both manifest metadata and per-tool rich result payloads.

### View state is declarative and additive
The runtime should treat rich `view` payloads as declarative structured data. Clients may render these payloads richly, but the rich-output channel is additive — it does not replace transcript continuity.

### View state needs stable identity and update semantics
Rich `view` payloads should include:

- `view_id`: stable identity for a replaceable/appendable client surface within a session
- `view_mode`: at least `replace`, `append`, and `transient`

This is preferable to treating every plugin rich result as a new chat message because rich clients need a way to update an existing surface such as a todo panel without spamming the transcript.

### View identity is session-scoped
`view_id` should be stable within a session, not globally across the runtime.

This is preferable to introducing unnecessary global identity constraints. Rich clients need a stable handle for reconciliation during a session, not cross-session portability guarantees.

### Chat-only clients remain first-class
The protocol must remain safe for clients that implement only a chat transcript. Those clients may ignore the `view` channel entirely and still present plugin-authored transcript text coherently.

This is preferable to assuming clients have sidebars, panels, or replaceable widgets.

### View kinds stay small and view-oriented in v1
The first version should focus on view/state payloads rather than interactive workflows. The supported v1 kinds should be:

- `todo_list`
- `status`
- `progress`

This is preferable to mixing rich output with generalized interaction because plugin-owned view state, model-owned user choice, and runtime-owned auth are separate concerns.

### Unknown or executable rich payloads are rejected, not sanitized
The runtime should reject unknown view kinds and reject executable or arbitrary frontend payloads.

This is preferable to best-effort sanitization because rejection gives clients and plugin authors a clear, deterministic contract.

### Transcript text always remains chronologically normal
Transcript text from plugin results should continue to flow through ordinary transcript ordering, while `view_mode` affects only rich-client rendering behavior.

This is preferable to coupling transcript ordering to replace/append semantics and keeps chat-only clients simple.

## Proposed Result Model

At a conceptual level, plugin tool execution should normalize into something like:

```json
{
  "kind": "plugin_tool_result",
  "transcript": {
    "text": "Updated todo list: completed 'Review tool efficiency'."
  },
  "view": {
    "id": "todo:session:abc123",
    "mode": "replace",
    "payload": {
      "kind": "todo_list",
      "title": "Current Tasks",
      "items": [
        { "id": "task_1", "label": "Review tool efficiency", "done": true },
        { "id": "task_2", "label": "Add plugin rich output", "done": false }
      ]
    }
  },
  "metadata": {
    "plugin_id": "todo-plugin",
    "tool_name": "update_todo_state"
  }
}
```

## Client UX Model

### Chat-only client
Uses only transcript text:

```text
Updated todo list: completed 'Review tool efficiency'.
```

### Rich client
Uses transcript text plus replaceable view state:

```text
Transcript
┌───────────────────────────────────────────────┐
│ Updated todo list: completed 'Review tool    │
│ efficiency'.                                 │
└───────────────────────────────────────────────┘

View surface (same view_id replaced over time)
┌───────────────────────────────────────────────┐
│ Current Tasks                                │
│ [x] Review tool efficiency                   │
│ [ ] Add plugin rich output                   │
└───────────────────────────────────────────────┘
```

## Public Runtime Shape

The current runtime already carries opaque JSON tool results end-to-end. This change should therefore focus on normalization and explicit client-visible fields rather than inventing a new transport.

At a conceptual level, rich-capable client surfaces should receive something equivalent to:

```text
ToolResult
├── status
├── raw result JSON                (compatibility path)
├── transcript_text                (normalized fallback path)
└── view                           (normalized rich path)
```

This preserves compatibility while ensuring rich-capable clients do not need to scrape transcript text or reverse-engineer arbitrary JSON to find supported rich payloads.

## View Mode Semantics

- `replace`: the payload is the latest canonical state for the same `view_id`
- `append`: the payload is an additional entry associated with the same `view_id` and does not replace prior entries
- `transient`: the payload may be rendered ephemerally and should not be assumed to be the durable canonical state for the same `view_id`

These semantics apply to the rich `view` channel only. Transcript text remains chronologically appended as usual.

## Risks / Trade-offs

- **Transcript and rich view state can drift** if plugin authors them carelessly.
  - Mitigation: make transcript text mandatory and document that it should summarize the meaningful state change, not mirror the full widget.

- **Even a small v1 kind set can be underspecified** if schemas stay too loose.
  - Mitigation: define concrete schemas and required fields for `todo_list`, `status`, and `progress`.

- **Clients may interpret view modes differently**.
  - Mitigation: define explicit semantics for `replace`, `append`, and `transient` in the spec and preserve them through runtime surfaces.

- **This does not solve user-choice or auth UX by itself**.
  - Mitigation: track those as separate changes with their own runtime/model ownership.

- **Result-level `presentation` terminology would collide with manifest metadata**.
  - Mitigation: use `view` for tool-result rich output and keep manifest `presentation` for plugin metadata only.

## Migration Plan

1. Define the normalized plugin rich-result envelope.
2. Extend plugin execution result handling to parse and validate that envelope.
3. Add additive result/event fields so clients can observe transcript and `view` channels distinctly.
4. Preserve transcript-only behavior for clients that ignore `view`.
5. Add tests for text-only and rich-capable client paths, including invalid rich payload rejection.

## Open Questions

- Do we want to expose the normalized `view` fields as additive facade fields alongside the raw result JSON, or replace the generic result shape for plugin-rich results entirely?
- How much metadata should be preserved in the normalized result envelope for client correlation and tooling beyond `plugin_id` and `tool_name`?
