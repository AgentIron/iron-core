## 1. Public Config Module and API Shape

- [ ] 1.1 Add public `iron_core::config` module exports.
- [ ] 1.2 Add `ConfigStore` as the public durable configuration abstraction; do not expose `ConfigDb` as the main caller-facing type.
- [ ] 1.3 Add `ConfigStore::open()` for the platform-default store.
- [ ] 1.4 Add `ConfigStore::open_at(path)` for explicit database paths.
- [ ] 1.5 Add an in-memory/test constructor or helper that avoids platform paths and user files.
- [ ] 1.6 Add grouped async config APIs for credentials, profiles, prompts, and schedule entries.
- [ ] 1.7 Add public record/input types for IC-1 storage operations while keeping profile, prompt, and schedule payload semantics opaque/versioned.
- [ ] 1.8 Add `ConfigError` covering path, directory, open, migration, query, serialization, deserialization, not-found, conflict, key, encryption, decryption, and busy-timeout failures.

## 2. SQLite Backend and Migrations

- [ ] 2.1 Add `sqlx` SQLite dependencies and runtime feature configuration.
- [ ] 2.2 Implement SQLite-backed `ConfigStore` internals without exposing SQL handles as the primary public API.
- [ ] 2.3 Implement platform-default path resolution for Linux, macOS, and Windows.
- [ ] 2.4 Create parent directories when opening the platform-default or explicit-path store.
- [ ] 2.5 Add migrations for `profiles`, `prompts`, `credentials`, and `schedule` tables.
- [ ] 2.6 Track schema/migration version and apply pending migrations on open.
- [ ] 2.7 Ensure opening an already-current database is a no-op from a caller perspective.
- [ ] 2.8 Configure SQLite WAL mode and a finite busy timeout.
- [ ] 2.9 Ensure writes and migrations are transactional.

## 3. Opaque Domain Storage APIs

- [ ] 3.1 Implement profile storage CRUD using stable IDs, schema/version metadata, opaque serialized payloads, and timestamps.
- [ ] 3.2 Implement prompt storage CRUD using stable IDs, schema/version metadata, opaque serialized payload/body, and timestamps.
- [ ] 3.3 Implement schedule storage CRUD using stable IDs, schema/version metadata, opaque serialized payloads, and timestamps.
- [ ] 3.4 Add list/query APIs needed to retrieve records by stable ID and any IC-1 lookup keys selected during implementation.
- [ ] 3.5 Ensure profile, prompt, and schedule APIs do not require IC-2/IC-6/IC-8 domain semantics.
- [ ] 3.6 Add tests for CRUD roundtrips, not-found behavior, serialization failures, and timestamp updates.

## 4. Credential Encryption and Key Sources

- [ ] 4.1 Add credential serialization format for `StoredCredential` payloads.
- [ ] 4.2 Add credential encryption/decryption abstraction used by the config credentials API.
- [ ] 4.3 Implement OS-keyring-backed master key source for the default `ConfigStore::open()` path.
- [ ] 4.4 Implement environment-variable key source for headless Linux/cron operation.
- [ ] 4.5 Document and validate the environment variable name and required key format.
- [ ] 4.6 Add in-memory/test key source or cipher for tests.
- [ ] 4.7 Ensure credential writes fail if no valid key source is available.
- [ ] 4.8 Ensure non-secret config operations can still work when credential key material is unavailable.
- [ ] 4.9 Ensure credential payloads are never silently stored in plaintext.
- [ ] 4.10 Ensure errors, logs, and debug output do not expose encryption key material or decrypted credential secrets.

## 5. Durable Provider Credential Store

- [ ] 5.1 Implement `ProviderCredentialStore` for the `ConfigStore`-backed credential API or a lightweight handle derived from it.
- [ ] 5.2 Store at most one credential per provider slug with a database uniqueness constraint.
- [ ] 5.3 Make storing an API-key credential replace any existing OAuth credential for that provider.
- [ ] 5.4 Make storing an OAuth credential replace any existing API-key credential for that provider.
- [ ] 5.5 Implement credential lookup, set, remove, and provider-slug listing through encrypted SQLite rows.
- [ ] 5.6 Ensure list/status APIs do not decrypt or expose secret payloads unless the existing credential-store method explicitly returns a `StoredCredential`.
- [ ] 5.7 Preserve existing in-memory and null credential stores for tests/lightweight callers.
- [ ] 5.8 Add tests proving provider credential resolver/orchestration works with the durable store.

## 6. Concurrency and Process Safety

- [ ] 6.1 Add tests or integration coverage for two `ConfigStore` instances opening the same database path.
- [ ] 6.2 Verify concurrent reads can proceed while using WAL mode.
- [ ] 6.3 Verify concurrent writes are serialized by SQLite and committed transactionally.
- [ ] 6.4 Verify write contention beyond the busy timeout returns an actionable `ConfigError`.
- [ ] 6.5 Document that frontends must use `iron_core::config` APIs and must not write the SQLite database directly.

## 7. Documentation and Follow-Up

- [ ] 7.1 Document the `iron_core::config` public module and the distinction between `ConfigStore` and the SQLite backend.
- [ ] 7.2 Document platform-default paths.
- [ ] 7.3 Document credential encryption behavior, OS keyring default mode, environment-variable key mode, and key-unavailable errors.
- [ ] 7.4 Reference AgentIron/AgentIron#56 as the follow-up app config audit/migration task.
- [ ] 7.5 Update provider credential documentation to describe the core-provided durable store and one-credential-per-provider behavior.

## 8. Verification

- [ ] 8.1 Run focused config-store tests for open/migration/CRUD/encryption/concurrency behavior.
- [ ] 8.2 Run provider credential tests covering durable store integration.
- [ ] 8.3 Run `cargo test` or the narrowest relevant Rust test suite for `iron-core`.
- [ ] 8.4 Run `cargo check` for `iron-core`.
- [ ] 8.5 Run `openspec status --change add-core-config-store` when the local OpenSpec CLI is usable.
