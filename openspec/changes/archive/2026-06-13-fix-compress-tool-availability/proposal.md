## Why

The `compress` tool is conditionally added to `SessionToolCatalog::definitions` but not to `tool_map`. The prompt composition layer checks availability via `catalog.contains("compress")`, which queries `tool_map`, so `compression_available` is always `false`. This prevents the Tool Philosophy prompt section from advertising compress to the model, blocking model-initiated compression entirely (GitHub issue #41).

The root cause is a design mismatch: compress is session-conditionally synthesized in `SessionToolCatalog::new()` rather than being a normal registered tool. This introduces a parallel path that must be kept in sync with the standard tool registration flow, and it's already out of sync.

## What Changes

- Register the `compress` tool in `ToolRegistry` at runtime construction time (like `activate_skill`), making it always available rather than conditionally synthesized per-session
- Remove `compress_tool_available_for_session()` and the conditional synthesis logic in `SessionToolCatalog::new()`
- Remove the `compression_available` plumbing from `prompt_runner` → `request_builder` → `prompt/system` — the tool is always present, so the conditional is unnecessary
- Always render the compress paragraph in Tool Philosophy
- Always include visible IDs in transcripts (compress being in catalog naturally drives this)
- Model calls to compress on an empty session get a clean `"Unknown start ID"` rejection from existing range validation

## Capabilities

### New Capabilities

(None — this is a fix, not a new feature)

### Modified Capabilities

- `context-compaction`: The compress tool availability model changes from session-conditional synthesis to unconditional registration. Requirement-level behavior for how compaction executes is unchanged; the change is in how the tool is surfaced to the model.

## Impact

- **`src/runtime.rs`**: Add `register_compress_tool()` method; call in `new()` and `from_handle()`
- **`src/mcp/session_catalog.rs`**: Remove `compress_tool_available_for_session()` and the conditional definitions push in `new()`
- **`src/prompt_runner.rs`**: Remove `compression_available` variable and `include_visible_ids` conditional; simplify to always-on
- **`src/prompt/system.rs`**: Remove `compression_available` from `SystemPromptInputs` and `SystemPromptFingerprint`; always render compress paragraph
- **`src/request_builder.rs`**: Remove `compression_available` from `PromptContext`
- **Tests**: Fix `catalog_exposes_compress_when_session_has_context` (now always exposed); add empty-session rejection test
