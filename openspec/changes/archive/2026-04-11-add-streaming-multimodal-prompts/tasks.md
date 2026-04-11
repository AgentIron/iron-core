## 1. Facade API and shared streaming path

- [x] 1.1 Add public `AgentSession::prompt_stream_with_blocks(&[ContentBlock])` on the Rust facade with the same `(PromptHandle, PromptEvents)` contract as `prompt_stream(&str)`
- [x] 1.2 Refactor streaming prompt submission so block-based and text-based streaming share one internal path for ACP block request construction, active-stream registration/removal, and completion handling
- [x] 1.3 Keep `prompt_stream(&str)` as a convenience wrapper that wraps text as a single `ContentBlock` before calling the shared streaming path
- [x] 1.4 Reuse the existing `prompt_with_blocks(&[ContentBlock])` ACP block conversion path so streaming block prompts preserve empty-input behavior and block-mapping parity
- [x] 1.5 Verify the shared streaming path preserves existing single-active-prompt, session-level cancellation, and prompt-event ordering semantics without changing legacy `drain_events()` behavior

## 2. Public documentation

- [x] 2.1 Update `src/lib.rs` docs to show that the stream-first facade supports multimodal `ContentBlock` prompts
- [x] 2.2 Update `src/facade.rs` docs to document `prompt_stream_with_blocks(&[ContentBlock])` and clarify that `prompt_stream(&str)` is the text convenience wrapper over the shared streaming path
- [x] 2.3 Keep documentation scope limited to `src/lib.rs` and `src/facade.rs` without introducing broader protocol, schema, or provider-behavior claims

## 3. Streaming-contract test coverage

- [x] 3.1 Extend existing streaming runtime coverage to verify mixed `ContentBlock` prompt submission streams through the standard `PromptEvents` contract
- [x] 3.2 Add or update tests confirming multimodal streaming preserves existing event-ordering expectations, including terminal `PromptEvent::Complete` as the final event
- [x] 3.3 Add or update tests confirming approval handling and cancellation for multimodal streaming follow the same existing session-level semantics as text streaming
- [x] 3.4 Add or update tests confirming empty `&[ContentBlock]` input for streaming matches `prompt_with_blocks(&[ContentBlock])` behavior rather than introducing a streaming-only validation rule

## 4. Scope verification

- [x] 4.1 Verify the change does not alter ACP protocol types, prompt event types, provider-specific multimodal behavior, transcript/schema models, handoff formats, or unrelated facade/runtime surfaces
