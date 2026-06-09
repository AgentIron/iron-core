# model-switching Specification

## Purpose
Define runtime behavior for switching models mid-session while preserving conversation continuity, adapting context to target model constraints, reconciling capabilities, and exposing client-visible switch status.

## Requirements

### Requirement: Model switches SHALL only occur at turn boundaries
The runtime SHALL apply model switches when the session is idle (no active prompt or running tools). If a switch is requested while a turn is active, the runtime SHALL queue the switch for application after the current turn completes or is cancelled.

#### Scenario: Switch requested while idle
- **WHEN** the user requests a model switch and the session has no active prompt
- **THEN** the switch is applied immediately and the next prompt uses the new model

#### Scenario: Switch requested while active
- **WHEN** the user requests a model switch while a prompt is actively streaming
- **THEN** the runtime queues the switch and applies it after the current turn completes
- **AND** the client receives a notification that the switch is pending

#### Scenario: Switch requested with running tools
- **WHEN** the user requests a model switch while tools are executing
- **THEN** the runtime waits for running tools to complete or be cancelled
- **AND** the switch is applied only when the session becomes idle

### Requirement: The runtime SHALL preserve conversation identity across model switches
The same `SessionId` and `DurableSession` SHALL continue across model switches. A `ModelSwitched` timeline entry SHALL be recorded to mark the continuation boundary.

#### Scenario: Session identity preserved after switch
- **WHEN** a model switch is applied to a session
- **THEN** the session ID remains unchanged
- **AND** all prior messages, tool records, and compressed blocks are retained
- **AND** a `ModelSwitched` timeline entry is appended

### Requirement: Context SHALL be adapted when the target model has a smaller context window
When switching to a model with a smaller context window than the current context estimate, the runtime SHALL compact older context to fit within the target window while preserving the recent tail and task-critical state.

#### Scenario: Target window is larger than current context
- **WHEN** switching to a model with a context window larger than the current estimated usage
- **THEN** no compaction is performed
- **AND** the full transcript is preserved

#### Scenario: Target window is smaller than current context
- **WHEN** switching to a model with a context window smaller than the current estimated usage
- **THEN** the runtime compacts older context using the existing compression tool
- **AND** the recent tail (configurable, default 20 messages) is preserved verbatim
- **AND** the total estimated context size after compaction is within the target window

#### Scenario: Critical context cannot fit target window
- **WHEN** switching to a model where even the minimal retained tail exceeds the target window
- **THEN** the switch is rejected with an error indicating the context is too large
- **AND** the user is advised to start a new session or manually compact context

### Requirement: The runtime SHALL reconcile capabilities between source and target models
The runtime SHALL compare the source and target model capabilities and report differences to the client. Tools, modalities, and features unsupported by the target model SHALL be hidden or disabled. Capability comparison SHALL use `ModelCapabilityRegistry` populated from the effective model catalog, covering both built-in and custom models.

#### Scenario: Target supports same tools
- **WHEN** switching to a model that supports the same tools as the current model
- **THEN** all currently visible tools remain available
- **AND** no capability difference report is generated

#### Scenario: Target lacks some tools
- **WHEN** switching to a model that does not support some currently visible tools
- **THEN** unsupported tools are hidden from the effective tool catalog
- **AND** the client receives a report listing the unavailable tools

#### Scenario: Target lacks image support
- **WHEN** switching to a model that does not support image input
- **THEN** the runtime flags that image content may not be processable
- **AND** the client receives a capability difference report

#### Scenario: Switching between built-in and custom models
- **WHEN** switching from a built-in model to a custom model or vice versa
- **THEN** the capability comparison uses effective catalog metadata for both models
- **AND** the capability diff reflects the merged built-in and custom model metadata

#### Scenario: Switching to an unregistered custom model
- **WHEN** switching to a model not present in the effective catalog
- **THEN** the runtime rejects the switch with an error indicating the model is unknown

### Requirement: Model switches SHALL be recorded per-turn in session metadata
Each user and assistant turn SHALL record the model used for that turn. The session metadata SHALL track the current model and maintain a history of model switches.

#### Scenario: Per-turn model recording
- **WHEN** a prompt is processed after a model switch
- **THEN** the user message records the new model identifier
- **AND** the assistant response records the same model identifier
- **AND** the session metadata is updated to reflect the current model

### Requirement: The runtime SHALL support both managed and unmanaged provider switching
Model switches SHALL support both managed provider resolution (via `ProviderPromptContext` and credential store) and unmanaged provider switching (via direct `Provider` instance).

#### Scenario: Managed provider switch
- **WHEN** the user switches to a managed provider using a provider slug and model ID
- **THEN** the runtime resolves credentials and constructs the provider
- **AND** the switch is applied at the next turn boundary

#### Scenario: Unmanaged provider switch
- **WHEN** the user provides a direct `Provider` instance for the new model
- **THEN** the runtime uses the provided provider for subsequent turns
- **AND** no credential resolution is performed

### Requirement: The runtime SHALL provide clear UX feedback for model switches
The runtime SHALL emit events and status information that allow clients to display appropriate feedback about model switches, including pending switches, applied switches, and capability differences.

#### Scenario: Switch applied successfully
- **WHEN** a model switch is successfully applied
- **THEN** the client receives a `ModelSwitched` event with source model, target model, and any context adaptations
- **AND** the timeline contains a `ModelSwitched` entry

#### Scenario: Switch rejected
- **WHEN** a model switch cannot be applied (e.g., context too large for target window)
- **THEN** the client receives an error with a clear explanation
- **AND** the session continues with the current model unchanged

### Requirement: Context compaction SHALL support target-model-aware sizing
The compaction engine SHALL accept a target context window size and compact context to fit within that window, rather than using only the current session's context window hint.

#### Scenario: Pre-switch compaction for smaller window
- **WHEN** the runtime initiates compaction as part of a model switch to a smaller window
- **THEN** the compaction engine uses the target model's context window as the size limit
- **AND** the retained tail is sized to fit within the target window

### Requirement: Handoff bundles SHALL include model switch metadata
The `HandoffBundle` structure SHALL include metadata about model switches, including the source model, target model, and any context adaptations performed.

#### Scenario: Export after model switch
- **WHEN** a session with a model switch history is exported as a handoff bundle
- **THEN** the bundle metadata includes the model switch history
- **AND** the current model is recorded in the bundle metadata
