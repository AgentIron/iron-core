## MODIFIED Requirements

### Requirement: Core SHALL define typed agent identity profiles

The system SHALL provide public `AgentProfile`, `AgentProfileId`, `AgentProfileEntry`, a profile provider context enum, profile load report/diagnostic types, `ToolFilter`, `SkillFilter`, and `AgentApproval` types that can represent the user-facing name, provider selection, managed provider/model selection, tool availability policy, skill availability policy, agent approval policy, and optional `identity_prompt` for an agent identity profile.

#### Scenario: Profile captures provider and model selection
- **WHEN** a caller constructs an `AgentProfile`
- **THEN** the profile includes a profile provider context that can represent either the runtime default provider path or a managed provider slug and model
- **AND** the profile can also carry a user-facing name, tool filtering, skill filtering, approval policy, and optional `identity_prompt`

#### Scenario: Runtime default provider is representable
- **WHEN** a caller or the built-in default profile needs to use the runtime's injected/default provider path
- **THEN** the profile provider context can represent `RuntimeDefault` without requiring a provider slug

#### Scenario: User profile can use runtime default provider
- **WHEN** a user-defined profile needs custom identity, tool, skill, or approval policy while keeping the runtime's current provider path
- **THEN** the profile provider context can use `RuntimeDefault`
- **AND** the profile remains distinct from the protected built-in default profile by stable ID and user-facing name

#### Scenario: Managed provider is representable
- **WHEN** a caller configures a user profile for managed provider resolution
- **THEN** the profile provider context can represent provider slug and model
- **AND** does not include an API-key field

#### Scenario: Profile entry pairs ID with profile
- **WHEN** a caller lists or loads profiles
- **THEN** each successfully registered profile can be represented as an `AgentProfileEntry` containing the stable `AgentProfileId` and the full `AgentProfile`

#### Scenario: Profile context omits API keys
- **WHEN** a caller constructs or loads an `AgentProfile`
- **THEN** the profile API does not expose an API-key field
- **AND** the profile cannot persist per-profile credential secret material through its provider/model context

#### Scenario: Tool filter variants are representable
- **WHEN** a caller configures tool filtering for a profile
- **THEN** the system can represent inherited tools, an allowlist of canonical model-visible tool names, or a denylist of canonical model-visible tool names

#### Scenario: Skill filter variants are representable
- **WHEN** a caller configures skill filtering for a profile
- **THEN** the system can represent no skills, an allowlist of skill names, or inherited skills

#### Scenario: Agent approval variants are representable
- **WHEN** a caller configures approval policy for a profile
- **THEN** the user-facing profile schema can represent per-tool approval or auto-approval
- **AND** it rejects read-only approval as an unsupported profile approval value

### Requirement: Core SHALL expose profile-to-provider resolution

The system SHALL provide profile provider resolution that resolves the selected profile's provider context. The resolution result SHALL be able to represent both the runtime-owned default provider path and an owned managed provider constructed through credential resolution. Explicit managed profile provider/model references that are unavailable on the current machine SHALL produce structured diagnostics and fall back to the runtime default provider path during session setup when the runtime default is usable.

#### Scenario: Managed profile provider resolves successfully
- **WHEN** a profile references a known provider and model with usable credentials
- **THEN** `IronRuntime::resolve_profile_provider` returns a resolved managed provider constructed through the provider registry
- **AND** credential resolution uses iron-core credential state and OAuth refresh behavior without a profile-supplied API key
- **AND** the resolution diagnostics do not include a fallback warning

#### Scenario: Runtime default provider resolves through existing provider
- **WHEN** profile provider resolution is requested for a profile using the runtime default provider context
- **THEN** the runtime uses a resolved provider representation for its existing injected/default provider path
- **AND** does not require a managed provider slug or credential resolver for that profile
- **AND** does not emit an unavailable-reference warning

#### Scenario: Explicit unavailable provider falls back to runtime default
- **WHEN** profile provider resolution is requested for a profile using an explicit managed provider/model reference
- **AND** that explicit reference cannot produce a usable provider because the provider slug is unknown, the model is unknown, the provider is disabled, credentials are missing, the credential mode is unsupported, or provider construction fails
- **AND** the runtime default provider path is usable
- **THEN** resolution returns the runtime default provider path
- **AND** includes a structured warning diagnostic describing the unavailable explicit reference and fallback

#### Scenario: Explicit unavailable provider fails when fallback unavailable
- **WHEN** profile provider resolution is requested for a profile using an explicit managed provider/model reference
- **AND** that explicit reference cannot produce a usable provider
- **AND** the runtime default provider path is not usable
- **THEN** resolution fails with an actionable error
- **AND** the error or diagnostics preserve the explicit-reference failure reason

#### Scenario: Setup-time fallback does not mask inference failure
- **WHEN** provider/model resolution succeeds and inference has started
- **AND** the selected provider later fails during a request
- **THEN** the runtime surfaces the provider failure
- **AND** does not silently fall back to another provider/model mid-turn

### Requirement: Core SHALL add AutoApprove approval plumbing

The system SHALL expose an `AgentApproval::AutoApprove` variant so profile approval policy can be represented distinctly from per-tool approval and can be applied as a functional session-effective approval policy.

#### Scenario: AutoApprove is representable
- **WHEN** code maps or stores an auto-approval profile policy
- **THEN** the policy can be represented with `AgentApproval::AutoApprove`

#### Scenario: AutoApprove bypasses session approval prompts
- **WHEN** a session is prepared from a profile whose approval policy is `AutoApprove`
- **AND** a model-visible tool call would otherwise require approval under per-tool policy
- **THEN** the session approval evaluator treats the call as approved without prompting

#### Scenario: PerTool behavior remains stable
- **WHEN** a session is prepared from a profile whose approval policy is `PerTool`
- **THEN** runtime tool approval checks continue to defer to each tool's approval requirement

#### Scenario: ReadOnly approval is rejected
- **WHEN** a profile payload, import, or registration request uses read-only approval policy
- **THEN** the profile is rejected with a diagnostic or validation error
- **AND** the runtime does not silently map read-only approval to a weaker policy

## ADDED Requirements

### Requirement: Core SHALL provide shipped default profile definitions

The system SHALL provide core-owned shipped `AgentProfile` definitions for `explore`, `plan`, and `apply`. These definitions SHALL be bootstrap templates for ordinary persisted profile records and SHALL NOT introduce runtime modes, reserved profile IDs, immutable built-ins, or profile-name special-casing.

#### Scenario: Shipped default definitions are ordinary profile payloads
- **WHEN** a caller requests the shipped default profile definitions
- **THEN** the system returns profile IDs and `AgentProfile` payloads for `explore`, `plan`, and `apply`
- **AND** those payloads can be stored through normal ConfigStore profile APIs

#### Scenario: Shipped defaults use runtime-neutral configuration
- **WHEN** shipped default profile definitions are generated
- **THEN** each profile uses `AgentProfileProvider::RuntimeDefault`
- **AND** each profile uses `ToolFilter::Inherit`
- **AND** each profile uses `SkillFilter::Inherit`

#### Scenario: Shipped defaults are not runtime modes
- **WHEN** a session is prepared from a profile whose ID is `explore`, `plan`, or `apply`
- **THEN** runtime behavior is derived from the persisted profile fields
- **AND** the runtime does not branch on those IDs or names to add hidden behavior

### Requirement: Core SHALL seed shipped default profiles non-destructively

The system SHALL provide a core-owned seed operation that can create shipped default profile records in ConfigStore without overwriting existing records. Normal first-run seeding SHALL use durable seed state so user-deleted shipped profiles are not silently recreated on later startup.

#### Scenario: First-run seeding creates missing defaults
- **WHEN** the default profile seed operation runs with `FirstRunOnly` policy
- **AND** the durable seed marker is absent
- **AND** no shipped default profile records exist
- **THEN** the operation creates `explore`, `plan`, and `apply` as normal profile records
- **AND** writes the durable seed marker
- **AND** returns a report listing the created profile IDs

#### Scenario: First-run seeding preserves existing records
- **WHEN** the default profile seed operation runs with `FirstRunOnly` policy
- **AND** the durable seed marker is absent
- **AND** a profile record already exists for one of the shipped default IDs
- **THEN** the operation does not overwrite that record
- **AND** creates only missing shipped default records
- **AND** writes the durable seed marker
- **AND** returns a report listing skipped existing IDs

#### Scenario: First-run seeding does not recreate deleted defaults
- **WHEN** the default profile seed operation runs with `FirstRunOnly` policy
- **AND** the durable seed marker is present
- **AND** one or more shipped default profile records are missing
- **THEN** the operation does not recreate the missing records
- **AND** returns a report indicating that first-run seeding had already occurred

#### Scenario: Restore missing defaults is explicit and non-destructive
- **WHEN** the default profile seed operation runs with `RestoreMissing` policy
- **AND** one or more shipped default profile records are missing
- **THEN** the operation creates the missing shipped default records
- **AND** does not overwrite existing records for shipped default IDs
- **AND** returns a report listing created and skipped profile IDs

#### Scenario: Seeding failure is actionable
- **WHEN** the default profile seed operation cannot read seed state, write seed state, or store a profile record
- **THEN** the operation returns an actionable error
- **AND** does not report the failed write as a successful seed

### Requirement: Core SHALL snapshot selected profile policy into sessions

The system SHALL snapshot the selected profile's effective execution policy into the session during session setup. Later edits, deletion, restoration, or import of stored profile records SHALL affect future sessions only and SHALL NOT implicitly mutate existing sessions.

#### Scenario: Session setup snapshots profile policy
- **WHEN** a session is prepared with a selected profile
- **THEN** the session records the selected profile ID for traceability
- **AND** snapshots the profile identity prompt used for system prompt Section 1
- **AND** snapshots the profile tool filter
- **AND** snapshots the profile approval policy
- **AND** snapshots the provider/model resolution outcome or diagnostics needed for the session

#### Scenario: Stored profile edit does not mutate active session
- **WHEN** a session has already snapshotted profile policy
- **AND** the stored profile record is edited afterward
- **THEN** the existing session continues using its snapshotted policy
- **AND** new sessions can use the edited profile record

#### Scenario: Stored profile deletion does not mutate active session
- **WHEN** a session has already snapshotted profile policy
- **AND** the stored profile record is deleted afterward
- **THEN** the existing session continues using its snapshotted policy
- **AND** the deletion affects future profile selection only

#### Scenario: Profile switching is out of scope
- **WHEN** a caller wants to change profile policy for an active session
- **THEN** this change provides no implicit reload or mid-session switch behavior
- **AND** a future explicit profile-switch API must handle cache invalidation and policy recomputation deliberately

### Requirement: Core SHALL apply profile tool filters to primary session tool catalogs

The system SHALL apply the session-effective profile `ToolFilter` to the primary session's effective tool catalog by exact canonical model-visible tool name. Filtering SHALL affect both provider-visible tool definitions and execution-time tool availability.

#### Scenario: Inherit leaves catalog unchanged
- **WHEN** a session-effective profile tool filter is `Inherit`
- **THEN** the session tool catalog is not restricted by profile policy

#### Scenario: Allow exposes only listed tools
- **WHEN** a session-effective profile tool filter is `Allow` with a list of canonical tool names
- **THEN** only available tools whose canonical model-visible names are in the list are exposed to the provider
- **AND** attempts to execute tools outside the allowlist are rejected by the session tool catalog

#### Scenario: Deny removes listed tools
- **WHEN** a session-effective profile tool filter is `Deny` with a list of canonical tool names
- **THEN** available tools whose canonical model-visible names are in the list are not exposed to the provider
- **AND** attempts to execute denied tools are rejected by the session tool catalog

#### Scenario: Unknown listed tools are diagnostic not load-fatal
- **WHEN** a stored or imported profile contains tool filter names that are not available in the current runtime
- **THEN** the profile data remains valid if its shape is valid
- **AND** session preparation may return diagnostics for unavailable tool names

### Requirement: Core SHALL keep profile serialization reference-only

The system SHALL serialize and deserialize `AgentProfile` payloads as profile behavior and provider/model references only. Profile JSON SHALL NOT embed provider definitions, model catalog entries, credentials, API keys, OAuth tokens, provider connection metadata, or provider protocol metadata.

#### Scenario: Profile export contains references only
- **WHEN** an `AgentProfile` with a managed provider/model selection is serialized for storage or export
- **THEN** the serialized payload includes the provider slug and model ID reference
- **AND** does not include provider credentials, provider definitions, model metadata, or connection metadata

#### Scenario: Profile import accepts unavailable references
- **WHEN** a profile payload with a valid provider/model reference shape is imported or loaded
- **AND** that provider/model is not currently available on the machine
- **THEN** the profile payload can still be accepted
- **AND** execution-time session preparation is responsible for warning and fallback behavior
