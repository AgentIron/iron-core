## ADDED Requirements

### Requirement: Provider slug validation SHALL consider effective provider profiles
Provider slug validation for custom models and default model settings SHALL consider provider slugs from built-in provider profiles plus persisted custom/override provider profiles loaded into the effective provider registry.

#### Scenario: Custom model references custom provider profile
- **WHEN** a caller stores a custom model for a provider slug supplied by a persisted custom provider profile
- **THEN** provider slug validation succeeds for that provider slug
- **AND** the custom model can participate in the effective model catalog

#### Scenario: Custom model references unknown provider slug
- **WHEN** a caller stores a custom model for a provider slug that is neither built-in nor present in persisted provider profiles
- **THEN** validation fails with an actionable unknown-provider error

#### Scenario: Provider profile override keeps slug valid
- **WHEN** a persisted provider profile overrides a built-in provider slug
- **THEN** that provider slug remains valid for custom model and default model validation
