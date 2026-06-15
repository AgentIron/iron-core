## MODIFIED Requirements

### Requirement: Core SHALL use identity prompts as profile identity layers

The system SHALL treat `identity_prompt` as the selected profile's model-facing identity instruction layer. A custom profile's non-blank `identity_prompt` SHALL replace the default profile's identity prompt for that selected profile rather than appending to it, and the selected profile identity SHALL be supplied to system prompt composition as identity content rather than as client/session injection content.

#### Scenario: Custom identity prompt is used
- **WHEN** a selected non-default profile has a non-blank `identity_prompt`
- **THEN** that prompt is the profile identity prompt used for execution preparation
- **AND** the default profile's identity prompt is not implicitly appended

#### Scenario: Blank identity prompt falls back to default
- **WHEN** a selected non-default profile has no `identity_prompt` or has an identity prompt that is blank after trimming
- **THEN** execution preparation uses the built-in default profile's identity prompt as the fallback identity prompt

#### Scenario: Default identity prompt is generic
- **WHEN** the built-in default profile is created
- **THEN** it has a short generic identity prompt suitable for general software engineering assistance

#### Scenario: Selected profile identity is distinct from session instructions
- **WHEN** execution preparation supplies the selected profile identity to provider request construction
- **THEN** the profile identity is available to the system prompt renderer as profile identity content
- **AND** it is not treated as client-owned session instruction content solely because it came from the selected profile
