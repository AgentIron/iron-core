## Why

`iron-core` currently assembles provider instructions through a fixed composition pipeline, but it does not model section ownership, mutability, or prompt rebuild triggers explicitly. Issue `#14` needs more than string interpolation: it needs a single system prompt with fixed section ordering, core-owned policy boundaries, narrow provider/client extension slots, and explicit invalidation hooks so prompt caching is preserved.

Without that structure, a generic block-override template system would make core policy too easy to replace, blur ownership between `iron-core`, `iron-providers`, and embedding clients, and encourage unnecessary full prompt rebuilds on every turn.

## What Changes

- Introduce a section-owned system prompt model with a fixed nine-section order.
- Add explicit authority boundaries for core-owned, provider-owned, and client-owned prompt sections.
- Add internal template rendering for the final system prompt without exposing a public generic block override surface.
- Add explicit prompt invalidation hooks, initially supporting static-context rebuilds on working directory changes and provider-guidance/tool-philosophy rebuilds when relevant inputs change.
- Add typed client/provider inputs for editing guidance, provider-specific guidance, and client injection fragments.
- Add tests that verify section ordering, ownership enforcement, and rebuild behavior.

## Capabilities

### New Capabilities
- `dynamic-system-prompt-templating`: Compose the system prompt from fixed sections with explicit ownership and invalidation rules.

### Modified Capabilities
- `dependency-version-guidance`: no direct behavior change, but provider instructions will now be inserted through the provider-owned prompt section rather than ad hoc composition.

## Impact

- Affected code: `src/prompt/*`, `src/request_builder.rs`, session/runtime prompt state management, and prompt composition tests.
- Affected systems: provider instruction assembly, runtime-context injection, provider/model guidance injection, and client prompt customization APIs.
- API/protocol impact: new internal section/state abstractions and likely new facade/runtime setters for client editing guidance and client injection content.
