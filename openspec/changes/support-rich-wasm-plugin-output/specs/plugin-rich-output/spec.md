## ADDED Requirements

### Requirement: Plugin tool results SHALL support structured presentation payloads alongside plugin-authored transcript text
WASM plugin tool calls SHALL be able to return a structured result that includes plugin-authored transcript text and an optional rich presentation payload for clients that support richer rendering.

#### Scenario: Plugin returns text-only result
- **WHEN** a plugin tool call completes with only plain text output
- **THEN** the runtime SHALL continue to expose that result through the existing transcript-safe path
- **AND** clients that do not support rich rendering SHALL remain fully functional

#### Scenario: Plugin returns rich result with transcript text
- **WHEN** a plugin tool call completes with a rich presentation payload
- **THEN** the runtime SHALL also expose plugin-authored transcript text for that same result
- **AND** clients SHALL be able to render the rich payload without having to scrape transcript text

### Requirement: Presentation payloads SHALL support stable identity and update semantics
Rich plugin presentation payloads SHALL include enough information for clients to update a visual surface over time without relying on transcript parsing.

#### Scenario: Plugin returns replaceable presentation state
- **WHEN** a plugin returns a presentation payload with a stable `presentation_id`
- **THEN** clients that support rich rendering SHALL be able to associate later payloads with that same surface

#### Scenario: Plugin declares presentation mode
- **WHEN** a plugin returns a presentation payload
- **THEN** the payload SHALL declare a presentation mode such as `replace`, `append`, or `transient`
- **AND** the runtime SHALL preserve that mode through client-visible result surfaces

### Requirement: Rich presentation payloads SHALL be declarative
The runtime SHALL treat plugin-provided presentation payloads as declarative structured data rather than executable frontend logic.

#### Scenario: Plugin requests supported v1 presentation kind
- **WHEN** a plugin returns a supported rich presentation kind
- **THEN** the runtime SHALL validate and normalize that payload before exposing it to clients

#### Scenario: Plugin requests unsupported executable UI behavior
- **WHEN** a plugin attempts to return arbitrary executable frontend logic rather than a supported declarative payload
- **THEN** the runtime SHALL reject or sanitize that payload rather than exposing executable client code
