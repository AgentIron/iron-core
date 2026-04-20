## ADDED Requirements

### Requirement: The runtime SHALL use a shared pending-interaction envelope for blocking approval and choice workflows
Blocking model/runtime interactions SHALL use one shared turn-level pending-interaction abstraction rather than separate waiting subsystems for approval and choice.

#### Scenario: Turn exposes a unified blocking interaction
- **WHEN** a running turn becomes blocked on structured external input
- **THEN** the runtime SHALL expose a single pending interaction envelope through turn status and event APIs
- **AND** that envelope SHALL identify its typed payload rather than relying on approval-specific waiting state names

### Requirement: The runtime SHALL support model-originated structured choice prompts
When the model needs a bounded user choice before it can continue safely, the runtime SHALL be able to expose that request as a structured pending choice interaction.

#### Scenario: Model requests a single choice
- **WHEN** the model emits a bounded choice request
- **THEN** the runtime SHALL expose a pending interaction envelope with a typed choice payload containing stable option IDs and labels
- **AND** the interaction SHALL identify whether the selection is single or multiple choice

### Requirement: The provider/runtime layer SHALL surface model-originated choice requests as first-class structured events
The runtime SHALL receive model-originated choice prompts through a first-class structured provider/runtime event rather than parsing assistant text.

#### Scenario: Provider emits a choice request
- **WHEN** the model produces a structured choice request during turn execution
- **THEN** the provider/runtime layer SHALL surface that request as a first-class structured event
- **AND** the runtime SHALL validate that event before converting it into a pending choice interaction envelope

#### Scenario: Pending interaction exposes interaction source
- **WHEN** the runtime exposes a pending choice interaction
- **THEN** the interaction envelope SHALL include an interaction identifier
- **AND** the interaction envelope SHALL identify the interaction source

### Requirement: A turn SHALL expose at most one blocking interaction envelope at a time
The runtime SHALL allow only one blocking pending interaction envelope per turn in v1, even if that interaction contains multiple approval items.

#### Scenario: Approval batches multiple calls under one interaction
- **WHEN** multiple tool calls in the same turn iteration require approval
- **THEN** the runtime SHALL expose one pending approval interaction envelope for that turn
- **AND** that approval interaction MAY contain multiple pending approval call items

#### Scenario: Runtime rejects ambiguous multiple blocking interactions
- **WHEN** a provider response attempts to produce more than one blocking interaction or mixes a choice request with approval-gated tool calls in the same unresolved interaction phase
- **THEN** the runtime SHALL reject that provider output deterministically rather than guessing how to continue

### Requirement: Clients SHALL resolve pending interactions with typed interaction resolutions
Clients SHALL resolve pending interactions by referencing the interaction identity and supplying a typed resolution payload whose kind matches the pending interaction kind.

#### Scenario: Client resolves approval interaction as a batch
- **WHEN** a client resolves a pending approval interaction
- **THEN** the response SHALL include the pending interaction identifier
- **AND** the response SHALL include an approval resolution payload containing decisions for the approval items in that interaction envelope

#### Scenario: Runtime rejects mismatched interaction resolution kind
- **WHEN** a client submits an interaction resolution whose kind does not match the pending interaction kind
- **THEN** the runtime SHALL reject that response

#### Scenario: Public API rejects invalid interaction resolution
- **WHEN** a client submits an interaction resolution with invalid selection cardinality, duplicate option IDs, or unknown option IDs
- **THEN** the public interaction-resolution API SHALL reject that response with an explicit error

### Requirement: A turn SHALL pause while waiting for a blocking choice response
If a model-originated choice is required for continuation, the current turn SHALL pause until the user submits or cancels the choice.

#### Scenario: Turn waits for user choice
- **WHEN** the runtime receives a blocking model-originated choice request
- **THEN** the current turn SHALL enter a waiting state
- **AND** the turn SHALL not continue generating model output until the interaction is resolved

### Requirement: Clients SHALL respond with structured choice results
Clients SHALL submit structured choice responses using the interaction identity and selected option identities rather than freeform prose.

#### Scenario: Client submits selected option
- **WHEN** a client resolves a pending choice
- **THEN** the response SHALL include the pending interaction identifier
- **AND** the response SHALL include a typed choice resolution containing selected option IDs or an explicit cancelled status

#### Scenario: Runtime rejects invalid choice response
- **WHEN** a client submits a choice response that references unknown or no-longer-pending interaction IDs, or invalid option IDs
- **THEN** the runtime SHALL reject that response

### Requirement: The runtime SHALL resume the same turn after choice resolution
After a valid choice response is received, the runtime SHALL resume the same paused turn rather than forcing the client to start a new one.

#### Scenario: Turn resumes with resolved choice context
- **WHEN** a valid pending choice response is submitted
- **THEN** the runtime SHALL resume the same turn
- **AND** the model SHALL receive a canonical representation of the resolved choice in continuation context

### Requirement: The runtime SHALL inject a stable structured choice-resolution record into continuation context
The runtime SHALL inject a runtime-defined structured choice-resolution record for resolved choices rather than ad hoc prose.

#### Scenario: Submitted choice includes IDs and labels
- **WHEN** a choice interaction is resolved with submitted selections
- **THEN** the continuation context SHALL include a `choice_resolution` record
- **AND** that record SHALL include the interaction identifier, original prompt, selection mode, submitted status, and selected items with both stable IDs and human-readable labels

#### Scenario: Cancelled choice uses the same record shape
- **WHEN** a choice interaction is cancelled
- **THEN** the continuation context SHALL include a `choice_resolution` record with cancelled status
- **AND** the record SHALL preserve the interaction identifier, original prompt, and selection mode
- **AND** `selected_items` SHALL be an empty list

### Requirement: The runtime SHALL persist resolved choices as structured runtime/system transcript entries
Resolved choices SHALL be stored in structured runtime/system transcript entries rather than serialized into assistant text.

#### Scenario: Resolved choice is stored structurally
- **WHEN** a pending choice interaction is resolved
- **THEN** the session/transcript model SHALL store the `choice_resolution` record as a structured runtime/system entry
- **AND** the resumed turn SHALL consume that structured entry as continuation context
