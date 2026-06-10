## ADDED Requirements

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
