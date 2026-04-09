## ADDED Requirements

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
