## ADDED Requirements

### Requirement: The runtime SHALL compose the system prompt from a fixed ordered section model
`iron-core` SHALL render the system prompt from a fixed ordered set of sections rather than from arbitrary caller-defined template blocks.

#### Scenario: System prompt uses the canonical section order
- **WHEN** a provider request is built
- **THEN** the final system prompt is rendered in this order:
  1. Identity
  2. Static Context
  3. Core Guidelines
  4. Tool Philosophy
  5. Editing Guidelines
  6. Safety & Destructive Actions
  7. Provider-Specific Guidance
  8. Communication & Formatting
  9. Client Injection

### Requirement: Core-owned prompt policy sections SHALL NOT be externally overridable
Core-owned policy sections SHALL remain under `iron-core` control even when providers or clients supply prompt-related inputs.

#### Scenario: Provider cannot replace core safety guidance
- **WHEN** a provider supplies provider-specific guidance
- **THEN** that content appears only in `Provider-Specific Guidance`
- **AND** `Safety & Destructive Actions` remains unchanged

#### Scenario: Client cannot replace core guidelines
- **WHEN** a client supplies editing guidance or client injection content
- **THEN** `Core Guidelines` and `Communication & Formatting` remain unchanged

### Requirement: Static context SHALL rebuild only on explicit invalidation events
`iron-core` SHALL cache the rendered system prompt and rebuild it only when tracked inputs affecting rendered content change.

#### Scenario: Working directory change invalidates static context
- **WHEN** the runtime working directory changes for a session
- **THEN** the cached system prompt is invalidated
- **AND** the next provider request renders an updated `Static Context` section

#### Scenario: Stable runtime state reuses cached prompt
- **WHEN** no tracked prompt inputs have changed since the last render
- **THEN** the runtime reuses the cached system prompt
- **AND** it does not rebuild the prompt for that request

### Requirement: Tool philosophy SHALL be core-owned and derived from available tools
`Tool Philosophy` SHALL be rendered by `iron-core` from actual tool/runtime availability rather than supplied by external callers.

#### Scenario: Tool availability change updates tool philosophy
- **WHEN** the available tool set changes for a session or runtime
- **THEN** the next rendered system prompt reflects the updated tool guidance
- **AND** no provider-supplied fragment can replace that section

### Requirement: Editing guidance SHALL support client override with core fallback
`Editing Guidelines` SHALL support client-supplied editing guidance while preserving a core default when no client guidance is supplied.

#### Scenario: Client editing guidance overrides default content
- **WHEN** a client supplies editing guidance for the session
- **THEN** the `Editing Guidelines` section renders that client-supplied guidance

#### Scenario: Missing client editing guidance falls back to core defaults
- **WHEN** a client does not supply editing guidance
- **THEN** the `Editing Guidelines` section renders the core default guidance

### Requirement: Provider-specific guidance SHALL be isolated to its designated section
Provider/model-specific system prompting from `iron-providers` SHALL appear only in the provider-owned section.

#### Scenario: Provider fragment is inserted in provider section only
- **WHEN** `iron-providers` supplies a provider/model guidance fragment
- **THEN** that fragment appears in `Provider-Specific Guidance`
- **AND** no other section ordering or ownership changes

### Requirement: Client injection SHALL support ordered trusted markdown fragments
`iron-core` SHALL provide a client-owned injection slot for additional system prompting without redefining core-owned sections.

#### Scenario: Client injection fragments render in order
- **WHEN** a client supplies multiple injection fragments
- **THEN** the `Client Injection` section renders them in caller-specified order

#### Scenario: Empty client injection omits extra content
- **WHEN** a client supplies no injection content
- **THEN** the `Client Injection` section is empty or omitted according to the renderer policy
- **AND** the rest of the section order remains stable
