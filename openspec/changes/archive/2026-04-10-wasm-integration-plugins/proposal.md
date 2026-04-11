## Why

AgentIron needs a first-class integration model for modern knowledge workers that does not assume engineering-oriented deployment patterns like MCP servers, shell access, or API-key setup. A runtime-local WASM integration system would let `iron-core` expose user-friendly OAuth and web-service tools while preserving isolation, portability boundaries, and room for trusted third-party extension.

## What Changes

- Add a runtime-local WASM integration plugin system distinct from built-in tools and MCP servers.
- Support loading plugins from local file paths and from HTTPS URLs, with remote plugins requiring a valid checksum.
- Introduce plugin manifests and client-facing metadata so clients can present integrations consistently across GUI, TUI, and chat surfaces.
- Add runtime inventory, health tracking, and session-scoped enablement for plugins, analogous to MCP but extended with plugin-specific availability state.
- Define a strict v1 auth and network capability contract where plugins declare OAuth requirements, `iron-core` owns auth state and tool availability, and clients provide the user interaction surface.
- Exclude plugin inventory, auth bindings, and session enablement from handoff bundles so portability remains runtime-local.

## Capabilities

### New Capabilities
- `wasm-integration-plugins`: Runtime-local WASM integration plugins with manifest-driven metadata, checksum-verified remote loading, session-scoped enablement, and strict runtime-governed OAuth flows mediated through clients.

### Modified Capabilities

## Impact

- Adds a new runtime subsystem for plugin inventory, loading, isolation, and effective tool exposure.
- Introduces new client/runtime APIs for plugin inspection, status, enablement, and auth interaction.
- Likely adds a WASM execution dependency such as Extism plus related manifest/verification logic.
- Extends prompt construction and tool exposure logic to merge built-in, MCP, and plugin-backed tools.
- Establishes a new security boundary around remote artifact verification, network permissions, manifest-declared auth requirements, and runtime-owned credential handling.
