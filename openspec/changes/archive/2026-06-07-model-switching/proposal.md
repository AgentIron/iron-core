## Why

iron-core sessions are bound to a specific model at creation time. Users need to switch models mid-session (e.g., from a large-context model to a faster one, or from one provider to another) without losing conversation continuity. The current `HandoffBundle` supports cross-session transfer but lacks target-aware context adaptation, capability reconciliation, and turn-boundary semantics needed for seamless model switching.

## What Changes

- **Model Switch Request API**: Add `AgentSession::switch_model()` to initiate a model change at the next turn boundary
- **Continuation Export/Import**: Extend `HandoffBundle` with target-aware context planning that adapts conversation state to the new model's constraints
- **Context Window Adaptation**: When switching to a model with a smaller context window, automatically compact older context while preserving the recent tail and task intent
- **Capability Reconciliation**: Compare source and target model capabilities (tools, modalities, context window) and report what features are gained or lost
- **Timeline Event Recording**: Add `ModelSwitched` timeline entry to record the continuation boundary for auditability
- **Turn-Boundary Semantics**: Apply model switches only at turn boundaries; active prompts complete or are cancelled before switching
- **Per-Turn Model Recording**: Record the model used for each turn in session metadata
- **BREAKING**: `HandoffBundle` structure will be extended with new fields (backward-compatible serialization)

## Capabilities

### New Capabilities

- `model-switching`: Core capability for switching models mid-session while preserving conversation continuity, including context adaptation, capability reconciliation, and turn-boundary semantics

### Modified Capabilities

- `context-compaction`: Extend compaction to support target-model-aware context sizing and pre-switch compaction when the target window is smaller than the current context

## Impact

- **Core**: `AgentSession`, `DurableSession`, `IronRuntime`, `HandoffBundle`, `HandoffExporter`, `HandoffImporter`
- **Context Management**: `ContextManagementConfig`, compaction engine, token estimation
- **Provider Layer**: `ProviderPromptContext`, `ProviderRegistry` (per-model metadata)
- **API Surface**: New `AgentSession::switch_model()` method; modified `export_handoff()` signature
- **Client UX**: Model switch events in timeline; capability difference reports
- **Serialization**: `HandoffBundle` v2 format with backward-compatible v1 reading
