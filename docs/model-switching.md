# Model Switching

`iron-core` supports changing the active model mid-session while preserving conversation continuity. This document covers the API, behavior, and capability reconciliation semantics.

## Overview

Model switching lets users continue a conversation with a different model or provider without losing context. The switch happens at turn boundaries: if the session is idle, it applies immediately; if a turn is active, it queues for the next turn.

## API

### Switching Models

Use `AgentSession::switch_model()` to request a model change:

```rust
use iron_core::{AgentSession, ModelSwitchRequest};

// Switch to a managed provider
let request = ModelSwitchRequest::Managed {
    provider_slug: "openai".into(),
    model: "gpt-4o".into(),
};
session.switch_model(request)?;

// Switch to an unmanaged provider (you handle credentials)
let request = ModelSwitchRequest::Unmanaged {
    provider: Box::new(my_custom_provider),
};
session.switch_model(request)?;
```

### Listening for Switch Events

Model switches emit `PromptEvent` variants that streaming consumers should handle:

```rust
use iron_core::PromptEvent;

while let Some(event) = events.next().await {
    match event {
        PromptEvent::ModelSwitchPending { target_model, target_provider } => {
            println!("Switch to {}/{} queued for next turn", 
                target_provider.unwrap_or_default(), 
                target_model
            );
        }
        PromptEvent::ModelSwitched { from_model, to_model, adapted, capability_diff } => {
            println!("Switched from {} to {} (adapted: {})", from_model, to_model, adapted);
            if let Some(diff) = capability_diff {
                if !diff.hidden_tools.is_empty() {
                    println!("Hidden tools: {:?}", diff.hidden_tools);
                }
            }
        }
        _ => {}
    }
}
```

## Turn-Boundary Semantics

Model switches are applied at turn boundaries, not mid-turn:

- **Idle session**: Switch applies immediately, next prompt uses the new model
- **Active session**: Switch is queued, applies when the current turn completes

This preserves in-flight tool calls and generation streams. The user sees a `ModelSwitchPending` event when queued.

## Context Adaptation

When switching to a model with a smaller context window, `iron-core` automatically:

1. Estimates current context size against the target window
2. Triggers compaction if needed to fit
3. Retains a configurable tail of recent messages
4. Injects a session briefing so the new model understands the conversation history

The adaptation is recorded in the `ModelSwitched` timeline entry with `adapted: true`.

## Capability Reconciliation

Different models support different capabilities. When switching, `iron-core` computes a `CapabilityDiff`:

### Tool Availability

Tools available in the previous model but not the new one are hidden from the tool catalog. The switch event reports which tools were hidden.

### Modality Support

If the previous model supported images or PDFs but the new one doesn't:
- Image content blocks are preserved in the transcript (for history)
- New image uploads are rejected with a clear error

### Context Window

If the target window is smaller, context is compacted. If compaction cannot shrink enough, the switch fails with an error.

### Streaming Support

If the new model doesn't support streaming, the session falls back to non-streaming mode transparently.

## Session Identity

Model switching preserves the same session identity. The timeline records the switch as a `ModelSwitched` entry, and the session's `model_switch_history` tracks all switches for auditability.

## Managed vs Unmanaged Switching

- **Managed**: `iron-core` resolves credentials from the credential store, handles auth refresh, and applies provider-specific transforms
- **Unmanaged**: You provide the provider directly; `iron-core` treats it as opaque

## Configuration

Model switching behavior is configured via `ContextManagementConfig`:

```rust
use iron_core::ContextManagementConfig;

let config = ContextManagementConfig {
    model_switch: ModelSwitchConfig {
        compact_on_window_shrink: true,
        default_tail_messages: 10,
        minimum_window_tokens: 4096,
        briefing_on_provider_change: true,
    },
    ..Default::default()
};
```

| Option | Default | Description |
|--------|---------|-------------|
| `compact_on_window_shrink` | `true` | Compact context when switching to smaller window |
| `default_tail_messages` | `10` | Messages to retain verbatim after compaction |
| `minimum_window_tokens` | `4096` | Minimum context window for any model |
| `briefing_on_provider_change` | `true` | Inject session briefing when provider changes |

## Error Handling

Switching can fail in these cases:

- **Context too large**: Even after compaction, context exceeds the target window
- **Invalid provider**: Managed provider not found in credential store
- **Capability mismatch**: The requested model lacks required capabilities and the user has configured strict mode

Errors are returned as `Result::Err(String)` from `switch_model()`.

## Timeline Events

Each successful switch records a `TimelineEntry::ModelSwitched` with:

- `from_model`: Previous model identifier
- `to_model`: New model identifier
- `from_provider`: Previous provider (if managed)
- `to_provider`: New provider (if managed)
- `adapted`: Whether context was compacted or modified

These entries appear in `session.timeline()` and `session.to_transcript()`.

## See Also

- [Architecture Overview](./architecture-overview.md)
- [Model Switching Examples](./model-switching-examples.md)
- `AgentSession::switch_model()` API docs (run `cargo doc -p iron-core --no-deps`)
