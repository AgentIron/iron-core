## 1. Schema And Types

- [x] 1.1 Add a compiled-in config-store migration that creates the `saved_handoffs` table with bundle JSON, derived metadata columns, and timestamps.
- [x] 1.2 Add public config-store types for saved handoff save input, saved handoff metadata, and loaded saved handoff records.
- [x] 1.3 Re-export the new saved handoff types from `iron_core::config` consistently with existing config-store records.

## 2. ConfigStore APIs

- [x] 2.1 Implement `ConfigStore::save_handoff` with ID/name validation, bundle compatibility validation, metadata derivation, transactional replace semantics, and timestamp handling.
- [x] 2.2 Implement `ConfigStore::load_handoff` returning `Ok(None)` for missing IDs and typed errors for malformed or unsupported stored bundles.
- [x] 2.3 Implement `ConfigStore::list_handoffs` returning metadata-only records without deserializing or returning full bundle contents.
- [x] 2.4 Implement `ConfigStore::delete_handoff` with missing-record behavior consistent with existing delete APIs.

## 3. Validation And Errors

- [x] 3.1 Add validation helpers for saved handoff IDs, names, bundle versions, metadata versions, serialization, and deserialization failures.
- [x] 3.2 Map invalid saved handoff input and corrupted stored bundle cases to actionable `ConfigError` variants without panics.
- [x] 3.3 Ensure persistence preserves exact `HandoffBundle` snapshots and does not redact, rewrite, or reinterpret bundle fields.

## 4. Tests

- [x] 4.1 Add in-memory config-store tests for save/load exact roundtrip behavior.
- [x] 4.2 Add tests for list metadata derivation, metadata-only listing, update replace semantics, and timestamp behavior.
- [x] 4.3 Add tests for delete behavior and missing load behavior.
- [x] 4.4 Add tests for invalid save input and malformed or unsupported stored bundle handling.
- [x] 4.5 Add migration tests proving existing config-store data remains available after the saved handoff migration.

## 5. Verification

- [x] 5.1 Run the focused config-store and handoff test suites.
- [x] 5.2 Run `cargo check --manifest-path Cargo.toml` if touched Rust APIs affect crate-wide compilation.
