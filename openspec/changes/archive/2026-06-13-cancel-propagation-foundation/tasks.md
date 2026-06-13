## 1. Runtime Session Metadata

- [x] 1.1 Add `CreateSessionOptions { hidden: bool }` with a visible default.
- [x] 1.2 Add hidden-session metadata to `RuntimeSession` without changing durable transcript state.
- [x] 1.3 Add runtime relationship indexes for `children_by_parent` and `parent_by_child` using deterministic child ordering.

## 2. Session Creation And Listing APIs

- [x] 2.1 Add a runtime session creation path that accepts `CreateSessionOptions`.
- [x] 2.2 Preserve the existing visible default session creation path by delegating to the options-based path.
- [x] 2.3 Update `sessions_for_connection` to support explicit hidden-session inclusion.
- [x] 2.4 Update facade and ACP call sites to use visible-session defaults and visible-only normal listings.

## 3. Child Relationship APIs

- [x] 3.1 Implement `IronRuntime::register_child(parent_session_id, child_session_id)`.
- [x] 3.2 Validate that parent and child sessions exist, are distinct, and share the same connection.
- [x] 3.3 Validate that a child has at most one parent.
- [x] 3.4 Validate that registering a relationship cannot create a cycle.
- [x] 3.5 Implement `IronRuntime::unregister_child(parent_session_id, child_session_id)` as a non-closing relationship removal.

## 4. Recursive Close And Cleanup

- [x] 4.1 Implement depth-first descendant collection for session close.
- [x] 4.2 Signal cancellation for active prompts before removing each closed descendant session.
- [x] 4.3 Remove closed sessions from runtime session storage and all relationship indexes.
- [x] 4.4 Ensure direct child close removes its descendants while leaving ancestors alive.
- [x] 4.5 Ensure connection close remains safe when multiple sessions in the same graph are selected for closure.

## 5. Tests And Verification

- [x] 5.1 Add tests for valid registration and unregistering without closing sessions.
- [x] 5.2 Add tests rejecting missing-session, self-parent, cross-connection, duplicate-parent, and cyclic registrations.
- [x] 5.3 Add tests for recursive parent close with child and grandchild sessions.
- [x] 5.4 Add tests that active child prompts are cancellation-signalled during recursive close.
- [x] 5.5 Add tests for hidden session creation and `sessions_for_connection` filtering with and without hidden sessions.
- [x] 5.6 Run `cargo check` and targeted runtime/session tests.
