## Why

AgentIron needs reliable, client-visible context compaction state so the chat UI can show when compaction is running, render the compaction transcript entry, display token reduction metrics, and draw the context usage threshold marker. Today the richest compaction details are available only through debug diagnostics or tool-result payloads, and some compaction paths can happen without a stable client-facing lifecycle contract.

## What Changes

- Add explicit compaction lifecycle events to the public prompt event stream: `CompactionStarted`, `CompactionFinished`, and `CompactionFailed`.
- Emit lifecycle events for every compaction path, including model-driven `compress`, manual compact/checkpoint behavior, and model-switch/window-shrink compaction.
- Include rough token metrics in successful finish events: `tokens_before`, `tokens_after`, and `method`.
- Expose the effective absolute compaction threshold in active context telemetry so clients can render a threshold marker alongside active token usage.
- Pin Rust CI and release workflows to toolchain `1.96.0` with `rustfmt` and `clippy` components.
- Defer exact token accounting improvements to issue #34; this change reuses the current estimates.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `context-compaction`: add client-visible lifecycle events, success metrics, failure events, all-path event coverage, and active context threshold telemetry.

## Impact

- Public API: `iron_core::PromptEvent` gains compaction lifecycle variants.
- Internal event plumbing: prompt lifecycle and facade/ACP bridge paths must carry compaction events to clients.
- Context compaction: model-driven `compress`, manual compact/checkpoint, and model-switch compaction paths must emit consistent lifecycle events.
- Context telemetry: active context snapshots must include the effective absolute compaction threshold when context management is enabled.
- Tests: add coverage for lifecycle event emission, metrics, failures, threshold telemetry, and CI toolchain pinning.
- Downstream clients: AgentIron consumes `PromptEvent` in its Tauri state layer and already has dormant UI affordances for compacting status, compaction metrics, and threshold markers.
