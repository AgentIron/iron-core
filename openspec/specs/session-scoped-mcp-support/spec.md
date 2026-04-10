# session-scoped-mcp-support Specification

## Purpose

This specification defines the session-scoped MCP (Model Context Protocol) support for iron-core, enabling runtime-local MCP server inventory, per-session enablement, and effective tool exposure rules.

## Requirements

### Requirement: Runtime-local MCP server inventory
The runtime SHALL maintain a runtime-local inventory of configured MCP servers. Each configured server SHALL have a stable runtime identity and SHALL expose client-visible metadata including transport type, connection health, and discovered tool summaries when available. The runtime SHALL be authoritative for which MCP servers are available to it, independent from any specific client process.

#### Scenario: Client lists configured runtime MCP servers
- **WHEN** a client requests the runtime MCP server inventory
- **THEN** the runtime returns all configured MCP servers known to that runtime
- **THEN** each returned server includes its runtime identity, transport type, and current health state

#### Scenario: Runtime inventory is local to the current runtime
- **WHEN** a session is handed off or imported into another runtime
- **THEN** the destination runtime does not assume the source runtime's MCP server inventory exists locally

### Requirement: Session-scoped MCP enablement
Each session SHALL independently track whether each configured MCP server is enabled or disabled for that session. Enabling or disabling an MCP server for one session SHALL NOT change the enablement state of any other session. New sessions SHALL initialize their MCP enablement state from a single runtime-level default that can be configured as enabled-by-default or disabled-by-default.

#### Scenario: New session uses runtime default enablement
- **WHEN** a new session is created
- **THEN** the session initializes MCP server enablement according to the runtime's configured default for new sessions

#### Scenario: Session toggle does not affect another session
- **WHEN** one session disables a configured MCP server
- **THEN** another session retains its own independent enablement state for that server

### Requirement: Effective MCP tool exposure depends on enablement and server health
The system SHALL expose MCP-backed tools to the model only when the backing MCP server is both enabled for the current session and currently connected or otherwise usable in the runtime. MCP-backed tools from servers in error or unavailable states SHALL be excluded from the effective tool surface presented to the model.

#### Scenario: Disabled server tools are hidden
- **WHEN** a configured MCP server is disabled for the current session
- **THEN** tools from that server are excluded from the effective tool surface for that session

#### Scenario: Errored server tools are hidden
- **WHEN** a configured MCP server enters an error state
- **THEN** tools from that server are excluded from the effective tool surface for every session until the server becomes usable again

#### Scenario: Reconnected server tools return for enabled sessions
- **WHEN** a configured MCP server becomes usable again after an error
- **THEN** its tools are restored to the effective tool surface for sessions where that server remains enabled

### Requirement: Client-visible MCP inspection and session control APIs
The system SHALL provide client-visible APIs to inspect runtime MCP inventory and to enable or disable configured MCP servers for a session. The system SHALL expose enough state for clients to distinguish runtime server health from session enablement intent.

#### Scenario: Client inspects runtime server state
- **WHEN** a client requests details for a configured MCP server
- **THEN** the response includes runtime health information separately from any session-specific enablement state

#### Scenario: Client toggles server for a session
- **WHEN** a client enables or disables a configured MCP server for a session
- **THEN** the session's effective MCP tool surface reflects that change on subsequent prompt construction

### Requirement: MCP state is excluded from handoff portability
Handoff export and import SHALL exclude runtime MCP inventory and session MCP enablement state. Importing a handoff bundle SHALL NOT automatically enable, disable, configure, or assume availability of MCP servers on the destination runtime.

#### Scenario: Handoff does not carry enabled MCP servers
- **WHEN** a session with one or more enabled MCP servers is exported and later imported
- **THEN** the imported session does not inherit enabled MCP server state from the source runtime

#### Scenario: Destination runtime decides its own MCP availability
- **WHEN** a handoff bundle is imported into a runtime with a different MCP configuration
- **THEN** the destination runtime determines available MCP servers using its own runtime-local inventory and policy
