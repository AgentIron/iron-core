## MODIFIED Requirements

### Requirement: Plugin-backed tool calls complete through the runtime with structured client-visible presentation results
The runtime SHALL execute plugin-backed tool calls through the same canonical session-effective runtime path used for local and MCP-backed tools. That path SHALL perform schema validation, approval handling, durable tool-call recording, and result propagation before and after delegating execution to the Extism/WASM host. Plugin-backed results SHALL additionally support additive structured presentation payloads for clients.

#### Scenario: Client observes plugin rich output through runtime surfaces
- **WHEN** a plugin-backed tool call completes with a validated presentation payload
- **THEN** the runtime SHALL expose that payload through client-visible runtime/facade result surfaces
- **AND** the runtime SHALL preserve plugin-authored transcript text for the same tool result

#### Scenario: Text-only client ignores presentation payload
- **WHEN** a client does not implement rich presentation rendering for plugin results
- **THEN** the client SHALL still be able to rely on the transcript-safe text result without losing core usability
