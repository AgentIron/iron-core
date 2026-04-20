## MODIFIED Requirements

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

## ADDED Requirements

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
