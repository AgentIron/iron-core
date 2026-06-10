# context-compaction Specification

## Purpose
TBD - created by archiving change fix-context-compaction. Update Purpose after archive.
## Requirements
### Requirement: Future prompts use compacted context and retained tail
After a session is compacted, the system SHALL construct future provider requests from the session instructions, the compacted context, and the retained tail only.

#### Scenario: Post-compaction request includes semantic summary
- **WHEN** a session has a non-empty `compacted_context` and a subsequent prompt is prepared
- **THEN** the provider-visible request includes the compacted context content
- **AND** the provider-visible request includes the retained tail

#### Scenario: Post-compaction request excludes summarized pre-tail transcript
- **WHEN** pre-tail messages have been summarized into `compacted_context`
- **THEN** those pre-tail messages are not included in subsequent provider requests

### Requirement: Compaction prunes historical tool transcript state
When compaction succeeds, the system SHALL remove pre-tail tool transcript state from future provider-visible context.

#### Scenario: Historical tool records are not replayed after compaction
- **WHEN** a session contains older completed tool calls and tool results that fall outside the retained tail during compaction
- **THEN** those tool calls and tool results are not included in subsequent provider requests

#### Scenario: Retained tail remains available after compaction
- **WHEN** compaction completes successfully
- **THEN** the session retains the configured tail portion of recent interaction history for subsequent requests

#### Scenario: Activated skill content survives compaction
- **WHEN** a session undergoes compaction
- **AND** skills have been activated in that session
- **THEN** the activated skill instructions are preserved and re-injected into subsequent provider requests
- **AND** they are not treated as historical tool transcript state for pruning purposes

### Requirement: Important historical tool outcomes survive semantically
The system SHALL preserve important pre-tail tool-derived state through `compacted_context` rather than by replaying historical tool transcript entries.

#### Scenario: Tool-derived results are preserved through compacted context
- **WHEN** historical tool output contains facts, decisions, recent results, or unresolved questions needed for continuity
- **THEN** the compacted context produced by compaction includes that information in the appropriate semantic fields

### Requirement: Compaction thresholds reflect provider-visible context growth
The system SHALL evaluate compaction triggers using accounting that reflects the provider-visible context, including retained tool traffic before compaction.

#### Scenario: Tool-heavy sessions still trigger maintenance compaction
- **WHEN** a session grows primarily through tool calls and tool results
- **THEN** maintenance compaction still triggers once the configured threshold is exceeded

#### Scenario: Hard-fit compaction reacts to actual request footprint
- **WHEN** the projected next provider request exceeds the configured context window hint
- **THEN** hard-fit compaction runs before the request is sent

### Requirement: Compaction behavior is verified end to end
The test suite SHALL verify compaction by inspecting resulting provider request composition, not only by asserting local session flags.

#### Scenario: Tests verify compacted context is re-used in future requests
- **WHEN** integration tests prepare a prompt after compaction
- **THEN** they assert that the outgoing provider request contains compacted context content

#### Scenario: Tests verify historical tool transcript is removed
- **WHEN** integration tests prepare a prompt after compaction in a tool-heavy session
- **THEN** they assert that pre-tail tool calls and tool results are absent from the outgoing provider request

### Requirement: Compacted context SHALL preserve activated skill instructions
The system SHALL treat activated skill instructions as protected content that survives compaction alongside the retained tail.

#### Scenario: Post-compaction prompt includes active skills
- **WHEN** a session with activated skills is compacted
- **THEN** the resulting provider request includes the activated skill instructions
- **AND** they appear in a dedicated instruction layer separate from compacted context and retained tail

#### Scenario: Active skills are not summarized into compacted context
- **WHEN** compaction produces a semantic summary of pre-tail conversation
- **THEN** activated skill instructions are excluded from that summary
- **AND** they are preserved in full for subsequent provider requests

### Requirement: Compaction lifecycle events are client-visible
The system SHALL emit explicit client-visible prompt events when a compaction attempt starts, finishes successfully, or fails.

#### Scenario: Client observes compaction start
- **WHEN** a compaction attempt begins
- **THEN** the prompt event stream emits `CompactionStarted`
- **AND** the event includes a `compaction_id`
- **AND** the event includes the compaction `method` when the method is known

#### Scenario: Client observes successful compaction
- **WHEN** a compaction attempt completes successfully
- **THEN** the prompt event stream emits `CompactionFinished`
- **AND** the event includes the same `compaction_id` as the matching start event
- **AND** the event includes `tokens_before`, `tokens_after`, and `method`

#### Scenario: Client observes failed compaction
- **WHEN** a compaction attempt fails after it starts
- **THEN** the prompt event stream emits `CompactionFailed`
- **AND** the event includes the same `compaction_id` as the matching start event
- **AND** the event includes a client-readable failure reason

### Requirement: All compaction paths emit lifecycle events
The system SHALL emit compaction lifecycle events for every path that creates, applies, or attempts to apply compressed context.

#### Scenario: Model-driven compress tool emits lifecycle events
- **WHEN** the runtime-owned `compress` tool is executed
- **THEN** the prompt event stream emits `CompactionStarted` before executing the compaction
- **AND** the prompt event stream emits `CompactionFinished` after successful compaction
- **AND** the regular tool call and tool result events remain available for the `compress` tool interaction

#### Scenario: Manual compact emits lifecycle events
- **WHEN** a client-initiated manual compact or checkpoint operation performs compaction
- **THEN** the client-visible event stream emits compaction lifecycle events for that compaction attempt

#### Scenario: Model-switch compaction emits lifecycle events
- **WHEN** context is compacted because a model switch shrinks the available context window
- **THEN** the client-visible event stream emits compaction lifecycle events for that compaction attempt

### Requirement: Compaction finish metrics use available estimates
Successful compaction finish events SHALL include token metrics based on the best estimates available at the time of compaction.

#### Scenario: Finish event includes rough estimates
- **WHEN** compaction succeeds before exact token accounting is available
- **THEN** `CompactionFinished` includes `tokens_before` and `tokens_after` using the current estimate values
- **AND** the event includes the compaction `method`

#### Scenario: Metrics do not require exact accounting
- **WHEN** exact token accounting improvements are not yet implemented
- **THEN** compaction lifecycle events are still emitted with the available estimated metrics

### Requirement: Active context telemetry exposes compaction threshold
When context management is enabled, the system SHALL expose the effective absolute compaction threshold in active context telemetry.

#### Scenario: Threshold appears in active context snapshot
- **WHEN** a client requests active context telemetry for a session with context management enabled
- **THEN** the snapshot includes `compact_threshold_tokens`
- **AND** the value is the effective absolute token threshold used for compaction decisions

#### Scenario: Threshold is absent when context management is disabled
- **WHEN** a client requests active context telemetry for a session with context management disabled
- **THEN** the snapshot does not report a compaction threshold

### Requirement: CI uses the pinned Rust toolchain
The repository CI and release workflows SHALL install Rust toolchain `1.96.0` with the `rustfmt` and `clippy` components.

#### Scenario: Pull request workflow installs Rust 1.96.0
- **WHEN** the pull request workflow installs Rust
- **THEN** it uses `dtolnay/rust-toolchain@master`
- **AND** it requests toolchain `1.96.0`
- **AND** it requests `rustfmt` and `clippy`

#### Scenario: Release workflows install Rust 1.96.0
- **WHEN** a release workflow installs Rust
- **THEN** it uses `dtolnay/rust-toolchain@master`
- **AND** it requests toolchain `1.96.0`
- **AND** it requests `rustfmt` and `clippy`
