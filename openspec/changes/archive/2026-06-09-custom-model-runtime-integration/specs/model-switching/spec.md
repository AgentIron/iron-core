## MODIFIED Requirements

### Requirement: The runtime SHALL reconcile capabilities between source and target models

The runtime SHALL compare the source and target model capabilities and report differences to the client. Tools, modalities, and features unsupported by the target model SHALL be hidden or disabled. Capability comparison SHALL use `ModelCapabilityRegistry` populated from the effective model catalog, covering both built-in and custom models.

#### Scenario: Target supports same tools
- **WHEN** switching to a model that supports the same tools as the current model
- **THEN** all currently visible tools remain available
- **AND** no capability difference report is generated

#### Scenario: Target lacks some tools
- **WHEN** switching to a model that does not support some currently visible tools
- **THEN** unsupported tools are hidden from the effective tool catalog
- **AND** the client receives a report listing the unavailable tools

#### Scenario: Target lacks image support
- **WHEN** switching to a model that does not support image input
- **THEN** the runtime flags that image content may not be processable
- **AND** the client receives a capability difference report

#### Scenario: Switching between built-in and custom models
- **WHEN** switching from a built-in model to a custom model or vice versa
- **THEN** the capability comparison uses effective catalog metadata for both models
- **AND** the capability diff reflects the merged built-in and custom model metadata

#### Scenario: Switching to an unregistered custom model
- **WHEN** switching to a model not present in the effective catalog
- **THEN** the runtime rejects the switch with an error indicating the model is unknown
