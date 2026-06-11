## ADDED Requirements

### Requirement: Model-switch fit uses best available token accounting
The runtime SHALL use the best available session context estimate when deciding whether a target model's context window can fit the current session during model switching.

#### Scenario: Usage baseline informs switch planning
- **WHEN** a model switch is planned for a session with a valid provider-reported input-token baseline
- **THEN** the runtime computes current context usage from the token tracker's baseline-plus-delta estimate
- **AND** compares that estimate against the target model's context window

#### Scenario: Switch planning falls back without usage baseline
- **WHEN** a model switch is planned for a session without a valid provider-reported input-token baseline
- **THEN** the runtime uses the existing heuristic context estimate
- **AND** model switching remains available

### Requirement: Model switches preserve token baseline unless context changes
The runtime SHALL preserve a valid provider-reported input-token baseline across model switches unless the switch process rewrites provider-visible context.

#### Scenario: Switch without compaction preserves baseline
- **WHEN** a model switch applies without compacting or rewriting provider-visible context
- **THEN** the session token tracker keeps its current provider-reported input-token baseline
- **AND** the next usage-bearing response from the target model can resynchronize the baseline

#### Scenario: Switch with compaction clears baseline
- **WHEN** model-switch adaptation compacts context before applying the switch
- **THEN** the session token tracker clears its provider-reported input-token baseline
- **AND** context estimates fall back until the target provider reports input usage
