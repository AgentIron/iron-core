## Context

`iron-core` now treats compaction as an explicit runtime-owned `compress` tool for model-driven context management. The existing `compress` path already produces useful result JSON with rough token estimates and emits debug-only compaction diagnostics, but clients do not have a stable prompt-stream lifecycle contract for all compaction paths.

AgentIron consumes `iron_core::PromptEvent` directly in its Tauri state layer and bridges those events to frontend Tauri events. Its UI already has dormant affordances for a compacting status, compaction token metrics, and a context threshold marker, but relying on tool-name heuristics is fragile and does not cover non-tool compaction such as model-switch/window-shrink adaptation.

The current estimates are intentionally rough. Exact accounting should be handled by issue #34 rather than blocking this client-facing lifecycle contract.

## Goals / Non-Goals

**Goals:**

- Provide a stable public prompt event contract for compaction start, successful finish, and failure.
- Emit compaction lifecycle events for every compaction path, not only the model-visible `compress` tool.
- Include `tokens_before`, `tokens_after`, and `method` on successful finish events using the current estimates.
- Surface the effective absolute compaction threshold in active context telemetry when context management is enabled.
- Keep the event contract simple enough for AgentIron and other clients to map directly to UI state.
- Pin CI and release Rust toolchains to `1.96.0`.

**Non-Goals:**

- Improve the accuracy of token estimates; issue #34 owns that work.
- Add reasoning-effort model metadata.
- Replace or remove the model-visible `compress` tool.
- Add a new external protocol dependency for compaction events.
- Redesign AgentIron UI behavior in this repository.

## Decisions

1. Add explicit prompt events instead of treating `ToolCall(compress)` and `ToolResult(compress)` as the lifecycle contract.

   Explicit events cover both tool-driven and non-tool compaction paths. They also avoid downstream clients needing to know that the current internal tool is named `compress` while older UI stubs looked for names such as `compact` or `compaction`.

   Alternative considered: make `ToolCall(compress)` mean started and `ToolResult(compress)` mean finished. That is lower friction for the model-driven path, but it does not describe model-switch compaction and would make the UI contract depend on a specific tool name.

2. Use three lifecycle events: `CompactionStarted`, `CompactionFinished`, and `CompactionFailed`.

   `CompactionFinished` represents successful compaction only, so success metrics can remain required. `CompactionFailed` carries failure details without forcing clients to inspect a status field or handle partial metrics.

   Alternative considered: one `CompactionFinished { status, error }` event. That makes the event shape more ambiguous and increases the chance that clients forget to clear compacting state on failures.

3. Include a correlation identifier on lifecycle events.

   Each compaction attempt should have a stable `compaction_id` so clients can pair start, finish, and failure events even if future implementations allow multiple compaction attempts within a prompt turn.

   Alternative considered: no identifier because compaction is currently sequential. That is simpler today but gives clients no robust pairing mechanism.

4. Reuse the current token estimates and method labels.

   The model-driven `compress` path already returns `tokens_before`, `tokens_after`, and `method` using current rough estimates. Model-switch compaction should emit comparable values from the estimates it already computes. Exact accounting can replace these values later without changing the event contract.

   Alternative considered: wait for issue #34 before exposing metrics. That would delay the UI lifecycle work even though rough values are already useful and explicitly acceptable for now.

5. Surface an absolute threshold token value in active context telemetry.

   AgentIron expects `compactThreshold` as an absolute token count and renders it relative to the active model context window. In `iron-core`, this should map to the effective compaction threshold in tokens. With the current configuration model, that is `maintenance_threshold` when context management is enabled.

   Alternative considered: expose ratio thresholds such as `soft_threshold`. Ratios are useful internally for pressure buckets, but clients need the absolute marker position without reconstructing core configuration logic.

## Risks / Trade-offs

- Public API expansion could require downstream client updates -> Keep event payloads small, serializable, and additive.
- Tool-driven compaction may produce both tool events and compaction lifecycle events -> Treat tool events as transcript/tool visibility and compaction events as semantic UI state.
- Failure events can be missed if they are only emitted after parsing succeeds -> Generate the compaction identifier and emit `CompactionStarted` before fallible work, then emit `CompactionFailed` for validation or execution errors.
- Model-switch compaction may happen outside an active prompt stream -> Emit through the same public event path available to clients for model switch activity, or document any cases where only subsequent telemetry can observe it.
- Rough token estimates may be interpreted as exact -> Use existing estimate values for now and leave exactness improvements to issue #34.

## Migration Plan

- Add the new prompt event variants in a backward-compatible way for Rust consumers that update intentionally.
- Thread lifecycle events through internal prompt lifecycle plumbing and facade stream conversion before changing compaction call sites.
- Update compaction call sites to emit start, finish, and failure events.
- Update active context telemetry to return `compact_threshold_tokens` for client use.
- Pin workflow toolchains to Rust `1.96.0`.

## Open Questions

- None for this proposal. Exact token accounting remains deferred to issue #34.
