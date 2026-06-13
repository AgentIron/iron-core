## Why

AgentIron needs agent profiles as durable agent identities, not just reusable launch configuration. Profiles define who an agent is allowed to act as, which provider/model it prefers, which tools and skills bound its behavior, which identity prompt augments its system prompt, and which approval posture applies when that identity is used for primary-agent or sub-agent execution.

All agent execution should flow through a profile. When a caller does not explicitly select a profile, AgentIron should use the built-in protected `default` profile instead of maintaining a separate non-profile execution path.

Provider resolution should be part of the same proposal because it directly depends on the profile data model and is required before profile identities can execute prompts or delegated tasks.

## What Changes

- Add public agent profile domain types for stable profile IDs, profile entries, profile loading diagnostics, profile provider context, user-facing name, tool filtering, skill filtering, agent approval policy, and optional `identity_prompt`.
- Represent profile provider selection with a profile-specific enum that supports the protected runtime default provider path and managed provider/model selection without API keys.
- Treat ConfigStore profile record IDs as stable generated profile IDs used by registration, loading, and future profile references.
- Defer profile ID generation helpers; callers and ConfigStore record creators supply validated, non-reserved profile IDs for this slice.
- Keep profile names as user-facing, renameable labels that must be unique within the loaded/registered profile set.
- Keep credential secret material out of profile APIs; profiles select provider slug and model, while auth remains owned by iron-core credential state.
- Add built-in default-profile semantics so unspecified profile selection resolves to the protected `default` `AgentProfile`.
- Protect `default` as a case-insensitive reserved profile ID/name that user profiles cannot overwrite.
- Add `IronAgent` APIs to register, unregister, list, and load named profiles from the core config store.
- Define durable profile loading from existing ConfigStore profile records without changing the opaque storage contract established by IC-1, including deterministic profile-ID-sorted best-effort loading diagnostics for invalid or duplicate profiles and atomic per-profile validation.
- Add `AgentApproval` profile policy and an `ApprovalStrategy::AutoApprove` variant as plumbing only; runtime auto-approval behavior remains out of scope for this change.
- Add profile provider resolution that supports both runtime-default provider references and managed provider construction using the existing credential resolution path and the profile's provider context.
- Surface actionable provider-resolution errors for unknown providers, missing credentials, unsupported credential modes, and expired or revoked OAuth credentials.
- Add unit coverage for profile registration, durable profile loading, and profile-to-provider resolution with mock credential stores.

## Capabilities

### New Capabilities

- `agent-profiles`: Durable agent identity profile definitions with stable IDs, unique user-facing names, protected built-in default-profile semantics, runtime registration APIs, ConfigStore loading diagnostics, approval-policy plumbing, and profile-to-provider resolution.

### Modified Capabilities

- None.

## Impact

- **Public API**: new `AgentProfile`, `AgentProfileId`, `AgentProfileEntry`, profile provider context enum, resolved profile provider abstraction, profile loading report/diagnostics, `ToolFilter`, `SkillFilter`, `AgentApproval`, and `IronAgent` profile-management methods; new `IronRuntime::resolve_profile_provider` helper.
- **Provider orchestration**: profile resolution reuses existing `CredentialResolver` behavior and provider-safe credential boundaries.
- **Config store integration**: profile records remain stored through ConfigStore's opaque profile API, with record IDs serving as stable generated profile IDs and domain decoding performed by profile-loading code.
- **Approval plumbing**: `ApprovalStrategy::AutoApprove` becomes representable but does not change tool execution policy until delegate-task/runtime behavior is implemented later.
- **Future work**: this proposal unblocks provider resolution, delegate-task execution, stored prompts, CLI/headless profile selection, and future references to durable agent identities while avoiding a separate non-profile execution path.
