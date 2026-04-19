## 1. Shared interaction architecture

- [x] 1.1 Generalize approval-specific waiting/event/status types into a shared pending-interaction envelope used by both approval and choice
- [x] 1.2 Define the single-envelope turn invariant for blocking interactions and document that approval remains batch-oriented inside that envelope
- [x] 1.3 Define canonical runtime/model serialization for resolved choices, including `choice_resolution`, prompt, selection mode, status, and selected items with both IDs and labels

## 2. Interaction payloads and validation

- [x] 2.1 Define the typed choice interaction payload schema (`prompt`, `selection_mode`, option items)
- [x] 2.2 Define the approval interaction payload schema for batched pending tool calls
- [x] 2.3 Define structured resolution schemas for approval and choice interactions, including a batch public approval resolution contract and invalid-kind rejection behavior

## 3. Turn pause/resume behavior

- [x] 3.1 Extend turn state/status to represent a single pending blocking interaction envelope
- [x] 3.2 Pause the current turn when a blocking model-originated choice is requested
- [x] 3.3 Resume the same turn after a valid interaction resolution is received, with kind-specific handling for approval vs. choice

## 4. Client-visible APIs

- [x] 4.1 Expose pending interactions through generalized event and status APIs rather than approval-specific surfaces
- [x] 4.2 Add client-facing interaction resolution APIs for pending approvals and pending choices, with approval resolved as a batch interaction envelope
- [x] 4.3 Validate interaction IDs, interaction kinds, and selected option IDs before resuming the turn

## 5. Migration and compatibility

- [x] 5.1 Identify approval-specific public APIs/events that need wrappers, adapters, or deprecation notices under the generalized interaction model
- [x] 5.2 Preserve compatibility for required approval-facing callers while establishing generalized interaction APIs as the canonical surface

## 6. Verification

- [x] 6.1 Add tests for single-choice and multi-choice interaction payloads
- [x] 6.2 Add tests confirming approval batching still works under the shared pending-interaction envelope
- [x] 6.3 Add tests for pausing and resuming a turn through a choice resolution
- [x] 6.4 Add tests for cancelled, invalid, and mismatched interaction resolutions
- [x] 6.5 Add tests covering compatibility wrappers or adapters for approval-facing callers that are retained in v1
- [x] 6.6 Add tests confirming continuation context receives the canonical `choice_resolution` record for both submitted and cancelled choices

## 7. Full choice-request implementation

- [x] 7.1 Extend the provider/runtime protocol to surface a first-class model-originated `ChoiceRequest` event
- [x] 7.2 Define and implement runtime validation for incoming choice requests, including size limits, duplicate item rejection, and empty-payload rejection
- [x] 7.3 Wire the turn driver to convert provider `ChoiceRequest` events into `PendingInteractionPayload::Choice` and `PendingInteractionRequest::Choice`
- [x] 7.4 Enforce v1 exclusivity rules by rejecting multiple simultaneous blocking interactions and mixed choice-plus-approval interaction phases

## 8. Resolution correctness and continuation storage

- [x] 8.1 Move interaction-resolution validation into the public API boundary so invalid or mismatched resolutions return explicit errors
- [x] 8.2 Keep defensive validation in the turn driver so runtime state remains protected from invalid interaction resolutions
- [x] 8.3 Introduce a structured runtime/system transcript entry for resolved choices instead of injecting serialized JSON into assistant text
- [x] 8.4 Resume the same turn using the structured `choice_resolution` transcript entry as continuation context

## 9. End-to-end verification

- [x] 9.1 Add end-to-end tests proving a model-originated choice request pauses a turn and emits `InteractionRequired` with a choice payload
- [x] 9.2 Add end-to-end tests proving submitted and cancelled choice resolutions resume the same turn correctly
- [x] 9.3 Add end-to-end tests proving invalid or mismatched choice resolutions are explicitly rejected by the public API
- [x] 9.4 Add end-to-end tests proving ambiguous provider output (multiple choice requests or mixed choice-plus-approval phases) fails deterministically
- [x] 9.5 Add end-to-end tests proving resolved choices are persisted as structured runtime/system transcript entries rather than assistant text
