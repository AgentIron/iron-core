## ADDED Requirements

### Requirement: Plugin-backed rich output SHALL normalize to a stable result envelope
When a WASM plugin tool call returns rich output, the runtime SHALL normalize that result into a structured envelope that separates transcript-safe text from optional rich view state.

The normalized envelope SHALL:
- include transcript text suitable for text-only clients
- include an optional rich `view` object
- include plugin/tool correlation metadata
- require a stable `view_id` and `view_mode` whenever a rich `view` object is present

#### Scenario: Plugin returns text-only result
- **WHEN** a plugin tool call completes with only plain text output
- **THEN** the runtime SHALL normalize that result into a text-only envelope
- **AND** the envelope SHALL include transcript text
- **AND** the envelope SHALL not require a `view` object

#### Scenario: Plugin returns rich result with transcript text
- **WHEN** a plugin tool call completes with a rich `view` payload
- **THEN** the runtime SHALL normalize that result into an envelope containing transcript text and a `view` object
- **AND** the normalized result SHALL include `view_id` and `view_mode`
- **AND** the normalized result SHALL preserve plugin/tool correlation metadata

### Requirement: Transcript text SHALL remain a standalone fallback channel
Rich plugin output SHALL not replace transcript continuity. The transcript channel SHALL remain safe for clients that do not implement rich rendering.

#### Scenario: Text-only client ignores rich payload
- **WHEN** a client does not implement rich view rendering
- **THEN** the client SHALL still be able to present the plugin result coherently using transcript text alone
- **AND** the client SHALL not be required to inspect the rich `view` payload

#### Scenario: Transcript ordering remains chronological
- **WHEN** a plugin result includes transcript text and a rich `view` payload
- **THEN** transcript text SHALL continue to participate in normal chronological transcript flow
- **AND** rich `view` lifecycle semantics SHALL not alter transcript ordering

### Requirement: Rich view payloads SHALL have session-scoped identity and lifecycle semantics
Rich plugin `view` payloads SHALL support stable identity within a session so clients can reconcile updates to the same visual surface over time.

#### Scenario: Plugin returns replaceable rich view state
- **WHEN** a plugin emits multiple rich payloads for the same logical surface during one session
- **THEN** those payloads SHALL use the same `view_id`
- **AND** clients MAY use that identifier to associate later updates with the same rendered surface

#### Scenario: View identifiers do not require transcript parsing
- **WHEN** a client receives a normalized rich result
- **THEN** the client SHALL be able to determine the target view identity directly from structured result fields
- **AND** the client SHALL not need to infer identity from transcript text

### Requirement: View mode SHALL have explicit client-visible semantics
Each rich `view` payload SHALL declare a `view_mode` that the runtime preserves through client-visible result surfaces.

#### Scenario: Replace mode supersedes prior surface state
- **WHEN** a rich payload is emitted with `view_mode` `replace`
- **THEN** clients SHOULD treat that payload as the latest canonical state for the same `view_id`

#### Scenario: Append mode preserves prior surface state
- **WHEN** a rich payload is emitted with `view_mode` `append`
- **THEN** clients SHOULD add that payload as an additional entry associated with the same `view_id`
- **AND** clients SHOULD not treat that payload as replacing prior entries for that identifier

#### Scenario: Transient mode does not imply durable replacement
- **WHEN** a rich payload is emitted with `view_mode` `transient`
- **THEN** clients MAY render it ephemerally
- **AND** clients SHOULD not treat it as the durable canonical state for that `view_id` unless client policy explicitly chooses to do so

### Requirement: Supported v1 view payloads SHALL be declarative and schema-validated
The runtime SHALL accept only supported declarative v1 `view` kinds and SHALL validate them before exposing them to clients.

The v1 `view` kinds SHALL be:
- `todo_list`
- `status`
- `progress`

#### Scenario: Plugin returns supported v1 view kind
- **WHEN** a plugin returns a supported v1 `view` kind with a valid schema
- **THEN** the runtime SHALL preserve that payload through client-visible result surfaces

#### Scenario: Unknown view kind is rejected
- **WHEN** a plugin returns a `view` kind outside the supported v1 set
- **THEN** the runtime SHALL reject that rich payload rather than exposing an undefined rich contract

#### Scenario: Executable or arbitrary frontend payload is rejected
- **WHEN** a plugin attempts to return executable frontend logic or arbitrary code-bearing `view` content
- **THEN** the runtime SHALL reject that rich payload
- **AND** the runtime SHALL not expose executable client code through the rich-output channel
