## Purpose

Define the runtime model catalog that merges built-in provider model metadata with ConfigStore custom model records for capability lookup, default model validation, and model-switch planning.
## Requirements
### Requirement: The runtime SHALL provide an effective model catalog merging built-in and custom model metadata

The system SHALL provide an `EffectiveModelCatalog` that merges built-in provider model metadata with ConfigStore custom model records into a unified queryable view. The catalog SHALL be a runtime-only structure built by loading built-in metadata and ConfigStore snapshot data.

#### Scenario: Catalog includes built-in models
- **WHEN** the effective model catalog is built
- **THEN** it includes model metadata from the compiled-in built-in model catalog
- **AND** each built-in entry includes provider slug, model ID, context window, capabilities, and modality information

#### Scenario: Catalog includes custom models
- **WHEN** the effective model catalog is built and ConfigStore contains custom model records
- **THEN** it includes all custom model entries alongside built-in entries
- **AND** each custom entry is queryable by provider slug and model ID

#### Scenario: Custom entries extend, not override, built-in entries
- **WHEN** a custom model record has the same `(provider_slug, model_id)` as a built-in entry
- **THEN** the catalog construction SHALL reject or flag the duplicate rather than silently overriding built-in metadata

#### Scenario: Catalog lookup by provider and model
- **WHEN** a caller queries the effective model catalog for a specific provider slug and model ID
- **THEN** the catalog returns matching model metadata whether it is built-in or custom
- **AND** returns `None` if no matching entry exists

#### Scenario: Catalog listing by provider
- **WHEN** a caller lists catalog entries for a specific provider slug
- **THEN** the catalog returns all matching entries including both built-in and custom models for that provider

### Requirement: The system SHALL maintain a compiled-in built-in model catalog

The system SHALL include a compiled-in catalog of model metadata for known iron-providers models. This catalog SHALL be maintained alongside iron-providers version updates and SHALL include context windows, capabilities, modalities, and cost metadata for each known model.

#### Scenario: Built-in catalog covers known providers
- **WHEN** the built-in model catalog is loaded
- **THEN** it includes model metadata for each provider and model known to the compiled iron-providers version
- **AND** metadata includes context window, tool support, streaming support, reasoning effort support, and supported modalities

#### Scenario: Built-in catalog is updated with iron-providers upgrades
- **WHEN** iron-providers is upgraded to a new version
- **THEN** the built-in model catalog SHOULD be reviewed and updated to reflect new or changed models

### Requirement: The runtime SHALL hydrate ModelCapabilityRegistry from the effective catalog

The runtime SHALL populate `ModelCapabilityRegistry` from the effective model catalog during startup or runtime settings initialization. All built-in and custom model entries SHALL be registered into the capability registry so that model switch planning, capability comparison, and context adaptation use merged metadata.

#### Scenario: Runtime startup registers all catalog entries
- **WHEN** the runtime initializes and the effective model catalog is available
- **THEN** all catalog entries are registered into `ModelCapabilityRegistry`
- **AND** `apply_model_switch()` can query capabilities for both built-in and custom models

#### Scenario: Runtime startup without custom models
- **WHEN** the runtime initializes and no custom models exist in ConfigStore
- **THEN** only built-in model entries are registered into `ModelCapabilityRegistry`

#### Scenario: Catalog entries map to capability metadata
- **WHEN** a custom model entry is registered into `ModelCapabilityRegistry`
- **THEN** the custom model's capabilities, context window, modalities, and streaming support are mapped to `ModelCapabilityMetadata` fields

### Requirement: Custom model records SHALL support streaming and reasoning effort metadata

`CustomModelInput` and `CustomModelRecord` SHALL include optional fields for streaming support and reasoning effort values to enable a complete mapping to `ModelCapabilityMetadata`.

#### Scenario: Custom model with streaming metadata
- **WHEN** a caller stores a custom model with `supports_streaming` set to false
- **THEN** the runtime capability registry reflects that the model does not support streaming

#### Scenario: Custom model with reasoning effort values
- **WHEN** a caller stores a custom model with `reasoning_effort_values` set to a non-empty list
- **THEN** the runtime capability registry reflects those reasoning effort values for capability comparison during model switches

#### Scenario: Custom model with default streaming metadata
- **WHEN** a caller stores a custom model without specifying streaming or reasoning effort
- **THEN** streaming defaults to true and reasoning effort values default to empty

### Requirement: Default model validation SHALL consider the effective catalog

When validating default model selection in the runtime settings snapshot, the system SHALL check that the selected `(provider_slug, model_id)` exists in the effective model catalog (built-in plus custom), not only in the custom models table.

#### Scenario: Default model is a built-in model
- **WHEN** the default model is set to a built-in provider/model combination
- **THEN** validation passes because the entry exists in the effective catalog

#### Scenario: Default model is a custom model
- **WHEN** the default model is set to a custom model entry
- **THEN** validation passes because the entry exists in the effective catalog

#### Scenario: Default model is unknown
- **WHEN** the default model is set to a provider/model combination that does not exist in either built-in or custom entries
- **THEN** validation returns an actionable error

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

