## MODIFIED Requirements

### Requirement: Auth-mediated plugin availability SHALL be directly understandable to clients
In v1, plugins SHALL declare auth requirements, requested scopes, and per-tool auth dependencies, but SHALL NOT define their own auth lifecycle semantics. The runtime SHALL be authoritative for auth state, credential bindings, and tool availability. Clients SHALL provide the user interaction surfaces required to complete authentication flows exposed by the runtime.

#### Scenario: Client starts runtime-owned auth flow
- **WHEN** a plugin is enabled but unavailable because required auth is missing
- **THEN** the runtime SHALL expose enough structured auth state for the client to start the runtime-owned auth flow directly

#### Scenario: Client observes auth state changes after direct auth interaction
- **WHEN** a client starts or completes a runtime-owned auth flow
- **THEN** the runtime SHALL recompute plugin tool availability and expose the resulting auth state to the client
