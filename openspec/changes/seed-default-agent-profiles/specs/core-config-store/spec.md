## ADDED Requirements

### Requirement: Config store SHALL support durable bootstrap metadata

The system SHALL provide a durable, domain-scoped metadata mechanism that core-owned bootstrap operations can use to record one-time initialization state without creating fake records in user-facing domains such as profiles, prompts, schedules, credentials, providers, or models.

#### Scenario: Bootstrap metadata roundtrips
- **WHEN** a caller stores a bootstrap metadata value for a domain-scoped key
- **THEN** a later read for the same key returns the stored value
- **AND** the caller does not need direct access to SQLite tables or SQL queries

#### Scenario: Missing bootstrap metadata is distinguishable
- **WHEN** a caller reads a bootstrap metadata key that has not been stored
- **THEN** the config API returns `None` or an equivalent typed absence value
- **AND** the absence is distinguishable from storage failure

#### Scenario: Bootstrap metadata does not pollute profile records
- **WHEN** default agent profile seeding stores its first-run seed marker
- **THEN** the marker is stored outside the user-visible profile record namespace
- **AND** listing, exporting, deleting, or loading profiles does not expose the marker as an `AgentProfile`

#### Scenario: Bootstrap metadata writes are transactional
- **WHEN** a bootstrap operation writes metadata as part of a larger initialization operation
- **THEN** storage failures are reported as actionable config errors
- **AND** the operation does not report successful initialization if required metadata could not be persisted

### Requirement: Config store SHALL preserve default-profile seed state

The system SHALL support durable seed state for shipped default agent profiles so callers can distinguish first-run bootstrap from later user deletion of seeded profile records.

#### Scenario: First-run seed state starts absent
- **WHEN** a config store has never completed shipped default profile seeding
- **THEN** the default-profile seed marker is absent

#### Scenario: First-run seed state can be recorded
- **WHEN** shipped default profile seeding completes its normal first-run attempt
- **THEN** the config store records the default-profile seed marker with a versioned value

#### Scenario: Seed state survives deleted profile records
- **WHEN** shipped default profile seeding has recorded its seed marker
- **AND** a user deletes a shipped default profile record such as `explore`
- **THEN** the seed marker remains present
- **AND** later first-run seeding can detect that automatic bootstrap has already occurred

#### Scenario: In-memory store supports seed state
- **WHEN** tests or embedders create an in-memory config store
- **THEN** the same bootstrap metadata APIs and default-profile seed state behavior are available
