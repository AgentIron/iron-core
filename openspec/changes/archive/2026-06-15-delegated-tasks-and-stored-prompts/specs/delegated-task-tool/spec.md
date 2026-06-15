## ADDED Requirements

### Requirement: Core SHALL provide a delegated task tool

The system SHALL provide a model-callable `delegate_task` tool that starts a bounded child-agent run for a goal, optional context, optional profile, and optional iteration limit.

#### Scenario: Delegate task schema is exposed
- **WHEN** `delegate_task` is registered as a tool
- **THEN** its input schema includes required `goal` text
- **AND** supports optional `context`, optional `profile`, and optional `max_iterations`

#### Scenario: Delegated run returns identifiers
- **WHEN** a delegated child run completes
- **THEN** the tool result includes the child session ID
- **AND** includes a delegation ID or equivalent audit handle
- **AND** includes the final child outcome

#### Scenario: Invalid delegate arguments are rejected
- **WHEN** the model calls `delegate_task` without a valid goal or with invalid bounds
- **THEN** the tool returns an actionable error
- **AND** no child session is created

### Requirement: Delegated child sessions SHALL be hidden by default and inspectable

The system SHALL create delegated child sessions as hidden from normal session listings by default, while exposing explicit APIs for frontend code to inspect hidden delegated sessions.

#### Scenario: Child session is hidden from default listing
- **WHEN** `delegate_task` creates a child session
- **THEN** the child session is hidden from normal session listings by default
- **AND** the parent session remains visible according to normal session visibility rules

#### Scenario: Frontend can inspect child sessions
- **WHEN** frontend code requests delegated children for a parent session or hidden delegated sessions
- **THEN** the system returns the delegated child session metadata needed to inspect or open those sessions

#### Scenario: Hidden does not mean unauditable
- **WHEN** a delegated child session is hidden from default listings
- **THEN** the system still retains durable child session history and parent-child relationship metadata needed for audit

### Requirement: Delegation SHALL preserve normal approval workflow semantics

The system SHALL evaluate approval for `delegate_task` itself through the existing tool approval strategy and workflow rather than hardcoding special approval behavior for delegation.

#### Scenario: Delegate task follows configured approval behavior
- **WHEN** the model proposes a `delegate_task` tool call
- **THEN** the runtime evaluates whether approval is required using the same approval workflow used for other tools
- **AND** approval, denial, or cancellation of the `delegate_task` call is recorded like other tool calls

#### Scenario: Denied delegate task does not create child
- **WHEN** a `delegate_task` tool call is denied before execution
- **THEN** no child session is created
- **AND** the parent tool call records the denied outcome

### Requirement: Delegated child approval requests SHALL be routable

The system SHALL support per-delegation child approval routing modes that either auto-approve child tool calls for that child run or propagate child approval requests to the parent/main UI one-by-one.

#### Scenario: Child tool calls are auto-approved for a delegated run
- **WHEN** a delegated run is configured with child tool auto-approval
- **AND** the child model calls a visible child tool that would otherwise require approval
- **THEN** the child tool call proceeds without prompting the user for that individual child call
- **AND** the child durable history records the tool call outcome

#### Scenario: Child approval request is propagated to parent UI
- **WHEN** a delegated run is configured to propagate child approvals
- **AND** the child model calls a visible child tool that requires approval
- **THEN** the approval request is sent to the parent/main UI approval workflow
- **AND** the child's tool execution follows the returned allow, deny, or cancel verdict

#### Scenario: Propagated denial is recorded in child history
- **WHEN** a propagated child approval request is denied
- **THEN** the child tool call is not executed
- **AND** the child session records a denied terminal tool outcome

### Requirement: Delegated child sessions SHALL participate in cancellation propagation

The system SHALL register delegated child sessions under their parent session so parent cancellation or closure propagates to active child prompts and descendants.

#### Scenario: Parent cancellation cancels child prompt
- **WHEN** a parent session with an active delegated child prompt is cancelled or closed
- **THEN** the child prompt is cancelled
- **AND** in-flight child tool records are tied off with terminal cancellation outcomes

#### Scenario: Delegation cleanup removes relationship
- **WHEN** a delegated child run completes or fails
- **THEN** runtime cleanup unregisters the child relationship when appropriate
- **AND** does not leave stale parent-child edges

### Requirement: Delegated child tool catalogs SHALL be policy-bounded

The system SHALL derive delegated child tool catalogs from an explicit sub-agent tool policy with the selected profile's `ToolFilter` as a hard boundary.

#### Scenario: Default child catalog inherits parent effective tools
- **WHEN** a delegated run uses the default sub-agent tool policy
- **THEN** the child catalog starts from the parent session's effective tool catalog
- **AND** the selected profile's `ToolFilter` is applied to that catalog

#### Scenario: Blocklist removes inherited tools
- **WHEN** sub-agent configuration blocklists a tool name
- **THEN** that tool is absent from the child catalog even if it was present in the parent effective catalog

#### Scenario: Profile filter bounds configured additions
- **WHEN** sub-agent configuration adds a tool that the selected profile's `ToolFilter` does not allow
- **THEN** the tool is absent from the child catalog
- **AND** the system reports that profile policy excluded the requested addition

#### Scenario: Added tools are privilege expansions when absent from parent
- **WHEN** sub-agent configuration adds a tool not present in the parent session's effective catalog
- **THEN** the delegation metadata reports the added tool as a privilege expansion
- **AND** the frontend-visible approval context can show the expansion before or during delegated execution

#### Scenario: Final child catalog has deterministic digest
- **WHEN** a delegated child catalog is built
- **THEN** the system computes a deterministic digest over the final model-visible tool definitions and approval flags
- **AND** the digest is available in delegation metadata

### Requirement: Delegated runs SHALL use selected profile provider resolution

The system SHALL resolve the selected child profile through existing profile provider resolution before running the child prompt.

#### Scenario: Delegated run uses default profile when unspecified
- **WHEN** `delegate_task` does not specify a profile
- **THEN** the delegated child run uses the built-in default profile

#### Scenario: Delegated run uses selected managed profile
- **WHEN** `delegate_task` specifies a managed profile
- **THEN** the runtime resolves the managed provider through the existing profile provider resolution path
- **AND** provider resolution errors are returned as child run failures without exposing credential secrets
