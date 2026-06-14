# debug-observation Specification

## Purpose
TBD - created by archiving change add-debug-observation-primitive. Update Purpose after archive.
## Requirements
### Requirement: Runtime Debug Sink Registration
The system SHALL provide a public debug observation sink that embedders can register to receive typed engine-level debug events, with a no-op sink used by default when no sink is registered.

#### Scenario: Default no-op debug sink
- **WHEN** a runtime is created without an explicit debug sink
- **THEN** debug emission does not require client setup and does not change runtime behavior

#### Scenario: Registered sink receives events
- **WHEN** an embedder registers a debug sink and the runtime reaches an instrumented decision point
- **THEN** the sink receives a typed debug event for that decision point

#### Scenario: Sink registration avoids constructor breakage
- **WHEN** existing code creates `IronRuntime` or `IronAgent` using current constructors
- **THEN** the code continues to compile without requiring a debug sink argument

### Requirement: Non-Interfering Debug Emission
Debug observation SHALL be best-effort and SHALL NOT be required for correct prompt execution, tool execution, model switching, compaction, session state updates, or provider requests.

#### Scenario: Debug sink is absent
- **WHEN** an instrumented runtime path executes without a registered sink
- **THEN** the runtime completes the path as it would without debug observation

#### Scenario: Debug sink performs no state transition
- **WHEN** a debug event is emitted
- **THEN** the event is not added to model-visible context, durable transcript state, or handoff/export data

### Requirement: Debug Event Envelope
The system SHALL emit debug events using a structured envelope that includes severity, scope/correlation metadata, and a typed domain payload.

#### Scenario: Event contains common metadata
- **WHEN** a debug event is emitted
- **THEN** the event includes common metadata sufficient for clients to classify and correlate the event

#### Scenario: Event payload is domain typed
- **WHEN** a client receives a debug event
- **THEN** the event payload identifies the relevant domain, such as prompt, context, compaction, tool, provider/model switch, config, or skill

### Requirement: Debug Scope Correlation
The system SHALL attach available correlation metadata to debug events so clients can reconstruct runtime causality across sessions, turns, model requests, and tool calls.

#### Scenario: Runtime-level event
- **WHEN** the runtime emits a configuration or catalog-level event outside a prompt turn
- **THEN** the event scope includes runtime-level correlation metadata where available and does not require a session id

#### Scenario: Prompt-turn event
- **WHEN** the runtime emits an event during prompt execution
- **THEN** the event scope includes available session and turn correlation metadata

#### Scenario: Tool-call event
- **WHEN** the runtime emits an event for a tool call
- **THEN** the event scope includes available session, turn, and tool call correlation metadata

### Requirement: Redaction-First Debug Payloads
The system SHALL avoid emitting sensitive raw content in phase-1 debug payloads and SHALL prefer safe metadata such as names, counts, lengths, fingerprints, hashes, statuses, durations, decisions, and reasons.

#### Scenario: System prompt debug event
- **WHEN** the runtime emits a debug event about a rendered or changed system prompt
- **THEN** the event includes safe summary metadata such as section names, lengths, fingerprints, and changed flags rather than the full prompt text

#### Scenario: Tool debug event
- **WHEN** the runtime emits a debug event about a tool call
- **THEN** the event includes safe metadata such as tool name, call id, source/kind, approval decision, execution status, duration, and truncation flag rather than raw arguments or raw results

#### Scenario: Skill debug event
- **WHEN** the runtime emits a debug event about skill discovery, availability, or activation
- **THEN** the event includes safe metadata such as skill names, source kind, counts, status, and rejection reason rather than skill contents

#### Scenario: Config debug event
- **WHEN** the runtime emits a debug event about runtime configuration
- **THEN** the event includes redacted feature and policy metadata rather than credentials, secrets, or raw sensitive values

### Requirement: Prompt Influence Observation
The system SHALL expose model input influences as debug events so clients can observe runtime-added, removed, changed, or suppressed hints and nudges without coupling the primitive to a single feature such as compaction.

#### Scenario: Context pressure hint is injected
- **WHEN** context pressure affects guidance included in the model input
- **THEN** the runtime emits a prompt influence debug event identifying the source, destination, effect, and safe reason metadata

#### Scenario: Hint is suppressed
- **WHEN** a runtime condition causes a possible model input influence to be omitted
- **THEN** the runtime can emit a prompt influence debug event identifying the suppressed effect and reason without exposing raw prompt content

### Requirement: System Prompt Render Observation
The system SHALL emit a debug event when building a model request that summarizes the rendered system prompt or composed prompt inputs.

#### Scenario: Model request is built
- **WHEN** the runtime builds an inference request with composed instructions
- **THEN** the runtime emits a debug event containing safe prompt summary metadata such as fingerprint, total size, and section summaries

#### Scenario: System prompt fingerprint changes
- **WHEN** the rendered system prompt fingerprint differs from the previous known fingerprint for the same relevant scope
- **THEN** the debug event indicates that the system prompt changed without exposing full prompt text

### Requirement: Context Snapshot Observation
The system SHALL emit debug events for active context snapshot estimation and pressure classification at prompt-relevant decision points.

#### Scenario: Context snapshot estimated
- **WHEN** the runtime estimates active context for a prompt or post-turn context check
- **THEN** the runtime emits a debug event containing total estimated tokens, context window limit when known, quality, pressure classification, and category summaries

#### Scenario: Token estimate quality is approximate
- **WHEN** a context snapshot uses heuristic or estimated token accounting
- **THEN** the debug event represents the estimate quality so clients do not treat the value as exact

### Requirement: Compaction Observation
The system SHALL emit debug events for model-driven compaction requests, rejected compaction attempts, and applied compaction results.

#### Scenario: Model requests compaction
- **WHEN** the model invokes the compaction tool
- **THEN** the runtime emits a debug event containing safe request metadata such as topic presence, range count, threshold metadata, and scope

#### Scenario: Compaction is rejected
- **WHEN** compaction validation rejects requested ranges or arguments
- **THEN** the runtime emits a debug event containing the rejection status and safe rejection reason

#### Scenario: Compaction is applied
- **WHEN** compaction successfully replaces timeline content with compressed blocks
- **THEN** the runtime emits a debug event containing safe result metadata such as block count, estimated old/new context sizes where available, pressure state, and reduction estimate where available

### Requirement: Tool Decision Observation
The system SHALL emit debug events for tool approval decisions and tool execution lifecycle transitions that expose engine decision reasons not already guaranteed by client lifecycle events.

#### Scenario: Tool approval evaluated
- **WHEN** the runtime evaluates whether a tool call requires approval
- **THEN** the runtime emits a debug event containing the tool name, approval strategy outcome, decision source, and whether user approval was requested

#### Scenario: Tool execution starts
- **WHEN** the runtime starts executing a tool call
- **THEN** the runtime emits a debug event containing safe tool metadata and correlation scope

#### Scenario: Tool execution finishes
- **WHEN** the runtime completes or fails a tool call
- **THEN** the runtime emits a debug event containing execution status, duration where available, and whether the result was truncated

### Requirement: Model Switch Observation
The system SHALL emit debug events for queued, planned, applied, and failed model switch decisions.

#### Scenario: Model switch queued
- **WHEN** a model switch is requested during an active prompt turn and must wait for a turn boundary
- **THEN** the runtime emits a debug event identifying the queued target model/provider and relevant session scope

#### Scenario: Model switch plan created
- **WHEN** the runtime evaluates how to switch a session to a target model
- **THEN** the runtime emits a debug event containing safe planning metadata such as estimated current tokens, target context window, whether adaptation is needed, and estimate quality

#### Scenario: Model switch applied
- **WHEN** the runtime applies a model switch at a turn boundary
- **THEN** the runtime emits a debug event identifying the previous and target model/provider and safe capability-diff metadata where available

#### Scenario: Model switch fails
- **WHEN** the runtime cannot complete a requested model switch
- **THEN** the runtime emits a debug event containing failure status and safe reason metadata

### Requirement: Runtime Configuration Observation
The system SHALL emit a redacted debug event summarizing runtime configuration relevant to engine decisions.

#### Scenario: Runtime is configured
- **WHEN** a runtime is initialized
- **THEN** the runtime emits a debug event summarizing safe configuration metadata such as selected provider/model identifiers, approval policy, context management flags, prompt composition settings, tool policy, plugin/MCP/skill enablement, and workspace root counts

#### Scenario: Future mutable config changes occur
- **WHEN** a future API changes runtime configuration after initialization
- **THEN** that API emits a debug event through the same config event family

### Requirement: Skill Loading Observation
The system SHALL emit debug events for skill catalog refresh, session skill availability, and skill activation outcomes.

#### Scenario: Skill catalog refreshed
- **WHEN** the runtime refreshes the skill catalog
- **THEN** the runtime emits a debug event containing safe metadata such as source kinds, discovered counts, trusted/untrusted counts, and diagnostic counts

#### Scenario: Skills made available to session
- **WHEN** a session is created or refreshed with available skills
- **THEN** the runtime emits a debug event containing safe availability metadata such as count and source categories

#### Scenario: Skill activation succeeds
- **WHEN** a skill is activated for a session
- **THEN** the runtime emits a debug event containing the skill name, source kind where available, activation source, and session scope

#### Scenario: Skill activation is rejected
- **WHEN** a skill activation cannot be completed
- **THEN** the runtime emits a debug event containing the skill name when available and a safe rejection reason such as unavailable, trust required, already active, or invalid arguments

### Requirement: Debug Events Remain Separate From Lifecycle And Logs
The system SHALL keep debug observation separate from client lifecycle events and process logging.

#### Scenario: Prompt lifecycle event emitted
- **WHEN** the runtime emits a `PromptLifecycleEvent`
- **THEN** debug observation does not replace the lifecycle event or require clients to consume debug events for normal prompt UX

#### Scenario: Runtime warning logged
- **WHEN** the runtime writes a warning through process logging
- **THEN** the warning does not require a corresponding debug event unless it represents an instrumented engine decision

### Requirement: Recording Sink Test Support
The system SHALL provide test support for recording emitted debug events so representative instrumentation can be verified without external clients.

#### Scenario: Test records debug events
- **WHEN** a test runs an instrumented runtime path with a recording sink
- **THEN** the test can inspect emitted events and assert expected event domains, safe payload fields, and relevant ordering

#### Scenario: Test verifies redaction
- **WHEN** a test exercises a prompt, tool, config, or skill path with sensitive-like input
- **THEN** the recorded debug events do not contain prohibited raw content

