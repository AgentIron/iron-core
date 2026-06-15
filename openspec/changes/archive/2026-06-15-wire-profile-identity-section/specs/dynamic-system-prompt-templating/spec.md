## ADDED Requirements

### Requirement: Identity section SHALL render selected profile identity

`iron-core` SHALL render selected profile identity content in the canonical `Identity` section when profile identity is supplied to system prompt composition. Profile identity content SHALL NOT be rendered in `Client Injection` merely because it originated from profile-backed session execution.

#### Scenario: Profile identity renders in identity section
- **WHEN** a provider request is built with selected profile identity content
- **THEN** the final system prompt renders that content in `## 1. Identity`
- **AND** the same profile identity content is not duplicated in `## 9. Client Injection`

#### Scenario: Missing profile identity uses core fallback
- **WHEN** a provider request is built without profile identity content
- **THEN** the final system prompt renders the existing core fallback identity in `## 1. Identity`
- **AND** the canonical section order remains unchanged

#### Scenario: Session instructions remain client injection
- **WHEN** a provider request includes explicit session instructions in addition to profile identity content
- **THEN** the profile identity content renders in `## 1. Identity`
- **AND** the explicit session instructions render in `## 9. Client Injection`

#### Scenario: Profile identity invalidates prompt cache
- **WHEN** the selected profile identity content differs from the cached prompt inputs
- **THEN** the system prompt fingerprint changes
- **AND** the next rendered system prompt reflects the new identity section content
