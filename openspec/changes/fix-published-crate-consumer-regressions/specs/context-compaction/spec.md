## MODIFIED Requirements

### Requirement: Slash compact runs an immediate compression turn
The `/compact` command and facade checkpoint operation SHALL immediately run a model turn whose task is to compress resolved context through the `compress` tool. Both entry points SHALL use the same model-driven compaction instruction and SHALL NOT execute a hidden runtime summarizer.

#### Scenario: User invokes compact command
- **WHEN** the user invokes `/compact`
- **THEN** the runtime starts a compression-focused model turn immediately
- **AND** the model receives a strong compression nudge
- **AND** hidden runtime summarizer compaction is not executed

#### Scenario: Facade consumer invokes checkpoint
- **WHEN** a facade consumer calls `AgentSession::checkpoint()` for an idle session with context management enabled
- **THEN** the runtime starts the same compression-focused model turn used by `/compact`
- **AND** the checkpoint call reports prompt execution failures instead of returning an unconditional not-implemented error

#### Scenario: Checkpoint preconditions fail
- **WHEN** a facade consumer calls `AgentSession::checkpoint()` while the session is active or context management is disabled
- **THEN** the call returns an actionable precondition error
- **AND** no compression turn starts
