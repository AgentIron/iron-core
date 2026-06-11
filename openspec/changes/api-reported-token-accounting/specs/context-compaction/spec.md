## ADDED Requirements

### Requirement: Compaction pressure uses tracker-backed estimates
The system SHALL evaluate context pressure for compaction using the session token tracker when provider-reported usage is available, falling back to existing heuristic estimates when no valid usage baseline exists.

#### Scenario: Usage baseline informs pressure
- **WHEN** a session has a valid provider-reported input-token baseline
- **AND** local transcript delta has been estimated after that baseline
- **THEN** compaction pressure is computed from the tracker-backed baseline-plus-delta estimate

#### Scenario: Compaction pressure falls back without usage
- **WHEN** a session has no valid provider-reported input-token baseline
- **THEN** compaction pressure is computed using the existing heuristic estimate
- **AND** compaction behavior remains available

### Requirement: Compaction resets provider usage baseline
The system SHALL clear any provider-reported input-token baseline after successful compaction rewrites provider-visible context.

#### Scenario: Compress tool invalidates baseline
- **WHEN** the runtime-owned `compress` tool successfully removes selected history and creates compressed blocks
- **THEN** the token tracker invalidates its provider-reported input-token baseline
- **AND** later estimates use heuristic fallback until a provider usage event establishes a new baseline

#### Scenario: Model-switch auto-compaction invalidates baseline
- **WHEN** model-switch adaptation creates compressed context before applying a switch
- **THEN** the token tracker invalidates its provider-reported input-token baseline
- **AND** the switched session resynchronizes on the next usage-bearing provider response

### Requirement: Compaction metrics use best available accounting
Successful compaction lifecycle events SHALL include token metrics derived from tracker-backed estimates when available and heuristic estimates otherwise.

#### Scenario: Finish event uses tracker estimate
- **WHEN** compaction succeeds while a valid tracker-backed context estimate is available
- **THEN** `CompactionFinished` reports `tokens_before` and `tokens_after` using the best available tracker-backed or recomputed estimates
- **AND** the event includes the compaction method

#### Scenario: Finish event falls back to heuristic metrics
- **WHEN** compaction succeeds without a valid provider usage baseline
- **THEN** `CompactionFinished` continues reporting estimated token metrics using heuristic accounting
