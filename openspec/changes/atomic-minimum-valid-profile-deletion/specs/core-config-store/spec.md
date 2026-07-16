## ADDED Requirements

### Requirement: Policy-aware profile deletion SHALL atomically enforce the valid-profile minimum
The SQLite-backed `ConfigStore` SHALL acquire a write reservation before reading prompt or profile state for policy-aware checked profile deletion. Prompt integrity checks, prompt-reference checks, profile classification, target contribution calculation, minimum enforcement, and deletion SHALL occur in the same transaction.

The store SHALL return `ConfigError::MinimumValidProfiles { minimum, remaining }` when the computed remaining valid profile count is below the requested minimum. Transaction failure or policy rejection SHALL preserve the target and all other records.

#### Scenario: Two stores concurrently delete different valid profiles
- **WHEN** two independently opened file-backed `ConfigStore` clients share a database containing exactly two valid profiles
- **AND** both clients concurrently delete different profiles with `RequireMinimumValid(1)`
- **THEN** at most one deletion succeeds
- **AND** at least one valid persisted profile remains after both operations complete

#### Scenario: Concurrent writer changes profile state
- **WHEN** one policy-aware deletion has acquired its write reservation and another connection attempts to insert, update, or delete a profile
- **THEN** SQLite serializes the writes using the configured busy timeout
- **AND** the deleting transaction computes its decision from one protected database state

#### Scenario: Concurrent writer creates a prompt reference
- **WHEN** policy-aware deletion has acquired its write reservation and another connection attempts to persist a prompt referencing the target profile
- **THEN** the prompt write cannot interleave between reference verification and profile deletion
- **AND** the database does not commit a successful checked deletion based on stale prompt-reference state

#### Scenario: Prompt conflict takes precedence over minimum failure
- **WHEN** the target profile is referenced by a supported decodable prompt and deleting it would leave fewer valid profiles than requested
- **THEN** the checked store operation returns `ConfigError::ProfileReferencedByPrompts`
- **AND** preserves the target profile

#### Scenario: Invalid target contributes zero to the valid count
- **WHEN** the target profile is malformed, unsupported, or structurally invalid
- **THEN** the transaction computes `remaining` without subtracting the target from the valid count

#### Scenario: Missing target contributes zero to the valid count
- **WHEN** no profile row matches the target ID
- **THEN** the transaction computes `remaining` without subtracting from the valid count
- **AND** the delete remains idempotent when the requested minimum is satisfied

### Requirement: Existing ConfigStore profile deletion SHALL remain unrestricted
Existing `ConfigStore::delete_profile` and checked deletion callers SHALL retain their current behavior by using the unrestricted deletion policy. They SHALL continue to enforce prompt integrity and direct-reference safety but SHALL NOT acquire a minimum-valid-profile requirement unless the caller opts into one.

#### Scenario: Existing store caller deletes the last valid profile
- **WHEN** an existing caller invokes unrestricted profile deletion for the only valid persisted profile and no prompt blocks deletion
- **THEN** deletion succeeds
- **AND** zero valid persisted profiles may remain

#### Scenario: Existing store caller deletes an invalid profile
- **WHEN** an existing caller invokes unrestricted profile deletion for a malformed or unsupported profile and prompt integrity can be proven
- **THEN** deletion retains its current cleanup behavior
