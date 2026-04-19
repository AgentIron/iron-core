## Why

`RuntimeContextRenderer::render()` accepts a `working_dir` parameter, but `request_builder.rs` currently passes `std::env::current_dir()` instead of the configured project root. In embedded applications this often points to the app bundle directory rather than the user’s project, so the model is told the wrong working directory and makes incorrect path assumptions.

## What Changes

- Derive the runtime context working directory from the configured builtin tool roots rather than the process current directory.
- Pass the configured allowed roots through to runtime context rendering so prompt context and actual tool policy stay aligned.
- Use the first configured allowed root as the primary displayed working directory, with fallback to process current directory only when no roots are configured.
- Add tests covering prompt/runtime context composition for configured roots and fallback behavior.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `session-scoped-mcp-support`: refine runtime prompt context generation so the displayed working directory and workspace roots reflect configured tool roots rather than the process current directory.

## Impact

- Affected code: `src/request_builder.rs`, `src/prompt/runtime_context.rs`, configuration-driven prompt composition tests, and any tests that assert runtime context rendering.
- Affected systems: prompt/runtime context generation for model requests.
- API/protocol impact: no public protocol changes; prompt context becomes aligned with configured working roots.
