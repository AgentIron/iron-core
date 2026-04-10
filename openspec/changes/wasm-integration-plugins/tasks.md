## 1. Plugin Runtime Foundations

- [x] 1.1 Add a runtime-local plugin registry and plugin state model distinct from built-in tools and MCP servers
- [x] 1.2 Add plugin configuration types for local-path and HTTPS sources, including remote checksum requirements
- [x] 1.3 Integrate a WASM execution host and plugin lifecycle management for install, load, unload, and health tracking

## 2. Metadata, Auth, and Policy Surfaces

- [x] 2.1 Define the plugin manifest and client-facing metadata structures for identity, publisher, presentation, network policy, and exported tools
- [x] 2.2 Add runtime status structures that separate plugin health, auth availability, and user-facing action/status hints
- [x] 2.3 Add strict v1 auth metadata for manifest-declared OAuth requirements, requested scopes, and per-tool auth dependencies
- [x] 2.4 Add a runtime-governed auth state model and credential binding interfaces so `iron-core` remains authoritative for auth availability and tool gating
- [x] 2.5 Add client-auth interaction APIs so clients can provide browser launch, redirect/code capture, and other user interaction surfaces required by the runtime
- [x] 2.6 Add declared network policy handling for allowlists, blocklists, and wildcard outbound access metadata

## 3. Session and Tool Exposure Integration

- [x] 3.1 Add session-scoped plugin enablement state and runtime defaults for new sessions
- [x] 3.2 Extend effective tool composition to merge built-in, MCP, and plugin-backed tools using enablement, health, and auth availability gates
- [x] 3.3 Exclude plugin inventory, auth bindings, and session enablement from handoff export/import

## 4. Verification and Validation

- [x] 4.1 Add tests for remote checksum validation and local-path plugin loading behavior
- [x] 4.2 Add tests for plugin metadata/status inspection and client-visible auth availability states
- [x] 4.3 Add tests for runtime-governed auth state transitions and per-tool availability gating under authenticated, unauthenticated, expired, and revoked states
- [x] 4.4 Add tests for session-scoped enablement and effective tool visibility under healthy, unauthenticated, partial-auth, and error states
- [x] 4.5 Add tests confirming plugin runtime state is excluded from handoff bundles
