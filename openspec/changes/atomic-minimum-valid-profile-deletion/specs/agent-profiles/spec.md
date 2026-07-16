## ADDED Requirements

### Requirement: Core SHALL support an opt-in minimum-valid-profile deletion policy
Core SHALL expose a public `ProfileDeletePolicy` that distinguishes unrestricted deletion from deletion requiring a caller-selected minimum number of valid persisted profiles to remain. `ConfigManagementService` SHALL expose a policy-aware profile deletion operation, and its existing `delete_profile` operation SHALL retain unrestricted behavior.

For this policy, a persisted profile SHALL count as valid only when its schema version is supported, its payload decodes as the current `AgentProfile`, and its stable ID and decoded fields pass the same structural validity rules used for a ready management profile record. Malformed, unsupported, structurally invalid, and built-in in-memory profiles SHALL NOT count.

#### Scenario: Deleting the only valid persisted profile is rejected
- **WHEN** one valid persisted profile exists and a caller deletes it with `RequireMinimumValid(1)`
- **THEN** the management operation returns a typed minimum-valid-profile error with `minimum` equal to 1 and `remaining` equal to 0
- **AND** the profile remains persisted

#### Scenario: Malformed records do not satisfy the minimum
- **WHEN** one valid persisted profile and one or more malformed, unsupported, or structurally invalid profile records exist
- **AND** a caller deletes the valid profile with `RequireMinimumValid(1)`
- **THEN** deletion is rejected with `remaining` equal to 0
- **AND** invalid records are not counted as valid profiles

#### Scenario: Deleting an invalid target does not reduce the valid count
- **WHEN** one valid persisted profile and one malformed, unsupported, or structurally invalid target profile exist
- **AND** a caller deletes the invalid target with `RequireMinimumValid(1)`
- **THEN** deletion succeeds
- **AND** the valid profile remains persisted

#### Scenario: Deleting a missing target does not reduce the valid count
- **WHEN** the requested minimum is already satisfied and the target profile does not exist
- **AND** a caller deletes the missing target with `RequireMinimumValid(1)`
- **THEN** the operation succeeds as an idempotent deletion
- **AND** the valid persisted profile count is unchanged

#### Scenario: Requested postcondition is not already satisfied
- **WHEN** the computed remaining valid profile count is below the requested minimum even though the target is missing or invalid
- **THEN** the operation returns a typed minimum-valid-profile error containing the requested minimum and computed remaining count
- **AND** no profile record is deleted

#### Scenario: Zero minimum permits zero valid profiles
- **WHEN** a caller deletes a profile with `RequireMinimumValid(0)`
- **THEN** the minimum policy does not reject deletion solely because zero valid persisted profiles will remain

### Requirement: Policy-aware deletion SHALL preserve management deletion semantics
Policy-aware management deletion SHALL continue to protect the built-in default profile, preserve prompt-reference and integrity checks, persist before synchronizing an attached profile registry, and report registry synchronization failure as the existing typed partial operation. Minimum-policy rejection SHALL use a distinct `ManagementError` variant containing `minimum` and `remaining` values.

#### Scenario: Referenced profile also violates the minimum
- **WHEN** a profile is directly referenced by a stored prompt and deleting it would also violate the requested minimum
- **THEN** deletion returns the existing typed reference conflict
- **AND** neither the profile nor its registry entry is removed

#### Scenario: Prompt integrity cannot be proven
- **WHEN** malformed or unsupported prompt records prevent profile-reference verification during policy-aware deletion
- **THEN** deletion returns the existing typed integrity-unknown error
- **AND** minimum counting does not permit deletion to bypass prompt safety

#### Scenario: Minimum policy rejects before registry synchronization
- **WHEN** durable deletion is rejected because too few valid profiles would remain
- **THEN** the attached profile registry remains unchanged

#### Scenario: Registry synchronization fails after policy-aware deletion
- **WHEN** policy-aware durable deletion commits and attached registry removal fails
- **THEN** the service returns the existing typed partial-operation error identifying durable success

#### Scenario: Existing unrestricted management deletion remains compatible
- **WHEN** an existing caller invokes `ConfigManagementService::delete_profile`
- **THEN** deletion retains its current unrestricted minimum-count behavior
- **AND** existing default protection, prompt safety, and registry synchronization behavior remain in effect
