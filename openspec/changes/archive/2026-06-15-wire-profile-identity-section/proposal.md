## Why

Agent profile identity prompts currently flow through `session.instructions`, which renders them in `## 9. Client Injection` rather than `## 1. Identity`. This weakens the prompt authority model by treating selected-agent identity as client/session injection instead of first-class identity content.

## What Changes

- Add an explicit profile identity input to system prompt composition.
- Render the selected profile's identity prompt in `## 1. Identity` when profile identity is available.
- Preserve the existing core identity fallback when no profile identity is supplied.
- Keep repo instructions, explicit session instructions, skills, and client fragments in `## 9. Client Injection`.
- Include profile identity in system prompt cache/fingerprint invalidation.
- Add tests proving section placement, fallback behavior, ordering, and cache invalidation.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `agent-profiles`: Selected profile identity must be exposed to prompt composition as agent identity, not as client/session injection.
- `dynamic-system-prompt-templating`: The Identity section must render profile identity when provided while preserving existing section order and fallback behavior.

## Impact

- Affected code: `src/prompt/system.rs`, `src/request_builder.rs`, and prompt-building call sites that currently pass profile identity via `session.instructions`.
- Affected tests: prompt composition tests and profile/session prompt tests that assert instruction placement or default identity behavior.
- No external dependency changes.
- No storage-schema changes.
