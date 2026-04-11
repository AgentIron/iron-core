## ADDED Requirements

### Requirement: Plugin-backed tool calls reuse the standard runtime execution lifecycle
The runtime SHALL execute plugin-backed tool calls through the same canonical session-effective runtime path used for local and MCP-backed tools. That path SHALL perform schema validation, approval handling, durable tool-call recording, and result propagation before and after delegating execution to the Extism/WASM host.

#### Scenario: Model-issued plugin tool call completes through the runtime
- **WHEN** the model issues a tool call for a visible plugin-backed tool
- **THEN** the runtime validates the arguments against the exported plugin tool schema
- **THEN** the runtime applies the configured approval strategy and durable tool-record lifecycle
- **THEN** the runtime executes the tool through the Extism/WASM host and records the returned result or failure

#### Scenario: Embedded Python child-tool call reaches plugin-backed tool
- **WHEN** embedded Python invokes a plugin-backed tool that is visible in the session-effective runtime tool catalog
- **THEN** the runtime executes that tool through the same child-tool execution path used for local and MCP-backed tools

### Requirement: Client-visible plugin inspection and session control APIs
The system SHALL provide client-visible APIs to inspect runtime plugin inventory, per-plugin metadata, per-tool availability, runtime status, and auth-related availability. The system SHALL also provide session-scoped APIs to enable or disable installed plugins for an individual session without changing runtime inventory state.

#### Scenario: Client toggles plugin for a session
- **WHEN** a client enables or disables an installed plugin for a session
- **THEN** the session's effective tool surface reflects that change on subsequent prompt construction and tool execution
- **THEN** the toggle does not change runtime plugin inventory or another session's enablement state

#### Scenario: Client inspects auth-mediated tool availability
- **WHEN** a client requests plugin details for an installed plugin
- **THEN** the response includes runtime health, auth availability, and per-tool availability details separately from model-facing tool definitions

## MODIFIED Requirements

### Requirement: Runtime-local WASM plugin inventory
The runtime SHALL maintain a runtime-local inventory of installed WASM integration plugins distinct from built-in tools and MCP servers. Each installed plugin SHALL have a stable runtime identity and SHALL expose client-visible metadata including its source, manifest identity, runtime health, and exported tool summaries. The runtime SHALL remain authoritative for plugin install/load state even when a plugin is disabled for a given session.

#### Scenario: Client lists installed plugins
- **WHEN** a client requests the plugin inventory
- **THEN** the runtime returns all plugins installed in that runtime
- **THEN** each plugin includes its stable identity, source information, runtime health, and exported tool summaries

#### Scenario: Plugin inventory is local to the runtime
- **WHEN** a session is exported and later imported into another runtime
- **THEN** the destination runtime does not assume the source runtime's plugin inventory exists locally

#### Scenario: Session default disablement does not change runtime health
- **WHEN** a plugin is installed successfully but runtime policy disables it by default for new sessions
- **THEN** the runtime inventory still reports the plugin's actual runtime health and install/load state
- **THEN** per-session enablement intent is exposed separately from runtime health

### Requirement: Remote plugin loading requires checksum verification
The runtime SHALL support loading plugins from local file paths and HTTPS URLs. Plugins loaded from HTTPS URLs SHALL require a valid checksum, and the runtime SHALL reject remote plugins whose checksum is missing or does not match the fetched artifact. Successful installation SHALL produce a runtime-owned plugin artifact whose manifest and exported tool metadata are available to the runtime inventory.

#### Scenario: Remote plugin with valid checksum loads successfully
- **WHEN** a client configures a remote plugin with a checksum that matches the fetched artifact
- **THEN** the runtime accepts the plugin for installation
- **THEN** the runtime persists or normalizes the fetched artifact into a runtime-owned installed plugin entry
- **THEN** the runtime extracts or loads the plugin manifest and exported tool metadata

#### Scenario: Remote plugin without checksum is rejected
- **WHEN** a client configures a remote plugin URL without a checksum
- **THEN** the runtime rejects the plugin configuration

#### Scenario: Remote plugin with invalid checksum is rejected
- **WHEN** a client configures a remote plugin whose fetched artifact does not match the provided checksum
- **THEN** the runtime rejects the plugin configuration

### Requirement: Plugins expose structured metadata and runtime status
Each plugin SHALL expose structured metadata for identity, publisher, network policy, auth requirements, exported tools, and user-facing status so clients can present plugin setup and health consistently across different interfaces. The runtime SHALL expose machine-readable plugin status separately from model-facing tool definitions. Exported tool metadata SHALL include enough information for the runtime to determine auth gating and approval behavior, including per-tool auth dependencies and required scopes.

#### Scenario: Client inspects plugin metadata
- **WHEN** a client requests details for an installed plugin
- **THEN** the runtime returns structured metadata for identity, publisher, network policy, auth requirements, and exported tools

#### Scenario: Client inspects plugin runtime status
- **WHEN** a client requests the current plugin status
- **THEN** the runtime returns machine-readable health and availability information separate from tool definitions

#### Scenario: Client inspects per-tool availability metadata
- **WHEN** a client requests details for an installed plugin tool
- **THEN** the runtime returns metadata describing whether the tool requires auth, which scopes it requires, and whether it requires approval

### Requirement: Session-scoped plugin enablement
Each session SHALL independently track whether each installed plugin is enabled or disabled for that session. Enabling or disabling a plugin for one session SHALL NOT change the enablement state of any other session. New sessions SHALL initialize plugin enablement according to a runtime-level default policy.

#### Scenario: New session initializes plugin enablement from runtime defaults
- **WHEN** a new session is created
- **THEN** the session initializes plugin enablement using the runtime's configured default policy

#### Scenario: Session toggle does not affect another session
- **WHEN** one session disables an installed plugin
- **THEN** another session retains its own independent enablement state for that plugin

### Requirement: Effective plugin tool exposure depends on enablement, health, and auth availability
The runtime SHALL expose plugin-backed tools to the model only when the backing plugin is enabled for the current session, the plugin is runtime-healthy enough to execute, and the tool's declared auth requirements are satisfied. The runtime SHALL support partial tool availability for a plugin when some tools are callable and others are gated by auth or status. The same session-effective runtime tool surface SHALL be used for provider request construction, approval checks, runtime execution, and embedded Python child-tool visibility.

#### Scenario: Disabled plugin tools are hidden
- **WHEN** a plugin is disabled for the current session
- **THEN** tools from that plugin are excluded from the effective tool surface for that session

#### Scenario: Unhealthy plugin tools are hidden
- **WHEN** a plugin enters a runtime error state
- **THEN** tools from that plugin are excluded from the effective tool surface until the plugin becomes healthy again

#### Scenario: Plugin with insufficient auth exposes only eligible tools
- **WHEN** a plugin is enabled and healthy but only some tool auth requirements are satisfied
- **THEN** the runtime exposes only the tools whose auth requirements are satisfied

#### Scenario: Prompt construction uses the canonical session-effective plugin surface
- **WHEN** the runtime builds a provider request for a session
- **THEN** plugin-backed tool definitions included in the provider-visible tool surface come from the same session-effective runtime tool catalog used for approval and execution

### Requirement: Strict v1 auth is runtime-governed and client-mediated
In v1, plugins SHALL declare auth requirements, requested scopes, and per-tool auth dependencies, but SHALL NOT define their own auth lifecycle semantics. The runtime SHALL be authoritative for auth state, credential bindings, and tool availability. Clients SHALL provide the user interaction surfaces required to complete authentication flows exposed by the runtime.

#### Scenario: Plugin requires user authentication
- **WHEN** an enabled plugin requires authentication before its tools can be used
- **THEN** the runtime reports an auth-related availability state for that plugin to the client
- **THEN** the client can use that state to guide the user through authentication

#### Scenario: Client distinguishes auth availability from plugin health
- **WHEN** a client inspects plugin status
- **THEN** the response distinguishes runtime health from auth availability

#### Scenario: Runtime controls tool availability after authentication changes
- **WHEN** a plugin's auth state changes from unauthenticated to authenticated or from authenticated to expired or revoked
- **THEN** the runtime recomputes effective plugin tool availability according to the tool metadata declared by that plugin

#### Scenario: Plugin cannot override auth state vocabulary
- **WHEN** a plugin declares OAuth requirements
- **THEN** the runtime maps that plugin into the runtime's standardized auth state model rather than accepting plugin-defined auth lifecycle states

#### Scenario: Per-tool scope requirements affect availability
- **WHEN** a plugin declares per-tool auth dependencies or scopes and the current credential binding satisfies only some of them
- **THEN** the runtime marks only the eligible plugin tools as available
- **THEN** the runtime exposes the remaining plugin tools as unavailable with auth-related availability metadata
