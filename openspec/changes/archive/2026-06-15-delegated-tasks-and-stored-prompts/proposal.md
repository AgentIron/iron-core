## Why

AgentIron needs a core sub-agent spawning primitive so a model, frontend, scheduler, or CLI can delegate bounded work to a child agent identity without losing normal approval, cancellation, and audit behavior. GitHub issue #57 describes this as `delegate_task`: a model-callable tool that creates a child session, runs it with an optional profile, and returns the result.

AgentIron also needs named reusable prompt templates that can invoke that same child-session machinery. GitHub issue #62 describes stored prompts and `run_task`, which should allow session-bound slash commands first and later scheduled tasks and one-shot CLI execution.

These issues should be designed together because stored prompts depend on delegated child execution, and the child execution design must define visibility, approval propagation, tool scoping, and audit semantics before higher-level invocation surfaces build on it.

## What Changes

- Add a `delegate_task` tool that can spawn a hidden child session, run a bounded goal/context with an optional profile, and return a structured result.
- Add runtime APIs for creating standalone child connections/sessions and for exposing hidden delegated sessions to frontend code without showing them in normal session listings by default.
- Preserve normal tool approval semantics for `delegate_task` itself; it must not be hardcoded to always require approval or always bypass approval.
- Add delegated child approval routing so child tool calls can either be auto-approved for that child run or propagated back to the parent/main UI one-by-one through the regular approval workflow.
- Add sub-agent tool policy semantics with a safe default that inherits the parent session's effective tool catalog, applies the selected profile's `ToolFilter` as a hard boundary, and applies configured sub-agent blocklists.
- Support configured child tool additions while reporting additions that were not present in the parent effective catalog as privilege expansions in delegation metadata and frontend-visible approval context.
- Reuse existing profile provider resolution, session tool catalogs, child-session relationship tracking, cancellation propagation, and prompt-running infrastructure where possible.
- Add a typed `StoredPrompt` model over existing ConfigStore prompt records with instructions, skills, and optional profile selection.
- Add prompt registry/load APIs for registering, unregistering, listing, and loading stored prompts from ConfigStore.
- Add a `run_task` invocation path for stored prompts that reuses delegated child-session machinery and can be used from session-bound slash commands first, with scheduler and CLI one-shot usage designed as future invocation surfaces.
- Add tests with mock providers for delegation, child approval propagation, tool policy enforcement, hidden-session visibility APIs, stored-prompt loading, and `run_task` invocation.

## Capabilities

### New Capabilities

- `delegated-task-tool`: Sub-agent delegation primitive for hidden child sessions, child approval routing, tool-scope policy, cancellation propagation, and audit metadata.
- `stored-prompts`: Typed reusable prompt definitions and invocation semantics layered on delegated child execution.

### Modified Capabilities

- `agent-profiles`: Existing profile `ToolFilter`, `SkillFilter`, provider resolution, and approval policy become execution inputs for delegated child runs.
- `core-config-store`: Existing opaque prompt records become the durable storage substrate for typed stored prompts without replacing the ConfigStore prompt table.

## Impact

- **Public API**: new delegation, child-session inspection, stored-prompt registration/loading, and stored-prompt invocation APIs.
- **Runtime orchestration**: child sessions are hidden by default but auditable; cancellation and closure should continue to propagate through existing parent-child relationships.
- **Approval flow**: child tool calls can be routed back to the parent/main UI instead of silently inheriting broad approval.
- **Tool policy**: sub-agents get explicit catalog derivation rules, including auditable privilege expansion for configured additions.
- **Config store integration**: typed stored prompts decode from existing opaque ConfigStore prompt records using schema-versioned payloads.
- **Future work**: this proposal prepares scheduler execution and CLI one-shot prompt execution without requiring those surfaces in the initial implementation.
