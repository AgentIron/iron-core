## Context

Issue #80 is the next profile-foundation slice after profile identity was moved into system prompt Section 1. Existing `AgentProfile` records can represent identity, provider/model selection, tool policy, skill policy, and approval posture, and `ConfigStore` already persists opaque profile records. What is missing is the product-level bootstrap and interpretation contract: how shipped profiles appear in the store, how deleting/editing them is respected, and how selected profile policy becomes effective for a session.

The central constraint is that `explore`, `plan`, and `apply` are ordinary records. They must not become hidden modes, reserved IDs, immutable built-ins, or runtime branches. The implementation should make profile foundations robust enough that the shipped default profile contents can evolve later without changing the architecture.

## Goals / Non-Goals

**Goals:**

- Provide core-owned shipped `AgentProfile` definitions for `explore`, `plan`, and `apply`.
- Provide a canonical, idempotent, non-destructive seeding API for those shipped defaults.
- Preserve user deletion and edits during normal startup by using durable first-run seed state.
- Provide explicit restore-missing behavior for callers that intentionally want shipped defaults recreated.
- Snapshot selected profile policy into the session so active sessions do not drift when stored profiles are edited.
- Make `AutoApprove` functionally bypass approval prompts for profile-backed sessions.
- Reject `ReadOnly` as a profile approval schema/API value.
- Apply profile tool filters to the primary session's effective tool catalog using exact canonical model-visible tool names.
- Resolve explicit profile provider/model references against the effective catalog and provider/credential machinery with warning-backed fallback to runtime default.

**Non-Goals:**

- No hard read-only sandboxing, path-scoped write policy, or semantic tool capability taxonomy.
- No mid-session profile switching or implicit live profile reload.
- No user-defined provider/model creation UI or profile-embedded provider definitions.
- No automatic updates of existing seeded profile records when shipped defaults change in a future release.
- No destructive reset/overwrite-defaults operation in this issue.
- No special runtime behavior based on the strings `explore`, `plan`, or `apply` after seeding.

## Decisions

### 1. Use durable first-run seed state, not profile existence alone

Normal bootstrap must distinguish "defaults have never been seeded" from "defaults were seeded and the user deleted one." Existence-only logic cannot make that distinction and would silently recreate deleted defaults. Add durable bootstrap metadata such as `agent_profiles.default_seed.version = 1` and have the seed operation write it after the first normal seed attempt.

Alternatives considered:

- **Existence-based seeding:** simpler, but violates deletability because missing shipped profiles reappear on every startup.
- **Tombstones:** precise, but requires delete-path special casing for shipped IDs and makes shipped defaults less ordinary.
- **Hidden profile marker record:** avoids a new metadata API, but pollutes the profile namespace and leaks special records into list/export concerns.

### 2. Core owns the seed operation; frontends call it

`iron-core` should expose the shipped definitions and the seed/restore operation instead of asking AgentIron UI, CLI, and future schedulers to reimplement bootstrap semantics. Frontends should receive a report describing created, skipped, and diagnostic outcomes.

Candidate API shape:

```rust
enum DefaultProfileSeedPolicy {
    FirstRunOnly,
    RestoreMissing,
}

struct DefaultProfileSeedReport {
    policy: DefaultProfileSeedPolicy,
    marker_was_present: bool,
    marker_written: bool,
    created: Vec<AgentProfileId>,
    skipped_existing: Vec<AgentProfileId>,
    diagnostics: Vec<DefaultProfileSeedDiagnostic>,
}
```

`FirstRunOnly` creates missing shipped defaults only when the seed marker is absent, then writes the marker. `RestoreMissing` creates missing shipped defaults regardless of marker state, but still preserves existing records.

### 3. Shipped defaults are bootstrap templates only

Once a shipped profile is persisted, it is user data. Normal startup must not overwrite existing `explore`, `plan`, or `apply` records, even if they differ from current shipped definitions. Future product UX may offer compare/reset/update flows, but this issue only provides first-run seeding and non-destructive restore-missing behavior.

Default seeded records should use:

- `AgentProfileProvider::RuntimeDefault`
- `ToolFilter::Inherit`
- `SkillFilter::Inherit`
- `AgentApproval::PerTool` unless a caller intentionally changes a profile later
- profile-specific identity prompts for explore/plan/apply stance

### 4. Snapshot selected profile policy into the session

Stored profiles are templates for future sessions. Selecting a profile should snapshot the effective execution policy into the session: profile ID, identity, tool filter, approval policy, and provider/model resolution outcome/diagnostics. Later edits or deletion of the stored profile must not implicitly mutate existing sessions.

This avoids prompt-cache churn, unexpected tool-catalog changes, and approval-policy drift in active conversations. A future explicit profile-switch/reload API can intentionally invalidate caches and recompute session policy; it is out of scope here.

### 5. Apply tool filters during session tool catalog construction

Profile `ToolFilter` should remain policy, not a one-time expansion into hidden tools. Store the session-effective filter and apply it whenever the session tool catalog is built or rebuilt. Exact names are canonical model-visible tool names after MCP/plugin namespacing.

Semantics:

- `Inherit`: no profile-level filtering.
- `Allow(names)`: only listed canonical tool names are visible/executable if available.
- `Deny(names)`: listed canonical tool names are hidden/rejected if available.

Unknown names should be accepted in stored/imported profile data to support cross-machine sync, but session preparation may emit diagnostics.

### 6. Make `AutoApprove` operational and reject `ReadOnly`

Profile approval surface for this milestone is exactly `PerTool` and `AutoApprove`. `AutoApprove` must bypass approval prompts for the session. `ReadOnly` should be rejected during profile registration/loading/import rather than silently mapped, because it implies an enforcement guarantee this milestone does not provide.

Read-only stance is represented through identity prompt text and optional explicit tool filtering.

### 7. Provider/model fallback is setup-time only

Profile configuration may reference provider/model pairs unavailable on the current machine. Import/loading should preserve those references if shape is valid. During session setup, explicit profile provider/model references are checked against the effective catalog and provider/credential resolution. If unavailable, return a warning diagnostic and fall back to the runtime default provider path. If the runtime default is unavailable, fail with an actionable error that includes the original explicit-reference failure where possible.

Fallback must not happen after inference starts. Mid-turn provider failures should remain explicit failures so users can debug the selected provider/model.

## Risks / Trade-offs

- **Risk: adding seed metadata expands ConfigStore scope.** → Keep the metadata API narrow and domain-oriented; do not expose SQLite tables directly or use fake profile records.
- **Risk: users expect profile edits to affect active sessions immediately.** → Document that profile edits affect future sessions; future explicit profile-switch/reload can be designed with cache invalidation and user-visible events.
- **Risk: `AutoApprove` increases destructive-action exposure.** → Make it explicit in profile policy, include it in session diagnostics/debug state, and keep shipped defaults at `PerTool`.
- **Risk: exact tool names are brittle across plugin/MCP availability.** → Accept unknown names in profile data and surface session diagnostics rather than failing load/import.
- **Risk: provider fallback hides model mismatch.** → Restrict fallback to setup-time resolution and surface warnings through structured diagnostics, not logs only.

## Migration Plan

1. Add config-store bootstrap metadata support or use an existing domain-scoped settings API if one is already present.
2. Add first-run seed marker migration/default handling without altering existing profile records.
3. Introduce shipped default profile definitions and seed/restore API.
4. Extend session state and handoff state, if needed, with effective profile policy snapshot fields.
5. Wire prompt/session preparation to use session-effective tool filter, approval policy, and provider/model resolution diagnostics.
6. Preserve existing profile records; do not modify user data during migration except when the caller explicitly invokes seed/restore.

Rollback is straightforward: seeded profiles are ordinary records and can be deleted. The seed marker can remain inert if the seeding API is not called.

## Open Questions

- What exact config-store primitive should hold the seed marker if a generic domain-scoped settings API already exists but is not currently exposed in public code?
- Should seed/restore diagnostics be profile-specific domain types only, or should they reuse a broader diagnostics/event mechanism if one exists?
- Should handoff carry only the effective policy snapshot, or both snapshot plus original profile ID for display? The design preference is snapshot for behavior plus profile ID for traceability.
