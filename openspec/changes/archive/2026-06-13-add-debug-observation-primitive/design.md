## Context

`iron-core` already exposes multiple observation surfaces, but none answer the same question as a debug observation primitive.

`PromptSink` is client-visible lifecycle plumbing: output deltas, tool call proposals/updates, script activity, and approval requests. It describes what the client/user observes during a prompt.

`tracing` is process logging. It is useful for operational logs and warnings, but it is not a typed engine decision stream that embedders can consume as structured data.

The missing surface is engine-owned debug observation: structured breadcrumbs emitted at semantic runtime transitions. These events should explain why the runtime made decisions around prompt composition, model hints, context pressure, compaction, tool approval/execution, model switching, configuration, and skills.

Relevant existing hook points include:

- `IronRuntime` / `RuntimeInner` for runtime configuration, session management, model switch queueing/application, skill catalog refresh, and sink ownership.
- `IronAgent` facade constructors and methods for ergonomic sink registration.
- `PromptRunner` for prompt request construction, active context snapshots, provider request failures, approval gating, tool execution, compaction tool execution, and post-turn context checks.
- `request_builder` and `prompt/system.rs` for system prompt rendering, prompt section fingerprints, provider/client/skill/repo instruction inputs, and context-pressure guidance.
- `context/accounting.rs` and `context/compaction.rs` for estimated context snapshots, pressure state, and model-driven compaction results or rejections.
- `runtime.rs` model switching paths for queued requests, planning, capability diffs, and turn-boundary application.
- Skill catalog refresh and skill activation paths for loaded/available/activated/rejected skill decisions.

This change should support future growth without overfitting to compaction. The stable primitive is the sink/envelope/scope/redaction model; the event vocabulary is expected to grow.

## Goals / Non-Goals

**Goals:**

- Provide a public, typed debug observation primitive for engine-level semantic decisions.
- Allow embedders such as AgentIron, iron-tui, tests, and other clients to consume structured debug events.
- Keep observation non-invasive: emitting debug data must not change runtime behavior, model-visible context, durable session state, or prompt results.
- Include a redaction-first payload policy from the first implementation.
- Cover representative phase-1 event families: prompt influences/system prompt rendering, context snapshots, model-driven compaction, tool approval/execution, model switching, runtime configuration, and skills.
- Preserve room for future events without frequent breaking changes.

**Non-Goals:**

- Building a UI for debug events.
- Persisting debug events in `DurableSession` or including them in handoff/export bundles.
- Replacing `PromptSink`, ACP lifecycle events, or `tracing`.
- Emitting provider token streaming telemetry or per-output-delta debug events.
- Capturing raw prompts, raw tool arguments, raw tool results, credentials, or skill contents.
- Solving accurate token counting; that remains separate from the debug observation primitive.
- Performing the broader runtime refactor tracked separately.

## Decisions

### Add one sink primitive, not per-domain callbacks

Introduce a single `DebugSink` trait that receives a typed event envelope. Do not add separate callbacks such as `on_tool_call`, `on_compaction`, or `on_model_switch`.

Rationale: one sink lets clients register once, preserves ordering, keeps correlation consistent, and avoids growing a callback interface every time a new event family appears.

Alternative considered: per-domain observer traits. This would improve narrow type discoverability but would fragment ordering/correlation and create more API churn as the runtime grows.

### Use synchronous, non-blocking emission for phase 1

Use a simple synchronous sink method such as `emit(&self, event: DebugEvent)`. Sink implementations are expected to return quickly and offload async work themselves if needed.

Rationale: debug hooks need to be callable from both synchronous and asynchronous code paths without infecting runtime internals with additional async boundaries. This keeps integration small and avoids making observability part of control flow.

Alternative considered: `async fn emit`. This would support async transports directly, but it would make debug observation capable of delaying or failing core runtime operations and would complicate call sites.

Alternative considered: runtime-owned channel/stream. This may become useful later for a built-in ring buffer or UI stream, but it is more infrastructure than phase 1 needs.

### Make debug emission best-effort and non-authoritative

`DebugSink::emit` should not return a runtime-relevant error. Debug emission failures must not change prompt execution, tool execution, model switching, session state, or compaction behavior.

Rationale: debug observation is diagnostic. If it can fail the runtime, it becomes part of the product behavior rather than observation.

Implementation can optionally guard against sink panics if that is practical, but panic isolation is not required to define the capability.

### Use a stable event envelope with extensible grouped payloads

Define an envelope carrying common metadata and a domain payload. Suggested shape:

- timestamp
- runtime-local sequence number, if cheap
- severity
- scope
- payload

Payloads should be grouped by domain, for example prompt, context, compaction, tool, provider/model switch, config, and skill. Public enums and structs should use `#[non_exhaustive]` where appropriate.

Rationale: common metadata gives every event consistent correlation and ordering. Grouped payloads keep growth organized and enable broad filtering by domain without flattening every event into one large enum.

Alternative considered: flat payload enum. This is simpler initially but will become harder to navigate as event families grow.

### Introduce explicit debug scope/correlation metadata

Events should carry a `DebugScope` with optional IDs and context that can be populated where available:

- runtime id
- connection id
- session id
- turn/prompt id
- tool call id
- provider name
- model id

Not all events have all IDs. Runtime-level events may only have runtime scope; tool events should include session/turn/tool call scope where possible.

Rationale: the primary value of debug events is reconstructing causality. Optional structured scope avoids stuffing correlation data into domain payloads and lets clients group timelines.

A lightweight scope object should be passed through prompt execution rather than reconstructing context at every hook point.

### Redaction-first payload policy

Phase-1 payloads must prefer metadata over content:

- system prompts: section names, owners, temperatures, lengths, fingerprints/hashes, changed flags, total chars; not full text
- model hints/influences: source, destination, effect, reason, lengths/counts; not raw injected prose unless explicitly safe and later allowed
- tool calls: tool name, source/kind, call id, approval decision/reason, duration, result status, truncation flag; not raw arguments/results
- compaction: requested/applied/rejected status, range counts, token estimates, pressure state, reduction estimate, rejection reason; not raw summaries unless a later opt-in policy exists
- skills: names, source kind, counts, activation status/reason, diagnostic counts; not skill contents
- config: selected redacted feature flags and policy values; not credentials, environment secrets, or raw workspace-sensitive data

Events should make truncation/redaction visible when applicable.

Rationale: debug systems often become accidental data exfiltration surfaces. The primitive should be safe by default and can add opt-in detail later.

### Model hints should use a general prompt influence concept

Represent nudges, hints, and prompt-affecting runtime decisions through a general model input influence event rather than compaction-specific events only.

A prompt influence should identify:

- source, such as context pressure, compaction availability, tool availability, skill activation, provider guidance, client instruction, or repo instruction
- destination, such as system prompt section, user prompt rewrite, tool definition, continuation context, or request metadata
- effect, such as added, removed, changed, or suppressed
- reason and safe metadata

Rationale: this directly addresses the need to observe compaction hints while providing a primitive that works for future model input changes.

### Emit system prompt render summaries, not full prompts

When an inference request is built, emit a summary of the rendered system prompt or prompt input build. Include fingerprint, total size, section summaries, and whether the fingerprint changed from the previous render when known.

Rationale: the existing prompt renderer already has section metadata and fingerprints. This is the safest and most useful way to observe system prompt changes.

### Keep debug events ephemeral in phase 1

Do not persist debug events in session timeline, durable storage, or handoff bundles. Clients that want persistence can implement it in their sink.

Rationale: debug events are diagnostic and can be high volume. Persisting them in core session state risks bloat, privacy issues, and coupling to UI/export requirements.

### Keep `tracing`, `PromptSink`, and `DebugSink` separate

Do not replace lifecycle events or logs with debug events. Existing `tracing::warn!` calls can remain. Future adapters may bridge debug events into tracing if useful.

Rationale: each surface has a different audience and contract: client-visible prompt lifecycle, process logs, and typed engine decisions.

## Risks / Trade-offs

- Debug sink blocks runtime → Document sinks as non-blocking, keep `emit` simple, and avoid expensive payload construction in phase 1.
- Event vocabulary churn breaks clients → Use grouped payloads and `#[non_exhaustive]`; document that clients should handle unknown future variants.
- Missing correlation makes events less useful → Add `DebugScope` early and pass it through prompt execution paths.
- Events leak sensitive content → Use redaction-first payloads, avoid raw prompts/tool data/skill contents, and test representative redaction behavior.
- Too many events create noise → Emit semantic transitions only, not streaming deltas; keep phase-1 coverage representative rather than exhaustive.
- Overlap with `PromptSink` confuses users → Document the distinction and ensure tool/prompt debug events include engine decision reasons not already present in lifecycle events.
- Under-instrumented phase 1 disappoints users → Cover at least one hook in each requested family: prompt hints, compaction, tools, model switching, config, and skills.

## Migration Plan

No persisted data migration is required.

Implementation should be additive:

1. Add public debug observation types and no-op sink.
2. Add sink registration without breaking existing constructors where possible.
3. Add internal emission helpers and scope propagation.
4. Add phase-1 events at representative hook points.
5. Add tests using a recording sink.

Rollback is straightforward because the default sink is no-op and no durable format changes are introduced.

## Open Questions

- Should `IronAgent` expose the same sink registration API as `IronRuntime`, or should it delegate only through runtime construction/accessors?
- Should the runtime isolate panics from sink implementations, or document that sinks must not panic?
- Is a runtime-local sequence number required in phase 1, or can timestamp plus scope be sufficient initially?
- Do we need a built-in `RecordingDebugSink` outside tests, or should it remain test-only until a UI/consumer requires it?
- Should provider request start/finish be included in phase 1, or deferred until provider-level diagnostics are designed more fully?
