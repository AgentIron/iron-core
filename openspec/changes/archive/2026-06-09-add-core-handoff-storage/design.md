## Context

`iron-core` already owns handoff bundle creation and hydration through `HandoffExporter`, `HandoffImporter`, and facade methods for exporting/importing active sessions. AgentIron desktop currently handles saved handoff files at the app command layer, which means persistence, JSON serialization, version validation, and list metadata semantics are not shared with CLI or headless consumers.

The core config store already provides the durable SQLite-backed API surface for shared AgentIron state. Saved handoffs fit that ownership boundary because they are core-defined artifacts that should be readable by any `iron-core` consumer using the same config database.

Saved handoff bundles are exact snapshots. Current `HandoffBundle` data may include sensitive fields such as `current_provider_api_key`, so this design treats saved handoffs as sensitive local state and does not alter bundle contents during persistence. Future security work may encrypt or protect the stored snapshot, but this change does not redefine snapshot semantics.

## Goals / Non-Goals

**Goals:**

- Add `ConfigStore` APIs for `save_handoff`, `load_handoff`, `list_handoffs`, and `delete_handoff`.
- Persist the full serialized `HandoffBundle` as the authoritative saved artifact.
- Derive list metadata from the bundle and caller-provided name/id so UI and headless consumers can list saved handoffs without loading every bundle.
- Validate identifiers, names, bundle version compatibility, metadata version compatibility, and deserialization failures at API boundaries.
- Cover save, load, list, delete, update, migration, and invalid-bundle behavior with tests.

**Non-Goals:**

- Do not introduce a separate `HandoffStore` abstraction in this change.
- Do not sanitize, redact, filter, transform, or reinterpret `HandoffBundle` fields during persistence.
- Do not design future encrypted-at-rest saved handoff storage beyond preserving exact snapshot semantics.
- Do not change session export/import behavior or handoff hydration semantics except where durable storage APIs call into existing serialization/version validation.
- Do not migrate existing app-owned handoff JSON files automatically; app migration/import can be handled by consumers using the new APIs.

## Decisions

### Use `ConfigStore` as the public storage API

Saved handoff APIs will be methods on `iron_core::config::ConfigStore` because saved handoffs are durable core-owned state that should share the existing config database, migration system, error model, and app/headless access path.

Alternative considered: add a separate `HandoffStore`. That may become useful if saved handoff storage needs an independent backend later, but it would add abstraction before there is a concrete need.

### Store exact `HandoffBundle` JSON as authoritative data

The table will store `bundle_json` containing the serialized `HandoffBundle`. The persistence layer validates compatibility but does not repair or modify the bundle. Loading returns the deserialized bundle and saved metadata derived from the stored row.

Alternative considered: normalize bundle contents across multiple tables. That would make listing and future querying easier but would make exact snapshot semantics harder to preserve and would couple storage to internal bundle shape.

### Keep metadata columns derived and secondary

The `saved_handoffs` table should include derived columns for `id`, `name`, `bundle_version`, `source_session_id`, `source_model`, `source_provider`, `size_estimate_tokens`, `created_at`, and `updated_at`. `list_handoffs` returns metadata only. `load_handoff` returns both metadata and bundle.

Alternative considered: calculate list metadata by deserializing every bundle on list. That keeps schema smaller but makes listing unnecessarily expensive and makes malformed stored rows harder to surface consistently.

### Use replace semantics for save

`save_handoff` should create or update by stable handoff ID. Existing rows keep `created_at`, replace `name`, replace `bundle_json`, refresh derived metadata, and update `updated_at`.

Alternative considered: fail on duplicate ID and require a separate update API. Existing `ConfigStore` opaque record APIs already use replace semantics, and save/load/list/delete is the requested vocabulary.

### Validate at save and load boundaries

`save_handoff` should reject empty/invalid IDs, empty names, unsupported bundle versions, metadata version mismatches, serialization failures, and numeric metadata that cannot be represented safely in storage. `load_handoff` should return `Ok(None)` for missing IDs and return a typed `ConfigError` for rows whose bundle JSON cannot deserialize or whose version is unsupported.

Alternative considered: validate only on save. Load-time validation is still necessary because databases can be shared across versions or externally corrupted.

## Risks / Trade-offs

- Saved handoffs can contain credentials or other sensitive local state -> Document and preserve exact snapshot semantics now; defer hardening such as encryption-at-rest to a dedicated security change.
- Metadata columns can drift from `bundle_json` if bugs write inconsistent rows -> Derive metadata inside `save_handoff`, validate on load, and test metadata derivation.
- Future bundle versions may need compatibility behavior -> Gate current APIs on supported version constants and surface typed errors for unsupported versions rather than silently accepting incompatible snapshots.
- App-owned JSON files may already exist -> This core change provides APIs for future app migration/import, but does not silently migrate desktop files from `iron-core`.

## Migration Plan

Add a compiled-in migration that creates `saved_handoffs` without altering existing config-store tables or data. Existing config databases should open normally and gain the new table. Rollback is source-level only; older binaries should ignore the extra table if they do not query it.

## Open Questions

- Should a future security change encrypt saved handoff snapshots with the same key-source infrastructure used for provider credentials, or should it use a distinct user-controlled export/import protection model?
- Should explicit JSON import/export helper APIs live on `ConfigStore`, the handoff module, or facade types after the core saved-state APIs exist?
