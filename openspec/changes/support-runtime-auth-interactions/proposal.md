## Why

Authentication workflows are different from both plugin presentation and model-originated disambiguation. They are runtime-owned workflows that often need direct user action, but they do not require the model to mediate every step. Treating auth as either a plugin-defined interaction or a model choice would blur responsibility and make clients harder to implement consistently.

The runtime already owns auth state and availability for plugins. It should also own the client-facing interaction surfaces needed to start, monitor, and complete those auth flows.

## What Changes

- Introduce runtime-owned auth interaction surfaces for clients.
- Define structured auth prompts and auth status views that clients can render without routing through the model.
- Allow clients to trigger runtime-owned auth actions directly.
- Preserve transcript and state visibility so the rest of the session can observe auth progress and final outcome.

## Capabilities

### New Capabilities
- `runtime-auth-interactions`: Allows the runtime to expose structured auth prompts and auth status interactions directly to clients without requiring model mediation.

### Modified Capabilities
- `wasm-integration-plugins`: Extends plugin/runtime auth visibility so clients can start and observe runtime-owned auth flows for plugin-backed tools.

## Impact

- Affected code: likely runtime/plugin auth state handling, client-visible event/status APIs, facade surfaces, and plugin auth tests.
- Affected APIs: runtime inspection/status APIs, auth-start and auth-complete client flows, and transcript/event visibility for auth transitions.
- Client impact: clients can render auth prompts and status directly, while still exposing auth outcomes coherently to users and sessions.
