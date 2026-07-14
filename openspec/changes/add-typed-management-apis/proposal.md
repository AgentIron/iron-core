## Why

AgentIron's configuration and scheduled-task UI needs a core-owned typed management boundary instead of decoding opaque ConfigStore records, reproducing identity rules, or inferring dependencies and scheduler state. The underlying profile, prompt, automation-task, credential, and scheduled-task domains now exist, so they can be composed into a stable UI-safe API.

## What Changes

- Add an application-facing `ConfigManagementService` for listing, reading, saving, and deleting profiles, stored prompts, automation tasks, API-key credentials, and optionally attached scheduled automation tasks.
- Give stored prompts core-generated immutable IDs, mutable display names, required uniquely indexed ASCII normalized lookup handles, deterministic collision rejection, and reference-safe rename behavior.
- Validate typed management writes and return per-record diagnostics when persisted profiles or prompts are malformed, unsupported, or reference unavailable profiles or skills.
- Add typed direct-reference and transitive delete-impact queries across profiles, stored prompts, automation tasks, provider credentials, and scheduled tasks.
- Return credential metadata and auth status without returning stored API keys, access tokens, refresh tokens, or other secret material; preserve the existing OAuth lifecycle APIs.
- Compose scheduled-task CRUD with the existing read-only inspection and explicit reconciliation contracts, without exposing arbitrary host scheduler commands or raw schedule payloads.
- Explicitly defer interactive stored-prompt preview and durable scheduled-run/session history to focused follow-up changes; neither creates a direct prompt scheduler or second headless prompt runner.

## Capabilities

### New Capabilities
- `config-management-api`: Typed UI-safe management operations, diagnostics, dependency impact, credential redaction, and composition of scheduled-task desired and observed state.

### Modified Capabilities
- `agent-profiles`: Add durable typed save/delete management behavior and profile dependency protection.
- `stored-prompts`: Add display and normalized lookup identity, rename semantics, management validation, and profile/skill diagnostics.
- `automation-tasks`: Expose automation tasks through the management boundary and report their prompt, profile, credential, and schedule dependencies.
- `provider-credential-orchestration`: Add secret-safe credential listing and typed API-key replacement/deletion operations while preserving OAuth behavior.

## Impact

- Adds public management types and async APIs over `ConfigStore`, existing registries, credential resolution, and `ScheduleManager`.
- Evolves stored-prompt payload identity while preserving ConfigStore prompt record IDs as stable references from automation tasks.
- Adds a public `ManagementError` hierarchy for fatal storage, validation, reference, conflict, integrity-unknown, unavailable capability, scheduler, and partial-operation failures.
- Adds schema-aware dependency traversal and tests covering malformed records, normalized-name collisions, rename stability, transitive impacts, secret redaction, and scheduler composition.
- Requires AgentIron callers to use typed core inputs and outputs rather than raw profile, prompt, credential, or schedule records.
