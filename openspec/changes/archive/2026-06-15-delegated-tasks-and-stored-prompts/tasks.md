## 1. Delegated Task Domain and API

- [x] 1.1 Define public/internal delegation input types for `goal`, optional `context`, optional `profile`, `max_iterations`, child approval routing mode, and child tool policy.
- [x] 1.2 Define delegation result and audit metadata types with delegation ID, parent session ID, parent tool call ID, child session ID, selected profile, approval routing mode, final outcome, and tool catalog digest.
- [x] 1.3 Add child-session inspection APIs for hidden sessions, child sessions, or delegation trees so frontend code can inspect delegated work explicitly.
- [x] 1.4 Add or expose `IronRuntime::new_connection()` or an equivalent child-session connection creation API while preserving session ownership and cancellation invariants.
- [x] 1.5 Resolve whether delegated child sessions share the parent connection or whether parent-child registration can span child connections safely.

## 2. Child Session Construction

- [x] 2.1 Create delegated child sessions hidden by default.
- [x] 2.2 Register child sessions under the parent session and ensure cleanup unregisters relationships on completion/failure.
- [x] 2.3 Ensure parent cancellation or closure cancels active child prompts and leaves terminal durable records.
- [x] 2.4 Apply selected profile identity prompt, provider resolution, and max-iteration bounds to child runs.
- [x] 2.5 Return enough child identifiers in results for frontend audit and session opening.

## 3. Child Tool Policy

- [x] 3.1 Implement default child tool policy as parent effective catalog inheritance with the selected profile `ToolFilter` as a hard boundary.
- [x] 3.2 Add configured sub-agent blocklist support applied after profile filtering.
- [x] 3.3 Add configured child tool additions from the child/default runtime catalog without bypassing profile filters.
- [x] 3.4 Report additions not present in the parent effective catalog as privilege expansions in delegation metadata.
- [x] 3.5 Add deterministic final tool catalog digesting based on model-visible tool definitions and approval flags.
- [x] 3.6 Add diagnostics for requested additions that are unavailable because the tool is missing, plugin/MCP state disables it, auth is missing, or profile policy excludes it.

## 4. Approval Routing

- [x] 4.1 Ensure `delegate_task` itself follows normal tool approval settings and workflows without hardcoded approval special-casing.
- [x] 4.2 Add child approval routing mode for auto-approving child tool calls for a delegated run.
- [x] 4.3 Add child approval routing mode for propagating each child tool approval request to the parent/main UI.
- [x] 4.4 Preserve child tool-call durable records when child approvals are allowed, denied, or cancelled through propagated approval.
- [x] 4.5 Add tests showing a propagated child approval request can be allowed, denied, and cancelled through the parent/main approval flow.

## 5. Delegate Task Tool

- [x] 5.1 Implement `DelegateTask` as a `Tool` with JSON schema for `goal`, optional `context`, optional `profile`, and optional `max_iterations`.
- [x] 5.2 Register `delegate_task` through `IronRuntime::register_delegate_task_tool()` or equivalent public registration API.
- [x] 5.3 Parse and validate tool arguments with actionable errors for missing goal, unknown profile, invalid max iterations, and invalid child policy inputs.
- [x] 5.4 Run the child prompt through the selected profile/provider path and delegated approval sink.
- [x] 5.5 Return a compact structured JSON result with delegation ID, child session ID, outcome, and final model-readable result text.

## 6. Stored Prompt Domain and Registry

- [x] 6.1 Define `StoredPrompt` with `instructions: String`, `skills: Vec<String>`, and `profile: Option<AgentProfileId>` using schema version 1.
- [x] 6.2 Add runtime or agent APIs to register, unregister, list, and retrieve stored prompts by stable prompt name/ID.
- [x] 6.3 Implement `load_prompts(store: &ConfigStore)` by decoding typed ConfigStore prompt records with per-prompt diagnostics for invalid payloads or unsupported schema versions.
- [x] 6.4 Validate stored prompt names/IDs and prompt payloads without changing ConfigStore's opaque prompt record contract.
- [x] 6.5 Add tests for prompt registration, replacement, unregister, list ordering, ConfigStore loading, invalid payload diagnostics, and unsupported schema diagnostics.

## 7. Stored Prompt Invocation

- [x] 7.1 Add a shared stored-prompt invocation path that composes stored instructions, optional extra context, selected profile, requested skills, and delegated child execution.
- [x] 7.2 Add `RunTask` or equivalent invocation tool for model-callable stored-prompt execution where appropriate.
- [x] 7.3 Ensure session-bound invocation can be used by frontend slash-command handling without requiring scheduler or CLI implementation in this change.
- [x] 7.4 Ensure requested stored-prompt skills cannot exceed the selected profile's `SkillFilter` boundary.
- [x] 7.5 Add tests for invoking a stored prompt within a session with mock providers and delegated child execution.

## 8. Verification

- [x] 8.1 Add unit tests for child tool policy inheritance, blocklists, profile-filter boundaries, additions, and privilege-expansion reporting.
- [x] 8.2 Add integration tests for hidden child sessions, child-session inspection APIs, cancellation propagation, and delegated approval propagation.
- [x] 8.3 Add mock-provider tests for `delegate_task` and stored-prompt invocation outcomes.
- [x] 8.4 Run the narrowest relevant Rust checks and tests for runtime, profile, stored prompt, and prompt runner changes.
