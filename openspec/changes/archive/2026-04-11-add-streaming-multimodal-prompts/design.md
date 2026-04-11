## Context

`iron-core` already stores and transports multimodal user input as structured `ContentBlock` values. The blocking facade API reflects that: `AgentSession::prompt_with_blocks(&[ContentBlock])` accepts text, image, and resource blocks and submits them as an ACP `PromptRequest`.

The stream-first facade API is narrower. `AgentSession::prompt_stream(&str)` sets up `active_streams`, returns the existing `(PromptHandle, PromptEvents)` pair, and then submits a text-only `PromptRequest`. That means the public API currently forces callers to choose between multimodal input and streaming output even though the underlying request shape already supports both.

The surrounding runtime architecture already establishes the boundaries this change must follow:
- `IronRuntime::try_start_prompt(...)` enforces that a session may have at most one active prompt at a time.
- `PromptHandle::cancel()` delegates to `AgentSession::cancel()`, which sends the existing ACP cancel notification and ultimately reaches the runtime's session-level cancellation path.
- `prompt_with_blocks(&[ContentBlock])` already defines the ACP block conversion used for structured prompt submission via `to_acp_content_block(...)`.
- Legacy `drain_events()` behavior is separate from the stream-first `PromptEvents` path and should remain unchanged.

The gap matters most for UI-oriented clients such as AgentIron, where prompts may include images but the caller still needs token-by-token output, approval handling, cancellation, and the existing prompt event flow.

## Goals / Non-Goals

**Goals:**
- Add a public streaming entry point that accepts `&[ContentBlock]` for multimodal prompts.
- Preserve the existing `PromptHandle` / `PromptEvents` contract and the same ordering guarantees documented for `prompt_stream(&str)`: incremental output may arrive before completion, `PromptEvent::ToolCall` precedes the corresponding `PromptEvent::ToolResult`, approval requests are emitted before approval resolution, and exactly one terminal `PromptEvent::Complete` is emitted last.
- Reuse the same streaming lifecycle already used by `prompt_stream(&str)`, including active-stream registration, approval resolution, completion, and cancellation.
- Preserve the same session-level semantics as existing streaming: one active prompt per session, with cancellation routed through the existing session-level cancellation path.
- Match the empty-input semantics of `prompt_with_blocks(&[ContentBlock])` rather than inventing a streaming-only validation rule.
- Keep text-only streaming ergonomic by retaining `prompt_stream(&str)` as a convenience wrapper.

**Non-Goals:**
- Redesigning `PromptEvent`, `PromptHandle`, or `PromptEvents`.
- Changing the blocking APIs (`prompt`, `prompt_with_blocks`) beyond shared conversion/helper reuse.
- Introducing ACP protocol changes, provider-specific multimodal normalization, transcript schema changes, durable content model changes, handoff/import-export format changes, or new event types.
- Changing legacy `drain_events()` / blocking event-buffer behavior.

## Decisions

### Add `prompt_stream_with_blocks(&[ContentBlock])` instead of overloading `prompt_stream`
The facade should gain a new public method dedicated to block-based streaming, while `prompt_stream(&str)` remains available for the common text-only case.

This matches the existing blocking split between `prompt(&str)` and `prompt_with_blocks(&[ContentBlock])`, keeps the API explicit, and avoids introducing a more disruptive generic or enum-based prompt surface.

`prompt_stream(&str)` should remain a convenience wrapper over the shared block-based streaming path by wrapping the text argument as a single text `ContentBlock` before request submission.

### Reuse the existing streaming contract instead of introducing multimodal-specific events
Block-based streaming should return the same `(PromptHandle, PromptEvents)` tuple as text streaming and should continue to emit the existing `PromptEvent` variants.

This is preferable to inventing a new event protocol because multimodal input changes request construction, not runtime event semantics. Output streaming, approval prompts, tool calls, tool results, and completion already fit the current model.

The ordering invariants are the same as for `prompt_stream(&str)`: incremental `PromptEvent::Output` items may arrive before completion, a `PromptEvent::ToolCall` is emitted before its corresponding `PromptEvent::ToolResult`, approval requests are emitted before callers resolve them through the handle, and there is exactly one terminal `PromptEvent::Complete`, emitted last.

### Factor request submission and stream setup so text and block streaming share one path
Today `prompt_stream(&str)` builds a text-only `PromptRequest` inline while also owning active-stream registration and background completion handling. The design should extract a shared internal path that accepts ACP content blocks, sets up the stream state once, and submits the prompt.

This shared internal refactor is specifically about the streaming submission path: ACP block request construction, active-stream registration/removal, completion handling, and reuse of the existing block-conversion helper.

This is preferable to duplicating the stream lifecycle in two public methods because duplication would make cancellation, completion, approval bookkeeping, and future stream behavior changes easier to drift apart. Blocking APIs remain separate and are not folded into the streaming implementation, except where they already share conversion/helper logic.

### Keep multimodal conversion aligned with the existing blocking path
The new streaming entry point should reuse the same `ContentBlock -> agent_client_protocol::ContentBlock` conversion already used by `prompt_with_blocks`.

This preserves parity between blocking and streaming multimodal requests and avoids introducing a second block conversion path with subtly different image or resource behavior.

The same parity rule should apply to empty input: if the caller passes `&[]`, the streaming block API should accept it and apply the same semantics that `prompt_with_blocks(&[ContentBlock])` already has for an empty slice.

### Update public docs/examples to describe stream-first multimodal usage
The crate-level docs in `src/lib.rs` and the facade docs in `src/facade.rs` should show that the canonical stream-first model also supports structured `ContentBlock` input.

This is preferable to leaving the new method discoverable only through API docs because the current crate docs explicitly frame streaming around `prompt_stream(text)`, which would otherwise continue to imply text-only streaming.

The documentation should also call out that this addition does not change legacy `drain_events()` semantics; it only extends the existing streaming path.

## Risks / Trade-offs

- [Adding a second public streaming entry point increases facade surface area] → Keep the naming consistent with the existing blocking methods and reuse the same return contract so the expansion is minimal and predictable.
- [Shared refactoring could accidentally change existing text-stream behavior] → Preserve `prompt_stream(&str)` as a thin wrapper over the shared stream path and verify parity with runtime tests that already cover the existing streaming completion, approval, cancellation, and event-ordering contract.
- [Multimodal provider behavior may vary by block composition] → Keep this change scoped to accepting and forwarding structured blocks through the existing ACP request model rather than adding provider-specific normalization logic.
- [Docs could overstate the scope of the change] → Keep wording specific to the Rust facade, `src/lib.rs`, `src/facade.rs`, and existing streaming test coverage rather than implying broader protocol or persistence changes.

## Migration Plan

- Add the new public facade method for block-based streaming.
- Refactor the existing streaming implementation so both public streaming methods use one internal request/lifecycle path for ACP block request construction, active-stream registration/removal, completion handling, and shared conversion/helper reuse.
- Update `src/lib.rs` and `src/facade.rs` documentation and examples to mention multimodal streaming and the convenience-wrapper role of `prompt_stream(&str)`.
- Add runtime coverage in existing streaming-contract tests for multimodal streaming requests, event completion ordering, and handle-based approval/cancellation parity.

## Open Questions

- Which multimodal example in `src/lib.rs` / `src/facade.rs` will be clearest for callers: an inline image block, a resource block, or a mixed text-plus-image prompt? The change does not depend on the choice, but the docs should pick one representative example.
