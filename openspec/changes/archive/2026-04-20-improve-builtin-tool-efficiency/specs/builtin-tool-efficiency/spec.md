## ADDED Requirements

### Requirement: Built-in search SHALL use embedded ripgrep ecosystem libraries
The built-in `glob` and `grep` tools SHALL be implemented with embedded Rust search/traversal libraries from the ripgrep ecosystem rather than custom recursive walkers or external `rg` process execution.

#### Scenario: Search runs without host ripgrep binary
- **WHEN** the runtime is built and executed on a host that does not have the `rg` executable installed
- **THEN** built-in `glob` and `grep` SHALL still function
- **AND** the runtime SHALL not require shelling out to an external ripgrep process

#### Scenario: Search respects repository ignore behavior by default
- **WHEN** built-in search is executed in a repository containing ignore files and hidden directories
- **THEN** search traversal SHALL apply a documented default policy for ignore-file handling, hidden files, symlinks, and binary content
- **AND** the policy SHALL reduce noisy traversal compared with searching every file blindly

#### Scenario: Search follows symlinks within allowed roots
- **WHEN** built-in search encounters a symlink whose canonical target remains inside allowed roots
- **THEN** traversal SHALL follow that symlink
- **AND** the implementation SHALL prevent traversal cycles and duplicate canonical targets from producing uncontrolled repeated results

### Requirement: Built-in search SHALL provide correct glob and regex semantics
The built-in `glob` and `grep` tools SHALL implement real glob and regex behavior instead of substring-based approximations.

#### Scenario: Glob pattern matching uses actual glob semantics
- **WHEN** the model calls `glob` with patterns such as `**/*.rs` or `src/**/*.ts`
- **THEN** the results SHALL be determined by true glob matching semantics
- **AND** the result set SHALL not rely on raw substring containment to approximate matching

#### Scenario: Grep supports richer filtering and matching controls
- **WHEN** the model calls `grep` with supported search controls such as include filtering, case-insensitive matching, multiline matching, or bounded result modes
- **THEN** the tool SHALL apply those controls consistently
- **AND** the response SHALL make the active search behavior legible to the model

#### Scenario: Explicit patterns can target ignored or hidden content
- **WHEN** the model provides an explicit scoped path, hidden-targeting glob pattern, or hidden/ignored `grep.include` pattern within allowed roots
- **THEN** built-in discovery filtering SHALL not suppress that explicitly targeted content
- **AND** allowed-root constraints SHALL still be enforced

### Requirement: Built-in search results SHALL minimize model-facing token overhead
The built-in `glob` and `grep` tools SHALL return compact model-facing outputs that avoid repetitive per-item JSON object wrappers while preserving enough structure for the model to interpret the result efficiently.

#### Scenario: Grep returns compact grouped output
- **WHEN** `grep` finds matches across one or more files
- **THEN** the tool SHALL return a compact textual representation of matches grouped or ordered for efficient scanning
- **AND** the output SHALL avoid repeating JSON keys for every match row

#### Scenario: Glob returns compact path output
- **WHEN** `glob` finds matching filesystem paths
- **THEN** the tool SHALL return the matched paths in a compact model-facing format
- **AND** the output SHALL avoid wrapping every path in repeated JSON field names

#### Scenario: Structured internal results remain available
- **WHEN** a built-in tool completes successfully or with a recoverable partial result
- **THEN** the runtime SHALL preserve structured internal result data for tooling/runtime use
- **AND** the model-facing output SHALL be produced by a shared renderer rather than by exposing the raw structured payload directly

#### Scenario: Rendered paths are root-relative when possible
- **WHEN** a rendered result path falls under a configured workspace or allowed root
- **THEN** the model-facing output SHALL render that path relative to the most specific matching root
- **AND** absolute paths SHALL remain available in structured internal results

#### Scenario: Glob output includes directories and files in deterministic order
- **WHEN** `glob` returns matches
- **THEN** files and directories SHALL both be eligible results
- **AND** directories SHALL be visually distinguishable from files in rendered output
- **AND** rendered results SHALL be ordered lexicographically by rendered path

#### Scenario: Grep output ordering is deterministic
- **WHEN** `grep` returns content matches across multiple files
- **THEN** rendered file groups SHALL be ordered lexicographically by rendered path
- **AND** matches within each file SHALL be ordered by ascending line number

### Requirement: Built-in tool truncation SHALL guide efficient follow-up actions
When built-in tools truncate output, the result SHALL tell the model exactly how to continue efficiently.

#### Scenario: Search truncation provides refinement guidance
- **WHEN** a built-in search result is truncated because of result-count or output-size limits
- **THEN** the tool result SHALL indicate that the output was truncated
- **AND** the result SHALL instruct the model to refine the search or use bounded follow-up reads rather than guess a recovery strategy

#### Scenario: File read truncation provides continuation guidance
- **WHEN** a built-in file read is truncated because of configured limits
- **THEN** the tool result SHALL identify how to continue reading later sections using the read tool's continuation parameters

#### Scenario: Truncation footer format is consistent
- **WHEN** a built-in tool renders truncated output
- **THEN** the rendered output SHALL append a standardized truncation footer shape
- **AND** the footer text SHALL include tool-specific recovery guidance

### Requirement: Read SHALL support lightweight directory inspection
The built-in `read` tool SHALL support direct inspection of directories in addition to files.

#### Scenario: Reading a directory returns entries instead of an error
- **WHEN** the model calls `read` on a directory path within allowed roots
- **THEN** the tool SHALL return a lightweight listing of directory entries
- **AND** subdirectories SHALL be distinguishable from files in the result

#### Scenario: Directory inspection shows actual contents
- **WHEN** the model explicitly reads a directory path within allowed roots
- **THEN** the result SHALL include hidden entries without applying ignore-file filtering
- **AND** entries SHALL be sorted lexicographically
- **AND** entries SHALL be rendered relative to the directory being read
- **AND** `.` and `..` SHALL not be included

#### Scenario: Reading a file still returns line-oriented content
- **WHEN** the model calls `read` on a text file path within allowed roots
- **THEN** the tool SHALL return line-oriented content with stable line numbering behavior
- **AND** the result SHALL remain compatible with follow-up exact edits

### Requirement: Edit SHALL support higher-efficiency mutation patterns
The built-in editing surface SHALL reduce repetitive edit calls while preserving exactness and approval safeguards.

#### Scenario: Replace-all applies repeated exact matches in one call
- **WHEN** the model requests an exact replacement with `replace_all` enabled
- **THEN** the edit operation SHALL replace every exact occurrence in the target file within a single approved tool call

#### Scenario: Replace-all still fails on zero matches
- **WHEN** the model requests an edit with `replace_all` enabled and the target text does not exist in the file
- **THEN** the edit operation SHALL fail rather than silently succeeding with no changes

#### Scenario: Multiple edits can be applied to one file in one call
- **WHEN** the model needs to make multiple exact-match edits to one file
- **THEN** the built-in editing surface SHALL provide a dedicated `multiedit` capability for applying those edits within one tool call
- **AND** the operation SHALL fail safely if the requested edit set cannot be applied as specified

#### Scenario: Multiedit is atomic
- **WHEN** one edit item in a `multiedit` request fails validation or matching
- **THEN** the tool SHALL apply no file changes
- **AND** the failure result SHALL identify the failing edit item with enough detail for recovery

#### Scenario: Multiedit supports replace-all per edit item
- **WHEN** the model includes multiple edit items in a `multiedit` request
- **THEN** each edit item SHALL be able to opt into `replace_all` semantics independently

#### Scenario: Multiedit requires prior file awareness
- **WHEN** the model attempts to use `multiedit` on an existing file that has not been read in the current conversation
- **THEN** the tool SHALL reject the request using the same read-before-edit safety model applied to single-edit operations

#### Scenario: Mutation success output remains compact
- **WHEN** `edit`, `multiedit`, or `write` succeeds
- **THEN** the model-facing result SHALL be a compact summary rather than a rendered diff
- **AND** the summary SHALL include the target path, operation kind, and count details when useful

#### Scenario: Mutation failure output remains diagnostic
- **WHEN** `edit` or `multiedit` fails because of matching or validation problems
- **THEN** the model-facing result SHALL remain concise
- **AND** the result SHALL still provide enough diagnostic detail for the model to recover in a follow-up call

### Requirement: Grep SHALL support distinct v1 result modes
The built-in `grep` tool SHALL expose a defined first-pass set of parameters and result modes rather than an underspecified search surface.

#### Scenario: Files-with-matches mode lists file paths only
- **WHEN** the model calls `grep` with `mode = files_with_matches`
- **THEN** the rendered output SHALL list matching file paths without per-file count annotations

#### Scenario: Count mode returns total and per-file counts
- **WHEN** the model calls `grep` with `mode = count`
- **THEN** the rendered output SHALL include the total number of matches across the search
- **AND** the rendered output SHALL also include per-file counts for the returned files

#### Scenario: Content mode pagination applies to match entries
- **WHEN** the model calls `grep` with `mode = content` and uses `offset` or `head_limit`
- **THEN** pagination SHALL apply to rendered match entries rather than whole files
- **AND** the rendered output SHALL preserve file grouping for the visible subset

#### Scenario: Count mode pagination does not alter global total
- **WHEN** the model calls `grep` with `mode = count` and uses `offset` or `head_limit`
- **THEN** pagination SHALL apply to visible per-file count entries
- **AND** the total count SHALL still reflect the full underlying search result

#### Scenario: Multiline matching spans lines
- **WHEN** the model calls `grep` with `multiline = true`
- **THEN** regex matches SHALL be allowed to span newline boundaries
- **AND** multiline matches SHALL be ordered by the first matched line in rendered output

#### Scenario: Case-insensitive flag composes with regex flags
- **WHEN** the model calls `grep` with `case_insensitive = true` and also supplies inline regex flags in the pattern
- **THEN** the tool-level flag SHALL set the default matcher mode
- **AND** inline regex flags SHALL still be respected for local overrides supported by the regex engine

### Requirement: Built-in tool descriptions SHALL teach efficient tool usage
Built-in tool descriptions SHALL include actionable guidance about efficient usage rather than only terse API summaries.

#### Scenario: Search tools describe batching and selection guidance
- **WHEN** the model receives built-in tool definitions for `glob` and `grep`
- **THEN** the descriptions SHALL explain when to use each tool, when to prefer a different tool, and when batching is beneficial

#### Scenario: Read and edit tools describe bounded efficient workflows
- **WHEN** the model receives built-in tool definitions for `read`, `write`, and `edit`
- **THEN** the descriptions SHALL guide the model toward larger bounded reads, editing existing files when possible, and avoiding repeated tiny slices or redundant writes

### Requirement: Baseline prompt guidance SHALL promote efficient built-in tool usage
The system prompt layers used for inference requests SHALL include concise operational guidance that encourages lower-call, lower-token workflows with built-in tools.

#### Scenario: Prompt instructs the model to batch independent work
- **WHEN** inference instructions are composed for a session with built-in tools available
- **THEN** the resulting prompt SHALL instruct the model to batch independent search/read operations when possible

#### Scenario: Prompt instructs the model to keep responses terse
- **WHEN** inference instructions are composed for routine coding tasks
- **THEN** the resulting prompt SHALL instruct the model to avoid unnecessary preambles, postambles, and verbose user-facing text
- **AND** the prompt SHALL explicitly encourage minimizing output tokens while remaining accurate

### Requirement: Search tools SHALL tolerate partial traversal failures
The built-in discovery tools SHALL remain useful when a subset of candidate paths cannot be read or traversed.

#### Scenario: Search continues after minor unreadable paths
- **WHEN** `glob` or `grep` encounters a small number of unreadable files, directories, or broken links during traversal
- **THEN** the tool SHALL continue producing results from readable content where possible
- **AND** minor traversal issues MAY remain only in structured internal metadata

#### Scenario: Significant skipped-path conditions can be surfaced
- **WHEN** traversal problems are large enough that the model could misinterpret the completeness of the result set
- **THEN** the rendered output SHALL include a concise warning about skipped paths
