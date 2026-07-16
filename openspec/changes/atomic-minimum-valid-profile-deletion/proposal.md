## Why

Consumers that must retain at least one usable profile currently count profiles before calling `ConfigManagementService::delete_profile`, but that check and the durable deletion occur in separate transactions. Two processes can therefore validate the same snapshot and delete different profiles, leaving no valid profile configuration.

## What Changes

- Add an opt-in profile deletion policy that can require a caller-selected minimum number of valid persisted profiles to remain after deletion.
- Keep the existing unrestricted `delete_profile` behavior and API compatible for callers that permit zero persisted profiles.
- Evaluate prompt-reference safety, profile validity, the target's contribution to the valid count, and deletion within one write-serialized SQLite transaction.
- Count only supported, decodable, structurally valid profile records; malformed and unsupported records do not satisfy the minimum.
- Return typed config-store and management errors containing the requested minimum and computed remaining valid count.
- Preserve post-commit profile-registry synchronization and its existing typed partial-operation behavior.
- Add store and management tests for malformed and unsupported records, missing and malformed targets, prompt conflicts, compatibility, and concurrent deletion from separate file-backed stores.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-profiles`: Add an opt-in minimum-valid-profile deletion policy, define which persisted profiles count as valid, expose a typed management failure, and retain unrestricted deletion compatibility.
- `core-config-store`: Require policy-aware profile validation and deletion to execute atomically under a write reservation shared across SQLite connections and processes.

## Impact

- Extends the public profile-management API with `ProfileDeletePolicy` and `delete_profile_with_policy` while retaining `delete_profile`.
- Extends `ConfigStore`'s checked profile deletion path and adds a typed `ConfigError::MinimumValidProfiles` mapped to a corresponding `ManagementError` variant.
- Reuses the profile schema, decoding, and structural validation rules already used by management profile loading; no database migration is required.
- Changes SQLite transaction acquisition for checked profile deletion so the writer lock is reserved before reading prompt or profile state.
- Builds on the completed `add-typed-management-apis` change and does not alter typed dependency-impact DTOs or registry synchronization ordering.
