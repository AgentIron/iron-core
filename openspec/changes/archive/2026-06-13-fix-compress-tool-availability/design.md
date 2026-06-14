## Context

The `compress` tool is the model-facing entry point for context compaction. It is currently synthesized conditionally in `SessionToolCatalog::new()` based on whether the session has uncompacted tokens or existing compressed blocks. When the condition is met, the tool's `ToolDefinition` is pushed to `definitions` but not to `tool_map`. The prompt composition layer uses `catalog.contains()` (which checks `tool_map`) to gate the Tool Philosophy compress paragraph, so the tool is never advertised to the model despite being technically in the definitions list.

The existing execution path works correctly: `prompt_runner` intercepts compress calls by name before delegating to `SessionToolCatalog::execute()`. The bug is purely in the advertisement/availability side.

`activate_skill` solved a similar problem (per-session definition variance) by registering a dummy `FunctionTool` in the `ToolRegistry` at runtime construction and intercepting/rebuilding the definition in `SessionToolCatalog::new()`. Compress doesn't need the definition-rebuild trick because its schema is static — it just needs to be in the registry.

## Goals / Non-Goals

**Goals:**
- Fix the `compression_available=false` bug by making compress a first-class registered tool
- Eliminate the conditional `compress_tool_available_for_session()` logic and its inconsistent definitions/tool_map split
- Simplify the prompt composition pipeline by removing the `compression_available` conditional
- Ensure empty sessions get a clean rejection if the model calls compress (already works via range validation)

**Non-Goals:**
- Changing how compaction executes (range validation, block creation, pressure thresholds)
- Changing the `/compact` slash command flow
- Addressing the secondary client-side `context_window_hint=None` issue mentioned in GitHub #41
- Optimizing visible ID overhead in early-session transcripts (negligible, can revisit if needed)

## Decisions

### Decision: Register compress in ToolRegistry unconditionally

Compress is registered via a new `register_compress_tool()` method on `IronRuntime`, called unconditionally in both `new()` and `from_handle()`. The tool uses a dummy `FunctionTool` handler (real execution lives in `prompt_runner`), following the same pattern as `register_activate_skill_tool()`.

**Rationale:** Compress has a static schema and no configuration dependencies. Unlike `activate_skill` (which varies its enum values per session), compress doesn't need per-session definition rebuilding. Registration in `ToolRegistry` means it enters both `definitions` and `tool_map` through the normal iteration path in `SessionToolCatalog::new()`.

**Alternatives considered:**
- `ToolInfo::Runtime` variant: Would fix the bug but adds a new enum arm for one tool, with a dead match arm in `execute()`. More ceremony for no additional benefit.
- Conditional registration (only when context exists): Defeats the purpose — would need session-aware registration timing, which the ToolRegistry doesn't support.
- Fix the map/definitions split in place (add to tool_map): Minimal fix but preserves the conditional complexity and the special-case path in `SessionToolCatalog`.

### Decision: Remove `compression_available` plumbing entirely

Since compress is always in the catalog, the `compression_available: bool` field propagated through `request_builder::PromptContext` → `prompt_runner` → `prompt::SystemPromptInputs` is always `true` and can be removed. The Tool Philosophy section always renders the compress paragraph.

**Rationale:** A field that's always true is noise. Removing it simplifies the prompt composition pipeline and removes a source of cache invalidation complexity in `SystemPromptFingerprint`.

### Decision: Always include visible IDs in transcripts

The `include_visible_ids` conditional in `prompt_runner` is currently gated on compress being in definitions. Since compress is always present, visible IDs are always included. Remove the conditional.

**Rationale:** The overhead is a `<id>\n` prefix per message. Negligible, and the simplification is worth it. Can be optimized later if needed.

## Risks / Trade-offs

- **[Visible ID overhead in early sessions]** → All turns from turn 1 will carry `<id>` prefixes even when there's nothing to compress. Overhead is ~20 bytes per message. Acceptable; revisit if token budget analysis shows impact.
- **[Model calling compress on empty session]** → Model receives `"Unknown start ID"` rejection from `resolve_range()`. This is already the behavior for invalid IDs. The model should learn not to call compress without ranges, but even if it does, the error is clean and non-destructive.
- **[No config toggle]** → Compress cannot be disabled. This is intentional — context compaction is fundamental to functional long-running sessions. If a toggle becomes necessary later, it can be added to `ContextManagementConfig`.
