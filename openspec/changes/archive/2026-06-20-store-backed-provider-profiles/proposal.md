## Why

Provider profile definitions are currently sourced only from `iron-providers` built-ins, while user/provider customization in `iron-core` is split across runtime provider configs, custom models, and credentials. This prevents `iron-core` from loading user-defined provider endpoints or user overrides from the durable config store, and causes prompt/provider resolution paths to repeatedly construct throwaway built-in registries.

## What Changes

- Add core-owned durable storage APIs for provider profile overrides and custom provider profiles.
- Keep built-in provider profiles compiled into `iron-providers`; the config store persists only custom profiles and explicit user overrides.
- Add an `iron-core` provider profile store/loading boundary that can merge persisted profiles into an effective `ProviderRegistry` without requiring `iron-providers` to own persistence.
- Use the effective provider registry for provider construction, provider slug discovery, provider credential support, and provider-specific prompt guidance.
- Support import/export of non-secret provider profile JSON through core-owned validation and storage APIs.
- Preserve existing built-in provider behavior when no provider profile store is configured or no overrides/custom profiles exist.

## Capabilities

### New Capabilities
- `provider-profiles`: Defines durable custom/override provider profile behavior, registry merge semantics, import/export rules, and effective provider profile lookup.

### Modified Capabilities
- `core-config-store`: Add typed non-secret provider profile storage APIs and schema migration support.
- `provider-credential-orchestration`: Derive provider credential support from the effective provider profile registry instead of hardcoded provider slugs where possible.
- `effective-model-catalog`: Validate provider slugs against built-ins plus persisted custom/override provider profiles when storing custom models or defaults.
- `dynamic-system-prompt-templating`: Resolve provider-specific guidance from the effective provider profile selected for the session/request instead of a fresh built-in-only registry.

## Impact

- Affected modules likely include `src/config/`, `src/provider_credential/`, `src/runtime.rs`, `src/request_builder.rs`, prompt composition tests, config store tests, and runtime/provider credential tests.
- Requires an `iron-providers` version that exposes `ProviderProfile` serialization and `provider_guidance` data suitable for core-owned persistence/loading.
- Adds public `iron-core` APIs for provider profile storage/loading/import/export while preserving the existing injected-provider and built-in registry paths.
- Does not move SQLite or other persistent storage into `iron-providers`.
