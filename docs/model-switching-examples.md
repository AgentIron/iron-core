# Model Switching Examples

This guide shows common model switching patterns with `iron-core`.

## Basic Model Switch

Switch from the current model to GPT-4o mid-conversation:

```rust
use iron_core::{IronAgent, PromptEvent, ModelSwitchRequest};

let agent = IronAgent::new(config, provider);
let connection = agent.connect();
let session = connection.create_session()?;

// Start a conversation
let (handle, mut events) = session.prompt_stream("Explain quantum computing.");
// ... consume events ...

// Switch to a different model for the next prompt
let request = ModelSwitchRequest::Managed {
    provider_slug: "openai".into(),
    model: "gpt-4o".into(),
};
if let Err(e) = session.switch_model(request) {
    eprintln!("Failed to switch model: {}", e);
}

// Continue the same session with the new model
let (handle, mut events) = session.prompt_stream("Now explain it to a 5-year-old.");
```

## Handling Switch Events

Listen for switch events in your event loop:

```rust
use iron_core::PromptEvent;

while let Some(event) = events.next().await {
    match event {
        PromptEvent::Output { text } => print!("{}", text),
        
        PromptEvent::ModelSwitchPending { target_model, target_provider } => {
            println!("\n[Switch queued: will use {} after this turn]", target_model);
        }
        
        PromptEvent::ModelSwitched { from_model, to_model, adapted, capability_diff } => {
            println!("\n[Switched from {} to {}]", from_model, to_model);
            if adapted {
                println!("[Context was adapted for the new model]");
            }
            if let Some(diff) = capability_diff {
                if !diff.hidden_tools.is_empty() {
                    println!("[Hidden tools: {}]", diff.hidden_tools.join(", "));
                }
            }
        }
        
        PromptEvent::Complete { outcome } => break,
        _ => {}
    }
}
```

## Switching During Active Turns

When a turn is active, the switch is queued and applied at the boundary:

```rust
// Start a long-running generation
let (handle, mut events) = session.prompt_stream("Write a novel chapter...");

// While it's generating, queue a switch for the next turn
let request = ModelSwitchRequest::Managed {
    provider_slug: "anthropic".into(),
    model: "claude-opus-4".into(),
};
session.switch_model(request)?; // Returns Ok(()) immediately, queues the switch

// The current generation continues with the old model
// The switch applies when this turn completes
```

## Provider Change with Context Adaptation

Switching to a smaller context window triggers automatic compaction:

```rust
// Session has been running with a large-context model
let request = ModelSwitchRequest::Managed {
    provider_slug: "google".into(),
    model: "gemini-2.0-flash".into(), // 32K window vs 200K
};
match session.switch_model(request) {
    Ok(()) => {
        // Context was compacted automatically
        println!("Switch succeeded with context adaptation");
    }
    Err(e) => {
        // Context couldn't fit even after compaction
        eprintln!("Switch failed: {}", e);
    }
}
```

## Unmanaged Provider Switch

Use a custom provider that you manage yourself:

```rust
use iron_core::ModelSwitchRequest;
use iron_providers::{Provider, ProviderProfile, ApiFamily};

// Your custom provider implementation
let my_provider = MyCustomProvider::new();

let request = ModelSwitchRequest::Unmanaged {
    provider: Box::new(my_provider),
};
session.switch_model(request)?;
```

## Checking Model History

Inspect the session's model switch history:

```rust
// After several switches
let durable = session.durable();
let history = &durable.model_switch_history;
for record in history {
    println!("{} -> {} at index {}", 
        record.from_model, 
        record.to_model, 
        record.timeline_index
    );
}
```

## Conditional Switch Based on Capability

Check capabilities before switching:

```rust
use iron_core::capability::{ModelCapabilityRegistry, compare_capabilities};

let registry = ModelCapabilityRegistry::default();
let source = registry.get("openai", "gpt-4o").unwrap();
let target = registry.get("google", "gemini-2.0-flash").unwrap();

let diff = compare_capabilities(source, target);

if diff.window_shrink > 0.5 {
    println!("Warning: context window will shrink by {}%", 
        (diff.window_shrink * 100.0) as u32
    );
}

if !diff.hidden_tools.is_empty() {
    println!("These tools won't be available: {:?}", diff.hidden_tools);
}

// Proceed with the switch anyway
let request = ModelSwitchRequest::Managed {
    provider_slug: "google".into(),
    model: "gemini-2.0-flash".into(),
};
session.switch_model(request)?;
```

## Complete Example: Multi-Model Session

```rust
use iron_core::{IronAgent, PromptEvent, ModelSwitchRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = IronAgent::new(config, provider);
    let connection = agent.connect();
    let session = connection.create_session()?;
    
    // Step 1: Use fast model for brainstorming
    let (handle, mut events) = session.prompt_stream(
        "Generate 10 ideas for a Rust CLI tool."
    );
    while let Some(event) = events.next().await {
        if let PromptEvent::Complete { .. } = event { break; }
    }
    
    // Step 2: Switch to reasoning model for implementation
    session.switch_model(ModelSwitchRequest::Managed {
        provider_slug: "anthropic".into(),
        model: "claude-opus-4".into(),
    })?;
    
    let (handle, mut events) = session.prompt_stream(
        "Implement the best idea from the list above."
    );
    while let Some(event) = events.next().await {
        match event {
            PromptEvent::Output { text } => print!("{}", text),
            PromptEvent::ModelSwitched { from_model, to_model, .. } => {
                println!("\n[Switched {} -> {}]", from_model, to_model);
            }
            PromptEvent::Complete { .. } => break,
            _ => {}
        }
    }
    
    Ok(())
}
```
