# Getting Started With `iron-core`

This guide covers the shortest path to embedding `iron-core` in an application.

## 1. Add Dependencies

```toml
[dependencies]
iron-core = { git = "https://github.com/AgentIron/iron-core", branch = "main" }
iron-providers = "0.1.1"
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

Enable the `embedded-python` feature on `iron-core` if you need the built-in
`python_exec` runtime and its Monty `tools` namespace for orchestrating visible
runtime tools from Python.

`iron-core` is currently consumed from git rather than crates.io because the
optional `embedded-python` feature depends on `monty` from git until a usable
crates.io release exists.

## 2. Configure a Provider

`iron-core` delegates model inference to a type implementing `iron_providers::Provider`.

```rust
use iron_core::{Config, IronAgent, PromptEvent};
use iron_providers::{OpenAiConfig, OpenAiProvider};

let config = Config::new().with_model("gpt-4o");
let provider = OpenAiProvider::new(OpenAiConfig::new("sk-example".to_string()));
let agent = IronAgent::new(config, provider);
```

If your application already owns a Tokio runtime, prefer `IronAgent::with_tokio_handle(...)`.

## 3. Register Tools

Register tools before creating sessions:

```rust
use iron_core::{FunctionTool, ToolDefinition};
use serde_json::json;

let echo = FunctionTool::new(
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
);

agent.register_tool(echo);
```

You can also register the built-in `read`, `write`, `edit`, `glob`, `grep`, and
`webfetch` tools, plus `bash` or `powershell` when available, through
`IronAgent::register_builtin_tools(...)`.

`iron-core` can also surface tools from MCP servers and the WASM
integration-plugin subsystem. MCP support is fully implemented with session-scoped
enablement, transport clients for stdio/HTTP/HTTP+SSE, and connection lifecycle
management. The WASM plugin system is also fully implemented with install lifecycle,
Extism-backed execution, and canonical tool availability gating.
See [integration-plugins.md](./integration-plugins.md) for the plugin API surface.

## 4. Use The Stream-First Session API

```rust,ignore
let connection = agent.connect();
let session = connection.create_session()?;
let (handle, mut events) = session.prompt_stream("Summarize the repository layout.");

while let Some(event) = events.next().await {
    match event {
        PromptEvent::Output { text } => print!("{text}"),
        PromptEvent::ApprovalRequest { call_id, .. } => {
            handle.approve(&call_id)?;
        }
        PromptEvent::ToolResult { tool_name, status, .. } => {
            eprintln!("tool {tool_name}: {status:?}");
        }
        PromptEvent::Complete { outcome } => {
            eprintln!("prompt finished: {outcome:?}");
            break;
        }
        _ => {}
    }
}
```

The event contract is strict:

- `ToolCall` appears before approval or result events for the same call.
- Each `ToolCall` resolves to exactly one terminal `ToolResult`.
- `Complete` is emitted once per prompt.

## 5. Manage Session State

Useful session APIs:

- `messages()` and `timeline()` to inspect the currently retained session history.
- `tool_records()` to inspect currently retained tool-call lifecycle state.
- `active_context(...)` to inspect the next request's context footprint.
- `checkpoint(...)` to compact context into `compacted_context + retained tail` when context management is enabled.
- `export_handoff(...)` and `create_session_from_handoff(...)` to transfer continuity between sessions.

Handoff bundles intentionally exclude runtime-local state such as repository
instruction files, runtime context, MCP inventory, plugin inventory, plugin auth
bindings, and session-scoped MCP/plugin enablement decisions.

## 6. Build API Docs

```bash
cargo doc -p iron-core --no-deps
```

The `iron-core` crate docs are the canonical API reference for the public embedding surface.

For the prompt composition model — how `iron-core` assembles provider instructions from ordered layers — see [prompt-composition.md](./prompt-composition.md).

For the experimental WASM plugin surface, see [integration-plugins.md](./integration-plugins.md).
