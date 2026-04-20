## 1. Search engine foundation

- [x] 1.1 Add `ignore`, `grep-searcher`, and `grep-regex` dependencies and document the rationale in code comments or module docs where appropriate
- [x] 1.2 Replace the current custom `glob` traversal/matching implementation with ripgrep ecosystem traversal and real glob matching semantics
- [x] 1.3 Replace the current custom `grep` traversal/matching implementation with ripgrep ecosystem search primitives
- [x] 1.4 Define and implement default search visibility rules for `.gitignore`, hidden files, symlinks, and binary files

## 2. Search API and result contracts

- [x] 2.1 Redesign `glob` model-facing outputs to use compact path-oriented text results instead of repetitive JSON wrappers
- [x] 2.2 Redesign `grep` model-facing outputs to use compact grouped text results instead of repetitive JSON wrappers
- [x] 2.3 Add richer grep controls needed for efficient follow-up searches (for example include filtering and bounded result behavior)
- [x] 2.4 Update truncation/continuation messaging for built-in search so follow-up actions are explicit

## 3. Read and edit efficiency upgrades

- [x] 3.1 Extend `read` so directory paths return lightweight directory listings instead of immediate errors
- [x] 3.2 Add `replace_all` support to the existing edit surface while preserving exact-match safeguards
- [x] 3.3 Add support for applying multiple edits to one file in a single approved tool call
- [x] 3.4 Update write/edit/read outputs and schemas as needed so follow-up edits remain stable and legible to the model

## 4. Prompt and tool guidance

- [x] 4.1 Rewrite built-in tool descriptions for `glob`, `grep`, `read`, `write`, and `edit` to include efficiency guidance, batching guidance, and tool-selection guidance
- [x] 4.2 Update baseline prompt guidance to encourage terse responses, batching independent operations, and avoiding tiny repeated reads
- [x] 4.3 Update runtime-context messaging and any truncation helper text so the model is told how to continue efficiently after large outputs

## 5. Verification

- [x] 5.1 Add or update unit tests for glob semantics, grep semantics, ignore behavior, hidden files, binary detection, and truncation guidance
- [x] 5.2 Add or update tests for directory reads, `replace_all`, and multi-edit behavior
- [x] 5.3 Add or update prompt-composition tests for the new efficiency guidance and tool descriptions
- [x] 5.4 Run the relevant built-in tool, prompt composition, and full cargo test suites to verify the change end to end
