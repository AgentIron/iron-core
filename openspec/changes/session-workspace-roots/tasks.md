## 1. Session Data Model

- [x] 1.1 Add serde-defaulted active workspace roots to `DurableSession`
- [x] 1.2 Add serde-defaulted pending workspace roots to `DurableSession`
- [x] 1.3 Add helper methods for reading active roots, setting pending roots, and applying pending roots
- [x] 1.4 Seed new sessions from `Config.workspace_roots` or current directory fallback
- [x] 1.5 Include workspace-root state in relevant session/debug inspection paths without exposing secrets

## 2. Public Session API and Turn-Boundary Semantics

- [x] 2.1 Add `AgentSession::set_workspace_roots(Vec<PathBuf>)` public API
- [x] 2.2 Add runtime helper to apply roots immediately when the session is idle
- [x] 2.3 Add runtime helper to defer roots when the session has an active prompt
- [x] 2.4 Ensure multiple deferred root updates keep only the latest pending roots
- [x] 2.5 Apply pending roots after prompt completion before the next prompt can start
- [x] 2.6 Expose or return enough status for callers to distinguish applied vs deferred updates

## 3. Prompt Root Snapshot

- [x] 3.1 Capture the session's active workspace roots at prompt start
- [x] 3.2 Thread the captured root snapshot through request construction
- [x] 3.3 Update runtime context rendering to use session roots for working directory and workspace root lines
- [x] 3.4 Preserve existing request-builder behavior for callers that do not provide a session root snapshot
- [x] 3.5 Add tests proving pending roots do not affect prompt rendering until the next prompt

## 4. Builtin Tool Root Enforcement

- [x] 4.1 Identify the session-effective local builtin execution path used for model and child tool calls
- [x] 4.2 Pass prompt-turn roots into builtin filesystem tool execution
- [x] 4.3 Pass prompt-turn roots into builtin glob/grep execution and default base path selection
- [x] 4.4 Pass prompt-turn roots into builtin shell execution and default working directory selection
- [x] 4.5 Preserve existing builtin policy, disabled tools, timeouts, output limits, and read tracking behavior
- [x] 4.6 Add tests proving builtin tools do not authorize paths from another session's roots
- [x] 4.7 Add tests proving pending roots do not change builtin authorization during an active prompt

## 5. Project Skill Refresh

- [x] 5.1 Add project-skill discovery helper that accepts a session workspace root snapshot
- [x] 5.2 Refresh the session's available skill snapshot when roots are applied immediately
- [x] 5.3 Refresh the session's available skill snapshot when deferred roots are applied after prompt completion
- [x] 5.4 Preserve already active skill instructions when available skills are refreshed
- [x] 5.5 Preserve project skill trust gating and diagnostics for newly active roots
- [x] 5.6 Add tests for root-change project skill discovery and active skill preservation

## 6. Multi-Session Integration Tests

- [x] 6.1 Add test for two sessions under one runtime with different active roots
- [x] 6.2 Add test that `set_workspace_roots` on one session does not affect another session
- [x] 6.3 Add test that prompt runtime context and builtin allowed roots use the same session snapshot
- [x] 6.4 Add test that existing `Config::with_workspace_roots()` remains the default for newly created sessions
- [x] 6.5 Add serialization compatibility test for sessions without workspace-root fields

## 7. Documentation and Validation

- [x] 7.1 Document `AgentSession::set_workspace_roots` behavior and deferral semantics
- [x] 7.2 Document that builtin tools use one root snapshot per prompt turn
- [x] 7.3 Run `cargo check --manifest-path src-tauri/Cargo.toml` if this repository is used through the Tauri workspace
- [x] 7.4 Run the narrowest relevant Rust tests for runtime, request builder, builtin tools, and skill refresh
- [x] 7.5 Run `openspec status --change session-workspace-roots` and confirm artifacts remain apply-ready
