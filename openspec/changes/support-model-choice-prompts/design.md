## Context

`iron-core` already supports turn pausing for approval workflows, but not for model-originated user disambiguation. This means a model that needs the user to choose among bounded options has no first-class structured interaction path. Falling back to freeform chat text loses option identity and forces clients and prompts to rely on prose rather than semantics.

This change defines a runtime interaction model for **model-originated choice prompts** by generalizing the existing approval pause/resume path into a shared pending-interaction envelope. It remains separate from plugin presentation and separate from runtime-owned auth flows.

## Goals / Non-Goals

**Goals**
- Keep pause/resume orchestration DRY across approval and choice workflows.
- Allow the model/runtime to request a structured user choice.
- Pause the current turn until the user submits or cancels that choice.
- Resume the same turn after resolution.
- Preserve structured option identity through the client response path.

**Non-Goals**
- Freeform user text entry workflows.
- Plugin-defined interaction prompts.
- Runtime-owned auth actions.
- A full generalized form system.

## Decisions

### Choice requests are model-originated and turn-blocking
When the model needs a bounded user choice before it can proceed, the runtime should treat that as a blocking interaction and pause the current turn.

This is preferable to creating a new turn because the model is waiting on a missing input within the current reasoning path.

### The provider/runtime protocol uses a first-class choice-request event
Model-originated choice prompts should enter the runtime as a first-class structured provider/runtime event rather than as assistant text or a fake tool call.

The preferred shape is a provider-level event such as:

```rust
ProviderEvent::ChoiceRequest {
    prompt: String,
    selection_mode: ChoiceSelectionMode,
    items: Vec<ChoiceItem>,
}
```

This keeps model-originated disambiguation distinct from tool execution and avoids brittle transcript parsing.

### Approval and choice share one pending-interaction envelope
Approval and choice should use the same turn-level pause/resume machinery rather than separate branching code paths.

The shared abstraction should cover:
- entering a waiting state
- exposing the pending interaction through event/status APIs
- validating client-provided resolutions
- resuming the same turn after resolution

Interaction-specific payloads and resolution rules should remain distinct.

### Only one blocking interaction envelope is allowed per turn in v1
The turn should expose at most one blocking pending interaction at a time.

This keeps turn state, client UX, and resume semantics simple. The pending interaction envelope may contain multiple approval items, but the turn itself should not wait on multiple heterogeneous interactions simultaneously.

### Mixed or repeated blocking interaction requests are rejected deterministically in v1
The runtime should reject ambiguous interaction-producing provider output rather than guessing.

At minimum, v1 should fail deterministically if a single provider response attempts to produce:
- more than one choice request
- a choice request plus approval-gated tool calls in the same unresolved interaction phase
- any new blocking interaction while another interaction is already pending

### Choice prompts are first-class pending interaction payloads
Choice prompts should appear in event/status APIs as first-class interaction payloads, not as opaque tool results or ad hoc text.

The interaction object should include at least:
- `interaction_id`
- `source`
- a typed payload

For a choice payload, the object should include at least:
- `kind = choice`
- `prompt`
- `selection_mode`
- structured option items with stable IDs

For an approval payload, the object should include the batch of pending tool calls that require approval.

### Client responses are structured and validated
Clients should submit a structured interaction resolution containing the interaction ID and a typed resolution payload. The runtime should validate that the interaction is still pending, that the resolution kind matches the pending interaction kind, and that submitted option IDs were actually offered.

Validation should occur both:
- at the public API boundary (so invalid input is rejected explicitly)
- in the turn driver (so runtime state remains defensively protected)

### Approval uses a batch public resolution contract in v1
Under the single-envelope interaction model, approval should use a batch public resolution contract rather than a per-call public waiting API.

This keeps the interaction model honest: one pending interaction envelope yields one public interaction resolution payload. Internally, the runtime may still track partial decisions while adapting older approval-oriented code paths, but the canonical client-facing contract should resolve the approval interaction envelope as a batch.

### The model sees a canonical representation of the resolved choice
Even though the client response is structured, the model likely still needs a canonical serialized representation of the chosen option(s) in its continuation context. That representation should be runtime-defined and stable, not ad hoc prose.

The canonical choice-resolution representation should include:
- `kind = choice_resolution`
- `interaction_id`
- `status = submitted | cancelled`
- the original `prompt`
- `selection_mode`
- `selected_items` as a list of objects containing both stable `id` and human-readable `label`

For consistency, `selected_items` should always be a list, even for single-choice interactions.

### Resolved choices are persisted as structured runtime/system transcript entries
The canonical choice-resolution record should be stored in a structured runtime/system transcript entry rather than being injected as assistant text.

That transcript entry becomes the continuation artifact the model sees when the same turn resumes.

## Proposed Interaction Model

### Turn status and event shape

The public turn state should generalize from approval-specific waiting to interaction-specific waiting:

```rust
TurnStatus::Running
TurnStatus::WaitingForInteraction { pending: PendingInteractionInfo }
TurnStatus::Finished { outcome: TurnOutcome }

TurnEvent::InteractionRequired { interaction: PendingInteractionInfo }
```

This replaces approval-specific waiting/event types as the primary abstraction for blocking interactions.

### Pending interaction envelope

```json
{
  "interaction_id": "interaction_123",
  "source": "model",
  "payload": {
    "kind": "choice",
    "prompt": "I found multiple repositories. Please choose one.",
    "selection_mode": "single",
    "items": [
      { "id": "repo_1", "label": "agentiron/iron-core" },
      { "id": "repo_2", "label": "agentiron/iron-tui" }
    ]
  }
}
```

### Model-origin choice-request event

Before the runtime can expose a pending choice interaction envelope, the provider/runtime layer must surface a first-class choice-request event:

```json
{
  "type": "choice_request",
  "prompt": "I found multiple repositories. Please choose one.",
  "selection_mode": "single",
  "items": [
    { "id": "repo_1", "label": "agentiron/iron-core" },
    { "id": "repo_2", "label": "agentiron/iron-tui" }
  ]
}
```

The runtime should validate this event before converting it into `PendingInteractionPayload::Choice`.

### Approval interaction envelope

Existing approval batching should be preserved by modeling approval as a single pending interaction envelope containing one or more approval items:

```json
{
  "interaction_id": "interaction_approval_456",
  "source": "runtime",
  "payload": {
    "kind": "approval",
    "calls": [
      {
        "call_id": "call_1",
        "tool_name": "bash",
        "arguments": { "command": "rm -rf tmp" }
      },
      {
        "call_id": "call_2",
        "tool_name": "write",
        "arguments": { "path": "notes.txt", "content": "..." }
      }
    ]
  }
}
```

### Choice resolution

```json
{
  "interaction_id": "interaction_123",
  "resolution": {
    "kind": "choice",
    "status": "submitted",
    "selected_ids": ["repo_2"]
  }
}
```

or

```json
{
  "interaction_id": "interaction_123",
  "resolution": {
    "kind": "choice",
    "status": "cancelled"
  }
}
```

### Approval resolution

Approval remains batch-oriented in the public client contract. The client resolves the approval interaction envelope rather than individual turn wait states:

```json
{
  "interaction_id": "interaction_approval_456",
  "resolution": {
    "kind": "approval",
    "decisions": [
      { "call_id": "call_1", "verdict": "allow_once" },
      { "call_id": "call_2", "verdict": "deny" }
    ]
  }
}
```

### Canonical model-facing choice-resolution context

When resuming the paused turn, the runtime should inject a stable structured continuation record for resolved choices.

Submitted choice:

```json
{
  "kind": "choice_resolution",
  "interaction_id": "interaction_123",
  "status": "submitted",
  "prompt": "I found multiple repositories. Please choose one.",
  "selection_mode": "single",
  "selected_items": [
    {
      "id": "repo_2",
      "label": "agentiron/iron-tui"
    }
  ]
}
```

This record should be stored as a structured runtime/system transcript entry, not serialized into ordinary assistant text.

Cancelled choice:

```json
{
  "kind": "choice_resolution",
  "interaction_id": "interaction_123",
  "status": "cancelled",
  "prompt": "I found multiple repositories. Please choose one.",
  "selection_mode": "single",
  "selected_items": []
}
```

## Risks / Trade-offs

- **Pause/resume complexity**: turn state management becomes more complex.
  - Mitigation: unify with existing approval pause semantics instead of layering a second waiting subsystem beside it.

- **Generalization could overreach into untyped interaction blobs**.
  - Mitigation: share orchestration and waiting state, but keep approval and choice as typed interaction payloads and typed resolution payloads.

- **Clients may vary in presentation richness**.
  - Mitigation: keep the interaction object simple enough for chat-only clients to represent textually.

- **Model continuation needs a stable choice representation**.
  - Mitigation: define canonical runtime serialization for resolved choices.

- **Provider protocol changes may be broader than this runtime refactor**.
  - Mitigation: make choice request emission explicit in the design instead of pretending the runtime can infer it from existing tool-call or transcript behavior.

- **Public API rejection behavior may diverge from internal driver behavior if validation is only done in one place**.
  - Mitigation: validate interaction resolutions at both the handle/API boundary and the driver.

- **Approval currently batches multiple pending calls**.
  - Mitigation: define approval as one interaction envelope that may contain multiple approval items, while keeping the invariant that the turn itself has only one blocking pending interaction at a time.

## Migration Plan

1. Generalize approval-specific waiting/event/status types into shared pending-interaction envelope types.
2. Define approval and choice payload/resolution schemas within that shared interaction model.
3. Preserve compatibility for existing approval-oriented callers with wrappers or adapters where necessary while establishing the generalized interaction APIs as canonical.
4. Extend the provider/runtime protocol with a first-class model-originated choice-request event.
5. Extend turn/event/status APIs to expose pending interactions.
6. Add client-facing interaction resolution APIs for approval and choice with explicit rejection behavior for invalid submissions.
7. Resume paused turns with kind-specific continuation behavior, including structured runtime/system choice-resolution transcript entries.
8. Add end-to-end tests for choice request emission, pause/resume, cancellation, invalid resolution rejection, and approval compatibility.

## Open Questions

- Should per-call approval cancellation be represented distinctly from denial, or should approval only support allow/deny while whole-interaction cancellation remains the turn-level escape hatch?
- Which existing approval-specific public APIs should remain as compatibility wrappers, and which should be formally deprecated in favor of generalized interaction APIs?
- What concrete provider/runtime crate changes are required to surface `ChoiceRequest` in a backwards-compatible way?
