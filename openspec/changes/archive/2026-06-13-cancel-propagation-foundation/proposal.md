## Why

Delegated agent work needs a runtime-level foundation for parent and child sessions before `delegate_task` can safely spawn child work. Today sessions are flat: closing or cancelling a parent session does not express ownership over any child sessions, so delegated work could outlive its parent and remain visible as a normal user session.

## What Changes

- Add runtime APIs to register and unregister parent-to-child session relationships.
- Add recursive child cancellation/closure when a parent session is closed.
- Add session creation options with a `hidden` flag for internal/delegated sessions.
- Add session visibility filtering so callers can list visible sessions by default or include hidden sessions explicitly.
- Add validation so session child relationships remain acyclic, live, and connection-safe.

## Capabilities

### New Capabilities

- `session-cancel-propagation`: Covers runtime parent-child session relationships, recursive cancellation/closure, hidden session creation, and filtered session listing.

### Modified Capabilities

None.

## Impact

- Affects `IronRuntime` session lifecycle APIs and internal session metadata.
- Affects facade and ACP session creation/close/list behavior where they call runtime session APIs.
- Adds tests for recursive cancellation, child relationship validation, and hidden session filtering.
- No new external dependencies are expected.
