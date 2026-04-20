## MODIFIED Requirements

### Requirement: Session-scoped MCP enablement
Each session SHALL independently track whether each configured MCP server is enabled or disabled for that session. Enabling or disabling an MCP server for one session SHALL NOT change the enablement state of any other session. New sessions SHALL initialize their MCP enablement state from a single runtime-level default that can be configured as enabled-by-default or disabled-by-default.

#### Scenario: New session uses runtime default enablement
- **WHEN** a new session is created
- **THEN** the session initializes MCP server enablement according to the runtime's configured default for new sessions

#### Scenario: Session toggle does not affect another session
- **WHEN** one session disables a configured MCP server
- **THEN** another session retains its own independent enablement state for that server

#### Scenario: Runtime default applies uniformly across configured servers
- **WHEN** a new session is created under a runtime whose MCP default policy is enabled or disabled
- **THEN** the session initializes each configured MCP server using that same runtime-level policy
- **THEN** per-server metadata does not override the single runtime-level default used for new-session MCP enablement

### Requirement: MCP tool calls reuse the standard runtime execution lifecycle
The runtime SHALL execute MCP-backed tool calls through the same validation, approval, durable recording, and result propagation lifecycle used for other runtime tools, while delegating the actual call to the connected MCP server. The canonical session-effective runtime path SHALL apply the configured runtime approval strategy rather than relying only on the tool's raw `requires_approval()` flag.

#### Scenario: Model-issued MCP tool call completes through the runtime
- **WHEN** the model issues a tool call for an MCP-backed tool that is visible for the current session
- **THEN** the runtime validates arguments against the discovered input schema
- **THEN** the runtime applies the standard approval and durable tool-record lifecycle
- **THEN** the runtime calls the backing MCP server tool and records the returned result or failure

#### Scenario: Child MCP tool call follows the same execution path
- **WHEN** a child tool execution path such as `python_exec` invokes a visible MCP-backed tool
- **THEN** the runtime executes that call through the same validation, approval, and durable-record flow used for non-child tool calls

#### Scenario: Approval strategy always forces approval for MCP tools
- **WHEN** the runtime approval strategy is configured as `Always` and a visible MCP-backed tool is invoked through the session-effective runtime path
- **THEN** the runtime requires explicit approval even if that tool's raw definition would not otherwise require it

#### Scenario: Approval strategy never skips approval for MCP tools
- **WHEN** the runtime approval strategy is configured as `Never` and a visible MCP-backed tool is invoked through the session-effective runtime path
- **THEN** the runtime does not request approval solely because the MCP-backed tool definition reports `requires_approval()`

### Requirement: Effective MCP tool exposure depends on enablement and server health
The system SHALL expose MCP-backed tools to the model only when the backing MCP server is both enabled for the current session and currently connected or otherwise usable in the runtime. MCP-backed tools from servers in error or unavailable states SHALL be excluded from the effective tool surface presented to the model. The same enablement and health gates SHALL apply to runtime tool execution so MCP-backed tools that are not visible for a session are also not executable for that session.

#### Scenario: Enabled usable server contributes tools to prompt construction
- **WHEN** a configured MCP server is enabled for the current session and is currently usable in the runtime
- **THEN** tools from that server are included in the effective tool surface used to build the provider request for that session

#### Scenario: Prompt construction uses the session-effective tool surface
- **WHEN** the runtime builds a provider request for a session
- **THEN** the provider-visible tool definitions come from the same session-effective tool surface used for approval and execution
- **THEN** enabled, healthy MCP-backed tools are visible to the model
- **THEN** disabled or unhealthy MCP-backed tools are not visible to the model

#### Scenario: Disabled server tools are hidden
- **WHEN** a configured MCP server is disabled for the current session
- **THEN** tools from that server are excluded from the effective tool surface for that session
- **THEN** tool calls for that server are rejected as unavailable for that session

#### Scenario: Errored server tools are hidden
- **WHEN** a configured MCP server enters an error state
- **THEN** tools from that server are excluded from the effective tool surface for every session until the server becomes usable again

#### Scenario: Reconnected server tools return for enabled sessions
- **WHEN** a configured MCP server becomes usable again after an error and rediscovery succeeds
- **THEN** its tools are restored to the effective tool surface for sessions where that server remains enabled

#### Scenario: Unavailable MCP execution reports a precise reason
- **WHEN** a caller attempts to execute an MCP-backed tool that is not executable for the current session
- **THEN** the runtime reports whether the tool is unavailable because the server is disabled for that session, because the server is unhealthy, or because the tool name is unknown for that server
- **THEN** the runtime does not report an ambiguous or session-inaccurate unavailable-tool reason
