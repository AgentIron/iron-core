## Why

ConfigStore already persists custom model records with full CRUD, validation, and snapshot loading (landed via issue #67). However, those records are a dead end: `ModelCapabilityRegistry` (used for model switch planning and capability diffs) is populated only in tests, and there is no "effective model catalog" that merges built-in iron-providers knowledge with ConfigStore custom entries. Custom model metadata never reaches the runtime paths that actually resolve providers, plan model switches, or validate default model selections.

## What Changes

- Add an effective model catalog that merges built-in iron-providers model metadata with ConfigStore custom model records into a unified queryable view.
- Bridge ConfigStore custom models into `ModelCapabilityRegistry` so custom model capabilities are available for model switch planning, capability comparison, and context adaptation decisions.
- Add provider-slug validation for custom model records: verify that `provider_slug` corresponds to a known provider (built-in iron-providers or persisted provider config).
- Extend `CustomModelRecord` fields where needed to support a complete mapping to `ModelCapabilityMetadata` (e.g., streaming support, reasoning effort values).
- Add a `ModelCatalogProvider` or equivalent trait/type that the runtime can use to enumerate available models, look up capabilities, and validate model selections against both built-in and custom sources.
- Ensure default model validation in the runtime settings snapshot considers the effective catalog (built-in plus custom) rather than only custom models.

## Capabilities

### New Capabilities
- `effective-model-catalog`: A unified model catalog that merges built-in iron-providers model metadata with ConfigStore custom model records, providing queryable model lookup, capability metadata, and provider-scoped enumeration.

### Modified Capabilities
- `core-config-store`: Provider-slug validation for custom model records will reference the effective catalog to verify that model IDs are associated with a known provider slug.
- `model-switching`: `ModelCapabilityRegistry` will be populated from the effective catalog so that model switch planning, capability diffs, and context adaptation use merged built-in and custom model metadata.

## Impact

- **Core API**: New effective model catalog types in `iron_core::config` or a new module. `ConfigStore` may gain provider-slug validation methods or accept a catalog reference.
- **Runtime**: `RuntimeState` will hydrate `ModelCapabilityRegistry` from the effective catalog during startup or on settings change, replacing the current empty/test-only population.
- **Model switching**: Model switch planning and capability comparison gain custom model awareness.
- **ConfigStore validation**: Custom model writes may require a provider-slug validation step that references known providers.
- **Schema**: Possible addition of optional fields to `CustomModelInput`/`CustomModelRecord` (e.g., `supports_streaming`, `reasoning_effort_values`) to complete the mapping to `ModelCapabilityMetadata`.
- **Dependencies**: May require `iron-providers` to expose a model metadata API or catalog if it does not already provide one.
