## Context

`IronRuntime` already owns most low-level mechanics needed for delegated execution: session creation, hidden-session flags, session-effective tool catalogs, parent-child session relationships, cancellation propagation, profile provider resolution, and prompt execution through `PromptRunner`. The missing design is how to expose these pieces as a safe sub-agent primitive without turning child sessions into a hidden approval bypass.

`ConfigStore` already has an opaque `prompts` table with versioned JSON payloads. Stored prompts can therefore define typed payload semantics on top of the existing storage contract rather than adding a new table. The first useful invocation surface is session-bound use from AgentIron slash commands; later scheduled tasks and CLI one-shots should reuse the same invocation engine.

## Goals / Non-Goals

**Goals:**

- Define `delegate_task` as the core child-agent spawning primitive.
- Make delegated child sessions hidden from normal session listings by default while exposing explicit APIs for frontend inspection.
- Preserve existing and expected tool approval behavior for `delegate_task` itself.
- Support child approval routing modes so a child run can either auto-approve child tool calls or propagate each approval request to the parent/main UI.
- Define sub-agent tool policy defaults and configuration semantics, including parent inheritance, profile boundaries, blocklists, and auditable additions.
- Use existing profile provider resolution for delegated runs.
- Use existing session relationship tracking for parent-child registration, cancellation propagation, and cleanup.
- Define typed stored prompts that can be loaded from ConfigStore prompt records.
- Define stored-prompt invocation through delegated child execution.

**Non-Goals:**

- Implementing the scheduler feature.
- Implementing CLI one-shot execution beyond designing compatible stored-prompt invocation semantics.
- Replacing existing ConfigStore prompt storage.
- Designing rich prompt-template variables, conditionals, or templating languages.
- Persisting long-term delegation history outside the existing durable session/audit structures unless required for the initial delegation record.
- Changing global approval strategy semantics unrelated to delegated execution.

## Decisions

### Treat delegation as a normal tool call plus child approval routing

`delegate_task` should be registered as a normal tool and should follow existing and expected tool approval settings and workflows. The runtime must not hardcode `delegate_task` to always require approval, always skip approval, or otherwise special-case it outside the normal tool approval model.

Once a delegation starts, the frontend/user chooses the child approval routing mode for that child run:

- `AutoApproveChildTools`: child tool calls are allowed without per-call UI prompts for the duration of the delegated run.
- `PropagateToParent`: child tool approval requests are passed back to the parent/main UI one-by-one and resolved through the regular approval workflow.

This keeps delegation from becoming a hidden privilege escalation path while still allowing intentional high-trust sub-agent runs.

Alternative considered: treating approval of `delegate_task` as approval for all child tool calls. That is simpler but collapses two different decisions: whether to spawn a child and whether to approve each child capability use.

### Hide delegated child sessions by default, not from audit

Delegated child sessions should be created hidden by default so they do not clutter normal session lists. Hidden sessions must still be explicitly inspectable by frontend APIs, and delegation results should include enough identifiers for the frontend to open or audit the child session.

Suggested API shape:

```rust
pub fn list_child_sessions(parent: SessionId) -> Vec<SessionSummary>;
pub fn list_hidden_sessions() -> Vec<SessionSummary>;
pub fn get_delegation_tree(root: SessionId) -> DelegationTree;
```

The exact names can change during implementation, but hidden must mean "omitted from default listings," not "secret."

### Use existing parent-child session relationships

`IronRuntime::register_child` and `unregister_child` already enforce parent/child existence, same-connection relationships, acyclic relationships, and cancellation/closure propagation. Delegated child runs should reuse this relationship graph where possible.

The existing same-connection restriction may need revisiting if `IronRuntime::new_connection()` creates a separate standalone connection for the child as described in issue #57. The proposal should resolve this tension explicitly: either child sessions share the parent connection for relationship tracking, or relationship registration is generalized to permit delegated child connections while preserving ownership and cancellation invariants.

### Keep profile policy as a hard boundary

Delegated execution should select an `AgentProfile` and use existing profile provider resolution. The selected profile's `ToolFilter` is a hard boundary over the child tool catalog. Configured tool additions must not bypass profile restrictions.

Recommended catalog derivation:

```text
base = ParentEffective | ChildDefault
candidate = base union configured_additions
profile_bounded = apply(selected_profile.tools, candidate)
final = profile_bounded minus subagent_blocklist
```

The default policy should use `ParentEffective` as the base catalog with no additions and no extra blocklist.

Alternative considered: letting sub-agent additions bypass profile filters. That gives configuration maximum power but makes profiles less meaningful as identity and safety boundaries.

### Report privilege expansion explicitly

If configured additions make a child tool visible when it was not present in the parent session's effective catalog, that is a privilege expansion. The runtime should surface this in delegation metadata and in any frontend-visible approval context for the delegated run.

Suggested metadata:

```text
inherited_tools
removed_tools
added_tools
unavailable_requested_tools
tool_catalog_digest
```

The exact digest algorithm can be an implementation detail, but it should be deterministic over the final model-visible tool definitions and approval flags.

### Keep tool visibility separate from approval

Tool policy decides which tools a child can see and call. Approval routing decides whether a visible child call is allowed immediately or must ask the parent/main UI. A tool may be visible and still require approval. A configured addition may be visible and still require propagated per-call approval.

This separation avoids encoding approval semantics into catalog construction and keeps `SessionToolCatalog::requires_approval` useful for both parent and child runs.

### Define a delegation record for audit and cleanup

Delegated runs should produce a record tying parent approval, child execution, and result together. At minimum, the runtime should know:

- parent session ID;
- parent tool call ID;
- delegation ID;
- child session ID;
- selected profile ID or default-profile marker;
- child approval routing mode;
- max iterations or budget;
- final child outcome;
- child tool catalog digest;
- goal/context digests where available.

This record may initially be in-memory plus represented in durable session history through tool records and child session transcripts. If richer persistence is needed, that should be called out during implementation rather than added implicitly.

### Keep delegate_task result compact

The exact structured `delegate_task` result payload is not fully settled. The result should be small enough for model context but include identifiers needed for audit and UI linking.

Likely fields:

```json
{
  "delegation_id": "...",
  "child_session_id": "...",
  "outcome": "end_turn",
  "final_text": "..."
}
```

Richer audit details should be available through runtime/frontend APIs rather than forcing every detail into the model transcript.

### Define stored prompts as reusable task definitions

Stored prompts should be typed payloads over ConfigStore prompt records. A minimal `StoredPrompt` should include:

```rust
pub struct StoredPrompt {
    pub instructions: String,
    pub skills: Vec<String>,
    pub profile: Option<AgentProfileId>,
}
```

The stored prompt ID can serve as the stable prompt name for this slice. Rename/alias semantics can be deferred.

### Invoke stored prompts through delegation machinery

Stored prompts should not define a separate execution path. Invoking a stored prompt should resolve the prompt, compose its instructions with optional extra context, apply the selected profile and requested skills, and run through the same child-session/delegation machinery used by `delegate_task`.

Invocation surfaces should be layered:

```text
StoredPrompt registry
        │
        ▼
Prompt invocation engine
        │
        ├─ session slash command
        ├─ run_task tool
        ├─ scheduled task runner (future)
        └─ CLI one-shot (future)
```

Session-bound invocation can inherit the current parent session and approval UI. Scheduler and CLI one-shot invocation need explicit non-interactive or interactive approval behavior, but those surfaces are future work.

## Risks / Trade-offs

- Child approval propagation can be subtle to implement because child prompt execution needs a sink that routes approval requests to the parent/main UI without confusing child and parent tool-call IDs.
- Parent-effective tool inheritance is safest by default but may surprise users who expect a selected profile to fully define child capabilities. Mitigation: document `ParentEffective` as the default and expose explicit policy choices.
- Allowing configured additions introduces privilege expansion. Mitigation: keep profile filters as hard boundaries and report expansions explicitly.
- Hidden child sessions reduce UI clutter but can hide important work if frontend inspection APIs are weak. Mitigation: require explicit child-session listing/tree APIs.
- Stored prompt execution across session, scheduler, and CLI contexts may diverge. Mitigation: define a shared invocation engine and treat surfaces as adapters.
- The final `delegate_task` result payload is not settled. Mitigation: keep a compact initial result and expose richer audit details through explicit APIs.
