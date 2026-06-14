## Context

`ConfigStore` already provides opaque profile records with stable IDs and versioned JSON payloads, but `iron-core` does not yet define the typed profile payload or expose runtime APIs for registering profiles. These records are the natural durable handles for agent identities: the record ID is the stable generated profile ID, while the typed payload describes the user-facing name, provider/model preference, behavioral boundaries, approval posture, and profile-specific `identity_prompt` text.

`IronRuntime` already owns optional managed-provider credential resolution through `CredentialResolver` and exposes `resolve_managed_provider(&ProviderPromptContext)`, so profile-to-provider resolution can reuse the existing provider-safe credential path rather than creating a second provider-construction flow. Profiles should not expose API keys or other credential secret material; they select a provider slug and model, and iron-core resolves auth from its broader credential state. In identity terms, provider resolution is one step in preparing an agent identity for execution; execution, delegation, and enforcement are still later work. The long-term model is that every agent execution uses a profile, falling back to the built-in protected `default` profile whenever a caller does not select one explicitly.

Issues 59 and 60 are implemented together because provider resolution depends on the `AgentProfile` data model and the same tests need typed profile payloads, profile registration, and managed credential resolution.

## Goals / Non-Goals

**Goals:**

- Define `AgentProfile`, `AgentProfileId`, `AgentProfileEntry`, a profile provider context enum, profile load report/diagnostic types, and related filter/`AgentApproval` enums as public, serializable Rust types for durable agent identities.
- Treat ConfigStore profile record IDs as stable generated profile IDs, separate from user-facing profile names.
- Defer profile ID generation helpers while validating caller-supplied IDs.
- Add profile identity registration APIs on `IronAgent` with unique-name validation, deterministic list behavior, and explicit unregister semantics.
- Load typed profiles from ConfigStore profile records while preserving the existing opaque storage contract and reporting per-profile diagnostics.
- Establish that unspecified profile selection resolves to the built-in protected `default` profile rather than a separate non-profile execution path.
- Add profile-to-provider resolution on `IronRuntime` by delegating to the existing credential resolver and provider registry flow.
- Add plumbing for an `ApprovalStrategy::AutoApprove` variant without changing runtime tool-approval behavior beyond making the policy representable.

**Non-Goals:**

- Executing sub-agents or implementing `delegate_task` behavior.
- Enforcing `ToolFilter` or `SkillFilter` during prompt execution.
- Defining identity rename, aliasing, ownership, sharing, or migration semantics beyond stable profile IDs.
- Persisting or exposing per-profile API keys or credential secret material.
- Adding profile-scoped memory, history, or session affinity.
- Defining stored-prompt, schedule, or CLI profile-selection behavior.
- Migrating existing profile data beyond decoding the new schema version from ConfigStore records.

## Decisions

### Keep profile storage opaque in ConfigStore

ConfigStore continues to store profile records as versioned JSON payloads. `IronAgent::load_profiles` is responsible for reading profile IDs, fetching records, validating `schema_version`, deserializing `AgentProfile`, and registering successfully decoded profiles.

Alternative considered: replacing ConfigStore profile records with typed SQL columns. This would prematurely specialize IC-1's generic profile storage and make later profile schema revisions more expensive.

### Use a profile-specific provider context enum

`AgentProfile` should not embed `ProviderPromptContext` because that type includes prompt-time API-key override state. Instead, this change should introduce a profile-specific provider context enum with a `RuntimeDefault` variant and a managed provider/model variant. `RuntimeDefault` preserves compatibility with existing injected-provider construction paths, while managed profiles carry provider slug and model only. Managed provider resolution can derive a `ProviderPromptContext` internally with no API key override so credentials resolve from iron-core credential state.

Suggested shape:

```rust
pub enum AgentProfileProvider {
    RuntimeDefault,
    Managed {
        provider_slug: ProviderSlug,
        model: String,
    },
}
```

Alternative considered: reusing `ProviderPromptContext` and requiring `api_key: None` for profiles. That preserves the issue's original shape but leaks an auth override field into a durable identity API and makes accidental secret persistence possible.

`RuntimeDefault` should not be restricted to the built-in default profile. User profiles may use `RuntimeDefault` when they want different identity/tool/skill/approval policy while continuing to use the runtime's current injected/default provider path.

### Separate stable profile IDs from user-facing names

The ConfigStore profile record ID is the stable generated profile ID for this slice. The typed `AgentProfile` payload includes a user-facing `name` that can be shown in UI and changed later without changing references. Registration and loading bind the payload to the supplied ID. Future stored prompts, delegate-task calls, and CLI selections can reference the stable profile ID without depending on display names or serialized payload internals. Public APIs should use an `AgentProfileId` newtype rather than a plain `String` to avoid confusing stable IDs with user-facing names.

Alternative considered: using the user-provided name as the stable identity handle. That is simpler but makes rename semantics difficult and forces display naming choices to become durable API references.

### Defer profile ID generation helpers

This slice should validate caller-supplied profile IDs rather than defining a durable creation API or generated ID format. Non-default profile IDs should be non-empty after trimming, contain no control characters, and not equal `default` using ASCII case-insensitive comparison. A future profile-management API can add helpers such as `AgentProfileId::new()` or `ConfigStore::create_profile(profile) -> AgentProfileId` once creation, import/export, and rename semantics are designed.

Alternative considered: requiring a generated format such as `prof_<ulid>` now. That would be reasonable long-term, but it expands this proposal into profile creation semantics that are not needed for registration, loading, or provider resolution.

### Return explicit profile entries and load diagnostics

`list_profiles` should return `Vec<AgentProfileEntry>` where each entry includes the stable profile ID and full `AgentProfile`. Profiles contain no credential secret material, so returning full profile values is acceptable. `load_profiles` should return `ProfileLoadReport` with loaded entries and skipped diagnostics for ConfigStore records only. The built-in default profile appears in `list_profiles` but should not appear in a `load_profiles` loaded list unless it came from ConfigStore, which reserved-default validation prevents. Diagnostics should include the profile ID, the parsed name when available, and an issue enum such as unsupported schema version, invalid payload, invalid name, reserved default, duplicate name, or missing listed record.

Alternative considered: returning tuples, maps, or only profile summaries. Explicit entry/report types are clearer public API, leave room for metadata later, and avoid leaking internal registry representation.

### Store registered profiles on IronAgent

`IronAgent` should expose a thread-safe identity profile registry keyed by stable profile ID, initialized by every constructor. Registration APIs can be synchronous because they only mutate in-memory state. `load_profiles` remains async because ConfigStore access is async. The registry should also reject duplicate user-facing names within the loaded/registered profile set so users can safely select profiles by name when a frontend or CLI chooses to support that path. For this slice, the registry lives behind the facade while ConfigStore remains the durable source of truth; runtime execution APIs can later decide how selected identities flow into sessions, delegation, and policy enforcement.

Alternative considered: storing profiles directly in `IronRuntime`. That may become appropriate once runtime execution selects and enforces identities, but doing it now would expand runtime responsibilities before delegate-task behavior, prompt selection, and policy enforcement are designed.

### Validate profile names minimally

Profile names are user-facing labels, so this change should avoid imposing slug-style syntax. Names should be accepted as arbitrary user text after rejecting empty or all-whitespace names and control characters. Duplicate detection should use the stored name exactly after trimming leading and trailing whitespace, except that `default` is a case-insensitive protected name reserved for the built-in profile. Richer case-insensitive or locale-aware uniqueness for non-reserved names can be added by frontends or a later profile-management change if needed.

Alternative considered: requiring slug-safe names. That would simplify CLI lookup but would make display names less natural and is unnecessary because stable profile IDs already provide machine-safe references.

### Normalize profile names by trimming

Registration and loading should trim leading and trailing whitespace before storing the profile name. If the trimmed name is empty or contains control characters, the profile is invalid. Duplicate-name checks use the trimmed stored name exactly. The core should not perform case folding or Unicode normalization in this slice.

Alternative considered: rejecting names that require trimming. That is stricter but unnecessarily hostile for user-facing labels, especially because stable profile IDs carry the machine reference semantics.

### Protect the built-in default profile

The registry should always include a built-in profile with ID `default` and name `default`. It uses the current default provider/model, `ToolFilter::Inherit`, `SkillFilter::Inherit`, `AgentApproval::PerTool`, and a short generic identity prompt such as `You are a helpful software engineering agent.` The `default` ID and name are protected case-insensitively, so user registration and ConfigStore loading cannot replace, shadow, or duplicate the built-in profile with `default`, `Default`, or any other ASCII case variant.

The built-in default profile should use `AgentProfileProvider::RuntimeDefault` rather than requiring `Config.provider_name` to be set. This keeps existing `IronRuntime::new(config, provider)` and `IronAgent::new(config, provider)` injected-provider construction paths usable while still ensuring execution has a selected profile.

Alternative considered: storing the default profile as a normal ConfigStore profile. That would make user customization simpler later, but it risks losing the fallback invariant in this slice and introduces default-profile creation/migration questions that are not needed yet.

### Use schema version 1 for typed AgentProfile payloads

`load_profiles` should accept the first typed profile schema version and report unsupported versions, invalid payloads, invalid names, reserved default conflicts, and duplicate names as per-profile diagnostics. The stable profile ID comes from the ConfigStore record ID, not from duplicated payload state. Valid profiles should still load when other records are invalid so one misconfigured profile does not disable the full profile set. To make duplicate-name behavior deterministic, records should be processed by stable profile ID in ascending order, and the first valid trimmed name wins after existing registry entries, including the built-in default profile, have already reserved their names.

Alternative considered: failing the full load on the first invalid profile. That is simpler but creates poor desktop and CLI behavior because one stale or hand-edited record would hide every other valid profile.

### Treat load_profiles as additive merge

`load_profiles` should merge ConfigStore profiles into the existing in-memory registry. Loaded records replace existing profiles with the same non-reserved profile ID when validation succeeds, but loading does not remove in-memory profiles that are absent from ConfigStore. Existing registry entries win duplicate-name conflicts against newly loaded records, and rejected replacements leave the previous profile unchanged.

Each profile record should validate fully before mutating the registry. If a replacement record has an invalid ID, invalid name, reserved default conflict, duplicate name owned by another profile ID, unsupported schema, or invalid payload, the existing profile for that ID remains unchanged and the load report records a diagnostic.

Alternative considered: making `load_profiles` synchronize the registry exactly to ConfigStore. That would make deletion propagation easier but could unexpectedly remove manually registered profiles or the built-in default profile.

### Use profiles for all agent execution

Profile selection should always produce an `AgentProfile`. When a caller does not explicitly request a profile, the system should use the built-in `default` profile instead of branching through legacy non-profile configuration. This keeps primary agents and sub-agents on the same conceptual path: select profile, resolve provider, apply identity/tool/skill/approval policy, execute.

Alternative considered: treating profiles as optional overrides on top of existing runtime behavior. That would preserve compatibility in the short term but would create special cases that future delegation and CLI flows would need to duplicate.

### Use identity_prompt as the profile identity layer

`identity_prompt` is the profile's model-facing identity instruction layer, not an append-only extension of the default profile's prompt. Custom profiles provide their own identity prompt. If a custom profile leaves `identity_prompt` absent or blank after trimming, profile selection should fall back to the default profile's generic identity prompt for execution so every selected profile has a usable identity layer.

Alternative considered: always appending custom prompts to the default identity prompt. That would make custom identities harder to reason about and would force the default persona into specialized profiles even when the author wants a different identity.

### Reuse existing provider resolution

`IronRuntime::resolve_profile_provider(&AgentProfile)` should branch on the profile provider context. `RuntimeDefault` uses the runtime's existing injected/default provider path. `Managed` builds a managed provider prompt context from provider slug and model, with no profile-supplied API key, then calls the existing managed-provider resolution path. This preserves current injected-provider compatibility, OAuth refresh behavior, provider-safe credential material, and provider registry support checks while keeping provider construction independent from identity registry ownership.

Because the runtime default provider is already owned by the runtime and managed providers are constructed on demand, this API should not be constrained to `Result<Box<dyn Provider>>` for every profile. It should return or otherwise expose a resolved provider abstraction capable of representing both the runtime-owned provider reference/handle and an owned managed provider. One possible shape is an enum with a runtime-default variant wrapping a cloned `Arc<dyn Provider>` and a managed variant wrapping `Box<dyn Provider>`.

Alternative considered: exposing `CredentialResolver` directly to callers for profile resolution. That would require callers to duplicate provider registry construction and error mapping already owned by `IronRuntime`.

### Treat AutoApprove as representational plumbing

`AgentApproval` should represent profile-level approval policy with `PerTool`, `AutoApprove`, and `ReadOnly` variants. `ApprovalStrategy::AutoApprove` should become a distinct public variant for profile policy plumbing, but this change should not make existing tool execution skip approvals in new places. Runtime approval behavior for delegated execution is deferred to delegate-task work.

Alternative considered: keeping the issue's original `SubAgentApproval` name. Because all agents are profile-backed, `AgentApproval` better describes both primary-agent and delegated-agent profiles.

## Risks / Trade-offs

- Profile payload schema may need revision once delegate-task execution, profile-scoped prompts, or profile-scoped memory are implemented. Mitigation: use explicit schema versions and keep ConfigStore storage opaque.
- Treating profile IDs as durable identity handles makes creation and import/export semantics important later. Mitigation: use ConfigStore record IDs as stable generated IDs now and keep user-facing names separate.
- Duplicate profile names can occur in existing or hand-edited stores. Mitigation: load valid non-duplicate profiles and return diagnostics for skipped duplicates.
- Exact name matching may allow visually similar names that differ by case or Unicode normalization. Mitigation: keep core validation minimal for now and let frontends apply stricter UX rules if needed.
- Default-profile customization may require a future decision about whether the built-in default can be replaced by a persisted user-selected default. Mitigation: protect the built-in `default` profile for this slice and defer customization.
- A profile-specific provider context diverges from the original issue wording. Mitigation: it preserves the intended provider/model selection while avoiding an API-key field in durable identity data and retaining runtime-default injected-provider compatibility.
- Deferring ID generation means callers can choose arbitrary valid IDs. Mitigation: validate IDs now and leave generated formats to future profile-management APIs.
- Keeping profile registration on `IronAgent` means `IronRuntime` cannot list identities directly. Mitigation: IC-3 only requires resolving a supplied profile; future execution APIs can move or share the registry if runtime ownership becomes necessary.
- `ToolFilter` and `SkillFilter` are representational before enforcement exists. Mitigation: document enforcement as out of scope and add tests only for serialization/registration until execution work lands.
- `AutoApprove` can be confused with existing no-approval behavior. Mitigation: add the variant without broad behavior changes and test existing approval strategy semantics remain stable.
