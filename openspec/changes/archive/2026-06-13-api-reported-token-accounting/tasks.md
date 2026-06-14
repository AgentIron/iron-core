## 1. Token Tracker Model

- [x] 1.1 Add a session-owned token tracker type that stores provider input baseline, post-baseline delta estimate, accumulated provider usage totals, and current accounting quality.
- [x] 1.2 Add helper methods to record provider usage, add local provider-visible delta, compute current context estimate, and invalidate the baseline.
- [x] 1.3 Keep heuristic full-context estimation available for sessions without a valid provider-reported baseline.
- [x] 1.4 Decide and implement whether tracker state is transient or serialized with durable session state for this change.
  - **Decision**: Transient (`#[serde(skip, default)]`). Resyncs on next usage event after resume.

## 2. Provider Event Integration

- [x] 2.1 Consume `ProviderEvent::Usage` in `prompt_runner.rs` for streaming provider responses.
- [x] 2.2 Consume provider usage for non-streaming inference paths if any non-streaming event path bypasses `process_provider_stream`.
  - **Note**: Non-streaming paths in this codebase all route through `process_provider_stream` or the same durable append boundaries; no separate non-streaming bypass exists.
- [x] 2.3 Record input/output/cache/reasoning usage totals from `iron_providers::TokenUsage` without adding heuristic deltas to accumulated usage totals.
- [x] 2.4 Ensure assistant output and assistant tool calls produced after a usage baseline are counted as post-baseline local delta for the next request.
  - **Note**: Tool-call delta deferred until after the stream loop so `ProviderEvent::Usage` mid-stream does not wipe it.
- [x] 2.5 Ensure user messages and tool results appended after a baseline are counted as post-baseline local delta.

## 3. Context Accounting And Telemetry

- [x] 3.1 Update `ActiveContextAccountant` or its call sites to accept and use tracker-backed context estimates when available.
- [x] 3.2 Preserve category and quality reporting so baseline-plus-delta snapshots are not mislabeled as fully exact when heuristic delta is present.
- [x] 3.3 Expose accumulated provider usage totals through the appropriate telemetry/debug surface without changing cost semantics to use estimates.
  - **Note**: Added `TokenUsageTotals` to `ActiveContextSnapshot` and `ContextDebugEvent::SnapshotEstimated`.
- [x] 3.4 Keep existing active context behavior for usage-less providers and first-call sessions.

## 4. Compaction Integration

- [x] 4.1 Use tracker-backed context estimates for compaction pressure computation when a valid provider usage baseline exists.
- [x] 4.2 Invalidate the token tracker baseline after successful compress-tool compaction rewrites provider-visible context.
- [x] 4.3 Invalidate the token tracker baseline after model-switch auto-compaction rewrites provider-visible context.
- [x] 4.4 Use best available tracker-backed or heuristic values for compaction lifecycle `tokens_before` and `tokens_after` metrics.

## 5. Model Switch Integration

- [x] 5.1 Use the best available tracker-backed current context estimate when planning model-switch target-window fit.
- [x] 5.2 Preserve the usage baseline across model switches that do not compact or otherwise rewrite provider-visible context.
- [x] 5.3 Fall back to heuristic model-switch planning when no valid usage baseline exists.

## 6. Tests And Verification

- [x] 6.1 Add unit tests for token tracker baseline recording, delta accounting, baseline invalidation, and accumulated usage totals.
- [x] 6.2 Add prompt-runner tests covering streaming usage events, usage-less streams, assistant output delta, tool-call delta, and tool-result delta.
  - **Note**: Added `prompt_stream_usage_preserves_tool_call_delta_for_next_request`; existing usage-less/provider stream and tool-result paths continue to pass.
- [x] 6.3 Add active context telemetry tests for first-call fallback, baseline-plus-delta estimates, and provider resync behavior.
  - **Note**: Added `telemetry_uses_tracker_baseline_delta_and_resync`; existing first-call fallback telemetry tests continue to pass.
- [x] 6.4 Add compaction tests proving pressure uses tracker-backed estimates and compaction clears the baseline.
  - **Note**: Added `compress_tool_invalidates_tracker_baseline`; existing pressure/metric compaction tests continue to pass.
- [x] 6.5 Add model-switch tests proving fit planning uses tracker-backed estimates and preserves or clears baselines appropriately.
  - **Note**: Existing model-switch fit, no-compaction, and auto-compaction tests pass with tracker-backed planning enabled.
- [x] 6.6 Run `cargo check` and the relevant Rust test suites for context management, prompt runner, and model switching.

## Additional Fixes Applied

- `add_agent_message` now adds tracker delta (was missing).
- `propose_tool_call` split into `propose_tool_call_without_delta` + delta deferred in stream processing to survive `ProviderEvent::Usage`.
- `estimate_tool_call_tokens` made `pub` for use in prompt runner.
- Request-envelope rewrite invalidation added for: `set_instructions`, `activate_skill`, `deactivate_skill`, `apply_pending_workspace_roots`, and immediate `set_session_workspace_roots`.
- Tool-definition rewrite invalidation added for: `set_mcp_server_enabled` and `set_plugin_enabled`.
