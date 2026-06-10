## 1. Public Event Contract

- [x] 1.1 Add public `PromptEvent` variants for `CompactionStarted`, `CompactionFinished`, and `CompactionFailed` with serializable payload fields.
- [x] 1.2 Add an internal prompt lifecycle representation for compaction lifecycle events.
- [x] 1.3 Thread compaction lifecycle events through ACP/facade conversion so `prompt_stream_with_blocks` clients receive them.
- [x] 1.4 Add or update event conversion tests for the new lifecycle variants.

## 2. Compaction Path Instrumentation

- [x] 2.1 Emit `CompactionStarted` before the runtime-owned `compress` tool performs validation or execution.
- [x] 2.2 Emit `CompactionFinished` after successful `compress` execution with `tokens_before`, `tokens_after`, and `method`.
- [x] 2.3 Emit `CompactionFailed` when `compress` parsing, validation, or execution fails after start.
- [x] 2.4 Emit lifecycle events for manual compact/checkpoint behavior when that path performs compaction.
- [x] 2.5 Emit lifecycle events for model-switch/window-shrink compaction using the estimates already computed by that path.
- [x] 2.6 Ensure regular `compress` tool call and tool result events remain unchanged for transcript/tool visibility.

## 3. Context Threshold Telemetry

- [x] 3.1 Ensure `AgentSession::active_context` reports `compact_threshold_tokens` when context management is enabled.
- [x] 3.2 Ensure context telemetry omits `compact_threshold_tokens` when context management is disabled.
- [x] 3.3 Add tests for active context telemetry threshold behavior.

## 4. CI Toolchain Pinning

- [x] 4.1 Update `.github/workflows/pull-request.yml` to install Rust toolchain `1.96.0` with `rustfmt` and `clippy`.
- [x] 4.2 Update `.github/workflows/release-patch.yml` to install Rust toolchain `1.96.0` with `rustfmt` and `clippy`.
- [x] 4.3 Update `.github/workflows/release-manual.yml` to install Rust toolchain `1.96.0` with `rustfmt` and `clippy`.

## 5. Verification

- [x] 5.1 Add tests that successful model-driven `compress` emits started and finished events with metrics.
- [x] 5.2 Add tests that failed compaction emits `CompactionFailed` and does not leave clients without a terminal lifecycle event.
- [x] 5.3 Add tests or coverage for model-switch/window-shrink compaction lifecycle events.
- [x] 5.4 Run `cargo check --manifest-path src-tauri/Cargo.toml` if a Tauri manifest exists; otherwise run the appropriate crate-level Rust checks for this repository.
- [x] 5.5 Run the narrowest relevant Rust test suite for context compaction and prompt event streaming.
- [x] 5.6 Validate the OpenSpec change before implementation is marked complete.
