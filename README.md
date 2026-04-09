# iron-core

`iron-core` is the core embedding crate for AgentIron.

It provides:

- the stream-first `IronAgent` / `AgentConnection` / `AgentSession` facade
- durable messages, timeline entries, and tool-call records
- tool registration with JSON Schema argument validation
- approval-gated tool execution
- ACP-native transports for in-process, stdio, and TCP integrations
- context compaction, active-context accounting, and handoff export/import
- layered prompt composition with repository instruction loading and runtime context injection
- optional embedded Python execution via the `embedded-python` feature

The facade API is the recommended integration surface for new code. Older `SessionHandle`-style APIs are still exported for compatibility, but are deprecated.

## Requirements

- Rust 1.91+
- Tokio for async embedding code

## Install

Use the git dependency for now:

```toml
[dependencies]
iron-core = { git = "https://github.com/AgentIron/iron-core", branch = "main" }
iron-providers = "0.1.1"
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

`iron-core` is not ready for crates.io publication yet because the optional `embedded-python` feature depends on `monty` from git until a usable crates.io release exists.

If you need the built-in `python_exec` tool, enable the feature explicitly:

```toml
[dependencies]
iron-core = { git = "https://github.com/AgentIron/iron-core", branch = "main", features = ["embedded-python"] }
iron-providers = "0.1.1"
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

## Quick Start

```rust,ignore
use iron_core::{Config, FunctionTool, IronAgent, PromptEvent, ToolDefinition};
use iron_providers::{OpenAiConfig, OpenAiProvider};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::new().with_model("gpt-4o");
    let provider = OpenAiProvider::new(OpenAiConfig::new(std::env::var("OPENAI_API_KEY")?));
    let agent = IronAgent::new(config, provider);

    agent.register_tool(FunctionTool::new(
        ToolDefinition::new(
            "echo",
            "Return the provided text",
            json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string" }
                },
                "required": ["text"]
            }),
        ),
        |args| Ok(json!({ "echo": args["text"].clone() })),
    ));

    let connection = agent.connect();
    let session = connection.create_session()?;
    let (handle, mut events) = session.prompt_stream("Call echo with hello.");

    while let Some(event) = events.next().await {
        match event {
            PromptEvent::Output { text } => print!("{text}"),
            PromptEvent::ApprovalRequest { call_id, .. } => {
                handle.approve(&call_id).expect("approval should be pending");
            }
            PromptEvent::Complete { outcome } => {
                println!("\nprompt finished: {outcome:?}");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
```

The canonical interaction model is stream-first:

- call `session.prompt_stream(...)`
- consume `PromptEvent`s as they arrive
- use `PromptHandle` to approve, deny, or cancel an active prompt

For non-streaming compatibility code, `session.prompt().await` and `session.drain_events()` are still available.

## Built-In Tools

`iron-core` can register built-in `read`, `write`, `edit`, `glob`, `grep`, and `webfetch` tools, plus `bash` or `powershell` when a shell is available.

Use `BuiltinToolConfig` to scope filesystem access, disable specific tools, and tune limits such as command timeouts and output caps.

## Documentation

- [Getting Started](docs/getting-started-iron-core.md)
- [Prompt Composition](docs/prompt-composition.md)

Build the API docs locally with:

```bash
cargo doc -p iron-core --no-deps
```

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
