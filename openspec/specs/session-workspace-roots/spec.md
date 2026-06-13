# session-workspace-roots Specification

## Purpose
TBD - created by archiving change session-workspace-roots. Update Purpose after archive.
## Requirements
### Requirement: Sessions have independent workspace roots
The system SHALL track active workspace roots per session. Changing workspace roots for one session SHALL NOT change the active workspace roots of any other session sharing the same runtime.

#### Scenario: New session inherits configured default roots
- **WHEN** a runtime creates a new session and `Config.workspace_roots` is non-empty
- **THEN** the session's active workspace roots are initialized from `Config.workspace_roots`

#### Scenario: New session falls back to process current directory
- **WHEN** a runtime creates a new session and `Config.workspace_roots` is empty
- **THEN** the session's active workspace roots are initialized from the process current directory fallback

#### Scenario: Session root update does not affect another session
- **WHEN** session A updates its workspace roots
- **THEN** session B keeps its own active workspace roots unchanged

### Requirement: Sessions expose a workspace root update API
The system SHALL provide `AgentSession::set_workspace_roots(Vec<PathBuf>)` to request a workspace root change for that session without recreating the runtime or replacing the session.

#### Scenario: Idle session applies roots immediately
- **WHEN** `set_workspace_roots` is called for an idle session
- **THEN** the provided roots become the session's active workspace roots before the method reports completion
- **THEN** existing messages, MCP enablement, plugin enablement, active skills, and provider/runtime state are preserved

#### Scenario: Active session defers roots until idle
- **WHEN** `set_workspace_roots` is called for a session with an active prompt
- **THEN** the provided roots are recorded as pending roots
- **THEN** the session's active workspace roots remain unchanged for the active prompt
- **THEN** the pending roots are applied after the active prompt finishes and the session returns to idle

#### Scenario: Later pending roots replace earlier pending roots
- **WHEN** `set_workspace_roots` is called multiple times while the same prompt is active
- **THEN** only the latest provided roots are applied when the prompt finishes

### Requirement: Prompt turns use one workspace root snapshot
The system SHALL use one active workspace root snapshot for each prompt turn. The runtime context rendered for a prompt and the builtin tool authorization used during that prompt SHALL be derived from the same snapshot.

#### Scenario: Runtime context reflects active session roots
- **WHEN** the runtime builds a provider request for a session
- **THEN** the `<runtime_context>` working directory is the first active workspace root from that session's prompt-turn snapshot
- **THEN** the `<runtime_context>` workspace root list contains the active workspace roots from that same snapshot

#### Scenario: Deferred roots do not affect active prompt context
- **WHEN** a session records pending workspace roots while a prompt is active
- **THEN** subsequent inference iterations within that active prompt continue rendering the original prompt-turn root snapshot

#### Scenario: Next prompt uses applied pending roots
- **WHEN** a prompt finishes after pending workspace roots were recorded
- **THEN** the pending roots become active before the next prompt is built
- **THEN** the next prompt renders runtime context from the newly active roots

### Requirement: Builtin tools enforce session prompt roots
The system SHALL authorize builtin filesystem, search, and shell tool paths against the active workspace root snapshot for the current session prompt. Builtin tools SHALL NOT authorize paths for one session using another session's roots.

#### Scenario: Builtin read uses current session roots
- **WHEN** session A and session B have different active workspace roots
- **THEN** a builtin `read` call in session A is authorized against session A's prompt-turn roots
- **THEN** the same path is not authorized merely because it is inside session B's roots

#### Scenario: Builtin search defaults to current session first root
- **WHEN** a builtin search tool call omits an explicit base path
- **THEN** the search defaults to the first root from the current session's prompt-turn root snapshot

#### Scenario: Builtin shell uses current session roots
- **WHEN** a builtin shell tool call omits an explicit working directory
- **THEN** the command runs from the first root in the current session's prompt-turn root snapshot
- **THEN** any explicit working directory must be within the current session's prompt-turn roots

#### Scenario: Pending roots do not move builtin authorization mid-prompt
- **WHEN** a session records pending workspace roots while a prompt is active
- **THEN** builtin tool calls for that active prompt continue authorizing paths against the original prompt-turn root snapshot

### Requirement: Project skills refresh when workspace roots become active
The system SHALL automatically rescan project-level skills for a session when new workspace roots become active. The refreshed project skills SHALL update that session's available skill snapshot without removing already active skill instructions.

#### Scenario: Idle root update refreshes available project skills
- **WHEN** an idle session applies new workspace roots containing project skills
- **THEN** the session's available skill snapshot is refreshed from those roots before the next prompt

#### Scenario: Deferred root update refreshes skills after prompt completion
- **WHEN** pending workspace roots are applied after an active prompt finishes
- **THEN** the session's available skill snapshot is refreshed from the newly active roots before the next prompt

#### Scenario: Active skills survive root changes
- **WHEN** a session has active skill instructions and then applies new workspace roots
- **THEN** the already active skill instructions remain active for that session
- **THEN** new model-initiated skill activations use the refreshed available skill snapshot

#### Scenario: Project skill trust policy still applies
- **WHEN** newly active workspace roots contain project skills and project skill trust is disabled
- **THEN** those project skills are not included in the refreshed available skill snapshot
- **THEN** the session records or exposes diagnostics consistent with existing project skill trust behavior

