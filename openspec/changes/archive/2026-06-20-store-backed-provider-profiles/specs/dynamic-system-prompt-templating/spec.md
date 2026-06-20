## ADDED Requirements

### Requirement: Provider guidance SHALL resolve from the effective provider profile
Provider-specific guidance in the canonical `Provider-Specific Guidance` section SHALL be resolved from the effective provider profile for the selected provider when profile guidance is available.

The system SHALL NOT construct a fresh built-in-only registry during request prompt composition when an effective provider registry or selected effective provider profile is available.

#### Scenario: Built-in provider guidance renders
- **WHEN** a provider request is built for a built-in provider with provider guidance
- **THEN** that guidance appears in `## 7. Provider-Specific Guidance`
- **AND** the canonical section order remains unchanged

#### Scenario: Custom provider guidance renders
- **WHEN** a provider request is built for a custom provider profile with provider guidance
- **THEN** the custom profile guidance appears in `## 7. Provider-Specific Guidance`
- **AND** no other prompt section ownership changes

#### Scenario: Override provider guidance wins
- **WHEN** a persisted provider profile override supplies guidance for a built-in provider slug
- **THEN** prompt composition uses the override guidance for that slug
- **AND** it does not use stale built-in-only guidance from a throwaway registry

#### Scenario: Missing provider guidance falls back to manual guidance
- **WHEN** no effective provider profile guidance is available for the selected provider
- **THEN** prompt composition uses manually configured provider guidance when present
- **AND** otherwise renders the provider guidance section empty according to existing prompt composition behavior
