## ADDED Requirements

### Requirement: Profile model availability SHALL use the effective model catalog

The system SHALL use the effective model catalog when checking whether an explicit profile provider/model reference is available on the current machine. This availability check SHALL consider built-in model metadata and ConfigStore custom model records without requiring the profile payload to embed provider or model definitions.

#### Scenario: Explicit profile model is available in built-in catalog
- **WHEN** a profile references a provider/model pair present in the built-in model catalog
- **THEN** profile session preparation treats the model reference as catalog-available

#### Scenario: Explicit profile model is available in custom catalog
- **WHEN** a profile references a provider/model pair present in ConfigStore custom model records
- **THEN** profile session preparation treats the model reference as catalog-available

#### Scenario: Explicit profile model is unknown
- **WHEN** a profile references a provider/model pair absent from both built-in and custom catalog entries
- **THEN** profile session preparation reports a structured unavailable-model diagnostic
- **AND** profile configuration or import is not rejected solely because the reference is unavailable on the current machine

#### Scenario: Profile payload remains reference-only
- **WHEN** a profile references an explicit provider/model pair
- **THEN** the profile payload stores only the provider slug and model ID reference
- **AND** effective model catalog metadata remains in built-in catalog data or ConfigStore custom model records
