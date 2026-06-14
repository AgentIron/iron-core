## 1. Prompt section model

- [x] 1.1 Define the fixed nine-section prompt model and section ordering in `iron-core`
- [x] 1.2 Add explicit section ownership metadata for core, provider, and client supplied sections
- [x] 1.3 Add typed inputs for provider guidance, client editing guidance, and client injection content
- [x] 1.4 Add internal defaults for core-owned sections and editing-guidance fallback content

## 2. Rendering and caching

- [x] 2.1 Add an internal compiled system prompt template and renderer
- [x] 2.2 Introduce a cached prompt state object that tracks rendered output and invalidation state
- [x] 2.3 Rebuild the full system prompt only when tracked inputs change
- [x] 2.4 Wire explicit invalidation for working directory changes affecting `Static Context`
- [x] 2.5 Wire invalidation for provider/model changes affecting `Provider-Specific Guidance`
- [x] 2.6 Wire invalidation for tool availability changes affecting `Tool Philosophy`

## 3. Integration with existing prompt composition

- [x] 3.1 Refactor current prompt assembly to emit the fixed section order rather than ad hoc concatenation
- [x] 3.2 Preserve repository instructions, active skills, and runtime context semantics within the new section model
- [x] 3.3 Ensure core-owned cold sections cannot be overridden by provider or client inputs
- [x] 3.4 Ensure `Provider-Specific Guidance` only accepts trusted provider-owned content
- [x] 3.5 Ensure `Editing Guidelines` uses client-supplied content when present and core fallback otherwise
- [x] 3.6 Ensure `Client Injection` accepts optional markdown fragments without affecting core section ownership

## 4. Runtime and facade surfaces

- [x] 4.1 Add runtime/session APIs for setting client editing guidance at startup
- [x] 4.2 Add runtime/session APIs for setting client injection content
- [x] 4.3 Add provider integration hooks for provider/model-specific guidance fragments
- [x] 4.4 Add explicit prompt invalidation hooks for future runtime state changes beyond working directory changes

## 5. Verification

- [x] 5.1 Add tests verifying the final prompt uses the fixed nine-section order
- [x] 5.2 Add tests verifying core-owned sections cannot be externally overridden
- [x] 5.3 Add tests verifying provider guidance only appears in its dedicated section
- [x] 5.4 Add tests verifying client editing guidance overrides core fallback correctly
- [x] 5.5 Add tests verifying client injection fragments render in order
- [x] 5.6 Add tests verifying prompt rebuild happens on working directory change and is skipped otherwise
- [x] 5.7 Add tests verifying tool availability changes update `Tool Philosophy`
- [x] 5.8 Run prompt composition, request builder, and regression test suites
