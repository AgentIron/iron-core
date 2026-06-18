## Why

Agent profiles are now the user-facing abstraction for reusable agent identity and execution policy, but the product still lacks a canonical bootstrap path for shipped profiles and a complete session-scoped interpretation of profile policy. This change establishes the foundation: ordinary persisted default profile records, non-destructive seeding, functional approval policy, exact-name tool filtering, and provider/model fallback diagnostics.

## What Changes

- Add core-owned shipped `AgentProfile` definitions for `explore`, `plan`, and `apply` as normal persisted profile payloads, not runtime modes or immutable built-ins.
- Add an idempotent default-profile seed API using durable first-run seed state so normal bootstrap does not recreate user-deleted shipped profiles.
- Add an explicit non-destructive restore-missing-defaults operation for callers that intentionally want deleted shipped defaults recreated.
- Snapshot the selected profile's effective execution policy into a session at session setup; later stored-profile edits affect future sessions only.
- Make `AgentApproval::AutoApprove` functional for profile-backed sessions.
- Reject `ReadOnly` as a user-facing profile approval value; read-only behavior belongs in identity prompts and explicit tool policy.
- Apply profile `ToolFilter` policy to primary session tool catalogs by exact canonical model-visible tool names.
- Resolve explicit profile provider/model references using the effective model catalog and existing provider/credential resolution, warning and falling back to runtime default when explicit references are unavailable.
- Keep shipped profiles provider/model-neutral by default (`RuntimeDefault`) and tool/skill-neutral by default (`Inherit`).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `agent-profiles`: Add shipped ordinary default profile definitions, seed/restore semantics, session policy snapshots, functional approval policy, profile tool filtering, provider/model fallback diagnostics, and rejection of `ReadOnly` profile approval.
- `core-config-store`: Add durable bootstrap metadata needed to distinguish first-run default-profile seeding from user-deleted shipped defaults.
- `effective-model-catalog`: Define how profile provider/model availability checks use the effective catalog without making unavailable references config-invalid.

## Impact

- Affected modules likely include `src/profile/`, `src/config/`, `src/runtime.rs`, `src/durable.rs`, `src/request_builder.rs`, `src/prompt_runner.rs`, `src/mcp/` or session tool catalog construction, and profile-related tests.
- Adds or extends public APIs for default profile definitions, seed/restore operations, seed reports/diagnostics, profile session preparation, and profile resolution diagnostics.
- May require a config-store migration or existing settings-domain extension for durable bootstrap metadata.
- Requires tests for idempotent seeding, deletion preservation, restore-missing behavior, session policy snapshotting, functional `AutoApprove`, `ReadOnly` rejection, exact-name tool filtering, and provider/model fallback diagnostics.
