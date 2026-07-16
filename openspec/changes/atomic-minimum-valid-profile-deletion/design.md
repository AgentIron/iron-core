## Context

`ConfigManagementService::delete_profile` currently protects the built-in default identity, delegates to `ConfigStore::delete_profile_checked`, and removes the profile from an attached registry only after durable deletion. The store transaction checks prompt schema support, decodes every prompt to prove reference safety, rejects direct references, deletes the profile, and commits.

Consumers that require at least one persisted usable profile currently call a list/count API before deletion. That count occurs outside the deletion transaction. Because the current SQLx transaction is deferred, separate processes can both read the same two-profile snapshot and delete different records before either observes the other's result.

The store contains opaque, versioned profile rows. Management listing treats a row as valid only when its schema is supported, its payload decodes as `AgentProfile`, and the decoded profile passes structural profile validation. Unsupported, malformed, reserved, and otherwise invalid records are reported as `ManagedRecord::NeedsAttention`, not usable profiles. The deletion policy must use the same classification without introducing a second interpretation of profile validity.

## Goals / Non-Goals

**Goals:**

- Make a caller-selected minimum-valid-profile invariant atomic across separate SQLite connections and processes.
- Preserve prompt integrity and direct-reference protection in the same transaction as the minimum check and deletion.
- Reuse one profile-validity classification for management reads and deletion counting.
- Preserve existing unrestricted deletion APIs and behavior.
- Return structured minimum/remaining values at both config-store and management boundaries.
- Preserve durable-first registry synchronization and existing partial-operation reporting.

**Non-Goals:**

- Require every consumer to retain a persisted profile; the policy remains opt-in.
- Count the built-in in-memory `default` profile as a persisted valid profile.
- Repair, migrate, or delete malformed or unsupported profile records automatically.
- Change prompt-reference rules, add cascading deletion, or change dependency-impact DTOs.
- Make the durable SQLite transaction and in-memory registry update atomic with each other.
- Add a database migration or persisted policy setting.

## Decisions

### Add one public policy type and preserve existing entry points

Expose `ProfileDeletePolicy` as a public profile-domain type with `AllowZero` and `RequireMinimumValid(usize)`. Both `ConfigStore` and `ConfigManagementService` can use the type without creating a config-to-management dependency.

Add a policy-aware checked store operation and `ConfigManagementService::delete_profile_with_policy`. Keep existing `ConfigStore::delete_profile`, `ConfigStore::delete_profile_checked`, and `ConfigManagementService::delete_profile` entry points, delegating them to `AllowZero`. `RequireMinimumValid(0)` is valid and imposes the same count floor as `AllowZero`, while still using the policy-aware path.

Alternative considered: change `delete_profile` to always require one valid profile. Rejected because zero persisted profiles is currently valid and existing callers rely on unrestricted deletion.

Alternative considered: represent the minimum as `Option<usize>`. Rejected because the named enum makes caller intent and future policy extension explicit.

### Share record-local profile validity with management reads

Extract or otherwise centralize a pure profile-record classification that accepts the stable ID, schema version, and raw payload and applies the rules used to decide whether management listing returns `ManagedRecord::Ready`. A profile counts only when:

1. Its schema version equals `PROFILE_SCHEMA_VERSION`.
2. Its payload is valid JSON that decodes as `AgentProfile`.
3. Its stable ID and decoded profile pass the same structural validation used by management reads, including protected-default and supported approval rules.

The count does not include the built-in in-memory default profile. It also does not depend on registry membership, provider credential availability, or prompt references. Record-local classification intentionally does not introduce new cross-record duplicate-name behavior beyond the rules already used by management listing.

Alternative considered: count rows with the current schema version in SQL. Rejected because malformed and structurally invalid payloads would satisfy the minimum.

Alternative considered: call `ConfigManagementService::list_profiles` from the store. Rejected because it would invert the dependency boundary, perform multiple non-transactional reads, and make atomicity impossible.

### Reserve the SQLite writer before any integrity read

The policy-aware store operation begins its transaction with SQLite `BEGIN IMMEDIATE` before reading prompts or profiles. SQLx 0.9 exposes `Pool::begin_with`, which returns a normal transaction that can execute queries and commit or roll back while using the custom begin statement. The existing configured busy timeout remains authoritative when another process holds the writer reservation.

Within that transaction, operations occur in this order:

```text
BEGIN IMMEDIATE
  -> verify supported prompt schemas
  -> decode prompts and identify direct references
  -> reject integrity-unknown or reference conflicts
  -> read and classify all persisted profile rows
  -> total_valid = number classified valid
  -> target_is_valid = target row exists and is classified valid
  -> remaining = total_valid - target_is_valid_as_0_or_1
  -> reject when remaining < requested minimum
  -> DELETE target row
COMMIT
```

This order preserves existing prompt-integrity and reference-conflict precedence. Once `BEGIN IMMEDIATE` succeeds, other SQLite writers cannot insert a new prompt reference, change profile validity, or delete a profile until this transaction commits or rolls back. A second policy-aware deleter therefore reads the first deleter's committed state rather than the same stale snapshot.

Alternative considered: begin a deferred transaction and issue a no-op write before counting. Rejected because the lock acquisition is implicit and easier to regress; `BEGIN IMMEDIATE` states the concurrency requirement directly.

Alternative considered: use an in-process mutex. Rejected because it cannot coordinate separate `ConfigStore` instances in different processes.

### Compute the target's contribution instead of assuming every target is valid

The remaining valid count is the total valid count minus one only if the exact target row is classified valid. A malformed, unsupported, structurally invalid, or missing target contributes zero.

Deletion remains idempotent for a missing target when the requested minimum is already satisfied and prompt integrity can be proven. The policy still rejects any operation whose computed remaining count is below the requested minimum, even when the target contributes zero; this reports that the requested postcondition cannot be met rather than claiming policy success.

Alternative considered: reject malformed targets unconditionally. Rejected because existing unrestricted deletion permits cleanup of opaque records and the minimum policy only needs to prevent loss of valid records.

### Add typed errors and retain post-commit synchronization

Add `ConfigError::MinimumValidProfiles { minimum, remaining }` and map it to a distinct `ManagementError::MinimumValidProfiles { minimum, remaining }`. Adapters can branch on the variant and display structured values without parsing strings.

The service performs registry removal only after the policy-aware store transaction commits. Policy, prompt-integrity, reference, busy-timeout, and other pre-commit failures leave the registry unchanged. A registry failure after commit retains the existing `ManagementError::Partial` behavior.

Alternative considered: map the minimum failure to generic validation or conflict text. Rejected because the caller needs the requested and computed values and issue #100 explicitly requires typed adapter handling.

## Risks / Trade-offs

- [Classifying every profile while holding the SQLite writer reservation increases write-lock duration] -> Keep classification pure and bounded to profile rows; correctness is preferred for a user-scale configuration database, and no schema migration or external I/O occurs inside the transaction.
- [Management and store validity semantics can drift later] -> Route both through one shared classifier and add paired tests for `ManagedRecord::Ready` versus minimum-count eligibility.
- [A high requested minimum can reject cleanup of malformed records when the database is already below that floor] -> Document the policy as a required postcondition; callers performing repair can use `AllowZero` or a lower explicit minimum.
- [Concurrent tests can pass without exercising separate connection locking] -> Use two independently opened file-backed `ConfigStore` instances, synchronize deletion attempts, and assert both result count and final durable state.
- [Changing from deferred to immediate acquisition can surface busy-timeout errors earlier] -> Preserve the existing typed busy-timeout mapping and document that writer reservation is intentional.
- [Registry state can diverge after durable success] -> Preserve and test the existing typed partial-operation contract rather than pretending SQLite and memory share one transaction.

## Migration Plan

1. Add and export the policy and typed error variants without changing existing deletion call sites.
2. Centralize profile-record classification and switch management listing to use it without changing observable diagnostics.
3. Add the write-reserved policy-aware checked store operation and delegate existing checked deletion to `AllowZero`.
4. Add the management policy method and delegate existing management deletion to `AllowZero`.
5. Add store, management, and concurrent file-backed tests before consumers remove their command-level prechecks.

No database migration is required. Rollback to an older binary preserves all stored data; callers compiled against the new API must be rolled back with the library because the new policy and error variants will no longer exist.

## Open Questions

None. Issue #100 defines the policy shape, validity requirement, atomicity boundary, compatibility contract, and error data needed for implementation.
