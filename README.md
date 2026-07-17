# iron-core

`iron-core` is the core embedding crate for AgentIron. It provides the agent
runtime, durable session state, tool registration, provider integration, and
ACP transport adapters.

The recommended entry point for embedded applications is the stream-first
`IronAgent` / `AgentConnection` / `AgentSession` facade. All modules and items
that are public in the crate are part of the public API; the facade and
`prelude` are recommendations for discovery, not a boundary around the API.

## Requirements

- Rust 1.96 or newer
- Tokio for asynchronous embedding code

## Install

`iron-core` is published on crates.io. Add the released crates to your
application:

```toml
[dependencies]
iron-core = "0.1"
iron-providers = "0.2"
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

The default feature set is empty. Enable `embedded-python` only when the
built-in `python_exec` tool is needed:

```toml
[dependencies]
iron-core = { version = "0.1", features = ["embedded-python"] }
```

## Quick Start

The following example compiles as a documentation test but is not run because
it reads an API key and sends a provider request over the network.

```rust,no_run
use iron_core::{Config, FunctionTool, IronAgent, PromptEvent, ToolDefinition};
use iron_providers::{ApiFamily, ProviderConnection, ProviderProfile, RuntimeConfig};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ProviderConnection::from_profile(
        ProviderProfile::new(
            "openai",
            ApiFamily::Responses,
            "https://api.openai.com/v1",
        ),
        RuntimeConfig::new(std::env::var("OPENAI_API_KEY")?),
    )?;
    let agent = IronAgent::new(Config::new().with_model("gpt-4o"), provider);

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

    let session = agent.connect().create_session()?;
    let (handle, mut events) = session.prompt_stream("Call echo with hello.");

    while let Some(event) = events.next().await {
        match event {
            PromptEvent::Output { text } => print!("{text}"),
            PromptEvent::ApprovalRequest { call_id, .. } => {
                handle.approve(&call_id)?;
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

The canonical interaction model is stream-first: call
`AgentSession::prompt_stream`, consume `PromptEvent` values, and use the
returned `PromptHandle` to approve, deny, or cancel an active prompt.

## Capabilities

- Durable messages, timeline entries, tool-call records, context accounting,
  compaction, and handoff export/import
- Custom tool registration with JSON Schema argument validation and approval
  strategies
- Layered prompt composition with repository instructions and runtime context
- Operational in-process ACP transport, with reserved stdio and TCP entry points
  that currently return compatibility errors
- Headless automation, scheduled tasks, profiles, stored prompts, provider
  credentials, skills, management APIs, and debug observation
- Optional embedded Python execution through the `embedded-python` feature

### Built-In Tools

`IronAgent::register_builtin_tools` can register `read`, `write`, `edit`,
`multiedit`, `glob`, `grep`, and `webfetch`, plus either `bash` or `powershell`
when that shell is available. `BuiltinToolConfig` controls allowed filesystem
roots, disabled tools, approval and network policy, shell selection, timeouts,
and result-size limits. Built-in tools enforce their configured policies at
execution time.

### MCP Servers

Runtime-managed MCP servers can provide tools alongside built-in and custom
tools. The public MCP APIs cover server inventory and health, stdio and HTTP
transport configuration, connection lifecycle, session-effective tool
catalogs, approval handling, and unavailable-tool diagnostics. MCP server state
is runtime-local and is not included in handoff bundles.

### Integration Plugins

The `plugin` module provides Extism-backed WASM integration-plugin APIs for
local or HTTPS artifacts, checksums, manifest extraction, registry and install
state, session enablement, authentication requirements, tool availability,
network policy declarations, and rich output. Plugin calls use a 30-second host
timeout. Availability and policy APIs describe and gate tools; embedders remain
responsible for configuring credentials, network access, and user-facing auth
flows.

## Development

The repository requires Rust 1.96. Install the Python
[`invoke`](https://www.pyinvoke.org/) package to use the convenience tasks, and
install `cargo-audit` for the security task:

```bash
cargo install cargo-audit --locked
git config core.hooksPath .githooks
```

The pre-commit hook checks formatting when staged Rust files are present. Run
the repository task groups before opening a pull request:

```bash
inv build
inv test
inv docs
inv security
```

`inv build` runs build, format, and all-feature Clippy checks; `inv test` runs
the test suite; `inv docs` runs strict all-feature rustdoc and doctests; and
`inv security` runs `cargo audit`. The corresponding documentation commands
are:

```bash
RUSTDOCFLAGS="-D warnings -D missing-docs" cargo doc --manifest-path Cargo.toml --no-deps --all-features
cargo test --manifest-path Cargo.toml --doc
```

The current `Pull Request` workflow runs locked build, format, all-feature
Clippy, test, and `inv docs` checks on Linux; all-feature link and scheduler
checks on macOS and Windows; and a fresh `embedded-python` consumer build. Its
`cargo audit` step is non-blocking and posts the result on same-repository pull
requests. Pull requests validate documentation but do not deploy it.

Pushes to `main` with unreleased changes run the patch-release workflow. After
its build, lint, test, audit, and fresh-consumer checks pass, the workflow bumps
the patch version, commits and tags it, publishes all features to crates.io
using trusted publishing, and creates a GitHub release. A manually dispatched
workflow performs the same process for minor or major releases. See the
[current releases](https://github.com/AgentIron/iron-core/releases) for published versions.

## Documentation

- [Published release API documentation](https://docs.rs/iron-core/latest/iron_core/)
- [Architecture Overview](docs/architecture-overview.md)
- [Getting Started](docs/getting-started-iron-core.md)
- [Integration Plugins](docs/integration-plugins.md)
- [Model Switching](docs/model-switching.md)
- [Model Switching Examples](docs/model-switching-examples.md)
- [Prompt Composition](docs/prompt-composition.md)

GitHub Pages is configured as a public workflow deployment with the custom
domain `core.agentiron.ai`. The `Deploy Documentation` workflow runs on pushes
to `main` and manual dispatch, builds strict all-feature rustdoc, publishes
`target/doc`, and redirects the site root to `iron_core/index.html`. Current
`main` API documentation is available at `https://core.agentiron.ai/` after a
successful deployment. The published docs.rs link above remains available
independently.

Build the same all-feature API documentation locally with:

```bash
RUSTDOCFLAGS="-D warnings -D missing-docs" cargo doc --manifest-path Cargo.toml --no-deps --all-features
```

Then open `target/doc/iron_core/index.html`.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
