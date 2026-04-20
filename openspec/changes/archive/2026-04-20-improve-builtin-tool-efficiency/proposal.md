## Why

Built-in filesystem and search tools in `iron-core` currently consume substantially more tool calls and prompt tokens than comparable coding agents for similar repository exploration and editing tasks. The gap appears to come from a combination of weaker tool capabilities, verbose JSON result payloads, and insufficient prompt/tool guidance about efficient tool selection, batching, and bounded reads.

## What Changes

- Replace the current custom `glob`/`grep` traversal and matching implementation with a native Rust integration built on ripgrep ecosystem crates (`ignore`, `grep-searcher`, `grep-regex`) rather than shelling out to the `rg` binary.
- Keep structured internal tool results for runtime/tooling use, but add a shared rendering layer that converts those results into compact model-facing text formats instead of repetitive JSON object arrays.
- Expand built-in tool descriptions so they explicitly teach efficient usage patterns: when to batch, when to use each tool, when not to use shell, and how to avoid repeated narrow reads.
- Strengthen the baseline/runtime prompt so the model is directly instructed to minimize output tokens, batch independent tool calls, avoid unnecessary preambles/postambles, and prefer larger bounded reads over many tiny slices.
- Add missing editing/search capabilities that currently force extra calls, including `edit.replace_all`, a new atomic `multiedit` tool for one file, richer grep modes, and directory-aware reading/listing behavior.
- Improve truncation and continuation guidance so large outputs tell the model exactly how to continue with `read` offsets or refined search calls instead of forcing exploratory retries.
- Align search semantics with user expectations by adding proper glob matching, `.gitignore`/hidden-file policy handling, explicit-path overrides, symlink-following within allowed roots, binary detection, and richer filtering options.
- Switch model-visible tool rendering to the new compact format as a deliberate hard break while preserving absolute-path structured internal results.
- Preserve portability by avoiding host `rg` binary dependencies and keeping the search stack cross-platform and library-embedded.

## Capabilities

### New Capabilities
- `builtin-tool-efficiency`: Defines efficient built-in search, read, edit, render, and prompt behavior so repository exploration and file modification require fewer calls and fewer tokens while preserving portability and safety.

### Modified Capabilities
- None.

## Impact

- Affected code: `src/builtin/search.rs`, `src/builtin/file_ops.rs`, `src/builtin/helpers.rs`, `src/builtin/config.rs`, `src/builtin/registration.rs`, shared tool-result rendering code, `src/prompt/baseline.rs`, `src/prompt/runtime_context.rs`, `src/request_builder.rs`, and related tests.
- Dependencies: adds ripgrep library crates (`ignore`, `grep-searcher`, `grep-regex`) and their transitive dependencies; does not require the external `rg` executable.
- Behavior: changes model-visible tool descriptions and rendered tool result formats for built-in tools via a hard switch, expands search/edit features, and updates prompt guidance for tool efficiency.
- Portability: remains cross-platform because the search implementation is embedded Rust code rather than a shell command dependency.
