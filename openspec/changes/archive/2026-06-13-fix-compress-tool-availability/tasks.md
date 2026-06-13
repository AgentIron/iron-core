## 1. Register compress tool in ToolRegistry

- [x] 1.1 Add `register_compress_tool()` method to `IronRuntime` in `src/runtime.rs`, following the pattern of `register_activate_skill_tool()`. Register a `FunctionTool` with `CompressTool::definition()` and a dummy handler that returns a runtime error.
- [x] 1.2 Call `register_compress_tool()` in `IronRuntime::new()` (after the `skills.enabled` block, unconditionally)
- [x] 1.3 Call `register_compress_tool()` in `IronRuntime::from_handle()` (same location as in `new()`)

## 2. Remove conditional compress synthesis from SessionToolCatalog

- [x] 2.1 Delete the `compress_tool_available_for_session()` method from `src/mcp/session_catalog.rs`
- [x] 2.2 Delete the conditional block in `SessionToolCatalog::new()` (lines 178-184) that checks `compress_tool_available_for_session` and pushes `CompressTool::definition()` to definitions

## 3. Remove compression_available plumbing from prompt composition

- [x] 3.1 Remove the `compression_available` variable and its `catalog.contains()` computation from `src/prompt_runner.rs` (around line 185-188)
- [x] 3.2 Remove `compression_available` from all `PromptContext` constructions in `src/prompt_runner.rs` (search for all usages)
- [x] 3.3 Remove `compression_available` field from `PromptContext` in `src/request_builder.rs`
- [x] 3.4 Remove `compression_available` field from `SystemPromptInputs` in `src/prompt/system.rs`
- [x] 3.5 Remove `compression_available` from `SystemPromptFingerprint::from_inputs()` in `src/prompt/system.rs`
- [x] 3.6 Simplify `render_tool_philosophy()` in `src/prompt/system.rs`: remove the `compression_available` parameter, always render the compress paragraph
- [x] 3.7 Update all call sites that pass `compression_available` to `render_tool_philosophy` or `SystemPromptInputs`

## 4. Simplify visible IDs to always-on

- [x] 4.1 In `src/prompt_runner.rs`, replace the `include_visible_ids` conditional (definitions-based check) with `true` — always pass `true` to `to_transcript_with_visible_ids()`

## 5. Clean up debug influence events

- [x] 5.1 In `src/prompt_runner.rs`, update the `CompactionAvailability` debug influence event to always show `effect=Added` (remove the `compression_available` conditional that could produce `Suppressed`)
- [x] 5.2 Simplify or remove the `compression_available` variable used in the debug event formatting (line ~358-359)

## 6. Fix and add tests

- [x] 6.1 Fix `catalog_exposes_compress_when_session_has_context` test in `src/mcp/session_catalog.rs`: assert `contains("compress") == true` and `get_definition("compress").is_some()` for both empty and non-empty sessions
- [x] 6.2 Add test: model call to compress on a fresh session returns a clean rejection error (range validation fails with "Unknown start ID")
- [x] 6.3 Verify existing compress tool tests in `tests/context_management_tests.rs` still pass (they call `CompressTool::execute` directly and should be unaffected)
- [x] 6.4 Run `cargo test` and `cargo clippy` to verify no regressions
