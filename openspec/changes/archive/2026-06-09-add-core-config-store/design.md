## Context

IC-1 is the foundation for later profile, stored-prompt, headless CLI, and scheduler work. The design goal is not to expose a SQLite database to clients; it is to move durable AgentIron configuration behind core-owned APIs so apps and frontends do not need to implement their own persistence.

The current provider credential design intentionally allowed app-owned storage. This change revises that direction: `iron-core` now owns the default durable credential implementation and may depend on SQLite and platform key storage. The existing `ProviderCredentialStore` trait remains useful as an internal boundary, for tests, and for future alternate backends.

## Goals

- Provide a public `iron_core::config` module for durable AgentIron configuration.
- Add durable configuration APIs to the existing `iron_core::config` module without replacing existing runtime `Config`/`ConfigSource` APIs.
- Make `ConfigStore` the public abstraction instead of exposing `ConfigDb` or SQL concepts.
- Use SQLite via `sqlx` as the initial default backend and hard dependency.
- Keep SQLite schema details private to `iron-core` except where documented for migration/debugging.
- Compile migrations into the binary/crate code so installed applications do not need packaged migration files.
- Store durable records needed by IC-1: profiles, prompts, provider credentials, and schedule entries.
- Keep profile, prompt, and schedule payload semantics minimal/opaque until their dedicated issues define domain types.
- Persist provider credentials through the existing provider credential boundary.
- Encrypt credential payloads at rest.
- Support both OS-keyring-backed encryption keys and environment-variable-provided encryption keys.
- Support multiple `iron-core` processes on one system with safe SQLite contention behavior.

## Non-Goals

- Defining final `AgentProfile` semantics; IC-2 owns that.
- Defining stored prompt template/run semantics; IC-6 owns that.
- Defining scheduler execution semantics; IC-8 owns that.
- Auditing/migrating existing AgentIron app-owned config; AgentIron/AgentIron#56 tracks that follow-up.
- Letting frontends write SQLite directly.
- Adding remote/server-backed config, config-file backends, or sync.
- Encrypting the entire SQLite database with SQLCipher.
- Providing an insecure plaintext credential fallback.

## Public API Boundary

`iron_core::config` should expose a domain-oriented API. The primary public type should be `ConfigStore`, with constructors such as:

- `ConfigStore::open()` for the platform default store.
- `ConfigStore::open_at(path)` for an explicit SQLite database path.
- An in-memory/test constructor or helper that avoids platform paths and user files.

The API should group operations by config domain, for example credentials, profiles, prompts, and schedule entries. Exact method names can be refined during implementation, but callers should not need to know SQL table names, SQLite connection strings, migration files, pragmas, or encryption row layout.

All async operations should return `Result<T, ConfigError>`. Expected failures such as path resolution, directory creation, database open, migration, serialization, deserialization, key lookup, encryption, decryption, query, uniqueness conflict, not found, and busy timeout should be represented as errors rather than panics.

The existing `iron_core::config` module already contains runtime configuration types such as `Config` and `ConfigSource`. IC-1 should add `ConfigStore` and related durable-store types to that module without renaming or removing the runtime configuration surface. Private implementation code may live in nested modules, but the caller-facing durable entry point should be `iron_core::config::ConfigStore`.

## SQLite Backend

SQLite is the initial implementation and a hard dependency for IC-1. Use `sqlx` with SQLite support. `ConfigStore::open()` resolves the default path, creates parent directories, opens the database, applies migrations, and configures the connection/pool.

Default paths:

- Linux: `$XDG_CONFIG_HOME/agentiron/config.db` when `XDG_CONFIG_HOME` is set and non-empty; otherwise `~/.config/agentiron/config.db`
- macOS: `~/Library/Application Support/com.agentiron/iron-core/config.db`
- Windows: `%APPDATA%/AgentIron/config.db`

The implementation should enable WAL mode and configure a finite busy timeout. Writes and migrations should be transactional. Multiple readers are allowed. Concurrent writers are serialized by SQLite; if another writer holds the database longer than the configured timeout, the API returns an actionable busy/timeout error.

Only `iron-core` should write the database. Frontends that need durable state should call `iron_core::config` APIs. This rule lets schema and migration behavior evolve without frontend coordination.

## Initial Schema Strategy

IC-1 creates tables for `profiles`, `prompts`, `credentials`, and `schedule`, but only credentials receive full domain behavior now.

Profiles, prompts, and schedule entries should use stable IDs, versioned/opaque payloads, metadata where useful, and created/updated timestamps. The purpose is to establish durable storage and CRUD semantics without freezing domain-specific schemas too early.

Recommended minimum shape:

- `profiles`: `id`, optional `name`/lookup key if needed, `schema_version`, opaque serialized payload, timestamps.
- `prompts`: `id`, optional `name`/lookup key if needed, `schema_version`, opaque serialized payload or body, timestamps.
- `schedule`: `id`, optional profile/prompt references if useful, `schema_version`, opaque serialized payload, enabled/status metadata if needed, timestamps.
- `credentials`: provider slug as the unique key, credential mode, encrypted payload, encryption metadata, timestamps.

Migrations should be idempotent from a caller perspective: opening an existing older database applies pending migrations, and opening a current database is a no-op.

Migration packaging decision: migrations must be embedded in the compiled `iron-core` binary or represented directly in crate code. `ConfigStore::open()`, `ConfigStore::open_at(path)`, and the in-memory/test constructor should all apply the same embedded migrations. The implementation must not depend on finding migration files at runtime because `iron-core` does not have a reliable packaging story for extra migration assets.

Opaque profile, prompt, and schedule records should use caller-provided non-empty string IDs, an integer schema version, an opaque JSON payload/body, and store-maintained created/updated timestamps. IC-1 should use simple full-replace update semantics rather than patch semantics. Read-by-ID APIs should return `Result<Option<Record>, ConfigError>` unless a method explicitly documents that missing records are exceptional.

## Credential Storage Model

Store at most one credential per provider. The provider slug should be unique in the credentials table. Setting an API key replaces an existing OAuth credential for that provider. Setting OAuth replaces an existing API key for that provider.

This aligns the durable implementation with `ProviderCredentialStore::set(slug, credential)`. Provider orchestration may still prefer a prompt-supplied or runtime-supplied API key over stored credentials for a prompt, but the core durable store itself does not retain both API-key and OAuth credential modes simultaneously for one provider. OAuth disconnect removes a stored OAuth credential and leaves a stored API-key credential unchanged, but it does not restore an API key that was previously replaced by OAuth in the durable store.

The existing `ProviderCredentialStore` boundary is currently infallible. IC-1 should make the boundary fallible or introduce an equivalent fallible durable credential boundary so SQLite, key-source, encryption, decryption, serialization, and busy-timeout failures are not silently treated as missing credentials or ignored writes. In-memory and null stores can return successful simple results, but durable persistence must surface typed errors through provider credential orchestration.

Credential rows should not expose secret material through list/status APIs. APIs that list configured providers may return provider slug, credential mode, status metadata, and timestamps, but not decrypted secrets.

## Encryption and Key Management

Use envelope-style local encryption for provider credentials:

- SQLite stores encrypted credential payloads plus non-secret metadata.
- A local credential cipher encrypts/decrypts serialized `StoredCredential` values.
- Key material comes from a configured key source.

Supported key sources for IC-1:

1. **OS keyring default**: `ConfigStore::open()` should use a platform keyring-backed key provider by default. The keyring stores or retrieves a generated master key for AgentIron, while SQLite stores encrypted credential blobs.
2. **Environment variable mode**: headless Linux/cron operation must be supported by reading an explicit environment variable containing the encryption key material. This mode must be documented and must work without a desktop keyring.
3. **Test/in-memory mode**: tests may use an in-memory or deterministic test cipher/key provider without touching OS keyrings or user configuration.

If no valid key source is available, credential read/write operations should fail with a credential-key error. Non-secret config operations may continue to work. The implementation must not silently store credential payloads in plaintext.

Environment variable mode details:

- The core-owned variable name is `AGENTIRON_CONFIG_ENCRYPTION_KEY`.
- The value must be base64-encoded 32-byte key material.
- Invalid base64, wrong key length, missing values in environment-key mode, or otherwise unusable key material return an actionable `ConfigError`/credential key error.
- Debug/log/error output must not include the key value.

Credential encryption details:

- Use an authenticated encryption scheme with 32-byte keys and random per-row nonces, such as XChaCha20-Poly1305.
- Store nonce and algorithm/key metadata needed for future migration alongside the encrypted payload.
- Include non-secret associated data such as provider slug, credential mode, and credential payload schema version where practical so encrypted rows are bound to their metadata.
- Corrupt ciphertext, authentication failure, unknown algorithm, unsupported encryption metadata, or key mismatch should return typed decryption/key errors rather than panicking or returning plaintext.
- Key rotation is out of scope for IC-1, but row metadata should not prevent a future rotation/migration path.

## Relationship to Provider Credential Orchestration

This change modifies the earlier provider credential storage direction. `iron-core` now provides the default durable credential store implementation. The existing trait remains available so runtime tests, in-memory stores, and future alternate config backends can satisfy provider credential orchestration without requiring SQLite in every code path.

Provider credential resolution, refresh, status, and disconnect behavior should continue to go through the existing provider credential APIs. The config store should be an implementation detail of credential persistence, not a parallel credential orchestration system.

## Testing Strategy

Tests should cover:

- Platform-default path resolution through deterministic helpers where practical.
- Explicit path open and parent directory creation.
- Migration on empty and already-current databases.
- In-memory database creation without user files.
- CRUD roundtrips for profiles, prompts, credentials, and schedule entries.
- Opaque payload versioning and serialization errors.
- `ProviderCredentialStore` compatibility backed by `ConfigStore`.
- One-credential-per-provider replacement behavior.
- Credential encryption: raw SQLite payload is not plaintext and decrypted API results roundtrip.
- OS keyring unavailable behavior, using a mocked/fake key provider if needed.
- Environment-variable key mode success and invalid-key failures.
- Concurrent writer contention with busy timeout behavior.
- Non-secret config operations continuing when credential key material is unavailable.

## Follow-Up Work

AgentIron/AgentIron#56 tracks auditing existing AgentIron app-owned persistent config and migrating it to the `iron_core::config` APIs added here. If that audit discovers config domains missing from `iron-core`, follow-up issues should extend the core config API rather than allowing direct app-owned database writes.
