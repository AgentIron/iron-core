## Why

Some model workflows cannot proceed safely without structured user disambiguation. Today the runtime can pause for tool approval, but it does not have a first-class mechanism for the model to request a bounded user choice, pause the current turn, receive a structured response, and then resume the same turn with that answer.

If this is handled as plain chat text, the runtime loses structure, clients must scrape or synthesize meaning, and the model cannot reliably distinguish user-selected option IDs from freeform prose. That is too weak for a clean multi-client interaction contract.

## What Changes

- Generalize the existing approval pause/resume architecture into a shared pending-interaction envelope used by both approval and choice workflows.
- Introduce model-originated structured choice prompts as a first-class pending interaction payload within that shared interaction system.
- Extend the provider/runtime protocol so a running turn can receive a first-class model-originated choice request rather than relying on ad hoc transcript text.
- Allow a running turn to pause while waiting for a user choice, then resume when the client submits a structured response.
- Expose pending choice requests through the generalized interaction event and status APIs so clients can render them appropriately.
- Define a structured client response format for submitted or cancelled choices.
- Preserve transcript continuity by giving the model a canonical structured representation of the resolved choice through a structured runtime/system transcript entry rather than assistant text stuffing.

## Capabilities

### New Capabilities
- `model-choice-prompts`: Allows the model/runtime to issue structured user-choice requests, pause a turn awaiting resolution, and resume the same turn when the user responds.

### Modified Capabilities
- Existing approval-facing turn interaction surfaces are generalized under a shared pending-interaction model while preserving approval batching semantics.

## Impact

- Affected code: turn state management, existing approval pause/resume logic, provider/runtime event protocol, event/status APIs, facade/session handle surfaces, session/transcript model, and prompt/continuation logic.
- Affected APIs: provider events, turn status, stream events, client input/resume APIs, approval handling surfaces, and transcript/model input normalization for resolved choices.
- Client impact: clients can render bounded user choices as buttons/lists/menus while chat-only clients can still display the prompt text and return a structured selection.
- Compatibility impact: approval-specific public surfaces may need compatibility wrappers or a staged migration path if generalized interaction APIs replace approval-specific names.
