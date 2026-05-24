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

### Model Switching Architecture

Model switching is implemented as a continuation export/import that preserves session identity:

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│  User Request   │────▶│  Export/Import   │────▶│  New Runtime    │
│  switch_model() │     │  Planning        │     │  Session        │
└─────────────────┘     └──────────────────┘     └─────────────────┘
                               │
                               ▼
                    ┌──────────────────────┐
                    │  Context Adaptation  │
                    │  - Size estimation   │
                    │  - Compaction        │
                    │  - Tail retention    │
                    └──────────────────────┘
                               │
                               ▼
                    ┌──────────────────────┐
                    │  Capability Recon    │
                    │  - Tool filtering    │
                    │  - Modality check    │
                    │  - Window comparison │
                    └──────────────────────┘
```

Key components:

- `ModelSwitchPlanner`: Creates adaptation plans comparing source/target constraints
- `CapabilityDiff`: Records capability differences (tools, modalities, window size)
- `ModelSwitchRequest`: Enum for managed vs unmanaged provider switches
- `TimelineEntry::ModelSwitched`: Records switch events in session timeline

Switching happens at turn boundaries:

1. **Idle**: Switch applies immediately, next prompt uses new model
2. **Active**: Switch queues until turn completes, preserving in-flight state

Context adaptation triggers when target window < current context:

1. Estimate current token count
2. Compact older messages if needed
3. Retain configurable tail of recent messages
4. Inject session briefing for provider changes

Capability reconciliation is permissive by default:

- Unavailable tools are hidden from the catalog
- Unsupported modalities are reported but don't block
- Context too large after compaction returns an error

## Practical Guidance

Use this architecture split when making changes:

- add new runtime behavior to the facade/runtime path
- keep the public surface centered on the facade/runtime path
- avoid reintroducing parallel orchestration APIs

## Related Docs

- [Getting Started](./getting-started-iron-core.md)
- [Prompt Composition](./prompt-composition.md)
- [Integration Plugins](./integration-plugins.md)
- [Model Switching](./model-switching.md)
- [Model Switching Examples](./model-switching-examples.md)
- [Architecture Cleanup Checklist](./architecture-cleanup-checklist.md)
