## Why

`iron-core` positions the session API as stream-first, but that promise currently only holds for text-only prompts. `AgentSession::prompt_with_blocks()` already accepts multimodal `ContentBlock` input, including images, yet it is blocking, while `AgentSession::prompt_stream()` streams events but only accepts `&str`. This leaves multimodal callers with an inconsistent API surface and prevents clients such as AgentIron from showing token-by-token output when prompts include images.

The repository already defines the streaming contract in the Rust facade: per-session prompt execution is single-active-prompt, cancellation flows through the existing session-level cancel path, and `PromptEvent` ordering is part of the documented stream behavior in `src/facade.rs`. The proposal should therefore extend that existing contract to block-based prompts rather than inventing multimodal-specific semantics.

## What Changes

- Add a streaming multimodal prompt entry point on `AgentSession` that accepts `&[ContentBlock]` and returns the same `(PromptHandle, PromptEvents)` contract as `prompt_stream()`.
- Specify that multimodal streaming preserves the same ordering guarantees as `prompt_stream(&str)`: incremental output may arrive before completion, `PromptEvent::ToolCall` precedes the corresponding `PromptEvent::ToolResult`, approval requests are emitted before approval resolution, and exactly one terminal `PromptEvent::Complete` is emitted last.
- Specify that the new API inherits the same session-level rules as existing streaming: a session may have at most one active prompt, and cancellation goes through the existing session-level cancellation path.
- Use parity with `prompt_with_blocks(&[ContentBlock])` for empty input by accepting an empty slice and applying the same semantics as the blocking block API rather than introducing streaming-only validation.
- Refactor the existing streaming submission path so text-only and block-based streaming share ACP block request construction, active-stream registration/removal, completion handling, and reuse of the existing `to_acp_content_block` helper instead of maintaining two separate streaming implementations.
- Keep blocking prompt entry points separate other than shared conversion/helper reuse.
- Extend public docs in `src/lib.rs` and `src/facade.rs` to show that the stream-first API also supports multimodal prompts, and clarify that `prompt_stream(&str)` remains a convenience wrapper that wraps text as a single text `ContentBlock` before using the shared streaming path.
- Add runtime tests that verify multimodal block requests stream output, preserve the existing streaming contract coverage, and keep approval/cancellation behavior aligned with the current streaming path.
- Keep scope limited to the public Rust facade API, docs, and tests; do not introduce ACP protocol changes, provider-specific multimodal normalization, transcript schema changes, durable content model changes, handoff or import/export format changes, or new event types.

## Capabilities

### New Capabilities
- `streaming-multimodal-prompts`: Expose a stream-first multimodal prompt API that accepts structured `ContentBlock` input while preserving the existing streaming event model.

### Modified Capabilities

## Impact

- Affected code: `src/facade.rs`, public crate docs in `src/lib.rs`, and streaming runtime tests in `tests/acp_runtime_tests.rs` or nearby existing streaming-contract coverage.
- Affected API: adds a new public `AgentSession::prompt_stream_with_blocks(&[ContentBlock])` method without removing or changing the existing `prompt()`, `prompt_with_blocks()`, or `prompt_stream()` entry points.
- Affected behavior: multimodal prompts can use the same streaming handle/event flow already available to text-only prompts, improving parity for UI clients and other stream-oriented integrations while leaving legacy `drain_events()` / blocking event-buffer semantics unchanged.
