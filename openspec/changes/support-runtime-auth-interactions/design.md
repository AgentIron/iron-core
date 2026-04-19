## Context

`iron-core` already owns plugin auth state, auth availability, and runtime visibility of whether a plugin tool can execute. That means auth is not really a plugin-defined UI problem and not really a model disambiguation problem. It is a runtime-owned workflow that clients need to present clearly.

This change defines auth interaction surfaces that are controlled by the runtime and can be used by clients directly without going through the model.

## Goals / Non-Goals

**Goals**
- Expose structured auth prompts and auth status to clients.
- Allow clients to start and observe auth flows directly through runtime-owned APIs.
- Preserve transcript/state visibility so auth outcomes remain visible to users and the wider session.

**Non-Goals**
- Plugin-defined auth UI protocols.
- Model-mediated auth initiation.
- A generalized plugin action framework.

## Decisions

### Auth interactions are runtime-owned
The runtime should expose auth prompts and auth status using runtime-defined structures. Plugins may depend on auth, but they should not define the auth interaction protocol.

### Clients may start auth directly
When a user chooses to connect an account or satisfy an auth requirement, the client should be able to invoke the runtime-owned auth flow directly rather than routing that action through the model.

### Auth remains observable to the session
Even though the model does not mediate auth initiation, auth state changes and outcomes should still be visible through transcript or state/event surfaces so the session can reflect the new capability state.

## Proposed Auth UX Model

### Auth prompt

```json
{
  "auth_id": "plugin:github",
  "state": "unauthenticated",
  "title": "Connect GitHub",
  "description": "This plugin needs GitHub access before it can continue."
}
```

### Auth status transition

```json
{
  "auth_id": "plugin:github",
  "state": "pending"
}
```

then

```json
{
  "auth_id": "plugin:github",
  "state": "authenticated"
}
```

## Risks / Trade-offs

- **Auth UX may diverge across clients**.
  - Mitigation: keep the runtime-owned auth payload simple and state-based.

- **Auth flow initiation may still require client-specific integration**.
  - Mitigation: keep the runtime authoritative for state, while allowing clients to render and launch the flow appropriately.

## Migration Plan

1. Define runtime-owned auth prompt/status structures.
2. Expose them through runtime/facade/client APIs.
3. Add direct client auth-start/auth-complete flows.
4. Preserve transcript/state visibility for auth transitions.
5. Add tests for auth prompt visibility and state changes.

## Open Questions

- Which auth transitions should be emitted as transcript-visible updates versus inspection-only state changes?
- Should auth prompts be exposed only through inspection APIs, or also as live event surfaces?
