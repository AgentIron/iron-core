## Why

`iron-core` currently has internal decisions that are difficult to inspect when behavior is wrong: prompt influences, context pressure hints, model-driven compaction, tool approval decisions, model switching, runtime configuration, and skill loading can all affect an agent turn without leaving a structured engine-level trace. This is becoming more important as context management and model switching grow, because debugging only from user-visible lifecycle events or process logs does not explain why the runtime made a decision.

## What Changes

- Add a typed debug observation primitive for engine-owned semantic breadcrumbs.
- Introduce a public `DebugSink` abstraction with a default no-op sink so embedders can observe runtime decisions without changing model-visible behavior.
- Introduce a stable debug event envelope carrying severity, scope/correlation metadata, and a domain payload.
- Introduce grouped, extensible debug payloads for prompt influences, context/compaction, tools, provider/model switching, runtime configuration, and skills.
- Emit phase-1 events at representative high-value hook points rather than attempting exhaustive coverage.
- Make redaction and privacy part of the primitive: phase-1 events expose names, counts, lengths, hashes/fingerprints, decisions, statuses, durations, and reasons, not raw prompts, credentials, tool arguments, tool results, or skill contents.
- Keep debug observation observational only: events are not durable transcript state, are not model-visible context, and must not be required for correct runtime behavior.

## Capabilities

### New Capabilities

- `debug-observation`: Defines the runtime debug observation API, event envelope, scope/correlation model, redaction expectations, and minimum phase-1 event coverage.

### Modified Capabilities

None.

## Impact

- Affected public API: new debug observation types and a way to register a sink on `IronRuntime`/`IronAgent` without changing existing constructor signatures where possible.
- Affected runtime paths: prompt request construction, system prompt rendering/influence reporting, active context snapshot estimation, model-driven compaction tool execution, tool approval/execution, queued/applied model switches, runtime configuration initialization, skill catalog refresh, and skill activation.
- Affected tests: add a recording debug sink test helper and assertions for representative event emission, redaction, and non-interference.
- Dependencies: no new external dependency is required for phase 1.
- Out of scope: UI rendering, durable debug logs, handoff/export inclusion, provider token streaming telemetry, full prompt/tool argument capture, accurate token counting improvements, and broad runtime refactors.
