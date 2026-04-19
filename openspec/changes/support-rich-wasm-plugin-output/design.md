## Context

`iron-core` already supports WASM-backed plugins as executable tool providers, and plugin tools already return structured JSON values through the runtime. The gap is not that plugins cannot return structure; the gap is that the runtime and clients do not yet share a standard way to treat some plugin results as **presentation state** rather than opaque generic values.

This matters most for plugin outputs that are useful to view visually but do not require user interaction through the plugin itself, such as:

- todo/task state owned by the model or plugin
- status panels
- progress views
- tables and dashboards

The goal of this change is to define a clean presentation protocol for plugin results. It is **not** to define model-originated user choice prompts or a generalized plugin action framework.

## Goals / Non-Goals

**Goals**
- Allow plugin tool calls to return plugin-authored transcript text plus optional structured presentation payloads.
- Expose presentation payloads through runtime/facade result surfaces so clients can render them richly when supported.
- Provide stable presentation identity and update semantics so rich clients can replace or append UI surfaces without transcript scraping.
- Preserve compatibility for chat-only clients by requiring plugin-authored transcript text.

**Non-Goals**
- Generalized plugin-defined user actions.
- Model-originated user choice/disambiguation prompts.
- Runtime-owned auth workflows.
- Arbitrary executable frontend code.
- A full widget/layout framework.

## Decisions

### Plugins author transcript text and presentation payloads separately
Plugin results should contain a transcript channel and an optional presentation channel.

The transcript text is authored by the plugin and is required whenever a presentation payload is emitted. This is preferable to runtime-generated markdown/text because `iron-core` should not make domain assumptions or attempt generic diffing of rich state.

### Presentation is declarative and additive
The runtime should treat presentation payloads as declarative structured data. Clients may render these payloads richly, but the presentation channel is additive — it does not replace transcript continuity.

### Presentation needs stable identity and update semantics
Presentation payloads should include:

- `presentation_id`: stable identity for a replaceable/appendable client surface within a session
- `presentation_mode`: at least `replace`, `append`, and `transient`

This is preferable to treating every plugin presentation result as a new chat message because rich clients need a way to update an existing surface such as a todo panel without spamming the transcript.

### Chat-only clients remain first-class
The protocol must remain safe for clients that implement only a chat transcript. Those clients may ignore the presentation channel entirely and still present plugin-authored transcript text coherently.

This is preferable to assuming clients have sidebars, panels, or replaceable widgets.

### Presentation kinds stay small and view-oriented in v1
The first version should focus on view/state presentation kinds rather than interactive workflows. Candidate kinds include:

- `todo_list`
- `status`
- `progress`
- `table`
- `card`

This is preferable to mixing presentation with generalized interaction because plugin-owned presentation, model-owned user choice, and runtime-owned auth are separate concerns.

## Proposed Result Model

At a conceptual level, plugin tool execution should normalize into something like:

```json
{
  "transcript": {
    "text": "Updated todo list: completed 'Review tool efficiency'."
  },
  "presentation_id": "todo:session:abc123",
  "presentation_mode": "replace",
  "presentation": {
    "kind": "todo_list",
    "title": "Current Tasks",
    "items": [
      { "id": "task_1", "label": "Review tool efficiency", "done": true },
      { "id": "task_2", "label": "Add plugin rich output", "done": false }
    ]
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
Uses transcript text plus replaceable presentation state:

```text
Transcript
┌───────────────────────────────────────────────┐
│ Updated todo list: completed 'Review tool    │
│ efficiency'.                                 │
└───────────────────────────────────────────────┘

Presentation surface (same presentation_id replaced over time)
┌───────────────────────────────────────────────┐
│ Current Tasks                                │
│ [x] Review tool efficiency                   │
│ [ ] Add plugin rich output                   │
└───────────────────────────────────────────────┘
```

## Risks / Trade-offs

- **Transcript and presentation can drift** if plugin authors them carelessly.
  - Mitigation: make transcript text mandatory and document that it should summarize the meaningful state change, not mirror the full widget.

- **Too many presentation kinds too early** will make the model unstable.
  - Mitigation: keep v1 limited to a small set of view-oriented kinds.

- **Clients may interpret presentation modes differently**.
  - Mitigation: define clear semantics for `replace`, `append`, and `transient`.

- **This does not solve user-choice or auth UX by itself**.
  - Mitigation: track those as separate changes with their own runtime/model ownership.

## Migration Plan

1. Define the plugin presentation result envelope.
2. Extend plugin execution result handling to parse and validate that envelope.
3. Add additive result/event fields so clients can observe transcript and presentation channels distinctly.
4. Preserve transcript-only behavior for clients that ignore presentation.
5. Add tests for text-only and rich-capable client paths.

## Open Questions

- Which v1 presentation kinds are worth standardizing immediately beyond `todo_list`, `status`, and `progress`?
- Should `presentation_mode` apply only to rich surfaces, while transcript text is always appended normally to chat?
- How much metadata should be preserved in the normalized result envelope for client correlation and tooling?
