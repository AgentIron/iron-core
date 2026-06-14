## MODIFIED Requirements

### Requirement: Future prompts use compacted context and retained tail
After a session is compacted, the system SHALL construct future provider requests from session instructions, chronological compressed blocks, and the retained active transcript only.

#### Scenario: Post-compaction request includes compressed blocks
- **WHEN** a session has one or more compressed blocks and a subsequent prompt is prepared
- **THEN** the provider-visible request includes those compressed blocks in chronological compaction order
- **AND** each rendered block includes its visible block ID, topic, source range, and summary

#### Scenario: Post-compaction request excludes compressed active transcript entries
- **WHEN** active transcript entries have been replaced by compressed blocks
- **THEN** those original active transcript entries are not included in subsequent provider requests
- **AND** the provider-visible request includes the retained active transcript that was not compressed

### Requirement: Compaction prunes historical tool transcript state
When compaction succeeds, the system SHALL remove compressed historical tool transcript state from future provider-visible active transcript while preserving structurally protected tool state.

#### Scenario: Completed historical tool records are not replayed after compaction
- **WHEN** a valid compression range contains completed historical tool calls and their corresponding terminal results
- **THEN** those tool calls and results are removed from future provider-visible active transcript
- **AND** the compressed block summary is the durable record of any important tool-derived facts

#### Scenario: Retained active transcript remains available after compaction
- **WHEN** compaction completes successfully
- **THEN** the session retains active transcript entries outside the compressed ranges for subsequent requests

#### Scenario: Activated skill content survives compaction
- **WHEN** a session undergoes compaction
- **AND** skills have been activated in that session
- **THEN** the activated skill instructions are preserved and re-injected into subsequent provider requests
- **AND** they are not treated as historical tool transcript state for pruning purposes

### Requirement: Important historical tool outcomes survive semantically
The system SHALL rely on model-authored compressed block summaries to preserve important historical tool-derived state rather than replaying compressed historical tool transcript entries.

#### Scenario: Tool-derived results are preserved through compressed block summaries
- **WHEN** historical tool output contains facts, decisions, recent results, errors, file paths, constraints, or unresolved questions needed for continuity
- **THEN** the `compress` tool instructions require the model-authored summary to preserve that information before the selected range is removed

### Requirement: Compaction thresholds reflect provider-visible context growth
The system SHALL evaluate context pressure using accounting that reflects provider-visible context, including compressed blocks, retained active transcript, instructions, and retained tool traffic.

#### Scenario: Tool-heavy sessions receive pressure nudges
- **WHEN** a session grows primarily through tool calls and tool results
- **THEN** context-pressure evaluation includes that provider-visible tool traffic
- **AND** the model receives the appropriate qualitative compression nudge when a pressure threshold is crossed

#### Scenario: Critical pressure requires compression before continuing
- **WHEN** the projected provider request is in the critical pressure bucket
- **THEN** the runtime presents compression as required before substantial continuation
- **AND** the runtime does not run hidden separate summarizer compaction

### Requirement: Compaction behavior is verified end to end
The test suite SHALL verify compaction by inspecting resulting provider request composition, range validation, and pressure behavior, not only by asserting local session flags.

#### Scenario: Tests verify compressed blocks are re-used in future requests
- **WHEN** integration tests prepare a prompt after model-driven compaction
- **THEN** they assert that the outgoing provider request contains the expected compressed block content

#### Scenario: Tests verify compressed historical tool transcript is removed
- **WHEN** integration tests prepare a prompt after compaction in a tool-heavy session
- **THEN** they assert that compressed historical tool calls and tool results are absent from the outgoing provider request

#### Scenario: Tests verify invalid ranges do not mutate session state
- **WHEN** tests execute `compress` with an invalid range
- **THEN** the session active transcript, tool records, and compressed blocks remain unchanged

### Requirement: Compacted context SHALL preserve activated skill instructions
The system SHALL treat activated skill instructions as protected instruction-layer content that survives compaction separately from compressed blocks and retained active transcript.

#### Scenario: Post-compaction prompt includes active skills
- **WHEN** a session with activated skills is compacted
- **THEN** the resulting provider request includes the activated skill instructions
- **AND** they appear in a dedicated instruction layer separate from compressed blocks and retained active transcript

#### Scenario: Active skills are not summarized into compressed blocks
- **WHEN** compaction produces a freeform summary of historical conversation
- **THEN** activated skill instructions are excluded from that summary
- **AND** they are preserved in full for subsequent provider requests

## ADDED Requirements

### Requirement: Model-visible compress tool compacts selected ranges
The system SHALL expose a runtime-owned `compress` tool to tool-capable models when compression is available or useful, allowing the active model to replace selected resolved history ranges with freeform durable summaries.

#### Scenario: Model compresses a resolved range
- **WHEN** the model calls `compress` with a topic, one or more valid source ranges, and summaries
- **THEN** the runtime stores chronological compressed blocks for those summaries
- **AND** removes the selected active transcript entries and selected older compressed blocks from future provider-visible active context
- **AND** returns a tool result describing the blocks created and current qualitative pressure state

#### Scenario: Compress tool is not dispatched as an external tool
- **WHEN** the model calls `compress`
- **THEN** the runtime intercepts the call internally
- **AND** it does not route the call through MCP, plugin, Python, approval, or child-tool execution paths

### Requirement: Visible context IDs address compressible context
The system SHALL render stable visible IDs for compressible timeline entries and compressed blocks when compression is available or useful.

#### Scenario: Provider-visible transcript includes timeline IDs
- **WHEN** compression is available for a provider request
- **THEN** user, assistant, and tool timeline entries that may be referenced by `compress` include compact visible IDs

#### Scenario: Compressed blocks include block IDs
- **WHEN** compressed blocks are rendered in a provider request
- **THEN** each block includes a compact visible ID
- **AND** that ID can be used in later valid compression ranges when the block is outside protected active context

### Requirement: Compress validation preserves structural integrity
The system SHALL validate all `compress` ranges before mutating durable session state and SHALL reject the entire call if any range is invalid.

#### Scenario: Tool call and result pairs cannot be split
- **WHEN** a `compress` range would include a tool call without its corresponding terminal result, or a terminal result without its corresponding tool call
- **THEN** the runtime rejects the entire `compress` call
- **AND** the tool result explains that tool call/result pairs must remain together

#### Scenario: Active context is protected
- **WHEN** a `compress` range includes the latest user request, the current assistant turn, a pending tool call, a running tool call, or a pending-approval tool call
- **THEN** the runtime rejects the entire `compress` call
- **AND** durable session state is not mutated

#### Scenario: Invalid or overlapping ranges are rejected
- **WHEN** a `compress` call contains an unknown ID, reversed range, overlapping range, already-removed ID, or synthetic non-compressible content
- **THEN** the runtime rejects the entire `compress` call
- **AND** durable session state is not mutated

#### Scenario: Previous completed turns can be compressed during an active turn
- **WHEN** the model is in an active turn and selects only previous completed turns or older compressed blocks outside protected context
- **THEN** the runtime may apply the valid compression without requiring the active turn itself to be included

### Requirement: Compression pressure uses qualitative model-visible nudges
The system SHALL compute exact context telemetry internally but render only qualitative pressure buckets and action guidance to the model.

#### Scenario: Prompt renders bucketed pressure
- **WHEN** context usage crosses a configured pressure threshold
- **THEN** the system prompt includes the corresponding qualitative pressure bucket
- **AND** the prompt does not include exact token counts or exact percentages as normative guidance

#### Scenario: Prompt cache changes only on pressure bucket transitions
- **WHEN** exact telemetry changes without changing the qualitative pressure bucket or compression availability
- **THEN** prompt cache fingerprinting does not invalidate solely because of the exact telemetry change

### Requirement: Pressure clears only after recomputed usage is below threshold
The system SHALL recompute provider-visible context pressure after successful compression and SHALL clear model-visible pressure only when usage falls below the active threshold.

#### Scenario: Successful compression clears pressure when below threshold
- **WHEN** a valid `compress` call is applied
- **AND** recomputed provider-visible context usage falls below the active pressure threshold
- **THEN** the next model prompt does not include the previous pressure nudge

#### Scenario: Successful compression keeps pressure when still above threshold
- **WHEN** a valid `compress` call is applied
- **AND** recomputed provider-visible context usage remains above the active pressure threshold
- **THEN** the tool result reports that compression succeeded but more compression may be needed
- **AND** the next model prompt retains the appropriate qualitative pressure nudge

### Requirement: Slash compact runs an immediate compression turn
The `/compact` command SHALL immediately run a model turn whose task is to compress resolved context through the `compress` tool.

#### Scenario: User invokes compact command
- **WHEN** the user invokes `/compact`
- **THEN** the runtime starts a compression-focused model turn immediately
- **AND** the model receives a strong compression nudge
- **AND** hidden runtime summarizer compaction is not executed

### Requirement: Critical pressure failure is visible to the user
The system SHALL surface a simple user-visible failure when compression cannot reduce critical context pressure below the required threshold.

#### Scenario: Compression cannot resolve critical pressure
- **WHEN** context pressure is critical
- **AND** compression fails validation, fails execution, or succeeds without reducing context usage below the required threshold after the allowed attempt
- **THEN** the user is shown an error that compaction could not get context usage under the threshold
- **AND** the error suggests starting a new session
