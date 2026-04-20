# Architecture Cleanup Checklist

This checklist turns the current architecture review into an implementation-tracking document.

## Goals

- Collapse the crate onto the facade/runtime architecture.
- Remove deprecated or dead-end compatibility APIs.
- Reduce duplicate tool-catalog construction and drift.
- Align public configuration types with actual runtime behavior.
- Eliminate stale documentation and status artifacts.

## Phase 1: Stop Expanding Legacy Surface

- [x] Remove legacy APIs from top-level README examples.
- [x] Remove legacy APIs from `prelude` exports in `src/lib.rs`.
- [x] Remove legacy APIs from prominent crate-level re-export sections where practical.
- [x] Add a policy note in crate docs that no new features land on legacy modules.
- [x] Audit tests and mark legacy-only coverage as compatibility coverage, not primary behavior coverage.

Exit criteria:
- New users are steered to `IronAgent`, `AgentConnection`, `AgentSession`, and `IronRuntime`.
- The public surface is centered on `IronAgent`, `AgentConnection`, `AgentSession`, and `IronRuntime`.

## Phase 2: Remove Dead-End MCP Compatibility APIs

- [x] Stop re-exporting `McpTool` from public module surfaces.
- [x] Stop re-exporting `EffectiveToolView` from public module surfaces.
- [x] Migrate MCP visibility and inspection tests to `SessionToolCatalog`.
- [x] Delete `src/mcp/effective_tools.rs` once all call sites and tests are migrated.
- [x] Update MCP module docs to point only to `SessionToolCatalog`.

Exit criteria:
- MCP inspection, prompt construction, and execution all use `SessionToolCatalog`.
- No public API remains that advertises MCP execution through a non-canonical path.

## Phase 3: Cache Session-Effective Tool State

- [x] Design the cache boundary: per-session snapshot or per-prompt snapshot.
- [x] Record the invalidation triggers:
- [x] local tool registration changes
- [x] MCP registry or discovery changes
- [x] plugin registry, auth, or health changes
- [x] session MCP/plugin enablement changes
- [x] Refactor `IronRuntime::get_session_tool_catalog()` to avoid cloning registries on every lookup.
- [x] Refactor `PromptRunner` to resolve the effective catalog once per prompt iteration or turn.
- [x] Add tests proving prompt construction, approval checks, and execution use the same catalog instance or snapshot.

Exit criteria:
- Tool catalog construction is not repeated unnecessarily in the hot path.
- The effective tool view used within a turn is explicit and internally consistent.

## Phase 4: Fix Context Policy Drift

- [x] Decide whether `ContextWindowPolicy::SummarizeAfter` is supported or should be removed.
- [x] If removing it, delete the variant and update docs and tests.
- [x] Remove the TODO in `src/config.rs` related to summary insertion.
- [x] Ensure `request_builder` behavior matches the public config contract.

Exit criteria:
- Public config types only advertise supported behavior.
- No context-window branch is partially implemented or rejected inconsistently.

## Phase 5: Retire the Larger Legacy Session/Turn Stack

- [x] Inventory all public legacy modules:
- [x] `session`
- [x] `session_handle`
- [x] `session_runtime`
- [x] `turn`
- [x] `events::StreamEvent`
- [x] `loop_state::AgentLoop`
- [x] Decide whether to hide them behind a `legacy-api` cargo feature or keep them temporarily as deprecated modules.
- [x] Migrate remaining internal examples and tests off the legacy stack where feasible.
- [x] Remove functionality drift by ensuring unsupported modern behaviors are not silently accepted on the legacy path.
- [x] Delete legacy modules once downstream compatibility requirements are satisfied.

Exit criteria:
- There is one primary orchestration architecture in the crate.
- The public surface has one primary orchestration architecture.

## Phase 6: Clean Documentation and Historical Status Files

- [x] Consolidate architecture guidance into one canonical document set.
- [x] Audit `README.md` for stale status language and deprecated guidance.
- [x] Move historical review/status docs to an archive location or remove them.
- [x] Ensure examples match the current facade/runtime architecture.
- [x] Add links from the README to current architecture docs if needed.

Exit criteria:
- Docs no longer duplicate status claims across multiple files.
- A new contributor can identify the supported architecture quickly.

## Suggested Execution Order

- [x] Phase 2: Remove dead-end MCP compatibility APIs
- [x] Phase 3: Cache session-effective tool state
- [x] Phase 4: Fix context policy drift
- [x] Phase 1: Stop expanding legacy surface
- [x] Phase 5: Retire the larger legacy stack
- [x] Phase 6: Clean documentation and status files

## Notes

- Treat legacy tests as compatibility protection, not proof that the legacy path should keep growing.
- Do not remove compatibility surface until downstream usage and migration timing are understood.
- Keep behavior changes and surface-area removals in separate pull requests where possible.
- The `AgentEvent`/`FacadeToolStatus`/`drain_events` residue on the facade has been removed; the streaming `PromptEvent` / `PromptEvents` path is the single event contract.
