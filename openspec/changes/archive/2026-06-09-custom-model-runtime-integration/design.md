## Context

Issue #68 asks for ConfigStore-backed custom model registry APIs. The CRUD storage layer already landed via issue #67: `ConfigStore` persists `CustomModelRecord` entries keyed by `(provider_slug, model_id)`, includes validation, and loads them into `RuntimeSettingsSnapshot`. However, those records are structurally disconnected from the runtime. Two separate model metadata systems exist:

1. **ConfigStore custom models** — persisted, validated, snapshot-loaded, but never consumed by runtime code.
2. **`ModelCapabilityRegistry`** — an in-memory `HashMap<(String, String), ModelCapabilityMetadata>` used by `apply_model_switch()` for capability diffs and context adaptation. Currently populated only in tests via `RuntimeState::register_model_capability()` (line 1561 of runtime.rs), never from ConfigStore or iron-providers data.

There is also no unified "effective model catalog" that merges built-in iron-providers model metadata with custom model records. The runtime resolves providers via `ProviderRegistry::default()` (runtime.rs:479, request_builder.rs:265), but this path has no awareness of custom models.

```text
CURRENT STATE                          TARGET STATE
─────────────                          ────────────

iron-providers                         iron-providers
  │                                      │
  │ ProviderRegistry::default()          │ built-in model metadata
  ▼                                      ▼
resolve_managed_provider()          ┌──────────────────────┐
  │                                 │  EffectiveModelCatalog │
  │                                 │  built-in + custom     │
  │                                 └──────┬───────────────┘
ConfigStore.custom_models                   │
  │                                         ├── RuntimeState startup
  ▼                                         ├── ModelCapabilityRegistry
RuntimeSettingsSnapshot                     ├── Default model validation
  │ (dead end)                              └── Provider construction path
  ▼
  unused
```

## Goals / Non-Goals

**Goals:**

- Define a unified `EffectiveModelCatalog` that merges built-in provider model metadata with ConfigStore custom model records.
- Bridge custom models into `ModelCapabilityRegistry` so capability diffs, context adaptation, and model switch planning use merged metadata.
- Add provider-slug validation for custom model writes, verifying the slug matches a known built-in provider or persisted provider config.
- Extend `CustomModelRecord`/`CustomModelInput` with optional fields needed to fully populate `ModelCapabilityMetadata` (streaming, reasoning effort values).
- Ensure runtime startup hydrates the capability registry from the effective catalog.
- Enable default model validation to consider both built-in and custom models.

**Non-Goals:**

- Changing how `ProviderRegistry` constructs provider instances. Provider construction continues to use `ProviderRegistry::default()`.
- Adding model metadata APIs to `iron-providers`. If iron-providers does not expose a model catalog, this change defines how iron-core handles built-in metadata internally.
- Migrating AgentIron app settings or custom model data. That is owned by AgentIron#56.
- Persisting `ModelCapabilityRegistry` separately. It remains a runtime-only structure hydrated from ConfigStore and built-in data.
- Replacing `ProviderPromptContext` or changing credential resolution paths.
- Adding a user-facing model catalog browsing API for the frontend in this change.

## Decisions

### D1: Effective model catalog is a runtime-only merge layer, not a new SQLite table

The catalog merges built-in provider knowledge with ConfigStore custom models at runtime. No new database table is needed; the catalog is built by loading built-in metadata and ConfigStore snapshot data, then merging.

**Rationale**: Built-in metadata comes from code or iron-providers, not from ConfigStore. Custom models are already persisted. A runtime merge avoids schema complexity and keeps ConfigStore as the single durable source for custom entries.

### D2: Custom entries extend the catalog; they do not override built-in entries with the same key

If a custom model record has the same `(provider_slug, model_id)` as a built-in entry, the custom record is treated as an error or warning rather than an override. Users who need different metadata for a known model should file an iron-providers issue.

**Rationale**: Allowing overrides creates silent divergence from iron-providers metadata, making debugging harder. The "extend only" model is simpler and safer. If override semantics are needed later, they can be added as an explicit opt-in.

**Alternative considered**: Custom overrides same key (replaces built-in). Rejected because it hides the source of truth and makes it unclear whether behavior comes from iron-providers or ConfigStore.

**Alternative considered**: Field-level overlay. Rejected for v1 because the added complexity (partial overrides, merge semantics for each field) is not justified by current use cases.

### D3: Extend `CustomModelInput`/`CustomModelRecord` with optional fields to map to `ModelCapabilityMetadata`

Add:
- `supports_streaming: bool` (default true for new custom models)
- `reasoning_effort_values: Option<Vec<String>>` (empty means not supported)

Do not add `unsupported_tools` to ConfigStore; that is a runtime concern best handled by the catalog consumer, not persisted config.

**Rationale**: The field mapping from `CustomModelRecord` to `ModelCapabilityMetadata` is currently lossy. Adding these fields makes the conversion complete without overcomplicating the persistence layer.

### D4: Provider-slug validation uses a known-providers set, not a SQL foreign key

When `set_custom_model` is called, validate that `provider_slug` matches either:
1. A built-in provider slug known to `ProviderRegistry::default()`, or
2. A persisted `ProviderConfigRecord` in ConfigStore.

This validation is informational: it should warn or return a validation error but should not silently reject entries for providers that might be registered dynamically in the future.

**Rationale**: A SQL foreign key to `provider_configs` would block custom models for built-in providers that don't have a persisted row. Rust-level validation is more flexible.

### D5: Built-in model metadata comes from a compiled-in catalog in iron-core

Define a `BUILTIN_MODEL_CATALOG` constant or function in iron-core that provides model metadata for known iron-providers models (context windows, capabilities, etc.). This catalog is maintained alongside iron-providers version updates.

**Rationale**: iron-providers does not currently expose a model catalog API. Rather than blocking on an upstream change, iron-core can maintain its own built-in catalog that is updated when iron-providers is upgraded. If iron-providers later adds a catalog API, the built-in catalog can be replaced.

**Alternative considered**: Querying iron-providers at runtime for model metadata. Rejected because iron-providers `ProviderRegistry` currently has no such API.

### D6: Runtime startup hydrates `ModelCapabilityRegistry` from effective catalog

`RuntimeState` initialization should call a new function that builds the effective catalog (built-in + custom) and registers all entries into `ModelCapabilityRegistry`. This replaces the current empty/test-only population.

**Rationale**: This is the minimal bridge needed to make custom models functional at runtime.

## Risks / Trade-offs

- **[Built-in catalog drift]** → The compiled-in `BUILTIN_MODEL_CATALOG` must be updated when iron-providers adds or changes models. Mitigate by documenting the coupling and adding a note to update the catalog when iron-providers is upgraded.
- **[Custom model without matching provider]** → Users could add custom models for providers without credentials or a persisted config. The extend-only model means these are valid additions. The runtime will fail at provider construction time if credentials are missing, which is the existing behavior.
- **[Schema migration for new fields]** → Adding `supports_streaming` and `reasoning_effort_values` to `CustomModelRecord` requires a migration v3 for the `custom_models` table. Mitigate by making new columns nullable with sensible defaults so existing rows migrate cleanly.
- **[iron-providers catalog API]** → If iron-providers later exposes a model catalog, the built-in catalog in iron-core becomes redundant. The effective catalog abstraction insulates consumers from this change; only the built-in data source needs updating.

## Open Questions

- Should provider-slug validation for custom models be a hard error (reject the write) or a soft warning (allow the write but log)? Proposal: hard error for unknown slugs, with a way to register custom providers via `set_provider_config` first.
- What is the exact set of built-in models for the initial `BUILTIN_MODEL_CATALOG`? This depends on which providers and models iron-providers currently supports and will be determined during implementation.
