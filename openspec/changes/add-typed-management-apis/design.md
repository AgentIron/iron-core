## Context

`ConfigStore` exposes opaque profile and prompt record CRUD, typed automation-task and scheduled-task CRUD, encrypted credential operations, and some direct reference queries. `IronAgent` separately owns in-memory profile and stored-prompt registries. The completed scheduled-automation-task change adds `ScheduleManager` for desired/observed status and explicit host reconciliation.

AgentIron needs one typed application boundary over these pieces. It must not decode opaque JSON, duplicate normalized identity logic, reveal credentials, or infer transitive dependencies. Existing databases may contain stored-prompt schema v1 payloads whose record ID is also their only user-facing identity.

## Goals / Non-Goals

**Goals:**

- Provide typed management CRUD and best-effort list diagnostics for UI callers.
- Preserve immutable IDs for all structural references while adding mutable normalized prompt handles.
- Centralize direct and transitive dependency analysis and structural delete conflicts.
- Reuse existing credential protection and scheduler desired/observed contracts.
- Keep valid legacy stored prompts readable and upgrade them on typed save.

**Non-Goals:**

- Interactive stored-prompt preview.
- Durable scheduled-run/session history or output retention.
- Direct scheduling or unattended execution of stored prompts.
- Arbitrary host scheduler command editing.
- Returning stored secret material.
- Replacing ConfigStore or runtime registries with a second persistence model.

## Decisions

### Add `ConfigManagementService` over existing domain stores

Introduce a public `ConfigManagementService` constructed from `ConfigStore`, attached profile and prompt registries, and explicit validation dependencies. Scheduler support is an optional attachment so profile, prompt, automation-task, and credential management remains available when host scheduling is unavailable. Domain-specific typed persistence remains close to ConfigStore, while the service coordinates cross-record validation, diagnostics, dependency traversal, registry synchronization, credential summaries, and optional schedule status.

The service is the facade in the architectural sense; no second facade wrapper is added. It returns management-specific result/report types rather than exposing `ProfileRecord`, `PromptRecord`, `CredentialRecord`, or raw schedule payloads. Fatal operation failures use a public `ManagementError` hierarchy with storage, validation, reference, conflict, integrity-unknown, scheduler-unavailable, scheduler, and partial-operation categories.

Single and bulk reads use the same record outcome:

```text
ManagedRecord<T>
  Ready(T)
  NeedsAttention {
    id,
    decoded: Option<T>,
    diagnostics,
  }
```

A malformed payload can therefore remain discoverable without being returned as valid. Fatal store failures remain outer `ManagementError` values.

Alternative considered: add every operation directly to `IronAgent`. Rejected because configuration management must work before a runtime is started and should not require active sessions or providers.

Alternative considered: let the desktop adapter compose ConfigStore, registries, and ScheduleManager. Rejected because that reproduces core-owned schema and dependency rules in the UI layer.

### Keep immutable prompt IDs separate from mutable handles

Core generates an immutable unique prompt ID when creating a stored prompt. Stored prompts gain `display_name` and a dedicated required, uniquely indexed `normalized_name` column; the ConfigStore prompt record ID remains authoritative for automation-task references. Save derives the normalized handle from display name. Lookup by ID and lookup by handle are separate methods. Rename changes display name and handle only.

Normalized handles are ASCII lowercase kebab case matching `[a-z0-9]+(?:-[a-z0-9]+)*`. Whitespace, underscores, and hyphens become a single hyphen; other punctuation is removed; leading and trailing separators are removed. A name that produces an empty handle is rejected. Using one canonical separator avoids treating visually equivalent `_` and `-` handles as distinct and aligns prompt identity with automation-task lookup. The indexed column enforces store-wide uniqueness transactionally. IDs retain exact identity semantics.

Alternative considered: use the normalized handle as the record ID and rewrite task references on rename. Rejected because rename would become a multi-record identity migration and partial failures could leave dangling references.

Alternative considered: generate numeric suffixes on collision. Rejected because lookup identity would depend on write order and could surprise callers.

### Read legacy prompt schema and upgrade on write

Stored-prompt schema advances from v1 to v2. A v1 payload derives a transitional title-cased display name from its kebab-case record ID and a normalized handle from that display name. Valid v1 entries remain executable and manageable; saving one writes v2 under the same record ID.

Migration never fails the entire ConfigStore because legacy IDs normalize to the same handle. Every colliding record receives a reserved deterministic `legacy-<stable-id-hash>` value in the required unique column and an explicit `NeedsRename` identity state. Reserved repair handles are not normal user-facing lookup handles. The records remain retrievable by immutable ID, and saving a valid new display name replaces the repair handle and clears the state. This migration-only exception keeps damaged identity repairable without silently choosing a winner or generating a permanent user-facing suffix.

This avoids a bulk migration that would need to choose user-facing names and collision outcomes for opaque records. New and updated records always write v2.

Alternative considered: reject all v1 records after the schema bump. Rejected because persisted prompt and automation-task references are shipped data that must remain usable.

### Validate structure at write time and availability at read time

Typed writes validate prompt instructions, skill identifier shape and uniqueness, and profile existence. Skill availability is a best-effort snapshot check against the skills currently available to the selected profile. New writes require requested skills to be available in that snapshot, which is sufficient creation-time validation but does not guarantee later execution availability. Existing records whose skills later become unavailable remain readable with diagnostics and retain their requested skills.

Profile writes reuse existing ID, name, provider-context, and approval deserialization rules, including rejection of `ReadOnly` and `RequireApproval`.

Alternative considered: accept any skill string and defer all checks to execution. Rejected because management callers need actionable feedback before creating automation tasks.

### Model dependency impact as a typed graph projection

Dependency results contain typed entity references, direction, direct/transitive classification, and a relationship path. Traversal follows:

```text
ProviderCredential <- AgentProfile <- StoredPrompt <- AutomationTask <- ScheduledTask
```

The arrows represent dependency on the entity to the left. Queries can project both dependencies and dependents without exposing payload schemas. Results are deterministic and deduplicated.

Structural deletion is blocked only for direct identity references: profile by prompts, prompt by tasks, and task by schedules. Conflicts list direct referrer IDs; callers can use impact queries for the wider warning surface, including transitive schedule impacts. If malformed or unsupported records prevent core from proving referential integrity, deletion returns `ManagementError::IntegrityUnknown` rather than assuming the target is unreferenced. Credential removal is not blocked because missing credentials are already a valid, recoverable auth state and OAuth disconnect must remain possible.

Alternative considered: cascade deletes. Rejected because a UI action could silently destroy reusable prompts, tasks, and host desired state.

### Reuse scheduler and credential boundaries

Scheduled-task desired-state CRUD (save, get, list, delete definitions) is available without a host scheduler because it operates only on ConfigStore. Host scheduler operations—inspection of observed state, reconciliation, and combined deletion—require an attached `ScheduleManager` and return `ManagementError::SchedulerUnavailable` when absent. Inspection remains read-only and reconciliation remains explicit. The management layer does not accept installation commands or host-native definitions.

A combined schedule deletion removes or disables the owned host entry before deleting desired ConfigStore state. If host removal fails, desired state is preserved. If host removal succeeds and desired deletion fails, the operation returns a typed partial outcome with `host_removed = true`, `desired_deleted = false`, and resulting drift diagnostics. Host-first ordering avoids leaving an orphan entry that can continue executing after desired state has disappeared.

Credential listing returns configured credential rows only. Summaries combine persisted metadata and persisted-state auth status; transient runtime states such as refresh-in-progress are not synthesized without persisted evidence. API-key replacement accepts only provider slug plus secret input, constructs `StoredCredential::ApiKey` internally, and replaces whichever credential mode is currently stored, including OAuth. Credential deletion removes whichever mode is configured. Both operations return redacted summaries or status, while existing OAuth-specific initiation, polling, and refresh APIs remain the only OAuth token-flow mutation paths.

### Synchronize attached registries after durable writes

Profile and prompt management writes persist first and update attached registries only after durable success. A registry update failure returns a typed partial-operation result identifying that durable state changed while the attached runtime view did not. The service never mutates a registry before persistence succeeds and does not attempt to coordinate other processes or unattached runtimes.

### Defer preview and durable run history explicitly

Prompt preview requires an active interactive session, approval flow, and structured delegation result, which is a separate execution-facing contract. Durable scheduled-run history requires persistence, retention, status transitions, and parent/child session linkage not provided by active runtime session lists. Neither is necessary for safe typed configuration management, and both receive focused follow-up changes.

## Risks / Trade-offs

- [A facade can duplicate existing domain APIs] -> Keep domain validation and persistence primitives authoritative; management methods delegate rather than reimplement them.
- [Legacy prompt handles can collide after normalization] -> Report deterministic per-record diagnostics and require an explicit rename instead of generating unstable suffixes.
- [Registry and durable state can diverge after writes] -> Persist first, update in-memory registries only after success, and make runtime reload behavior explicit in API documentation and tests.
- [Skill inventory differs by project] -> Inject the inventory used for a management operation and preserve unavailable persisted requests with diagnostics.
- [Dependency traversal becomes stale as schemas evolve] -> Keep traversal in core and cover every relationship edge with focused tests.
- [Credential summaries accidentally gain secret fields] -> Use dedicated redacted types that contain no secret-bearing values and test serialization/debug output.
- [Combining CRUD and host status can imply atomic scheduling] -> Keep desired-state save and host reconciliation as separate calls and report drift after partial failure.

## Migration Plan

1. Add stored-prompt v2 domain fields and dual-version decoding without changing record IDs.
2. Add transactional normalized-handle collision checks for new and updated prompt records.
3. Add typed profile and prompt persistence primitives plus best-effort diagnostic reports.
4. Add dependency projection and direct delete-conflict enforcement.
5. Add redacted credential management and compose existing scheduler operations.
6. Expose the management service after domain-level and migration tests pass.

Rollback keeps v2 prompt records in ConfigStore. Older binaries will treat their unsupported schema as diagnostics rather than corrupting them; restoring write compatibility requires the newer binary or a database backup made before v2 writes.

## Resolved Questions

- Interactive stored-prompt preview is tracked in #97. Durable scheduled-run/session history is tracked in #98. Both are explicitly excluded from this change.
