## 1. Dependency and Schema Foundation

- [x] 1.1 Upgrade `iron-providers` to the release that exposes serializable `ProviderProfile` data with `provider_guidance` support.
- [x] 1.2 Add a config-store migration for a provider profile records table without seeding built-in provider rows.
- [x] 1.3 Add provider profile record/input types with slug, payload, source metadata, and timestamps.
- [x] 1.4 Add typed `ConfigStore` CRUD/list APIs for provider profile records.
- [x] 1.5 Add config-store tests for migration, roundtrip, deterministic listing, deletion, and preservation of existing config rows.

## 2. Provider Profile Store and Import/Export

- [x] 2.1 Add an `iron-core` provider profile store/loading boundary backed by `ConfigStore`.
- [x] 2.2 Validate provider profile payloads before persistence, including rejecting credential secret material.
- [x] 2.3 Implement import from provider profile JSON into the provider profile store.
- [x] 2.4 Implement export of persisted provider profile JSON by slug.
- [x] 2.5 Add tests for valid import, invalid import, override import, custom profile import, and credential-material rejection.

## 3. Effective Provider Registry

- [x] 3.1 Add effective provider registry construction that starts from `ProviderRegistry::default()` and applies persisted provider profiles.
- [x] 3.2 Ensure custom persisted slugs are added alongside built-ins.
- [x] 3.3 Ensure persisted profiles with built-in slugs override the built-in profile for that slug only.
- [x] 3.4 Add runtime/config APIs or builders needed to load and share the effective provider registry.
- [x] 3.5 Add tests proving built-in fallback, custom profile resolution, and override precedence.

## 4. Runtime and Prompt Integration

- [x] 4.1 Use the effective provider registry for managed provider construction instead of constructing fresh built-in-only registries.
- [x] 4.2 Use the effective provider profile for provider-specific prompt guidance resolution.
- [x] 4.3 Preserve manual provider guidance fallback when no effective provider profile guidance is available.
- [x] 4.4 Update provider slug discovery to include built-ins plus persisted custom/override provider profiles.
- [x] 4.5 Add tests for built-in guidance, custom profile guidance, override guidance, and manual guidance fallback.

## 5. Credential and Model-Catalog Integration

- [x] 5.1 Derive generic provider credential support from effective provider profile credential auth metadata.
- [x] 5.2 Preserve provider-specific OAuth device-code metadata for existing V1 OAuth flows.
- [x] 5.3 Validate custom model/default provider slugs against built-ins plus persisted provider profiles.
- [x] 5.4 Add tests for API-key, OAuth bearer, no-auth, and unsupported credential support derived from profile metadata.
- [x] 5.5 Add tests for custom model validation with custom provider profiles and unknown provider slugs.

## 6. Verification

- [x] 6.1 Run `cargo fmt --check`.
- [x] 6.2 Run `cargo clippy --locked --manifest-path Cargo.toml --all-targets --all-features -- -D warnings`.
- [x] 6.3 Run `cargo test`.
- [x] 6.4 Run `openspec validate store-backed-provider-profiles --strict`.
