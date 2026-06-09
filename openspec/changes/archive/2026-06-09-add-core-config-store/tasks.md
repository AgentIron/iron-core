## 1. Public Config Module and API Shape

- [x] 1.1 Add public `iron_core::config` module exports.
- [x] 1.2 Add `ConfigStore` as the public durable configuration abstraction in the existing `iron_core::config` module; do not expose `ConfigDb` as the main caller-facing type and do not remove existing runtime `Config` APIs.
- [x] 1.3 Add `ConfigStore::open()` for the platform-default store.
- [x] 1.4 Add `ConfigStore::open_at(path)` for explicit database paths.
- [x] 1.5 Add an in-memory/test constructor or helper that avoids platform paths and user files.
- [x] 1.6 Add grouped async config APIs for credentials, profiles, prompts, and schedule entries.
- [x] 1.7 Add public record/input types for IC-1 storage operations using non-empty string IDs, integer schema versions, opaque JSON payload/body fields, and store-maintained created/updated timestamps while keeping profile, prompt, and schedule domain semantics opaque/versioned.
- [x] 1.8 Add `ConfigError` covering path, directory, open, migration, query, serialization, deserialization, not-found, conflict, key, encryption, decryption, and busy-timeout failures.
- [x] 1.9 Define read-by-ID methods as `Result<Option<T>, ConfigError>` unless a method explicitly treats missing records as `ConfigError::NotFound`.

## 2. SQLite Backend and Migrations

- [x] 2.1 Add `sqlx` SQLite dependencies and runtime feature configuration.
- [x] 2.2 Implement SQLite-backed `ConfigStore` internals without exposing SQL handles as the primary public API.
- [x] 2.3 Implement platform-default path resolution for Linux, macOS, and Windows, including `$XDG_CONFIG_HOME/agentiron/config.db` on Linux when set and `~/.config/agentiron/config.db` fallback otherwise.
- [x] 2.4 Create parent directories when opening the platform-default or explicit-path store.
- [x] 2.5 Add compiled-in migrations for `profiles`, `prompts`, `credentials`, and `schedule` tables; do not rely on runtime migration files.
- [x] 2.6 Track schema/migration version and apply pending migrations on open.
- [x] 2.7 Ensure opening an already-current database is a no-op from a caller perspective.
- [x] 2.8 Configure SQLite WAL mode and a finite busy timeout.
- [x] 2.9 Ensure writes and migrations are transactional.
- [x] 2.10 Ensure `ConfigStore::open()`, `ConfigStore::open_at(path)`, and in-memory/test stores all use the same compiled-in migrations.

## 3. Opaque Domain Storage APIs

- [x] 3.1 Implement profile storage CRUD using caller-provided non-empty string IDs, schema/version metadata, opaque JSON payloads, full-replace updates, and timestamps.
- [x] 3.2 Implement prompt storage CRUD using caller-provided non-empty string IDs, schema/version metadata, opaque JSON payload/body, full-replace updates, and timestamps.
- [x] 3.3 Implement schedule storage CRUD using caller-provided non-empty string IDs, schema/version metadata, opaque JSON payloads, full-replace updates, and timestamps.
- [x] 3.4 Add list/query APIs needed to retrieve records by stable ID and any IC-1 lookup keys selected during implementation.
- [x] 3.5 Ensure profile, prompt, and schedule APIs do not require IC-2/IC-6/IC-8 domain semantics.
- [x] 3.6 Add tests for CRUD roundtrips, not-found behavior, serialization failures, and timestamp updates.

## 4. Credential Encryption and Key Sources

- [x] 4.1 Add credential serialization format for `StoredCredential` payloads.
- [x] 4.2 Add credential encryption/decryption abstraction used by the config credentials API.
- [x] 4.3 Implement OS-keyring-backed master key source for the default `ConfigStore::open()` path.
- [x] 4.4 Implement environment-variable key source for headless Linux/cron operation.
- [x] 4.5 Document and validate `AGENTIRON_CONFIG_ENCRYPTION_KEY` as base64-encoded 32-byte key material.
- [x] 4.6 Add in-memory/test key source or cipher for tests.
- [x] 4.7 Ensure credential writes fail if no valid key source is available.
- [x] 4.8 Ensure non-secret config operations can still work when credential key material is unavailable.
- [x] 4.9 Ensure credential payloads are never silently stored in plaintext.
- [x] 4.10 Ensure errors, logs, and debug output do not expose encryption key material or decrypted credential secrets.
- [x] 4.11 Use authenticated encryption with random per-row nonces, encryption metadata, and associated data binding provider slug, credential mode, and credential payload schema version where practical.
- [x] 4.12 Return typed errors for invalid keys, corrupt ciphertext, authentication failure, unsupported encryption metadata, and key mismatch.

## 5. Durable Provider Credential Store

- [x] 5.1 Make `ProviderCredentialStore` fallible or add an equivalent fallible durable credential boundary, then implement it for the `ConfigStore`-backed credential API or a lightweight handle derived from it.
- [x] 5.2 Store at most one credential per provider slug with a database uniqueness constraint.
- [x] 5.3 Make storing an API-key credential replace any existing OAuth credential for that provider.
- [x] 5.4 Make storing an OAuth credential replace any existing API-key credential for that provider.
- [x] 5.5 Implement credential lookup, set, remove, and provider-slug listing through encrypted SQLite rows.
- [x] 5.6 Ensure list/status APIs do not decrypt or expose secret payloads unless the existing credential-store method explicitly returns a `StoredCredential`.
- [x] 5.7 Preserve existing in-memory and null credential stores for tests/lightweight callers.
- [x] 5.8 Add tests proving provider credential resolver/orchestration works with the durable store.
- [x] 5.9 Preserve prompt-supplied API-key precedence over stored OAuth credentials for dual-mode providers.
- [x] 5.10 Ensure OAuth disconnect removes stored OAuth credentials, leaves stored API-key credentials unchanged, and does not restore API keys previously replaced by OAuth.
- [x] 5.11 Ensure durable credential read/write failures are surfaced as actionable errors rather than missing credentials or ignored writes.

## 6. Concurrency and Process Safety

- [x] 6.1 Add tests or integration coverage for two `ConfigStore` instances opening the same database path.
- [x] 6.2 Verify concurrent reads can proceed while using WAL mode.
- [x] 6.3 Verify concurrent writes are serialized by SQLite and committed transactionally.
- [x] 6.4 Verify write contention beyond the busy timeout returns an actionable `ConfigError`.
- [x] 6.5 Document that frontends must use `iron_core::config` APIs and must not write the SQLite database directly.

## 7. Documentation and Follow-Up

- [x] 7.1 Document the `iron_core::config` public module and the distinction between `ConfigStore` and the SQLite backend.
- [x] 7.2 Document platform-default paths, including Linux `XDG_CONFIG_HOME` behavior and fallback.
- [x] 7.3 Document credential encryption behavior, OS keyring default mode, environment-variable key mode, and key-unavailable errors.
- [x] 7.4 Reference AgentIron/AgentIron#56 as the follow-up app config audit/migration task.
- [x] 7.5 Update provider credential documentation to describe the core-provided durable store and one-credential-per-provider behavior.
- [x] 7.6 Document that migrations are compiled in and no external migration files need to be packaged.

## 8. Verification

- [x] 8.1 Run focused config-store tests for open/migration/CRUD/encryption/concurrency behavior.
- [x] 8.2 Run provider credential tests covering durable store integration.
- [x] 8.3 Run `cargo test` or the narrowest relevant Rust test suite for `iron-core`.
- [x] 8.4 Run `cargo check` for `iron-core`.
- [x] 8.5 Run `openspec status --change add-core-config-store` when the local OpenSpec CLI is usable.
- [x] 8.6 Verify Linux default path resolution with and without `XDG_CONFIG_HOME`.
- [x] 8.7 Verify migrations work without runtime migration files and in-memory stores use the same compiled-in migrations.
