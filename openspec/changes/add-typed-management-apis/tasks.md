## 1. Stored-Prompt Identity

- [x] 1.1 Add stored-prompt display-name, required uniquely indexed normalized-handle, and repair-state columns; bump the schema version and implement canonical ASCII kebab normalization.
- [x] 1.2 Implement dual-version prompt decoding that derives title-cased v1 display identity from the stable record ID and writes v2 on typed save.
- [x] 1.3 Add core-generated immutable prompt IDs and transactional typed prompt create/save/get/list and handle-lookup operations with deterministic case-insensitive collision rejection.
- [x] 1.4 Migrate colliding legacy handles to deterministic reserved repair handles, mark them as needing rename, and keep them retrievable by immutable ID.
- [x] 1.5 Preserve immutable prompt IDs during rename and verify existing automation-task references require no rewrite.
- [x] 1.6 Add tests for generated IDs, ASCII normalization, indexed uniqueness, explicit ID versus handle lookup, repair collisions, rename stability, v1 compatibility, and v2 upgrade-on-save.

## 2. Typed Profile And Prompt Management

- [x] 2.1 Add typed profile save/get/list/delete persistence operations that apply existing ID, name, default-profile, provider-context, schema, and approval validation.
- [x] 2.2 Add typed stored-prompt validation for instructions, skill identifier shape and uniqueness, profile existence, and best-effort creation-time availability under the selected profile.
- [x] 2.3 Define uniform ready/needs-attention managed-record outcomes and extend profile and prompt get/list operations with stable diagnostics and optional decoded repair values.
- [x] 2.4 Block profile deletion when stored prompts directly reference it and return a typed conflict containing sorted prompt IDs.
- [x] 2.5 Return integrity-unknown errors when malformed or unsupported records prevent safe profile, prompt, or task deletion.
- [x] 2.6 Add tests for valid typed round trips, `ReadOnly` and `RequireApproval` rejection, partial list success, missing profiles, skill snapshot behavior, blocked deletion, and integrity-unknown deletion.

## 3. Dependency Impact

- [x] 3.1 Define public typed dependency entity, direction, direct/transitive classification, relationship-path, and impact-report types.
- [x] 3.2 Implement deterministic profile impact traversal through prompts, automation tasks, and schedules.
- [x] 3.3 Implement deterministic provider-credential impact traversal through profiles, prompts, automation tasks, and schedules.
- [x] 3.4 Implement prompt and automation-task dependency/dependent traversal, reusing existing direct task and schedule reference queries.
- [x] 3.5 Add tests for direct and transitive paths, deduplication, deterministic ordering, empty impact, and existing prompt/task structural conflicts.

## 4. Credential Management

- [x] 4.1 Define a secret-safe configured-credential summary type containing only provider slug, credential mode, metadata, and persisted-state auth status.
- [x] 4.2 Add deterministic summary listing over configured credential rows only without loading secret material into returned values or synthesizing transient resolver state.
- [x] 4.3 Add typed API-key add-or-replace validation that constructs `StoredCredential::ApiKey` inside core, replaces either existing mode, and returns only redacted status.
- [x] 4.4 Add typed deletion of whichever credential mode is configured while preserving dependent definitions and existing OAuth initiation, polling, refresh, revocation, and status paths.
- [x] 4.5 Add tests proving list, debug, and serialization outputs cannot reveal secret material, unconfigured providers are omitted, OAuth is replaceable, and invalid replacement preserves the prior credential.

## 5. Config Management Service

- [x] 5.1 Add public `ConfigManagementService`, attached registry dependencies, optional scheduler attachment, `ManagementError` hierarchy, managed-record outcomes, and crate exports.
- [x] 5.2 Delegate profile, prompt, and automation-task CRUD to authoritative typed domain/store operations without exposing opaque records or schema-version inputs.
- [x] 5.3 Compose typed scheduled-task CRUD with an optionally attached `ScheduleManager`, returning scheduler-unavailable when absent.
- [x] 5.4 Document desired-state save versus host reconciliation as non-atomic and ensure no management input can represent arbitrary host commands or direct prompt scheduling.
- [x] 5.5 Implement host-first combined schedule deletion with typed host-removal failure and partial desired-deletion outcomes.
- [x] 5.6 Update attached profile and prompt registries only after durable success and report durable-success/registry-failure as a typed partial operation.

## 6. Verification And Follow-Ups

- [x] 6.1 Add integration tests covering the end-to-end UI management flow across profile, prompt, task, credential impact, schedule status, and structural deletion warnings.
- [x] 6.2 Run formatting, clippy, targeted module tests, and the full Rust test suite; resolve all failures without weakening diagnostics or redaction.
- [x] 6.3 Create focused follow-up issues for interactive stored-prompt preview and durable scheduled-run/session history, and link them from issue 83 or the change documentation.
- [x] 6.4 Verify public documentation states that stored prompts are never direct scheduler targets and that active child-session APIs are not durable run history.
