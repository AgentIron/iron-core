# runtime-auth-interactions Specification

## Purpose

This specification defines client-visible runtime auth interaction surfaces so clients can render auth state and start runtime-owned auth flows without model mediation.

## Requirements
### Requirement: The runtime SHALL expose structured auth interaction state to clients
When plugin-backed capabilities require authentication, the runtime SHALL expose structured auth prompt/status information directly to clients.

#### Scenario: Client observes unauthenticated auth state
- **WHEN** a plugin-backed capability requires authentication and is currently unauthenticated
- **THEN** the runtime SHALL expose structured auth interaction state that clients can render

#### Scenario: Client observes authenticated auth state
- **WHEN** the auth flow completes successfully
- **THEN** the runtime SHALL expose the updated authenticated state to clients

### Requirement: Clients SHALL be able to start runtime-owned auth flows directly
Clients SHALL be able to trigger runtime-owned auth actions without requiring model mediation.

#### Scenario: Client starts auth directly
- **WHEN** a user chooses to connect an auth-gated plugin capability
- **THEN** the client SHALL be able to invoke the runtime-owned auth start flow directly

### Requirement: Auth state transitions SHALL remain observable to the session
Even when auth does not route through the model, the resulting state changes SHALL remain visible through transcript or state/event surfaces.

#### Scenario: Session observes auth completion
- **WHEN** auth completes successfully
- **THEN** the session SHALL expose that auth state transition through the runtime's client-visible surfaces
