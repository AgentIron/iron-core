## Context

`IronRuntime` currently stores sessions in a flat map keyed by `SessionId`, with each session bound to one `ConnectionId`. Cancellation is prompt-local through `cancel_active_prompt`, and `close_session` removes only the specified session. Issue 61 introduces the runtime foundation needed before delegated tasks can create child sessions: parent sessions must be able to own child sessions, closing a parent must recursively stop child work, and internal child sessions must be hideable from normal session listings.

The current connection ownership model is a key constraint. ACP and facade paths reject operations on sessions not owned by the caller's connection, so child-session propagation must preserve that isolation rather than allowing one connection to close another connection's work through a parent relationship.

## Goals / Non-Goals

**Goals:**

- Represent parent-to-child session relationships inside `IronRuntime`.
- Provide `register_child(parent_session_id, child_session_id)` and `unregister_child(parent_session_id, child_session_id)` runtime APIs.
- Reject invalid child relationships, including missing sessions, self-parenting, cycles, cross-connection edges, and attempts to assign a child to multiple parents.
- Recursively cancel and close descendants when a parent session is closed.
- Add `CreateSessionOptions { hidden: bool }` so internal/delegated sessions can be created without appearing in normal session listings.
- Add explicit hidden-session filtering for session listing while preserving a way for runtime/test/debug callers to include hidden sessions.

**Non-Goals:**

- Implement the `delegate_task` tool or any child-session spawning policy.
- Add persistence for parent-child relationships or hidden-session metadata beyond the in-memory runtime.
- Wait for child prompts to finish asynchronously before `close_session` returns.
- Change provider behavior, prompt composition, tool execution semantics, or durable transcript structure.

## Decisions

### Store Child Relationships as Runtime Indexes

Add runtime-owned relationship indexes alongside the existing session map:

- `children_by_parent: HashMap<SessionId, BTreeSet<SessionId>>`
- `parent_by_child: HashMap<SessionId, SessionId>`

This keeps graph metadata independent from `RuntimeSession` locking and makes cleanup straightforward when sessions are closed. `BTreeSet` gives deterministic traversal for tests and stable behavior. An alternative was storing children and parent directly on `RuntimeSession`, but that would require recursive traversal through per-session locks while also consulting the session map.

### Require Same-Connection, Single-Parent, Acyclic Edges

`register_child` will validate that both sessions exist, are owned by the same connection, are different sessions, and that the child has no existing parent. It will also reject any edge that would create a cycle.

Same-connection validation preserves the existing ownership boundary: one connection must not be able to close or cancel another connection's session indirectly. Single-parent semantics match delegated task ownership and avoid ambiguous cancellation when multiple parents close independently.

### Treat Recursive Close as Signal-and-Remove

`close_session(parent)` will collect descendants depth-first, signal cancellation for any active prompt on each descendant, remove descendant sessions, clean graph indexes, then signal and remove the parent. The operation does not wait for prompt tasks to observe cancellation or finish cleanup. This matches the existing runtime behavior where cancellation marks active ephemeral turns and prompt runners complete cleanup when they next observe cancellation.

### Make Hidden a Session Metadata Flag

Add `hidden: bool` to runtime session metadata and introduce `CreateSessionOptions { hidden: bool }`. Existing creation paths should continue to create visible sessions by default. Hidden status affects listing only; hidden sessions remain normal owned sessions for prompt, cancel, close, and child-relationship validation.

### Preserve Ergonomic Defaults

Existing callers should continue to have a visible-session creation path. A runtime helper such as `create_session_with_options` can support hidden sessions while `create_session` delegates to it with default visible options. Listing APIs should make hidden inclusion explicit, e.g. `sessions_for_connection(connection_id, include_hidden)`, with facade `active_sessions()` using the visible-only behavior.

## Risks / Trade-offs

- Recursive close while prompts are still running can leave prompt tasks holding durable session arcs after the runtime map entry is gone. This is consistent with current close semantics, and cancellation is still signalled before removal.
- Adding relationship indexes introduces consistency risk. Keep all registration, unregistration, and close cleanup centralized in `IronRuntime` methods and cover edge cases with tests.
- Hidden sessions could make debugging harder if normal listings omit them. Preserve an include-hidden listing path for tests, diagnostics, and future delegated-task observability.
- Cross-connection child relationships might be useful for future worker models, but allowing them now would conflict with current ownership enforcement. Reject them for this foundation and revisit only if a future design explicitly introduces cross-connection delegation.
