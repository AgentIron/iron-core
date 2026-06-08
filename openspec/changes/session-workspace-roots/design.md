## Context

`Config.workspace_roots` is currently a construction-time runtime setting. Request composition reads it to render `<runtime_context>`, while builtin tools receive a separate `BuiltinToolConfig.allowed_roots` snapshot when they are registered. This works for one workspace per runtime, but AgentIron needs multiple tabs/sessions to share one live `IronRuntime` while each tab has independent roots.

The existing architecture already has durable per-session state (`DurableSession`) and session-effective tool surfaces (`SessionToolCatalog`) for MCP/plugin enablement and skill availability. Workspace roots should follow the same per-session pattern rather than becoming another runtime-global mutable setting.

## Goals / Non-Goals

**Goals:**

- Support independent workspace roots for multiple live sessions under one `IronRuntime`.
- Preserve session history, MCP connections, plugin enablement, active skills, and runtime state when roots change.
- Apply root changes at turn boundaries so one prompt turn has one consistent root snapshot.
- Keep prompt runtime context and builtin tool authorization aligned for each prompt turn.
- Automatically refresh project-skill availability when a session's new roots become active.
- Preserve `Config::with_workspace_roots()` as the default root source for newly created sessions.

**Non-Goals:**

- Runtime-wide live root switching for all sessions.
- Changing builtin tool roots in the middle of an active prompt.
- Automatically deactivating skills that were already active before a root change.
- Rebuilding MCP connections, plugin registries, or provider state when roots change.
- Adding client UI behavior in AgentIron; this change only defines the iron-core API and runtime behavior.

## Decisions

### Store Workspace Roots on the Session

Add session-owned active roots and pending roots to the session state. New sessions initialize their active roots from `Config.workspace_roots` when present, otherwise from `std::env::current_dir()`.

Rationale: multiple tabs require roots to vary by session while sharing one runtime. `DurableSession` already owns other per-session state such as MCP enablement, plugin enablement, current model, active skills, and available skills.

Alternative considered: make `IronRuntime` config mutable. Rejected because it would change roots for every session and would conflict with multi-tab isolation.

### Apply Root Changes at Turn Boundaries

`AgentSession::set_workspace_roots(Vec<PathBuf>)` applies roots immediately only when the session is idle. If a prompt is active, the method records pending roots and the runtime applies them when the prompt finishes.

Rationale: the model sees a working directory and workspace roots in the system prompt. If roots changed mid-turn, the model could plan tool calls under one root while builtin tools authorize another. Turn-boundary application keeps the safety boundary and prompt context consistent.

Alternative considered: reject changes during active prompts. Rejected because AgentIron can queue tab directory changes without forcing callers to retry manually after prompt completion.

Alternative considered: apply immediately. Rejected because prompt text and tool authorization could diverge within one turn.

### Use One Prompt-Turn Root Snapshot

At prompt start, capture the session's active roots and use that snapshot for both request composition and local builtin tool execution throughout the prompt turn. The first root is the working directory shown in `<runtime_context>`; all active roots are rendered as workspace roots.

Rationale: this directly encodes the invariant that there is always one root set per prompt. It also avoids accidental reads from `Config.workspace_roots` after a session has its own roots.

Alternative considered: let builtin tools look up latest session roots at execution time. Rejected because pending roots could become active after prompt completion while late tool execution or child tool paths still expect the prompt snapshot.

### Route Builtin Tools Through Session-Effective Execution

Local builtin filesystem, search, and shell tools must enforce the session prompt snapshot rather than the global `BuiltinToolConfig.allowed_roots` captured at registration. The session-effective tool path should provide or wrap builtin tool execution with the active roots for that prompt.

Rationale: builtin tools are the security boundary for filesystem and shell access. They must match the roots shown to the model and must vary by session.

Alternative considered: re-register builtin tools whenever a session changes roots. Rejected because tools are registered runtime-wide, so re-registration would affect other sessions and would need to preserve unrelated builtin config exactly.

### Rescan Project Skills When Roots Become Active

When roots are applied, the runtime rescans project-level skills under each active root and updates that session's available skill snapshot. Already active skills remain active even if their original root is no longer active.

Rationale: a directory change should expose skills from the new project on the next prompt, but removing active skill instructions mid-conversation would be surprising and could invalidate prior context.

Alternative considered: refresh only the runtime skill catalog. Rejected because project skills are now session-root-dependent and sessions need independent available skill snapshots.

### Preserve Runtime Config as Session Defaults

`Config.workspace_roots` remains a construction-time default used to seed new sessions. Existing callers that configure one runtime root and never call `set_workspace_roots()` continue to get the same prompt and builtin behavior.

Rationale: this keeps the old setup API useful while moving live mutation to the session API required by multi-tab clients.

Alternative considered: remove `Config.workspace_roots`. Rejected because that would be an unnecessary breaking change.

## Risks / Trade-offs

- **Builtin tool coupling**: Retrofitting session roots into local builtin execution can touch several tools → Mitigation: centralize root snapshot handling and keep builtin policy/timeouts/read tracking unchanged.
- **Serialized session shape changes**: Adding roots to `DurableSession` changes serialized session data → Mitigation: use serde defaults so older sessions load with runtime/default roots.
- **Skill catalog complexity**: Runtime-global and session-specific skill discovery can diverge → Mitigation: treat the runtime catalog as a source baseline and session available skills as the prompt-facing snapshot.
- **Pending root visibility**: Callers may expect `set_workspace_roots()` to take effect immediately during active prompts → Mitigation: return or expose whether roots were applied immediately or deferred.
- **Security regression risk**: Any mismatch between prompt roots and builtin roots could allow confusing or unsafe tool behavior → Mitigation: add tests that assert prompt rendering and builtin path checks use the same prompt-turn snapshot.

## Migration Plan

1. Add serde-defaulted active and pending workspace root fields to session state.
2. Seed new sessions from `Config.workspace_roots` or current directory.
3. Add `AgentSession::set_workspace_roots(Vec<PathBuf>)` and runtime/session helpers for idle vs active application.
4. Pass session root snapshots into request composition instead of reading roots only from `Config`.
5. Update session-effective builtin execution to enforce prompt-turn roots.
6. Add root-application hooks that refresh the session's available project skills.
7. Add tests for multiple sessions with different roots, active-prompt deferral, prompt/tool consistency, and skill rescanning.

Rollback: revert the change; sessions serialized with new root fields remain loadable by current code only if unknown fields are ignored by the consumer, so this should be released as a forward migration with compatibility tests.

## Design Decisions

1. `AgentSession::set_workspace_roots(...)` returns `Result<bool, RuntimeError>` where `true` = roots applied immediately, `false` = deferred until the next turn boundary.
2. Clients can read active roots via `AgentSession::workspace_roots()`. Pending roots are internal and applied automatically at turn boundaries.
3. Skill rescanning uses the full discovery pipeline: project dirs from workspace roots, user-level dirs (`~/.agents/skills/`), additional configured dirs, and runtime-registered skills.
