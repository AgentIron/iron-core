## Why

Issue #58 / IC-1 needs a durable configuration foundation shared by the future `agent-iron` headless CLI and the AgentIron app. Today `iron-core` has runtime configuration types and in-memory provider credential storage, but it does not own a durable config API. That leaves frontends to invent their own persistence and blocks follow-up work for agent profiles, stored prompts, headless runs, and scheduling.

This change makes `iron-core` the owner of AgentIron durable configuration. The public surface is a domain-oriented `iron_core::config` API, not a SQLite API. SQLite is the initial hard dependency and default backend, but callers should not write SQL or depend on the database layout directly.

## What Changes

- Extend the public `iron_core::config` module with `ConfigStore::open()`, `ConfigStore::open_at(path)`, and an in-memory/test constructor.
- Integrate `ConfigStore` into the existing `iron_core::config` module without removing or renaming existing runtime `Config` APIs.
- Provide a SQLite-backed default durable store behind the `ConfigStore` API using `sqlx`.
- Create and migrate the initial schema for `profiles`, `prompts`, `credentials`, and `schedule`.
- Compile migrations into the binary/crate code rather than relying on packaged external migration files; in-memory stores use the same migrations.
- Keep profile, prompt, and schedule records intentionally minimal/opaque in IC-1; IC-2, IC-6, and IC-8 define their domain semantics later.
- Provide async config-domain CRUD APIs that return typed `Result` errors rather than panicking.
- Make the provider credential store boundary fallible and implement it using `ConfigStore` credentials so provider credentials can persist without frontend-owned storage while surfacing SQLite/key/encryption failures.
- Store at most one credential per provider; setting a new credential for a provider replaces any prior credential for that provider, regardless of mode.
- Encrypt credential payloads at rest before writing SQLite rows.
- Support credential encryption keys from OS keyring by default and from an explicit environment variable mode for headless Linux/cron operation.
- Resolve platform-default config paths for Linux, macOS, and Windows, honor `XDG_CONFIG_HOME` on Linux, and create parent directories as needed.
- Configure SQLite for multiple `iron-core` processes with WAL, transactional writes, and a finite busy timeout.

## Capabilities

### New Capabilities

- `core-config-store`: Defines the public core-owned config API, SQLite-backed default store, schema ownership, platform paths, migrations, concurrency behavior, opaque domain record storage, error behavior, and test support.

### Modified Capabilities

- `provider-credential-orchestration`: Changes provider credential persistence from app-owned storage to a core-provided durable store implementation, while preserving the existing trait boundary for tests and alternate embedders.

## Impact

- **Core API**: existing public `iron_core::config` module gains `ConfigStore`, domain record/input types, grouped config APIs, and `ConfigError`.
- **Storage**: new hard SQLite dependency through `sqlx`; schema and migrations are owned by `iron-core`.
- **Credentials**: `iron-core` owns the default durable credential store and encryption-at-rest policy; existing in-memory/client-supplied stores remain supported.
- **Platform behavior**: `ConfigStore::open()` uses OS-specific application config locations; explicit paths and in-memory stores support tests and embedders.
- **Concurrency**: multiple `iron-core` instances may read concurrently; writes are serialized by SQLite locking and fail with an actionable busy timeout if contention persists.
- **Compatibility**: existing provider credential APIs and injected-provider paths remain valid; frontends should migrate durable config access to `iron_core::config` APIs rather than writing storage directly.
- **Out of scope**: full `AgentProfile` semantics, stored prompt template semantics, scheduler execution, `run_task`, AgentIron app config audit/migration, SQLCipher/full-database encryption, cloud/server-backed config, and direct frontend database writes.
