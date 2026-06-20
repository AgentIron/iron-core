## Context

`iron-core` currently depends on `iron-providers` for provider definitions and constructs fresh `ProviderRegistry::default()` instances in several paths. That registry contains built-in provider profiles compiled into `iron-providers`. `iron-core` already owns durable configuration for agent profiles, provider runtime config, custom models, and provider credentials, but it has no durable representation for custom provider profiles or user overrides to built-in provider profiles.

The intended direction is that `iron-providers` remains a provider protocol/profile library and does not own persistent storage. `iron-core` owns loading from `ConfigStore`, validation, import/export, and construction of an effective registry for runtime use.

This change assumes an `iron-providers` release that exposes `ProviderProfile` serialization and profile-level `provider_guidance` suitable for storage and prompt composition.

## Goals / Non-Goals

**Goals:**

- Persist non-secret custom provider profiles and explicit built-in overrides in `iron-core`'s durable config store.
- Leave built-in provider profiles compiled into `iron-providers` and available without a config store.
- Build an effective provider registry by starting from `iron-providers` built-ins and applying persisted custom/override provider profiles.
- Use the effective provider registry for managed provider construction, provider guidance, known provider slug discovery, and credential support.
- Provide import/export APIs for provider profile JSON without storing credentials or frontend-specific preferences.
- Preserve existing behavior for callers that do not use durable provider profile storage.

**Non-Goals:**

- No persistent storage inside `iron-providers`.
- No automatic seeding of built-in provider profiles into the config database.
- No migration of existing `provider_configs`, `custom_models`, `credentials`, or `profiles` rows into provider profile rows.
- No UI for managing provider profiles.
- No credential material, OAuth token state, API keys, starred models, or app-specific display preferences inside provider profile payloads.
- No live reload of active sessions when provider profile records change.

## Decisions

### 1. Persist only custom profiles and explicit overrides

The config store will not mirror every built-in provider profile into SQLite. Built-ins stay in `iron-providers`; persisted provider profile rows represent user-owned additions or replacements.

Alternatives considered:

- **Seed all built-ins into SQLite:** makes all definitions data-backed, but creates update/delete semantics for built-ins and duplicates source of truth across crate releases and user data.
- **Store only custom slugs, disallow overrides:** simpler, but prevents fixing or adapting built-in provider metadata locally.
- **Store custom profiles and overrides:** keeps built-ins code-owned while allowing users to add or replace definitions deliberately.

### 2. `iron-core` owns the store boundary and registry loading

The provider profile storage abstraction belongs in `iron-core` because `iron-core` owns `ConfigStore` and already depends on `iron-providers`. `iron-providers` should not depend on `iron-core` to load profiles.

The effective registry construction should follow this shape:

```text
ProviderRegistry::default()
        |
        v
apply persisted provider profiles from ConfigStore
        |
        v
Effective provider registry used by runtime paths
```

Alternatives considered:

- **Define `ProfileStore` in `iron-providers`:** enables `ProviderRegistry::load_from_store`, but pushes async storage concerns into a library that otherwise has no persistence backend.
- **Define the trait in `iron-core` and call it from `iron-providers`:** creates an invalid dependency direction.
- **Create a bridge crate:** keeps dependencies clean but is unnecessary until multiple crates need the same persistence abstraction.

### 3. Use typed ConfigStore APIs and a separate table

Provider profiles will use a dedicated config-store domain rather than the existing `profiles` table. The existing `profiles` table stores `AgentProfile` records; overloading it would make export/list/delete semantics ambiguous.

The stored record should include slug, schema version or payload version, serialized provider profile JSON, source metadata, and timestamps. A `builtin` flag is not necessary for rows if built-ins are not seeded, but an `override`/`source` distinction can be represented through record metadata if callers need diagnostics.

Alternatives considered:

- **Reuse `profiles`:** rejected because it conflates agent identity/policy with provider protocol metadata.
- **Use `provider_configs`:** rejected because provider configs intentionally exclude provider protocol metadata owned by `iron-providers`.

### 4. Runtime should use a shared effective registry

Registry construction should happen at runtime setup or explicit settings reload, not repeatedly during prompt building or provider construction. This allows request building, managed provider resolution, provider auth support, and known-provider discovery to share one effective view.

Active sessions should keep using their runtime-effective registry until a future explicit reload/switch mechanism is designed. Stored profile edits should affect future registry construction or explicit reloads, not mutate an in-flight request unexpectedly.

### 5. Credential support should be profile-driven where possible

Credential support should derive from the selected/effective `ProviderProfile` credential auth metadata instead of hardcoded slug lists. Existing OAuth metadata for provider-specific device-code flows can remain provider-specific until those flows are also represented declaratively.

This allows imported/custom provider profiles to advertise API-key, OAuth bearer, or no-auth support consistently with provider construction.

### 6. Provider guidance comes from the effective provider profile

Provider-specific prompt guidance should be resolved from the effective provider profile for the configured or session-selected provider. Manual `prompt_composition.provider_guidance` remains fallback behavior when no provider profile guidance is available.

This removes the current built-in-only throwaway registry lookup during prompt composition and lets persisted overrides/custom profiles affect the provider guidance section.

## Risks / Trade-offs

- **Risk: persisted overrides can break provider construction.** -> Validate imported/stored provider profile shape before accepting it and surface actionable errors during registry loading.
- **Risk: built-in release updates are hidden by stale overrides.** -> Treat overrides as explicit user data; expose diagnostics listing overridden built-in slugs so callers can review them.
- **Risk: runtime registry reload semantics become surprising.** -> Scope this change to startup/explicit load paths and avoid implicit live reload for active sessions.
- **Risk: credential support remains partly hardcoded for OAuth flows.** -> Derive generic support from profiles now, while keeping provider-specific OAuth device metadata as a separate known limitation.
- **Risk: custom provider profiles reference models absent from the effective model catalog.** -> Provider profile validity should not require model availability; custom model/default validation remains the effective catalog's job.

## Migration Plan

1. Add a config-store migration for provider profile records without modifying existing rows.
2. Add typed ConfigStore APIs for provider profile CRUD/list/import/export support.
3. Add an `iron-core` effective provider registry loader that starts from `ProviderRegistry::default()` and applies persisted records.
4. Wire runtime-managed provider construction, provider guidance lookup, known provider slug discovery, and credential support to the effective registry.
5. Preserve fallback behavior when no durable store or no persisted provider profiles are available.

Rollback is straightforward because the new table contains only optional user additions/overrides. If the loader is not used, built-in-only provider behavior remains available through `ProviderRegistry::default()`.

## Open Questions

- Should provider profile import accept only one profile JSON document at a time, or also support bundles?
- Should overrides require an explicit `allow_override` option, or is replacing an existing slug always acceptable through the provider profile APIs?
- What exact `iron-providers` version should this change require for `provider_guidance` support?
