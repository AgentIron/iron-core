## 1. Debug Observation API

- [x] 1.1 Add a public debug observation module with `DebugSink`, `NullDebugSink`, `DebugEvent`, `DebugSeverity`, `DebugScope`, and grouped payload types.
- [x] 1.2 Mark public event enums/structs as extensible where appropriate so future payload variants can be added without unnecessary breaking changes.
- [x] 1.3 Add internal debug emission helpers that use the default no-op sink when no sink is registered.
- [x] 1.4 Add sink registration on `IronRuntime` and, where ergonomically appropriate, `IronAgent` without requiring existing constructor call sites to pass a sink.

## 2. Scope, Redaction, And Test Support

- [x] 2.1 Add runtime/session/turn/tool correlation scope propagation through prompt execution paths where needed for phase-1 events.
- [x] 2.2 Add safe summary/redaction helpers for prompt sections, tool calls, compaction requests/results, config summaries, and skill metadata.
- [x] 2.3 Add a recording debug sink test helper for asserting emitted events in unit/integration tests.
- [x] 2.4 Add tests proving debug events are not persisted to model-visible context, durable transcript state, or handoff/export data.

## 3. Prompt And Context Instrumentation

- [x] 3.1 Emit a redacted runtime configuration debug event during runtime initialization.
- [x] 3.2 Emit a system prompt render or prompt input build debug event when constructing model requests, including fingerprint, total size, section summaries, and changed status where available.
- [x] 3.3 Emit model input influence debug events for context-pressure guidance, compression availability, and other phase-1 prompt-affecting hints already represented during request construction.
- [x] 3.4 Emit active context snapshot debug events at prompt-relevant estimation points, including estimate quality, total estimated tokens, context window limit where known, pressure classification, and category summaries.

## 4. Compaction Instrumentation

- [x] 4.1 Emit a compaction requested debug event when the model invokes the compaction tool.
- [x] 4.2 Emit a compaction rejected debug event when argument parsing or range validation prevents compaction.
- [x] 4.3 Emit a compaction applied debug event when compaction succeeds, including safe block counts, pressure state, and estimated reduction metadata where available.
- [x] 4.4 Add tests for compaction requested, rejected, and applied debug events without exposing raw compressed content.

## 5. Tool Instrumentation

- [x] 5.1 Emit tool approval evaluation debug events that include approval strategy outcome, decision source, and whether user approval was requested.
- [x] 5.2 Emit tool execution started and finished debug events with safe tool metadata, execution status, duration where available, and result truncation status.
- [x] 5.3 Emit tool validation/failure status through the tool event family without raw tool arguments or raw results.
- [x] 5.4 Add tests for approval, execution success, execution failure, and truncation metadata.

## 6. Model Switch Instrumentation

- [x] 6.1 Emit model switch queued debug events when a requested switch is deferred until a turn boundary.
- [x] 6.2 Emit model switch planning debug events with safe target window, current token estimate, adaptation-needed, and estimate-quality metadata.
- [x] 6.3 Emit model switch applied and failed debug events with previous/target model/provider metadata and safe capability-diff or failure metadata where available.
- [x] 6.4 Add tests for queued and applied model switch debug events.

## 7. Skill Instrumentation

- [x] 7.1 Emit skill catalog refreshed debug events with safe source kind, discovered count, trusted/untrusted count, and diagnostic count metadata.
- [x] 7.2 Emit session skill availability debug events when sessions receive available skill metadata.
- [x] 7.3 Emit skill activation success and rejection debug events with safe skill name/source/reason metadata.
- [x] 7.4 Add tests for skill catalog refresh, activation success, and activation rejection debug events without exposing skill contents.

## 8. Verification And Documentation

- [x] 8.1 Add API documentation explaining the distinction between `DebugSink`, `PromptSink`, and `tracing`.
- [x] 8.2 Add documentation for redaction guarantees and the expectation that sink implementations must be fast/non-blocking.
- [x] 8.3 Run the narrowest relevant Rust verification, including `cargo check --manifest-path src-tauri/Cargo.toml` if this repository path remains the correct verification target.
- [x] 8.4 Run or add targeted tests covering representative debug event emission and redaction behavior.
- [x] 8.5 Run `crg update` after implementation changes.
