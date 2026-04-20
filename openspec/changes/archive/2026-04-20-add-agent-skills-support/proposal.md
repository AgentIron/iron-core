## Why

`iron-core` currently has no structured mechanism for injecting specialized, reusable behavioral guidance into agent sessions. Agents running on `iron-core` must either embed all expertise in static prompts or rely on plugins, which are executable WASM tools with their own complexity. Agent Skills provide a lightweight alternative: declarative instruction sets that models can discover, activate on demand, and apply to tasks without requiring plugin infrastructure. Adding skills support makes `iron-core` compatible with the emerging Agent Skills ecosystem while keeping the implementation minimal and source-agnostic.

## What Changes

- Add a unified skill catalog with pluggable discovery sources (filesystem and client-provided)
- Implement standard filesystem scanning for `.agents/skills/` directories (project and user scopes)
- Add session-scoped skill activation with deduplication and compaction protection
- Introduce a dedicated `activate_skill` model tool with progressive disclosure semantics
- Extend prompt composition to include activated skill instructions in subsequent turns
- Add runtime/facade APIs for skill discovery, activation, and inspection
- Add trust gating for project-level filesystem skills

## Capabilities

### New Capabilities
- `agent-skills`: Skill discovery, cataloging, activation, and session-scoped injection into prompts

### Modified Capabilities
- `context-compaction`: Extend compaction rules to protect activated skill content from being summarized away

## Impact

- `src/runtime.rs`: Add skill source registration and catalog lifecycle
- `src/facade.rs`: Expose skill listing and activation APIs
- `src/prompt/`: Extend prompt assembly to include active skill instructions
- `src/context/compaction.rs`: Add skill content protection markers
- `src/mcp/session_catalog.rs` or new module: Skill source abstractions and filesystem scanning
- New test suites for skill discovery, activation, and prompt injection
- No breaking changes to existing APIs or session formats
