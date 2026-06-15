## Context

`AgentProfile` already carries an optional `identity_prompt`, and execution setup resolves an effective identity prompt from the selected profile. Today that value is stored in `DurableSession.instructions`, then passed to `SystemPromptInputs.session_instructions`, which renders under `## 9. Client Injection`.

The prompt renderer has a fixed section model. `## 1. Identity` is rendered by a hardcoded fallback function, while profile identity is indistinguishable from other session/client instructions. This creates an authority-boundary mismatch: selected-agent identity is model-facing identity content, not client injection.

## Goals / Non-Goals

**Goals:**

- Introduce an explicit profile identity input for system prompt rendering.
- Render selected profile identity in `## 1. Identity`.
- Preserve the existing hardcoded identity text when no profile identity is supplied.
- Keep explicit session/client/repo/skill instructions in `## 9. Client Injection`.
- Ensure prompt caching/fingerprinting changes when profile identity changes.

**Non-Goals:**

- Redesign agent profile storage or profile loading.
- Change provider-specific guidance resolution.
- Implement store-backed provider profiles or provider registry changes from issue 79.
- Change the public meaning of client injection fragments.

## Decisions

### Add a dedicated `profile_identity` prompt input

`SystemPromptInputs` will gain a `profile_identity: Option<&str>` field. `PromptSection::Identity` will render the trimmed non-empty profile identity when present, otherwise it will use the existing core fallback identity.

Alternative considered: continue using `session_instructions` and special-case it in the renderer. That would keep the identity/client-injection ambiguity and make it hard to distinguish profile identity from explicit session instructions.

### Do not use `effective_identity_prompt()` as the only fallback signal inside the renderer

`AgentProfile::effective_identity_prompt()` always returns a string, either custom profile identity or the profile default. If prompt composition always receives that value, the renderer's existing fallback identity can never be used. Call sites should pass profile identity deliberately, and tests should cover the no-profile-identity path.

Alternative considered: replace the renderer fallback with the profile default. That would change the established Section 1 fallback text and broaden the behavior change beyond issue 78.

### Preserve `session_instructions` for client/session injection

Profile identity should not be added to `Client Injection` merely because it is selected at session setup. Existing explicit session instructions, repo payloads, skill instructions, and configured client fragments remain in Section 9.

Alternative considered: remove profile identity from session state entirely in this change. That may be cleaner long term, but it risks expanding the change into durable-session semantics. This proposal focuses on rendered prompt authority boundaries.

### Include profile identity in the prompt fingerprint

The system prompt cache must invalidate when profile identity changes. `SystemPromptFingerprint::from_inputs` will include `profile_identity` alongside other prompt inputs.

Alternative considered: rely on `session_instructions` fingerprinting. That only works while profile identity is incorrectly carried as session instructions and would fail once the inputs are separated.

## Risks / Trade-offs

- Profile identity may still be stored in `DurableSession.instructions` in some execution paths → Tests should prove it no longer appears in `Client Injection` when used as profile identity.
- Existing tests may assert the old default profile identity string in places where Section 1 now uses the richer core fallback or selected profile identity → Update expectations to match the new distinction between profile identity and session instructions.
- Public prompt-builder helpers may not all know the selected profile → Keep the new input optional so non-profile-aware call sites continue using fallback identity.
