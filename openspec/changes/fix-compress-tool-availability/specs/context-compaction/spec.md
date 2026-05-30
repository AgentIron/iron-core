## MODIFIED Requirements

### Requirement: The `compress` tool SHALL be unconditionally available in the session tool catalog
The `compress` tool SHALL be registered in the runtime `ToolRegistry` at construction time and SHALL appear in every session's `SessionToolCatalog` (both `definitions` and `tool_map`), regardless of session state. The tool availability SHALL NOT depend on `uncompacted_tokens` or `compressed_blocks`.

#### Scenario: Fresh session exposes compress tool
- **WHEN** a new session is created with no messages, no compressed blocks, and `uncompacted_tokens = 0`
- **THEN** `SessionToolCatalog::contains("compress")` returns `true`
- **AND** `SessionToolCatalog::get_definition("compress")` returns `Some`

#### Scenario: Compress tool appears in definitions and tool map consistently
- **WHEN** a `SessionToolCatalog` is constructed for any session
- **THEN** the compress tool entry is present in both `definitions` and `tool_map`
- **AND** `catalog.contains("compress") == catalog.get_definition("compress").is_some()`

#### Scenario: Model call to compress on empty session returns clean rejection
- **WHEN** the model calls `compress` on a session with no timeline entries
- **THEN** the runtime returns a rejection with an error message indicating no compressible ranges exist

### Requirement: Tool Philosophy prompt SHALL always advertise the compress tool
The Tool Philosophy system prompt section SHALL include the compress tool guidance paragraph unconditionally, without gating on a `compression_available` flag.

#### Scenario: Fresh session prompt includes compress guidance
- **WHEN** a prompt is prepared for a session with no context history
- **THEN** the Tool Philosophy section includes the compress tool paragraph
- **AND** no `compression_available` field is consulted to make this determination

#### Scenario: Debug output reflects compress availability
- **WHEN** a prompt is prepared with the debug sink enabled
- **THEN** the `CompactionAvailability` influence event shows `effect=Added` (never `Suppressed`)
