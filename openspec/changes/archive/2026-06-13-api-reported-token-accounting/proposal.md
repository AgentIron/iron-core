## Why

Context pressure, compaction, and model-switch fit decisions currently depend on rough full-context token heuristics that drift across long sessions. `iron-providers` now exposes provider-reported token usage, so `iron-core` can use API input-token counts as the authoritative baseline and estimate only the small local delta since the last usage-bearing response.

## What Changes

- Add session-level token accounting that records provider-reported input, output, cache, and reasoning token usage from `ProviderEvent::Usage`.
- Estimate current provider-visible context as the latest provider-reported input-token baseline plus locally estimated transcript/request deltas added after that baseline.
- Keep heuristic token counting only as a fallback and delta estimator for first calls, usage-less providers, usage-less stream responses, and invalidated baselines.
- Reset or invalidate the usage baseline when provider-visible context is rewritten by compaction.
- Use tracker-backed context estimates for active context telemetry, context pressure, post-turn critical pressure checks, compaction metrics where available, and model-switch fit planning.
- Expose provider-reported accumulated token totals for telemetry and future cost accounting without deriving billing totals from heuristic estimates.
- Preserve compatibility with providers that do not emit usage by retaining the existing estimated behavior and quality labeling.

## Capabilities

### New Capabilities

- `context-token-accounting`: Tracks provider-reported token usage, baseline-plus-delta context estimates, fallback quality, and accumulated usage telemetry.

### Modified Capabilities

- `context-compaction`: Compaction pressure and finish metrics use tracker-backed token estimates when provider usage is available, and compaction invalidates the usage baseline after rewriting provider-visible context.
- `model-switching`: Model-switch target-window fit uses the best available tracker-backed context estimate while preserving model-switch resync behavior.

## Impact

- Affected Rust modules include prompt runner provider-event processing, durable session/accounting state, active context telemetry, compaction pressure computation, model-switch planning, debug telemetry, and facade APIs.
- Depends on `iron-providers` 0.2.9 or newer for `ProviderEvent::Usage` and `TokenUsage`.
- The `TokenCounter`/tokenizer-library direction is explicitly out of scope; no tokenizer crate is added.
- Existing heuristic behavior remains as fallback for first calls and providers or streams that do not report usage.
