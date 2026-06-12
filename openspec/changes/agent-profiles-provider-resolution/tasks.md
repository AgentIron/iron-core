## 1. Profile Domain Types

- [x] 1.1 Add public `AgentProfile`, `AgentProfileId`, `AgentProfileEntry`, profile provider context enum, `ToolFilter`, `SkillFilter`, and `AgentApproval` types with serde support and derives consistent with adjacent public config/domain types, framed as agent identity profile types.
- [x] 1.2 Add profile schema-version constants or validation helpers for ConfigStore typed profile payload decoding.
- [x] 1.3 Add `ApprovalStrategy::AutoApprove` and update approval-strategy matches without changing existing `Always`, `Never`, or `PerTool` behavior.
- [x] 1.4 Re-export the new profile types from the appropriate public module boundary.
- [x] 1.5 Document that profile record IDs are stable generated profile IDs while `AgentProfile.name` is the user-facing unique label.
- [x] 1.6 Ensure profile provider/model context does not expose or serialize API keys or credential secret material.
- [x] 1.7 Add `identity_prompt: Option<String>` to `AgentProfile` for profile-specific model-facing identity instructions, with blank values falling back to the default profile identity prompt during profile selection/execution preparation.
- [x] 1.8 Add `ProfileLoadReport`, `ProfileLoadDiagnostic`, and profile load issue types for best-effort loading feedback.
- [x] 1.9 Validate non-default `AgentProfileId` values as caller-supplied IDs with no generation helper in this slice.
- [x] 1.10 Add a resolved profile provider abstraction that can represent both the runtime default provider path and an owned managed provider.

## 2. IronAgent Identity Profile Registry

- [x] 2.1 Add an in-memory identity profile registry to `IronAgent` initialized by every constructor.
- [x] 2.2 Implement `IronAgent::register_profile(id, profile)` with replacement semantics for existing stable profile IDs.
- [x] 2.3 Validate user-facing profile names by trimming leading/trailing whitespace before storage, rejecting empty or control-character-containing names, and enforcing uniqueness on the trimmed stored name.
- [x] 2.4 Implement `IronAgent::unregister_profile(id)` with explicit missing-profile reporting.
- [x] 2.5 Implement `IronAgent::list_profiles()` with deterministic ordering and entries that include stable profile ID, user-facing name, and profile value.
- [x] 2.6 Ensure replacing an existing profile ID rejects duplicate names owned by other profile IDs and leaves the previous profile unchanged on rejection.
- [x] 2.7 Add the built-in protected `default` profile to every registry, reject user registration of profile ID/name `default` using ASCII case-insensitive comparison, and ensure it cannot be unregistered or replaced.
- [x] 2.8 Ensure registration validates the full replacement profile before mutating the registry.

## 3. ConfigStore Profile Loading

- [x] 3.1 Implement `IronAgent::load_profiles(store: &ConfigStore)` using ConfigStore profile ID listing and record retrieval.
- [x] 3.2 Decode supported schema-version 1 profile payloads into `AgentProfile` values and register them by record ID as stable profile IDs.
- [x] 3.3 Return fatal errors for ConfigStore list/read failures while reporting unsupported schema versions, missing listed records, invalid profile payloads, reserved default conflicts, and duplicate names as per-profile load diagnostics.
- [x] 3.4 Process stored profile records by stable profile ID in ascending order so duplicate-name winners are deterministic.
- [x] 3.5 Implement additive merge semantics so loaded profiles replace matching non-reserved IDs, existing registry entries win duplicate-name conflicts, and profiles absent from ConfigStore remain registered.
- [x] 3.6 Ensure invalid loaded replacements leave existing in-memory profiles unchanged.
- [x] 3.7 Add tests for successful load, replacement on load, unsupported schema version diagnostics, invalid payload diagnostics, invalid ID diagnostics, reserved default diagnostics, duplicate-name diagnostics, ID-sorted duplicate handling, additive merge behavior, atomic invalid replacement behavior, and best-effort loading of other valid profiles.

## 4. Profile Provider Resolution

- [x] 4.1 Implement `IronRuntime::resolve_profile_provider(profile: &AgentProfile)` by returning a resolved provider abstraction that uses the existing injected/default provider path for `RuntimeDefault` profiles and derives a managed provider context from managed provider slug/model with no API-key override for managed profiles.
- [x] 4.2 Add tests with mock credential stores for successful managed profile provider resolution.
- [x] 4.3 Add tests for runtime-default profile resolution without a credential resolver.
- [x] 4.4 Add tests for missing resolver, missing credential, unsupported provider or credential mode, and OAuth failure surfaces where existing test utilities support them.

## 5. Default Profile Semantics

- [x] 5.1 Define how the default profile is represented for this implementation slice without adding a separate non-profile execution path.
- [x] 5.2 Add tests showing unspecified profile selection resolves to the default `AgentProfile` before provider resolution.
- [x] 5.3 Add tests showing non-blank profile `identity_prompt` is used as the profile identity layer and blank/missing `identity_prompt` falls back to the built-in default identity prompt.

## 6. Verification

- [x] 6.1 Run targeted Rust tests for profile domain, IronAgent registry/loading, default profile selection, and provider resolution.
- [x] 6.2 Run `cargo check --manifest-path src-tauri/Cargo.toml` if this crate is checked through the workspace app, or the narrowest equivalent Rust check for `iron-core` if available.
- [x] 6.3 Update any rustdoc examples or public API documentation needed for the new profile APIs.
