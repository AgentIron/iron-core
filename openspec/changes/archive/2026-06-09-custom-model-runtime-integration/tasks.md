## 1. Built-in Model Catalog

- [x] 1.1 Create `src/config/builtin_models.rs` with a `BuiltinModelEntry` struct containing provider slug, model ID, context window, tool support, streaming support, reasoning effort support and values, supported modalities, and unsupported tools.
- [x] 1.2 Add a `builtin_model_catalog()` function returning `Vec<BuiltinModelEntry>` with metadata for known iron-providers models (OpenAI, Anthropic, Google, and other supported providers).
- [x] 1.3 Add unit tests verifying the built-in catalog is non-empty, entries have non-empty provider slugs and model IDs, and context windows are positive.

## 2. Custom Model Record Extension

- [x] 2.1 Add `supports_streaming: bool` and `reasoning_effort_values: Vec<String>` fields to `CustomModelInput` in `src/config/records.rs`.
- [x] 2.2 Add corresponding fields to `CustomModelRecord` in `src/config/records.rs`.
- [x] 2.3 Add SQLite migration v3 to `src/config/migrations.rs` adding `supports_streaming INTEGER NOT NULL DEFAULT 1` and `reasoning_effort_values_json TEXT NULL` columns to `custom_models`.
- [x] 2.4 Update `set_custom_model` in `src/config/store.rs` to serialize/deserialize the new fields.
- [x] 2.5 Update `get_custom_model`, `list_custom_models`, and `load_runtime_settings` to read the new columns with backward-compatible defaults for existing rows.

## 3. Provider-Slug Validation

- [x] 3.1 Add a `known_provider_slugs()` function or method that returns the union of built-in provider slugs from `ProviderRegistry::default()` and persisted `ProviderConfigRecord` slugs from ConfigStore.
- [x] 3.2 Add provider-slug validation to `set_custom_model` that checks the provider slug against `known_provider_slugs()` and returns `ConfigError` for unrecognized slugs.
- [x] 3.3 Add tests for provider-slug validation: valid built-in slug, valid persisted slug, unknown slug rejection.

## 4. Effective Model Catalog

- [x] 4.1 Create `src/config/effective_catalog.rs` with an `EffectiveModelCatalog` struct containing a merged collection of model entries keyed by `(provider_slug, model_id)`.
- [x] 4.2 Add an `EffectiveModelEntry` type that represents a unified model record with provider slug, model ID, display name, context window, output limit, capabilities (tools, streaming, reasoning effort, vision/modalities), and cost metadata.
- [x] 4.3 Add a `build_effective_catalog(builtin: &[BuiltinModelEntry], custom: &[CustomModelRecord]) -> Result<EffectiveModelCatalog, CatalogError>` function that merges built-in and custom entries.
- [x] 4.4 Implement extend-only semantics: if a custom entry duplicates a built-in `(provider_slug, model_id)`, return an error or warning rather than overriding.
- [x] 4.5 Add `get(provider: &str, model: &str) -> Option<&EffectiveModelEntry>` and `list_for_provider(provider: &str) -> Vec<&EffectiveModelEntry>` query methods.
- [x] 4.6 Add a conversion from `EffectiveModelEntry` to `ModelCapabilityMetadata` for bridging into the capability registry.
- [x] 4.7 Add tests for catalog construction, built-in/custom merge, extend-only duplicate detection, lookup, and provider-scoped listing.

## 5. Runtime Integration

- [x] 5.1 Add a method or function on `RuntimeState` that builds the effective catalog from built-in metadata and `RuntimeSettingsSnapshot.custom_models`, then registers all entries into `ModelCapabilityRegistry`.
- [x] 5.2 Call the catalog hydration method during runtime initialization so the capability registry is populated before any model switch or capability comparison can occur.
- [x] 5.3 Update default model validation in the snapshot loader to check the effective catalog (built-in plus custom) rather than only the custom models table.
- [x] 5.4 Add integration tests verifying that custom models registered via ConfigStore appear in `ModelCapabilityRegistry` lookups.
- [x] 5.5 Add integration tests verifying model switch capability diffs include custom model metadata.

## 6. Schema Migration Tests

- [x] 6.1 Add migration tests for v2-to-v3 upgrade on the `custom_models` table, verifying new columns have correct defaults.
- [x] 6.2 Add migration tests for opening a v3 database without re-running migration.
- [x] 6.3 Add tests for custom model round-trips with `supports_streaming` and `reasoning_effort_values` fields.
- [x] 6.4 Add tests for custom model round-trips where new fields use defaults (existing-row backward compatibility).

## 7. Documentation and Verification

- [x] 7.1 Document the effective model catalog API and its extend-only merge semantics.
- [x] 7.2 Document the custom model record field extensions and migration v3.
- [x] 7.3 Document provider-slug validation behavior for custom model writes.
- [x] 7.4 Run focused config store and catalog tests.
- [x] 7.5 Run `cargo check` on the workspace root (this is the iron-core crate; no `src-tauri/Cargo.toml` exists).
- [x] 7.6 Run `openspec status --change custom-model-runtime-integration` or the repo-supported OpenSpec validation command.

## Verification Summary

- `cargo check`: passed
- `cargo clippy --all-targets`: passed (no warnings)
- `cargo fmt -- --check`: passed
- `cargo test`: passed — 391 unit tests, 88 ACP runtime tests, 49 builtin tool tests, 42 config store tests, 9 config tests, 81 context management tests, 8 interop tests, 6 MCP e2e tests, 22 MCP integration tests, 15 MCP outstanding tests, 6 MCP tests, 3 MCP visibility tests, 33 prompt composition tests, 10 tool tests, 3 transport bench tests
