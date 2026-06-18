## 1. Default Profile Definitions and Seeding

- [x] 1.1 Add core-owned shipped `AgentProfile` definitions for `explore`, `plan`, and `apply` with `RuntimeDefault`, `ToolFilter::Inherit`, `SkillFilter::Inherit`, `PerTool`, and profile-specific identity prompts.
- [x] 1.2 Add or expose durable ConfigStore bootstrap metadata APIs for domain-scoped seed markers without using fake profile records.
- [x] 1.3 Add default-profile seed state keys and versioned marker handling for shipped agent profiles.
- [x] 1.4 Implement `FirstRunOnly` seed policy that creates missing shipped profiles only when the seed marker is absent, preserves existing records, and records the marker.
- [x] 1.5 Implement `RestoreMissing` seed policy that recreates missing shipped profiles on explicit request while preserving existing records.
- [x] 1.6 Return structured seed reports and diagnostics listing created profiles, skipped existing profiles, marker state, and storage failures.

## 2. Profile Validation and Serialization

- [x] 2.1 Update profile validation/deserialization so user-facing `AgentApproval` accepts only `PerTool` and `AutoApprove`.
- [x] 2.2 Reject `ReadOnly` profile approval payloads with a validation error or load diagnostic rather than silently mapping them.
- [x] 2.3 Preserve profile serialization as reference-only provider/model data with no embedded provider definitions, model metadata, API keys, OAuth tokens, or connection metadata.
- [x] 2.4 Ensure unavailable provider/model/tool references do not make otherwise well-formed profile import/load fail solely due to machine-local availability.

## 3. Session-Effective Profile Policy

- [x] 3.1 Add session state for snapshotted profile policy: profile ID, profile identity, profile tool filter, profile approval policy, and provider/model resolution diagnostics or effective selection metadata.
- [x] 3.2 Apply selected profile policy during primary session setup and managed prompt setup without runtime branches for `explore`, `plan`, or `apply`.
- [x] 3.3 Ensure stored profile edits or deletion after session setup do not implicitly mutate existing session policy.
- [x] 3.4 Preserve behavior-critical profile policy snapshot fields through handoff export/import where required for session continuity.

## 4. Tool Filtering and Approval Enforcement

- [x] 4.1 Apply session-effective `ToolFilter` during primary session tool catalog construction using exact canonical model-visible tool names.
- [x] 4.2 Ensure `Allow` filters provider-visible tool definitions and execution-time availability to only listed available tools.
- [x] 4.3 Ensure `Deny` filters provider-visible tool definitions and execution-time availability by removing listed tools.
- [x] 4.4 Emit diagnostics for unavailable tool names referenced by session-effective profile filters without failing profile load/import.
- [x] 4.5 Make session-effective `AgentApproval::AutoApprove` bypass approval prompts for tool calls that would require approval under `PerTool`.
- [x] 4.6 Keep `AgentApproval::PerTool` behavior aligned with each tool's `requires_approval` setting.

## 5. Provider/Model Resolution

- [x] 5.1 Check explicit profile provider/model references against the effective model catalog built from built-in and custom model entries.
- [x] 5.2 Resolve available explicit managed provider/model references through existing credential/provider resolution without profile-stored credentials.
- [x] 5.3 For unavailable explicit references, return structured warning diagnostics and fall back to the runtime default provider path when usable.
- [x] 5.4 If both explicit reference and runtime default are unusable, fail with an actionable error preserving the explicit-reference reason.
- [x] 5.5 Ensure fallback occurs only during setup/resolution and does not silently replace a provider after inference has started.

## 6. Tests and Validation

- [x] 6.1 Add ConfigStore tests for bootstrap metadata roundtrip, missing-marker behavior, marker persistence after profile deletion, and in-memory store support.
- [x] 6.2 Add seeding tests for first-run creation, existing-record preservation, deleted-default non-recreation under `FirstRunOnly`, and explicit `RestoreMissing` recreation.
- [x] 6.3 Add profile validation tests for functional `AutoApprove` representation and `ReadOnly` rejection.
- [x] 6.4 Add session tests proving profile policy is snapshotted and stored profile edits/deletion affect only future sessions.
- [x] 6.5 Add primary session tool catalog tests for `Inherit`, `Allow`, `Deny`, unknown tool diagnostics, provider-visible definitions, and execution-time rejection.
- [x] 6.6 Add approval tests proving `AutoApprove` bypasses prompts and `PerTool` still respects tool approval requirements.
- [x] 6.7 Add provider/model resolution tests for available explicit references, unavailable explicit references with fallback, unavailable fallback failure, and reference-only serialization.
- [x] 6.8 Run `cargo fmt --check`, `cargo clippy --locked --manifest-path Cargo.toml --all-targets --all-features -- -D warnings`, `cargo test`, and `openspec validate seed-default-agent-profiles --strict`.
