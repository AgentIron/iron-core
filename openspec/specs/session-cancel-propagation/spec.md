# session-cancel-propagation Specification

## Purpose
This specification defines how cancellation and closure requests propagate through the runtime's parent/child session graph. It is intended for implementers of the ACP runtime and session lifecycle layer. The spec guarantees that cancelling or closing a parent session deterministically cancels or closes all registered descendant sessions owned by the same connection, and that child relationships can be registered and unregistered safely. It does not define cross-connection cancellation, persistence of cancellation state, or user-visible cancellation UI.
## Requirements
### Requirement: Runtime Tracks Parent Child Session Relationships

The runtime SHALL allow callers to register and unregister parent-to-child relationships between live sessions owned by the same connection.

#### Scenario: Register valid child relationship
- **WHEN** two live sessions owned by the same connection are registered as parent and child
- **THEN** the runtime records the relationship for recursive cancellation and closure

#### Scenario: Unregister existing child relationship
- **WHEN** an existing parent-to-child relationship is unregistered
- **THEN** the runtime removes the relationship without closing either session

### Requirement: Runtime Rejects Invalid Child Relationships

The runtime SHALL reject child relationship registrations that reference missing sessions, use the same session as parent and child, cross connection ownership boundaries, assign a child to more than one parent, or create a cycle.

#### Scenario: Reject self parent relationship
- **WHEN** a caller registers a session as its own child
- **THEN** the runtime rejects the relationship and leaves existing relationships unchanged

#### Scenario: Reject cross connection relationship
- **WHEN** a caller registers a child session owned by a different connection than the parent session
- **THEN** the runtime rejects the relationship and leaves existing relationships unchanged

#### Scenario: Reject cyclic relationship
- **WHEN** a caller registers a relationship that would make an ancestor become a descendant of itself
- **THEN** the runtime rejects the relationship and leaves existing relationships unchanged

### Requirement: Closing Parent Session Recursively Cancels And Closes Descendants

The runtime SHALL recursively signal cancellation for active prompts in descendant sessions and remove descendant sessions when a parent session is closed.

#### Scenario: Close parent with active child prompt
- **WHEN** a parent session with a registered child session is closed while the child has an active prompt
- **THEN** the runtime signals cancellation for the child prompt and removes the child session

#### Scenario: Close parent with nested descendants
- **WHEN** a parent session with children and grandchildren is closed
- **THEN** the runtime recursively closes all descendants and removes their child relationships

#### Scenario: Close child leaves parent alive
- **WHEN** a child session is closed directly
- **THEN** the runtime removes the child session and its descendant sessions while leaving the parent session alive

### Requirement: Runtime Supports Hidden Session Creation

The runtime SHALL support session creation options with a `hidden` flag and SHALL create visible sessions by default for existing session creation paths.

#### Scenario: Create visible session by default
- **WHEN** a caller creates a session without explicit creation options
- **THEN** the runtime creates a visible session

#### Scenario: Create hidden session with options
- **WHEN** a caller creates a session with `hidden` set to true
- **THEN** the runtime creates a live owned session marked hidden

### Requirement: Session Listing Filters Hidden Sessions

The runtime SHALL filter hidden sessions from normal session listings and SHALL provide an explicit include-hidden listing mode.

#### Scenario: Normal listing excludes hidden sessions
- **WHEN** a connection has one visible session and one hidden session
- **THEN** the normal session listing returns only the visible session

#### Scenario: Include hidden listing returns all sessions
- **WHEN** a connection has one visible session and one hidden session
- **THEN** the include-hidden session listing returns both sessions

