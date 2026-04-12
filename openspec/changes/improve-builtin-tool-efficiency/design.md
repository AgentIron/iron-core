## Context

`iron-core` currently exposes built-in `glob`, `grep`, `read`, `write`, and `edit` tools through compact Rust implementations and a minimal baseline prompt. That minimalism keeps the runtime simple, but it pushes too much discovery burden onto the model. Compared with other coding agents, the current built-in tool stack has three structural weaknesses:

1. **Search quality and capability are limited.** The current recursive directory walkers do not implement real glob semantics, do not respect ignore-file conventions, cannot expose richer search modes, and offer only a narrow regex/file-filter surface.
2. **Model-facing results are too verbose in the wrong places.** Tool outputs use repetitive JSON envelopes that cost tokens on every turn but do not actually teach the model how to continue efficiently.
3. **Prompt and tool descriptions do not teach efficient behavior.** The model is not explicitly instructed to batch independent operations, avoid tiny reads, prefer specialized tools, or keep user-facing responses terse.

This change is cross-cutting: it touches built-in tools, model-visible schemas and descriptions, truncation behavior, prompt composition, and tests. It also introduces new dependencies from ripgrep's Rust ecosystem. Because `iron-core` is a reusable library rather than a CLI application, the design must avoid requiring the host to install an external `rg` binary.

## Goals / Non-Goals

**Goals:**
- Reduce the number of tool calls needed for common repository search and edit workflows.
- Reduce model-visible token consumption per tool call without reducing actionable information.
- Replace custom search traversal/matching logic with native Rust ripgrep ecosystem crates embedded into the library.
- Improve built-in tool descriptions and prompt guidance so the model learns more efficient tool usage patterns.
- Add the highest-value missing capabilities that currently force repeated exploratory or repetitive edit calls.
- Preserve cross-platform portability and avoid external runtime dependencies.

**Non-Goals:**
- Building a full semantic/codebase-search system in this change.
- Introducing external services, indexes, embeddings, or background daemons.
- Reworking the entire tool registry or approval model.
- Matching every feature from ForgeCode/OpenCode/Kilocode in a single change.
- Optimizing user-facing prose generation beyond the tool-efficiency guidance needed for repository work.

## Decisions

### Use ripgrep library crates, not the `rg` binary
Adopt `ignore`, `grep-searcher`, and `grep-regex` as direct dependencies for built-in search.

This is preferable to shelling out to `rg` because `iron-core` is a library and cannot safely assume the executable is installed or version-compatible on every host. It is also preferable to keeping `regex` plus a custom walker because the current implementation already reimplements globbing and traversal poorly, while the ripgrep crates provide the same cross-platform traversal and search primitives used by ripgrep itself.

Alternatives considered:
- **Shell out to `rg`** → rejected because it introduces a host dependency and version/platform fragility.
- **Use only `ignore` plus existing `regex`** → rejected because it still leaves `iron-core` reimplementing too much search behavior and context handling.

### Separate model-facing output format from internal structured state with a shared renderer
Keep structured internal tool results for tests, runtime logic, and future tooling, but introduce a shared tool-result rendering layer that converts those structured results into compact model-facing text blocks for search/read/edit/write operations.

This is preferable to continuing with JSON because the model pays repeated token overhead for every key/value wrapper (`path`, `line_number`, `line`, `count`, `meta`) across every call. A shared renderer also keeps tools focused on semantics instead of presentation and gives `iron-core` one place to standardize truncation messages, path rendering, and compact success/error summaries.

Alternatives considered:
- **Keep pure JSON everywhere** → rejected because it is the highest recurring token tax in the current design.
- **Render inside each tool** → rejected because it entangles tool semantics with presentation and makes consistency harder.
- **Adopt heavy XML for every tool** → partially rejected; light structural wrappers may help, but the primary objective is compactness rather than markup for its own sake.

### Use root-relative rendered paths and absolute internal paths
Structured internal results should continue to store absolute paths. The shared renderer should emit root-relative paths whenever a path falls under a configured workspace/allowed root and fall back to absolute paths otherwise.

When multiple roots match, the renderer should choose the most specific matching root so rendered paths are as short as possible. This preserves precise internal semantics while reducing repeated token cost in model-visible output.

Alternatives considered:
- **Absolute paths everywhere** → rejected because they waste tokens and reduce scanability.
- **Relative paths everywhere, including internals** → rejected because absolute internal paths are easier to reason about for tooling and safety checks.

### Treat tool descriptions as operational guidance, not just API summaries
Rewrite built-in tool descriptions so they encode usage policy and efficiency guidance: when to batch calls, when to prefer a different tool, what parameter defaults mean, and how to continue after truncation.

This is preferable to keeping terse descriptions because other successful agents achieve lower call counts partly by teaching the model how the tools are intended to be used. The model should not need exploratory calls to discover basic operational semantics.

Alternatives considered:
- **Keep descriptions minimal and rely on baseline prompt only** → rejected because tool-local guidance is most effective when attached to the tool being selected.

### Strengthen the baseline/runtime prompt with explicit efficiency rules
Update the baseline prompt and runtime context to tell the model to minimize output tokens, batch independent tool calls, avoid preambles/postambles, prefer larger bounded reads over many small slices, and prefer specialized tools over shell for file exploration.

This is preferable to preserving the current minimal prompt because the other systems examined consistently spend some prompt budget teaching efficient operational behavior and recover that cost in fewer downstream calls and smaller assistant replies.

Alternatives considered:
- **Leave prompt minimal and change tools only** → rejected because tool capability changes alone will not teach the model to exploit those capabilities efficiently.

### Add the highest-leverage missing capabilities first
Add `edit.replace_all`, a dedicated atomic `multiedit` tool for one file, allow `read` to return directory listings, and enrich `grep` with more expressive modes before pursuing more ambitious search features.

This is preferable to attempting semantic search because the biggest current inefficiencies come from repeated exact-match edits, directory-exploration retries, and limited regex search controls. These improvements reduce calls immediately without introducing new services or heavy architecture.

Alternatives considered:
- **Add semantic search now** → deferred because it has a larger design surface and is not required to fix the current built-in efficiency gap.

### Define clear ignore and visibility semantics
Built-in search should follow a predictable policy for hidden files, ignore files, symlinks, and binary files.

Default discovery behavior:
- honor `.gitignore` and `.ignore`
- exclude hidden files/directories by default
- skip binary content for text search
- follow symlinks by default only when the resolved target remains within allowed roots
- detect cycles and deduplicate repeated canonical targets

Explicit intent behavior:
- explicitly scoped paths override ignore/hidden filtering but never allowed-root constraints
- explicit hidden/ignored glob or include patterns likewise override ignore/hidden filtering
- explicit scope does not force binary content to be searched as text

Direct inspection behavior:
- `read(directory)` is an inspection tool rather than a discovery traversal and should list actual directory contents, including hidden entries, subject only to allowed-root constraints

This is preferable to today's hard-coded dotfile skipping because the current behavior is both underspecified and inconsistent with user expectations in modern repositories.

Alternatives considered:
- **Search everything always** → rejected because it increases noise and token use.
- **Keep blind dotfile skipping only** → rejected because it is too coarse and ignores repository ignore semantics.

### Roll out in phases so search correctness lands before prompt tuning depends on it
Implement the change in phases: search engine replacement and result format first, then capability additions, then prompt/description tuning, then comparative verification.

This is preferable to a single monolithic rewrite because prompt guidance should be written against stable tool semantics, and search regressions must be isolated from prompt regressions during testing.

### Make the model-facing format change a deliberate hard switch
The rendered form shown to the model should switch directly to the new compact format without a compatibility mode. The project is early enough that clean interfaces are more valuable than preserving the current rendered format.

This is preferable to carrying dual renderers or feature flags because internal structured results already preserve the implementation discipline needed by tests and tooling, while a compatibility layer would add complexity without much value at this stage.

Alternatives considered:
- **Introduce a transition flag or dual format support** → rejected because the project is early and the additional complexity is not justified.

## Risks / Trade-offs

- **[Model-visible output format changes may break tests or downstream assumptions]** → Mitigation: update built-in tool tests to assert stable textual contracts and audit any code that assumes JSON keys in tool results.
- **[ripgrep crate integration may surface behavior changes around hidden files, ignores, symlinks, and binary detection]** → Mitigation: codify default policy in specs and add explicit tests for hidden files, `.gitignore`, binaries, and scoped searches.
- **[Prompt changes could overfit one model family while regressing others]** → Mitigation: keep efficiency guidance concise, operational, and model-agnostic; verify across the provider abstractions already exercised in tests.
- **[Adding richer tool descriptions increases up-front prompt tokens]** → Mitigation: accept modest up-front growth when it reduces repeated exploratory calls and result-token overhead across the session.
- **[Multi-edit and replace-all features increase mutation power]** → Mitigation: preserve approval requirements for write/edit operations and require exact-match semantics per edit item.
- **[Directory-aware read may blur the distinction between read and glob]** → Mitigation: keep directory read intentionally lightweight and position `glob` as the pattern-search tool while `read` remains the direct inspection tool.
- **[The new search stack increases dependency surface and binary size]** → Mitigation: use only embedded Rust crates, document the dependency rationale, and avoid optional external process fallbacks.
- **[Following symlinks by default may widen traversal or create duplicate/looping paths]** → Mitigation: only follow symlinks whose canonical targets remain within allowed roots, detect cycles, and deduplicate by canonical path.

## Migration Plan

1. Introduce ripgrep ecosystem dependencies and add targeted unit/integration tests that define the new search semantics before removing the old implementation.
2. Introduce the shared tool-result rendering layer and define rendered path, truncation, and success/error summary contracts.
3. Rebuild `glob`/`grep` on top of the ripgrep stack and switch model-facing outputs to the new compact rendered contracts.
4. Add `read` directory listing support, `edit.replace_all`, and atomic `multiedit` support with approval-preserving behavior.
5. Update tool descriptions, truncation messages, baseline prompt, and runtime context guidance to reflect the new capabilities.
6. Validate behavior and perceived efficiency manually in the AgentIron GUI in addition to automated correctness tests.
7. If regressions appear, revert prompt/description changes independently of the search-engine change because search behavior and prompt guidance should land in separable commits.

## Open Questions

- What is the cleanest shared renderer abstraction boundary so tool implementations do not become tightly coupled to provider-facing formatting code?
- Should grep context-line flags remain deferred after v1, or become the next incremental extension once the compact renderer and ripgrep integration are stable?
