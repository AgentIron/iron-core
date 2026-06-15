# agent-profiles Specification

## Purpose
TBD - created by archiving change agent-profiles-provider-resolution. Update Purpose after archive.
## Requirements
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
- **THEN** the system can represent inherited tools, an allowlist of tool names, or a denylist of tool names

#### Scenario: Skill filter variants are representable
- **WHEN** a caller configures skill filtering for a profile
- **THEN** the system can represent no skills, an allowlist of skill names, or inherited skills

#### Scenario: Agent approval variants are representable
- **WHEN** a caller configures approval policy for a profile
- **THEN** the system can represent per-tool approval, auto-approval, or read-only execution policy

### Requirement: Core SHALL separate profile IDs from user-facing names

The system SHALL use ConfigStore profile record IDs and in-memory registration IDs as stable generated profile IDs for agent identities. The typed `AgentProfile` payload SHALL include a user-facing profile name that is distinct from the stable profile ID.

#### Scenario: ConfigStore ID is stable profile ID
- **WHEN** `IronAgent::load_profiles` loads a profile record from ConfigStore
- **THEN** the profile record ID is used as the registered stable profile ID
- **AND** the `AgentProfile` payload provides the user-facing profile name

#### Scenario: Registered ID names in-memory identity
- **WHEN** a caller registers an `AgentProfile` directly with `IronAgent`
- **THEN** the supplied registration ID is used as the stable in-memory profile ID
- **AND** the profile payload's name remains the user-facing label

#### Scenario: Profile ID is validated
- **WHEN** a caller registers or loads a non-default profile ID
- **THEN** the profile ID must be non-empty after trimming, contain no control characters, and not equal `default` using ASCII case-insensitive comparison
- **AND** invalid profile IDs are rejected or reported without mutating existing profiles

#### Scenario: Profile ID generation is caller-owned for this slice
- **WHEN** a caller creates a new durable profile record
- **THEN** this change does not require a core-generated ID format or creation helper
- **AND** the caller supplies a valid non-reserved profile ID

#### Scenario: Profile names are unique labels
- **WHEN** a caller registers or loads a profile
- **THEN** the profile name must not duplicate another registered profile name after trimming leading and trailing whitespace
- **AND** duplicate names are rejected or reported without changing the stable profile IDs of existing profiles

#### Scenario: Default name is protected
- **WHEN** a caller registers or loads a user profile with profile ID or profile name equal to `default` using ASCII case-insensitive comparison
- **THEN** the profile is rejected or reported as reserved
- **AND** the built-in default profile remains unchanged

#### Scenario: Profile name is normalized
- **WHEN** a caller registers or loads a profile with leading or trailing whitespace in the profile name
- **THEN** the stored profile name is the trimmed name
- **AND** duplicate-name checks use the trimmed stored name exactly

#### Scenario: Invalid profile name is rejected
- **WHEN** a caller registers or loads a profile with an empty, all-whitespace, or control-character-containing name
- **THEN** the profile is rejected or reported as invalid
- **AND** the invalid name is not registered as a selectable profile label

#### Scenario: Identity rename semantics are deferred
- **WHEN** a caller needs to rename, alias, share, or migrate an agent identity handle
- **THEN** this change does not define those semantics
- **AND** callers must create, replace, or delete profile records through existing ConfigStore/profile APIs until a future identity-management change defines richer behavior

### Requirement: Core SHALL keep profile auth in credential state

The system SHALL NOT persist or expose per-profile API keys or credential secret material as part of `AgentProfile`. Provider resolution for a profile SHALL use the profile's provider slug and model together with iron-core's existing credential state.

#### Scenario: Profile resolves provider without profile API key
- **WHEN** `IronRuntime::resolve_profile_provider` resolves a profile provider
- **THEN** managed profile resolution derives a managed provider context from the profile provider slug and model without a profile-supplied API key
- **AND** credentials for managed profiles are resolved from iron-core's credential resolver and credential stores

#### Scenario: Prompt-time API key remains separate
- **WHEN** prompt APIs support an app-supplied API key override
- **THEN** that prompt-time override remains separate from durable profile identity APIs
- **AND** it is not stored in `AgentProfile`

### Requirement: Core SHALL expose in-memory agent identity registration

The system SHALL provide `IronAgent` APIs to register, unregister, and list `AgentProfile` identity values by stable profile ID for the lifetime of the agent instance.

#### Scenario: Default profile is always registered
- **WHEN** an `IronAgent` is constructed
- **THEN** its profile registry includes the built-in profile with ID `default` and name `default`
- **AND** list operations include that built-in profile entry

#### Scenario: Profile is registered
- **WHEN** a caller registers a profile with a valid stable profile ID and unique profile name
- **THEN** `IronAgent` stores that profile under the supplied profile ID
- **AND** a later list operation includes an `AgentProfileEntry` with that profile ID, profile name, and profile value

#### Scenario: Profile is replaced
- **WHEN** a caller registers a profile ID that is already registered
- **THEN** the new profile replaces the previous profile for that ID if its name remains unique within the registry

#### Scenario: Invalid replacement leaves existing profile unchanged
- **WHEN** a caller attempts to replace an existing profile ID with an invalid profile
- **THEN** the replacement is rejected
- **AND** the previous profile for that ID remains registered unchanged

#### Scenario: Replacement cannot duplicate another profile name
- **WHEN** a caller replaces a registered profile with a profile name already used by a different profile ID
- **THEN** the replacement is rejected
- **AND** the previously registered profile for the replaced ID remains unchanged

#### Scenario: Default profile cannot be replaced
- **WHEN** a caller registers a profile using the protected `default` profile ID or name with any ASCII case variant
- **THEN** the registration is rejected
- **AND** the built-in default profile remains unchanged

#### Scenario: Profile is unregistered
- **WHEN** a caller unregisters a registered profile ID
- **THEN** `IronAgent` removes that profile from the in-memory registry
- **AND** a later list operation no longer includes that profile

#### Scenario: Unregistering a missing profile is explicit
- **WHEN** a caller unregisters a profile ID that is not registered
- **THEN** the API reports that no profile was removed without mutating other profiles

#### Scenario: Unregistering frees profile name
- **WHEN** a caller unregisters a profile ID whose profile name is registered
- **THEN** a later registration using that same profile name for another profile ID can succeed if no other registered profile uses the name

### Requirement: Core SHALL load typed agent identities from ConfigStore

The system SHALL provide `IronAgent::load_profiles` to read ConfigStore profile records, decode supported typed `AgentProfile` payloads, register valid agent identities by ConfigStore record ID, and return a `ProfileLoadReport` with loaded profile entries and per-profile diagnostics for invalid, reserved, or duplicate records.

#### Scenario: Stored profile loads successfully
- **WHEN** ConfigStore contains a profile record with a supported typed profile schema version and valid profile payload
- **THEN** `IronAgent::load_profiles` deserializes the payload into an `AgentProfile`
- **AND** registers it under the profile record ID as the stable profile ID

#### Scenario: Invalid stored profile is skipped
- **WHEN** ConfigStore contains one valid profile record and one invalid profile record
- **THEN** `IronAgent::load_profiles` registers the valid profile
- **AND** reports a diagnostic for the invalid profile
- **AND** does not fail the entire load because of the invalid profile payload

#### Scenario: Load report includes loaded and skipped profiles
- **WHEN** `IronAgent::load_profiles` completes with valid and skipped profile records
- **THEN** the returned load report includes loaded `AgentProfileEntry` values for registered profiles
- **AND** includes diagnostics for skipped profile records with their profile ID, parsed name when available, and issue category

#### Scenario: Load report excludes built-in default
- **WHEN** `IronAgent::load_profiles` returns a load report
- **THEN** the report describes ConfigStore profile records that were loaded or skipped
- **AND** does not include the built-in default profile as a loaded ConfigStore profile

#### Scenario: Unsupported profile schema is skipped
- **WHEN** ConfigStore contains a profile record with an unsupported profile schema version
- **THEN** `IronAgent::load_profiles` reports an actionable diagnostic identifying the unsupported profile record
- **AND** does not silently register that invalid profile

#### Scenario: Invalid profile payload is skipped
- **WHEN** ConfigStore contains a profile record whose payload cannot be decoded as an `AgentProfile`
- **THEN** `IronAgent::load_profiles` reports an actionable diagnostic identifying the invalid profile record
- **AND** does not silently register that invalid profile

#### Scenario: Duplicate loaded profile name is skipped
- **WHEN** ConfigStore contains multiple valid profile records with duplicate user-facing names
- **THEN** `IronAgent::load_profiles` processes records by stable profile ID in ascending order
- **AND** registers the first valid profile for that trimmed name
- **AND** reports diagnostics for skipped duplicate profiles

#### Scenario: Existing registry entry wins duplicate loaded name
- **WHEN** the in-memory registry already contains a profile name
- **AND** ConfigStore contains a different profile ID with the same trimmed profile name
- **THEN** `IronAgent::load_profiles` keeps the existing registry entry
- **AND** reports a duplicate-name diagnostic for the loaded profile

#### Scenario: Reserved default loaded profile is skipped
- **WHEN** ConfigStore contains a profile record with profile ID or profile name equal to `default` using ASCII case-insensitive comparison
- **THEN** `IronAgent::load_profiles` skips that stored profile
- **AND** reports a reserved-default diagnostic
- **AND** the built-in default profile remains unchanged

#### Scenario: Profile loading is additive
- **WHEN** `IronAgent::load_profiles` loads profiles from ConfigStore
- **THEN** valid loaded profiles are merged into the existing in-memory registry
- **AND** existing in-memory profiles absent from ConfigStore remain registered

#### Scenario: Invalid loaded replacement leaves existing profile unchanged
- **WHEN** the in-memory registry contains a profile ID
- **AND** ConfigStore contains a record for the same profile ID that fails validation
- **THEN** `IronAgent::load_profiles` reports a diagnostic for the stored record
- **AND** the existing in-memory profile remains registered unchanged

#### Scenario: Fatal store failure aborts profile loading
- **WHEN** ConfigStore cannot list or read profile records because of a storage failure
- **THEN** `IronAgent::load_profiles` returns a fatal error for the store operation
- **AND** does not report the failure as a per-profile payload diagnostic

### Requirement: Core SHALL expose profile-to-provider resolution

The system SHALL provide profile provider resolution that resolves the selected profile's provider context. The resolution result SHALL be able to represent both the runtime-owned default provider path and an owned managed provider constructed through credential resolution.

#### Scenario: Managed profile provider resolves successfully
- **WHEN** a profile references a known provider and model with usable credentials
- **THEN** `IronRuntime::resolve_profile_provider` returns a resolved managed provider constructed through the provider registry
- **AND** credential resolution uses iron-core credential state and OAuth refresh behavior without a profile-supplied API key

#### Scenario: Runtime default provider resolves through existing provider
- **WHEN** profile provider resolution is requested for a profile using the runtime default provider context
- **THEN** the runtime uses a resolved provider representation for its existing injected/default provider path
- **AND** does not require a managed provider slug or credential resolver for that profile

#### Scenario: Resolver is not configured
- **WHEN** profile provider resolution is requested on a runtime without a configured credential resolver
- **THEN** the call returns a typed provider-auth error instead of panicking

#### Scenario: Provider resolution fails actionably
- **WHEN** a profile references an unknown provider, missing credential, unsupported credential mode, expired credential, or revoked OAuth credential
- **THEN** `IronRuntime::resolve_profile_provider` returns an actionable provider-auth error from the existing credential/provider resolution surface

### Requirement: Core SHALL support default profile selection

The system SHALL model all agent execution as profile-backed execution. When a caller does not explicitly specify a profile, profile selection SHALL resolve to the built-in protected `default` `AgentProfile` rather than bypassing profile semantics.

#### Scenario: Built-in default profile exists
- **WHEN** the profile registry is initialized
- **THEN** it includes a built-in profile with ID `default` and name `default`
- **AND** that profile uses the runtime default provider context, inherited tools, inherited skills, per-tool approval, and a short generic identity prompt

#### Scenario: Explicit profile is selected
- **WHEN** a caller requests execution with a specific profile ID
- **THEN** the system uses the selected `AgentProfile` for provider/model selection and future identity policy application

#### Scenario: Default profile is selected
- **WHEN** a caller requests execution without a specific profile ID
- **THEN** the system resolves execution to the built-in `default` `AgentProfile`
- **AND** provider/model selection still flows through profile-to-provider resolution

#### Scenario: Non-profile execution path is avoided
- **WHEN** primary-agent or sub-agent execution is prepared
- **THEN** the system has an `AgentProfile` available before provider resolution
- **AND** execution does not require a separate special-case path for missing profile configuration

### Requirement: Core SHALL use identity prompts as profile identity layers

The system SHALL treat `identity_prompt` as the selected profile's model-facing identity instruction layer. A custom profile's non-blank `identity_prompt` SHALL replace the default profile's identity prompt for that selected profile rather than appending to it, and the selected profile identity SHALL be supplied to system prompt composition as identity content rather than as client/session injection content.

#### Scenario: Custom identity prompt is used
- **WHEN** a selected non-default profile has a non-blank `identity_prompt`
- **THEN** that prompt is the profile identity prompt used for execution preparation
- **AND** the default profile's identity prompt is not implicitly appended

#### Scenario: Blank identity prompt falls back to default
- **WHEN** a selected non-default profile has no `identity_prompt` or has an identity prompt that is blank after trimming
- **THEN** execution preparation uses the built-in default profile's identity prompt as the fallback identity prompt

#### Scenario: Default identity prompt is generic
- **WHEN** the built-in default profile is created
- **THEN** it has a short generic identity prompt suitable for general software engineering assistance

#### Scenario: Selected profile identity is distinct from session instructions
- **WHEN** execution preparation supplies the selected profile identity to provider request construction
- **THEN** the profile identity is available to the system prompt renderer as profile identity content
- **AND** it is not treated as client-owned session instruction content solely because it came from the selected profile

### Requirement: Core SHALL add AutoApprove approval plumbing

The system SHALL expose an `AgentApproval::AutoApprove` variant so profile approval policy can be represented distinctly from existing profile approval policies.

#### Scenario: AutoApprove is representable
- **WHEN** code maps or stores an auto-approval profile policy
- **THEN** the policy can be represented with `AgentApproval::AutoApprove`

#### Scenario: Existing approval behavior remains stable
- **WHEN** runtime tool approval checks use existing `AgentApproval::PerTool` or `AgentApproval::ReadOnly` policies
- **THEN** their approval decisions remain unchanged by the addition of `AutoApprove`
