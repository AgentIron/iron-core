## MODIFIED Requirements

### Requirement: Plugin-backed tool calls complete through the runtime with normalized client-visible rich results
The runtime SHALL execute plugin-backed tool calls through the same canonical session-effective runtime path used for local and MCP-backed tools. That path SHALL perform schema validation, approval handling, durable tool-call recording, and result propagation before and after delegating execution to the Extism/WASM host. When a plugin-backed tool returns rich output, the runtime SHALL normalize that result and expose transcript and rich `view` fields distinctly through client-visible surfaces.

#### Scenario: Client observes plugin rich output through runtime surfaces
- **WHEN** a plugin-backed tool call completes with a validated rich `view` payload
- **THEN** the runtime SHALL expose transcript text through a structured transcript field
- **AND** the runtime SHALL expose the rich `view` payload through a structured `view` field
- **AND** rich-capable clients SHALL not need to scrape transcript text to render the rich payload

#### Scenario: Durable result propagation preserves normalized rich output
- **WHEN** a plugin-backed tool call completes with normalized rich output
- **THEN** the runtime SHALL preserve that normalized result shape through durable tool-result recording and client-visible propagation

#### Scenario: Text-only client ignores presentation payload
- **WHEN** a client does not implement rich `view` rendering for plugin results
- **THEN** the client SHALL still be able to rely on transcript text as the complete fallback representation of the plugin result
