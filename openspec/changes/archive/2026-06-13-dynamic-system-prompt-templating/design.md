## Context

`iron-core` already composes multiple prompt layers, including baseline prompt text, repository instructions, session instructions, active skills, and runtime context. That composition is effective, but it treats prompt assembly mostly as ordered concatenation. Issue `#14` requires a stronger model: each section must have a fixed owner, a fixed place in the final system prompt, and clear rules for when the full prompt should be rebuilt.

The main architectural constraint is prompt caching. We should not rebuild the system prompt every turn. Instead, we need an explicit invalidation model so prompt rebuilds happen only when relevant inputs change.

## Goals / Non-Goals

**Goals:**
- Produce one final system prompt from a fixed nine-section order.
- Make section ownership explicit and enforceable in code.
- Allow provider-specific and client-specific content only in their designated sections.
- Rebuild the system prompt only when explicit invalidation events occur.
- Preserve the ability to extend invalidation triggers later without redesigning the prompt API.

**Non-Goals:**
- Exposing a public generic template DSL or arbitrary section override API.
- Recomputing the system prompt on every turn.
- Allowing providers or clients to replace core policy sections.
- Solving future model-specific prompt shortening or adaptive compression in this change.

## Decisions

### Represent the system prompt as fixed ordered sections with typed ownership
Model the prompt as a fixed set of section slots rather than as free-form blocks. Each section has:
- a stable position in the final prompt
- an owning authority
- a rebuild temperature / invalidation policy

This is preferable to a generic `update(section, content)` interface because ownership rules remain encoded in the type surface instead of being left to convention.

### Keep the template engine internal
`iron-core` may compile a template into the binary with `include_str!` and render it with Tera or a similar engine, but callers should not manipulate raw template blocks directly.

This is preferable to exposing template names publicly because core safety and policy sections must remain non-overridable.

### Separate ownership from invalidation temperature
Ownership answers who may supply content. Temperature answers when prompt content may change.

For example:
- `Tool Philosophy` is core-owned but warm because it depends on actual tool availability.
- `Editing Guidelines` may accept client content but should usually be cold after startup.

This is preferable to a single mutable/not-mutable flag because it captures the real constraints of the issue.

### Use explicit invalidation hooks rather than per-turn recomposition
The prompt manager should cache the rendered system prompt and invalidate it only when tracked inputs change. Initial invalidation support should cover:
- working directory changes affecting `Static Context`
- provider/model changes affecting `Provider-Specific Guidance`
- tool availability changes affecting `Tool Philosophy`
- explicit client configuration changes affecting `Editing Guidelines` or `Client Injection`

This is preferable to unconditional rebuilds because it preserves prompt caching while still allowing future dynamic behavior.

### Treat client editing guidance and client injection as distinct external slots
`Editing Guidelines` should be modeled as client-supplied editing guidance with core fallback defaults.

`Client Injection` should be modeled as optional client-provided markdown fragments, likely as an ordered list with optional labels/titles, while keeping `iron-core` mostly unopinionated about fragment contents.

This is preferable to combining the two because they serve different semantic roles and update frequencies.

## Proposed Structure

Introduce an internal prompt state model along these lines:

- `SystemPromptSections`
  - core literal sections
  - core derived sections
  - provider fragment section
  - client editing guidance section
  - client injection section
- `SystemPromptInputs`
  - client name
  - runtime context snapshot
  - tool availability snapshot
  - provider guidance fragment
  - client editing guidance override
  - client injection fragments
- `SystemPromptCache`
  - last rendered prompt
  - invalidation flags / input fingerprinting

The existing prompt assembly pipeline can then evolve from plain concatenation into a section renderer that emits the final single system prompt in fixed order.

## Section Ownership Model

### Core-owned cold sections
- Identity
- Core Guidelines
- Safety & Destructive Actions
- Communication & Formatting

These are literal sections or mostly literal sections owned entirely by `iron-core`.

### Core-owned warm sections
- Static Context
- Tool Philosophy

These are derived by `iron-core` from runtime state and tool state.

### External fragment sections
- Editing Guidelines: client-supplied guidance with core fallback
- Provider-Specific Guidance: provider-owned trusted fragment
- Client Injection: client-owned optional markdown fragment list

## Risks / Trade-offs

- [The API surface may become more complex than today’s prompt concatenation] → Keep the public API narrow and typed; complexity should stay mostly internal.
- [Working directory may not be the only runtime context field that eventually needs invalidation] → Build the invalidation mechanism generically, even if only CWD is wired first.
- [Client injection can become a dumping ground] → Keep the slot intentionally narrow and clearly separated from core policy sections.
- [Template rendering can obscure where content came from] → Preserve section-level tests and section-specific builders so failures remain diagnosable.

## Migration Plan

- Introduce section and input structs alongside the current composition path.
- Move existing baseline/runtime/provider composition into the new section renderer without changing external behavior first.
- Add explicit invalidation support, initially for working directory changes.
- Add client/provider input APIs for editing guidance and client injection.
- Update prompt composition tests to assert section order, ownership boundaries, and cache-sensitive rebuild behavior.

## Open Questions

- Should `Client Injection` require titles for fragments, or should plain ordered markdown fragments be first-class?
- Should runtime date/time be captured once per rendered prompt or once per session to avoid cache churn from clock movement?
- Should repository instructions remain a separate pre-section input, or should they be folded into one of the core sections in a follow-up change?
