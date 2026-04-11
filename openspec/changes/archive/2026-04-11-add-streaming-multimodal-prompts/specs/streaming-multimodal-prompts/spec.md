## ADDED Requirements

### Requirement: Stream-first multimodal prompts
The facade SHALL provide a streaming multimodal prompt entry point on `AgentSession` that accepts `&[ContentBlock]` and returns the existing `(PromptHandle, PromptEvents)` contract used by `prompt_stream(&str)`. This entry point SHALL submit the provided blocks as the user prompt without degrading multimodal content into a text-only request.

#### Scenario: Caller streams a mixed text-and-image prompt
- **WHEN** a caller invokes the streaming multimodal prompt API with user `ContentBlock` values that include text and image blocks
- **THEN** the runtime submits those blocks in the prompt request as structured multimodal content
- **THEN** the call returns the same `PromptHandle` and `PromptEvents` types used by the existing text streaming API

#### Scenario: Text-only streaming remains available as a convenience path
- **WHEN** a caller invokes `prompt_stream(&str)`
- **THEN** the facade continues to support text-only streaming prompts
- **THEN** it remains a convenience wrapper that wraps the text input as a single text `ContentBlock`
- **THEN** its behavior remains aligned with the shared streaming lifecycle used for block-based streaming

#### Scenario: Empty multimodal input follows existing blocking semantics
- **WHEN** a caller invokes the streaming multimodal prompt API with an empty `&[ContentBlock]`
- **THEN** the API accepts the empty slice
- **THEN** it applies the same semantics already defined by `prompt_with_blocks(&[ContentBlock])` for empty input
- **THEN** it does not introduce a streaming-only validation error for empty input

### Requirement: Multimodal streaming reuses the existing prompt event lifecycle
Streaming multimodal prompts SHALL use the existing `PromptHandle` / `PromptEvents` model and SHALL preserve the same prompt event semantics already defined for text streaming, including output streaming, approval requests, tool-call events, tool-result events, and terminal completion.

#### Scenario: Multimodal stream emits the standard event sequence
- **WHEN** a multimodal streaming prompt produces model output, tool activity, or approval requests
- **THEN** the caller receives those updates through the existing `PromptEvents` stream
- **THEN** the stream emits the same `PromptEvent` variants used for text-only streaming rather than a multimodal-specific event protocol
- **THEN** incremental output may arrive before prompt completion
- **THEN** each `PromptEvent::ToolCall` is emitted before its corresponding `PromptEvent::ToolResult`
- **THEN** approval requests are emitted before approval resolution through the returned `PromptHandle`
- **THEN** the stream emits exactly one terminal `PromptEvent::Complete` event
- **THEN** that `PromptEvent::Complete` event is the final event in the stream

#### Scenario: Approval flow matches text streaming
- **WHEN** a multimodal streaming prompt triggers a tool call that requires approval
- **THEN** the caller receives a standard `PromptEvent::ApprovalRequest`
- **THEN** `PromptHandle::approve(...)` and `PromptHandle::deny(...)` control that pending approval using the same semantics as text streaming

#### Scenario: Cancellation flow matches text streaming
- **WHEN** a caller cancels an active multimodal streaming prompt through the returned `PromptHandle`
- **THEN** the runtime cancels the active prompt through the same session-level cancellation path used by text streaming
- **THEN** pending approvals are resolved consistently with the existing streaming cancellation behavior

### Requirement: Multimodal streaming preserves existing session-level semantics
The streaming multimodal prompt API SHALL inherit the same session-level semantics as existing streaming prompt submission.

#### Scenario: A session still allows at most one active prompt
- **WHEN** a session already has an active prompt
- **THEN** the streaming multimodal prompt API follows the same single-active-prompt rule enforced for other prompt submissions on that session
- **THEN** this change does not introduce concurrent active prompts within one session

#### Scenario: Legacy blocking event buffering remains unchanged
- **WHEN** this capability is added
- **THEN** `drain_events()` and the legacy blocking event-buffer behavior remain unchanged
- **THEN** the new API follows the existing streaming-path semantics instead of altering blocking event collection

### Requirement: Blocking and streaming multimodal prompts preserve the same block mapping
The streaming multimodal prompt path SHALL map `ContentBlock` values into ACP prompt content using the same block conversion rules already used by the blocking `prompt_with_blocks(&[ContentBlock])` path.

#### Scenario: Image blocks are forwarded consistently across prompt modes
- **WHEN** the same multimodal `ContentBlock` sequence is sent through the blocking and streaming block-based prompt APIs
- **THEN** both APIs forward equivalent ACP content blocks for text, image, and resource content
- **THEN** neither API applies a divergent multimodal-to-text fallback during request construction

### Requirement: Scope remains limited to the Rust facade streaming API
This change SHALL remain limited to the public Rust facade API, documentation, and tests for the existing streaming contract.

#### Scenario: Out-of-scope systems remain unchanged
- **WHEN** this capability is implemented
- **THEN** it does not introduce ACP protocol changes
- **THEN** it does not add provider-specific multimodal normalization
- **THEN** it does not change transcript schemas, durable content models, or handoff/import-export formats
- **THEN** it does not add new prompt event types
