# Architecture Overview

`iron-core` has one supported public architecture.

## Primary Architecture

New integrations should use the facade/runtime path:

- `IronAgent`
- `AgentConnection`
- `AgentSession`
- `IronRuntime`

This is the supported architecture for new features and ongoing design work.

### Runtime Model

- `IronRuntime` owns the provider, runtime configuration, tool registry, MCP registry, plugin registry, session store, and orchestration runtime handle.
- `IronAgent` is the ergonomic embedding facade over `IronRuntime`.
- `AgentConnection` represents one client association with the runtime.
- `AgentSession` owns durable conversation state and prompt orchestration for that connection.

### Prompt Execution Model

The canonical interaction model is stream-first:

- create a session
- call `prompt_stream(...)` or `prompt_stream_with_blocks(...)`
- consume `PromptEvent`s
- resolve approvals or cancellation through `PromptHandle`

Within the runtime, prompt execution flows through:

- request construction in `request_builder`
- provider streaming
- approval gating
- session-effective tool execution through `SessionToolCatalog`
- durable transcript and tool-call state updates

### Tool Architecture

`iron-core` exposes three tool sources through one canonical session-effective catalog:

- local/custom tools from `ToolRegistry`
- MCP tools from runtime-managed MCP servers
- plugin tools from the WASM integration-plugin subsystem

`SessionToolCatalog` is the single source of truth for:

- provider-facing tool definitions
- approval requirements
- runtime execution dispatch
- unavailable-tool diagnostics

### Context Architecture

Context management has three distinct concepts:

- `active_context`: the provider-visible footprint of the next request
- `compacted_context`: the structured semantic summary retained after compaction
- `handoff_bundle`: portable continuity state for cross-session transfer

Prompt-layer composition is handled separately from transcript compaction:

- prompt layers are assembled by the prompt-composition subsystem
- transcript retention is controlled by `ContextWindowPolicy`
- summarization/compaction lives under `context_management`

## Practical Guidance

Use this architecture split when making changes:

- add new runtime behavior to the facade/runtime path
- keep the public surface centered on the facade/runtime path
- avoid reintroducing parallel orchestration APIs

## Related Docs

- [Getting Started](./getting-started-iron-core.md)
- [Prompt Composition](./prompt-composition.md)
- [Integration Plugins](./integration-plugins.md)
- [Architecture Cleanup Checklist](./architecture-cleanup-checklist.md)
