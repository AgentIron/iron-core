## 1. Public Policy And Validity Boundary

- [x] 1.1 Add the public `ProfileDeletePolicy` enum in the profile domain and re-export it from the crate API.
- [x] 1.2 Centralize raw profile record classification so supported schema, decoding, stable-ID validation, profile-field validation, and management diagnostics use one implementation.
- [x] 1.3 Refactor management profile get/list paths to use the shared classifier without changing ready versus needs-attention outcomes or diagnostic categories.
- [x] 1.4 Add focused classifier tests covering valid, malformed JSON, unsupported schema, reserved default, unsupported approval, and structurally invalid profile records.

## 2. Typed Errors

- [x] 2.1 Add `ConfigError::MinimumValidProfiles { minimum, remaining }` with public field documentation and an actionable display message.
- [x] 2.2 Add `ManagementError::MinimumValidProfiles { minimum, remaining }` and map the config-store variant without reducing it to a string or generic storage error.
- [x] 2.3 Add error-mapping tests that assert both typed variants preserve the requested minimum and computed remaining count.

## 3. Atomic Store Deletion

- [x] 3.1 Add a policy-aware checked profile deletion method that starts with SQLx `Pool::begin_with("BEGIN IMMEDIATE")` before any prompt or profile read.
- [x] 3.2 Preserve existing prompt schema, decode-integrity, sorted direct-reference checks, and their error precedence inside the write-reserved transaction.
- [x] 3.3 Read and classify profile rows inside the transaction, subtract one only when the exact target is valid, and reject when the computed remaining count is below the requested minimum.
- [x] 3.4 Delete and commit only after all checks pass, relying on transaction rollback for integrity, reference, policy, query, and busy-timeout failures.
- [x] 3.5 Delegate existing `ConfigStore::delete_profile` and checked deletion APIs to `AllowZero` so current callers retain unrestricted behavior.

## 4. Management API And Registry Synchronization

- [x] 4.1 Add and document `ConfigManagementService::delete_profile_with_policy`, preserving protected-default validation and delegating durable checks to the store.
- [x] 4.2 Keep attached profile-registry removal after durable commit and preserve the existing typed partial-operation result for registry failure.
- [x] 4.3 Delegate existing `ConfigManagementService::delete_profile` to `AllowZero` without changing its public signature or behavior.
- [x] 4.4 Update crate and module API documentation to describe valid-profile counting, opt-in semantics, typed failures, and unrestricted compatibility.

## 5. Store Tests

- [x] 5.1 Test that one valid profile plus malformed, unsupported, and structurally invalid records cannot satisfy `RequireMinimumValid(1)` when deleting the valid profile.
- [x] 5.2 Test deleting malformed, unsupported, structurally invalid, and missing targets when one valid profile satisfies the minimum, verifying each target contributes zero.
- [x] 5.3 Test that a missing or invalid target is rejected with accurate values when the requested postcondition is already unsatisfied.
- [x] 5.4 Test `RequireMinimumValid(0)` and existing unrestricted deletion, including deletion of the final valid profile and cleanup of invalid records.
- [x] 5.5 Test prompt-reference conflict and prompt-integrity-unknown precedence when the minimum policy would also reject deletion.
- [x] 5.6 Test transaction rollback and durable target preservation for minimum, prompt, and storage failures.

## 6. Cross-Connection Concurrency Tests

- [x] 6.1 Open two independent file-backed `ConfigStore` clients against one temporary database and synchronize deletion attempts for two valid profiles with `RequireMinimumValid(1)`.
- [x] 6.2 Assert at most one concurrent deletion succeeds, the loser returns a typed minimum or actionable busy-timeout result as appropriate, and at least one valid profile remains durably stored.
- [x] 6.3 Add a cross-connection prompt-write/delete test proving a new prompt reference cannot interleave between checked reference reads and profile deletion.

## 7. Management Tests And Verification

- [x] 7.1 Add management-service tests for minimum rejection, invalid and missing targets, prompt conflict precedence, and typed error values.
- [x] 7.2 Add attached-registry tests proving pre-commit rejection leaves memory unchanged and post-commit registry failure retains the existing partial-operation shape.
- [x] 7.3 Add backward-compatibility tests proving existing management deletion can remove the final valid persisted profile when no prompt blocks it.
- [x] 7.4 Run targeted config-store and management tests, then `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and the full test suite.
