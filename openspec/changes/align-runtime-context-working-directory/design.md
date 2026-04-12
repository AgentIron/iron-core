## Context

`RuntimeContextRenderer::render()` already accepts both a singular `working_dir` and a list of `workspace_roots`, but `request_builder.rs` currently derives `working_dir` from `std::env::current_dir()` and passes an empty workspace root list. Meanwhile the built-in filesystem and shell tools are governed by `config.builtin.allowed_roots`, which often represent the actual user project roots. In embedded environments, the process current directory may point to an application bundle or launcher directory rather than the user project, so the prompt context diverges from actual tool behavior.

The design constraint is to keep the model’s runtime context aligned with the actual filesystem boundary enforced by tools without introducing redundant configuration concepts that can drift.

## Goals / Non-Goals

**Goals:**
- Make the displayed runtime working directory reflect configured builtin tool roots instead of the process current directory when roots are configured.
- Surface configured roots as workspace roots in runtime context rendering.
- Preserve a sensible fallback to process current directory when no configured roots exist.
- Keep prompt context and tool policy aligned from a single source of truth.

**Non-Goals:**
- Adding a new top-level `Config::working_directory` field.
- Changing built-in tool root validation semantics.
- Designing a full multi-workspace prompt model beyond exposing the current root list.

## Decisions

### Use builtin allowed roots as the source of truth for runtime context location
Derive the primary displayed `working_dir` from `config.builtin.allowed_roots.first()` and pass the full `allowed_roots` set as `workspace_roots`.

This is preferable to adding a separate `working_directory` config field because it avoids introducing another independently configurable location concept that could drift from actual tool policy.

### Fall back to process current directory only when no allowed roots are configured
If `allowed_roots` is empty, use `std::env::current_dir()` as a fallback so prompt rendering remains functional.

This is preferable to requiring configured roots in all cases because it preserves existing behavior for minimal/default configurations.

### Keep the primary working directory concept even with multiple roots
When multiple roots exist, display the first root as the primary working directory and list all roots under workspace roots.

This is preferable to removing the singular working directory field because the renderer and prompt format already expect one, and the first configured root provides a stable primary location.

## Risks / Trade-offs

- [The first allowed root may not always be the user’s conceptual primary project root] → Document the ordering significance and expose all roots in `workspace_roots` so the model still sees the full set.
- [Some configurations may rely on current process CWD semantics] → Preserve fallback to current directory only when no roots are configured.
- [Prompt context may still be ambiguous in true multi-root workflows] → This change narrows the mismatch substantially without solving broader workspace semantics; revisit later if multi-root prompting needs richer treatment.

## Migration Plan

- Update request builder prompt composition to derive working directory and workspace roots from builtin allowed roots.
- Add prompt/runtime context tests covering configured roots and fallback behavior.
- Verify existing built-in tool and prompt composition tests still pass.

## Open Questions

- Should runtime context explicitly label the first allowed root as the “primary workspace root” to make the ordering assumption clearer to the model?
