## 1. Prompt Input Model

- [x] 1.1 Add `profile_identity: Option<&str>` to `SystemPromptInputs` and update all direct constructors.
- [x] 1.2 Include `profile_identity` in `SystemPromptFingerprint::from_inputs` so identity changes invalidate cached prompts.

## 2. Identity Section Rendering

- [x] 2.1 Update `PromptSection::Identity` rendering to prefer non-empty `profile_identity` content.
- [x] 2.2 Preserve the existing core fallback identity when `profile_identity` is absent or blank.
- [x] 2.3 Ensure profile identity is not also rendered through `session_instructions` in `Client Injection` for profile-backed execution.

## 3. Request Construction Wiring

- [x] 3.1 Extend request-building context so prompt construction can receive selected profile identity separately from session instructions.
- [x] 3.2 Update primary managed/default profile-backed prompt paths to pass selected profile identity into request construction.
- [x] 3.3 Review delegation/sub-agent request construction and preserve fallback behavior where no explicit profile identity is available.

## 4. Verification

- [x] 4.1 Add or update prompt composition tests proving custom profile identity renders in `## 1. Identity`.
- [x] 4.2 Add or update tests proving explicit session instructions still render in `## 9. Client Injection`.
- [x] 4.3 Add or update tests proving missing profile identity uses the existing core fallback identity.
- [x] 4.4 Add or update fingerprint/cache tests proving profile identity changes alter the prompt fingerprint.
- [x] 4.5 Run the relevant Rust test suite for prompt composition and profile prompt behavior.
