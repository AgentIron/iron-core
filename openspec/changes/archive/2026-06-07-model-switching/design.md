## Context

iron-core currently binds sessions to a model at creation time. The `HandoffBundle` supports cross-session transfer but lacks target-aware context adaptation, capability reconciliation, and turn-boundary semantics needed for seamless model switching. This design builds on the existing `HandoffBundle`, `DurableSession`, and `IronRuntime` architecture to add model switching as a first-class operation.

## Goals / Non-Goals

**Goals:**

- Allow users to switch models mid-session without losing conversation continuity
- Preserve the same session identity (`SessionId`) across model switches
- Apply model switches only at turn boundaries (when session is idle)
- Automatically adapt context when switching to a model with a smaller context window
- Reconcile capabilities (tools, modalities) between source and target models
- Record model switch events in the session timeline for auditability
- Support both managed (provider slug + model ID) and unmanaged (direct `Provider`) switching
- Provide clear UX feedback about switch status and capability differences

**Non-Goals:**

- Mid-turn model switching (switching while a prompt is actively streaming)
- Forking sessions on model switch (always continue the same session)
- Migrating in-flight tool execution state to the new model
- Automatic model selection or fallback (user-driven only)
- Provider-side prompt cache migration or warmup
- Token counting accuracy improvements (tracked separately in GH#34)

## Decisions

### Use Export/Import Planning for Context Adaptation

Rather than mutating session state in place, model switching uses the existing `HandoffBundle` export/import path with target-aware planning:

1. Export current durable state to a `ContinuationBundle`
2. Create a `ModelSwitchPlan` that compares source/target constraints
3. Apply context compaction if target window is smaller
4. Import adapted state back into the same session
5. Record a `ModelSwitched` timeline entry

Rationale: Reusing the handoff path leverages existing serialization, compaction, and hydration logic. The export/import abstraction cleanly separates "what the conversation means" from "how the target model receives it."

Alternative considered: mutate `DurableSession` directly. Rejected because it entangles context adaptation logic with session state management and makes testing harder.

### Preserve Session Identity, Record Boundary Event

The same `SessionId` continues across switches. A `ModelSwitched` timeline entry marks the boundary:

```
timeline: [
  UserMessage { ... },
  AgentMessage { ... },
  ModelSwitched { from: "gpt-4o", to: "claude-sonnet-4", adapted: true },
  UserMessage { ... }, // uses new model
]
```

Rationale: Users expect continuity. The boundary event provides auditability and allows clients to render a "Switched to..." message without synthetic transcript injection.

Alternative considered: create a new session ID. Rejected because it breaks continuity expectations and complicates client state management.

### Turn-Boundary Application Only

Model switches are queued if requested during an active turn and applied when the session becomes idle:

```
SessionState::Idle -> apply switch immediately
SessionState::Active -> queue switch, apply on turn completion
```

Rationale: In-flight provider requests, tool execution, and streaming state are ephemeral and provider-specific. Migrating them mid-turn introduces significant complexity and edge cases.

Alternative considered: abort active turn and apply immediately. Rejected because it risks losing user-visible output and creates confusing UX.

### Target-Aware Compaction Uses Existing Compress Tool

When the target window is smaller, the runtime uses the existing model-driven `compress` tool to compact context:

1. Estimate current context size vs target window
2. If oversized, trigger a compaction turn using the compress tool
3. The model selects ranges to summarize
4. Apply compression, verify fit
5. If still oversized, report error

Rationale: The existing compress tool is model-driven and produces better summaries than runtime heuristics. It already handles structural protections and validation.

Alternative considered: runtime-imposed summarization. Rejected because it produces lower quality summaries and duplicates the existing compress infrastructure.

### Capability Reconciliation is Reported, Not Blocking

Capability differences (unsupported tools, modalities) are reported to the client but do not block the switch:

```rust
pub struct CapabilityDiff {
    pub hidden_tools: Vec<String>,
    pub unsupported_modalities: Vec<String>,
    pub window_shrink: Option<usize>, // tokens lost
}
```

Rationale: Users knowingly make lossy switches. Blocking on capability differences creates friction for legitimate use cases (e.g., switching to a cheaper model temporarily).

Alternative considered: require explicit confirmation for any loss. Rejected because it adds UX friction without clear benefit.

## Risks / Trade-offs

- **Context quality after compaction**: Compacted summaries may lose nuance → Mitigation: preserve recent tail verbatim; compress tool prompt emphasizes preserving durable facts
- **Token estimation inaccuracy**: Heuristic token counts may misjudge fit → Mitigation: conservative estimates; clear error when context cannot fit
- **Provider format differences**: Different providers may interpret the same transcript differently → Mitigation: iron-providers normalizes message format; provider-specific quirks are handled at the provider layer
- **Tool state portability**: Tool results from one model may not be interpretable by another → Mitigation: tool results are provider-neutral JSON; capability reconciliation hides unsupported tools
- **Session bloat**: Long-running sessions with many model switches accumulate history → Mitigation: compaction is available; switch metadata is lightweight

## Migration Plan

1. Add `ModelSwitchPlan` and `CapabilityDiff` types
2. Extend `HandoffBundle` with model switch metadata (backward-compatible)
3. Add `AgentSession::switch_model()` method
4. Implement turn-boundary queuing in `IronRuntime`
5. Add `ModelSwitched` timeline entry type
6. Update request builder to record per-turn model
7. Add capability reconciliation logic
8. Update tests and documentation

Rollback: Revert to previous version; sessions with `ModelSwitched` entries will display the entry as an unknown timeline type (graceful degradation).

## Open Questions

1. Where to store per-model context window and capability metadata? `iron-providers` currently lacks per-model specs.
2. Should the runtime support switching to a provider that requires different credentials without reconstructing the `IronAgent`?
3. How to handle model switches in the Tauri/SolidJS frontend? Does the frontend need to reload the session?
4. Should we support "switch and compact" as a single operation, or separate "compact then switch" steps?
5. How to surface capability differences in the client protocol (ACP)? New event type or extend existing?
