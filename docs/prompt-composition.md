# Prompt Composition

`iron-core` builds provider instructions from ordered prompt layers instead of a single opaque string. This page describes the layer model, configuration, and portability rules.

## Layer Order

Provider instructions are assembled in this fixed order:

| # | Layer | Owner | Portable |
|---|-------|-------|----------|
| 1 | Baseline prompt | `iron-core` | always present |
| 2 | Repository instruction files | runtime | no |
| 3 | Additional instruction files | caller | no |
| 4 | Additional inline instruction blocks | caller | no |
| 5 | Session instructions (`session.instructions`) | caller | yes |
| 6 | Runtime context | `iron-core` | no |

Absent layers are omitted without affecting the relative order of remaining layers. At minimum, every request contains the baseline prompt (layer 1) and runtime context (layer 6).

## Baseline Prompt

The `iron-core` baseline prompt provides generic agent behavior rules: tool use guidelines, protected-resource rules, and (when enabled) `python_exec` guidance. It is always present as the first layer and cannot be suppressed or reordered.

## Repository Instruction Loading

Repository instruction files (such as `AGENTS.md` or `CLAUDE.md`) are resolved at session creation time and stored on the durable session for the session's lifetime. The loaded content does not change if the file changes on disk.

### Configuration

```rust
use iron_core::prompt::config::{
    PromptCompositionConfig, RepoInstructionConfig, RepoInstructionFamily,
};

let repo_config = RepoInstructionConfig::new()
    .with_enabled(true)
    .with_family(RepoInstructionFamily::PreferAgentsFallbackClaude)
    .with_scopes(vec![
    std::path::PathBuf::from("/home/user/.config"),
    std::path::PathBuf::from("/project/root"),
]);

let prompt_config = PromptCompositionConfig::new()
    .with_repo_instructions(repo_config);

let config = iron_core::Config::new()
    .with_prompt_composition(prompt_config);
```

### File-family selection

At each configured scope, `iron-core` looks for files in the order defined by the configured family:

| Family | Candidate order |
|--------|----------------|
| `PreferAgentsFallbackClaude` (default) | `AGENTS.md`, then `CLAUDE.md` |
| `AgentsOnly` | `AGENTS.md` only |
| `ClaudeOnly` | `CLAUDE.md` only |

When both files exist at the same scope, only the first match is used. `AGENTS.md` is preferred over `CLAUDE.md` by default.

### Scope ordering

Scopes are processed in the order given. Use general-to-specific ordering (for example, user-level config before project working directory) so that broader instructions appear first in the composed prompt.

### Disabling

Set `with_enabled(false)` to skip repository instruction loading entirely.

## Additional Instruction Files

Callers can supply additional files to load into the prompt. These are loaded at session creation and injected after repository instructions.

```rust
let prompt_config = PromptCompositionConfig::new()
    .with_additional_files(vec![
    std::path::PathBuf::from("custom-instructions.md"),
]);
```

If a selected file cannot be read, the session is still created but the file is skipped. Source metadata identifies which files were loaded.

## Additional Inline Instruction Blocks

Callers can inject arbitrary inline text blocks. These are appended after additional files and before session instructions.

```rust
let prompt_config = PromptCompositionConfig::new()
    .with_additional_inline(vec![
    "Always respond in JSON.".to_string(),
    "Prefer concise answers.".to_string(),
]);
```

## Session Instructions

Session instructions are the caller-owned, portable instruction string set on a session. They are the only instruction layer included in handoff bundles.

```rust,ignore
let session = connection.create_session()?;
session.set_instructions("You are a helpful assistant.");
```

## Runtime Context

`iron-core` injects a runtime context section at the end of every composed prompt. This section communicates authoritative environment facts:

- Current date and platform
- Working directory and workspace roots
- Whether the working directory is inside a git repository (when known)
- Default approval strategy
- Capability summaries for registered tools
- Protected-resource paths and policy
- Embedded Python availability, restrictions, and limits (when enabled)

Runtime context is recomputed on each request, so it always reflects the current state.

## Protected Resources

By default, `iron-core` marks `.git`, `.ssh`, `.env`, and `.envrc` as protected resources. The baseline prompt and runtime context both instruct the model that protected resources must never be read or modified, whether through direct tool use or scripted access.

To customize the protected resource list:

```rust
let prompt_config = PromptCompositionConfig::new()
    .with_protected_resources(vec![
    ".git".to_string(),
    ".ssh".to_string(),
    ".env".to_string(),
    "secrets/".to_string(),
]);
```

## Handoff Portability

When a session is exported as a handoff bundle:

- **Included**: session instructions (layer 5) and the conversation transcript.
- **Excluded**: repository instruction files, additional instruction files, additional inline blocks, and runtime context. These are runtime-owned and will be re-resolved in the receiving environment.

This ensures handoff bundles remain portable across machines and do not carry stale or environment-specific instruction content.

## Host Integration Example

`iron-tui` configures repository instruction scopes from the user's `$HOME/.config` directory and the current working directory:

```rust
let mut repo_scopes = Vec::new();
if let Ok(home) = std::env::var("HOME") {
    let config_dir = std::path::PathBuf::from(home).join(".config");
    if config_dir.is_dir() {
        repo_scopes.push(config_dir);
    }
}
if let Ok(cwd) = std::env::current_dir() {
    repo_scopes.push(cwd);
}
```

## API Reference

The key types live in `iron_core::prompt::config`:

- `PromptCompositionConfig` — top-level config for all prompt layers.
- `RepoInstructionConfig` — controls repository instruction loading, family, and scopes.
- `RepoInstructionFamily` — file-family selection (`PreferAgentsFallbackClaude`, `AgentsOnly`, `ClaudeOnly`).
- `RepoInstructionSource` — provenance record for a loaded instruction file.
- `AdditionalInstructionFile` — a loaded additional file with its content.
- `RepoInstructionPayload` — the resolved payload stored on the durable session.

Build API docs with:

```bash
cargo doc -p iron-core --no-deps
```
