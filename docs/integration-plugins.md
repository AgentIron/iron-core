# Integration Plugins

`iron-core` exposes an experimental WASM-based integration-plugin surface as a third tool source alongside built-in tools and MCP servers.

This document describes what is available today and what is still placeholder-only.

## Current Status

The plugin work is currently scaffolding, not a finished execution pipeline.

Available now:

- Runtime-level plugin configuration through `Config::with_plugins(...)`
- Runtime plugin inventory through `IronRuntime::register_plugin(...)`
- Public manifest, auth, status, checksum, and network-policy types
- Session-scoped plugin enablement state on `DurableSession`
- Effective tool composition that reserves namespaced plugin tool names

Not implemented yet:

- Remote HTTP fetching for plugin artifacts
- Manifest extraction from WASM binaries
- Loading and executing plugin tools in a WASM runtime
- Full OAuth code exchange, refresh, and credential persistence flow

Because those pieces are still placeholder-only, registering a plugin today does not make it fully usable end to end.

## Two Plugin Config Types

There are two similarly named plugin config types in the public API:

- `RuntimePluginConfig`: global runtime settings for the plugin subsystem
- `PluginSourceConfig`: a single registered plugin entry in the runtime inventory

The crate root re-exports those aliases to avoid confusion with the two internal `PluginConfig` structs.

## Enable The Plugin Subsystem

Use `RuntimePluginConfig` on the top-level runtime `Config` to turn the subsystem on and choose default enablement behavior for new sessions.

```rust
use iron_core::{Config, RuntimePluginConfig};

let config = Config::new().with_plugins(
    RuntimePluginConfig::new()
        .with_enabled(true)
        .with_enabled_by_default(false),
);
```

Fields:

- `enabled`: enables plugin handling for the runtime at all
- `enabled_by_default`: controls whether registered plugins start enabled in new sessions
- `artifact_cache_dir`: target cache directory for downloaded or installed plugin artifacts

## Register Plugin Inventory

Register plugins on the runtime after creating the agent or runtime:

There is not yet an `IronAgent::register_plugin(...)` convenience wrapper, so facade users should go through `agent.runtime()`.

```rust
use iron_core::{Checksum, ChecksumAlgorithm, PluginSource, PluginSourceConfig};
use std::path::PathBuf;

agent.runtime().register_plugin(PluginSourceConfig {
    id: "com.example.slack".to_string(),
    source: PluginSource::LocalPath {
        path: PathBuf::from("./plugins/slack.wasm"),
    },
    enabled_by_default: true,
});

agent.runtime().register_plugin(PluginSourceConfig {
    id: "com.example.gmail".to_string(),
    source: PluginSource::Remote {
        url: "https://plugins.example.com/gmail.wasm".to_string(),
        checksum: Checksum {
            algorithm: ChecksumAlgorithm::Sha256,
            value: "<sha256-hex>".to_string(),
        },
    },
    enabled_by_default: false,
});
```

Notes:

- `LocalPath` accepts a path to a local WASM artifact
- `Remote` models a remote artifact plus a required checksum
- Remote plugin fetching is not implemented yet, so `Remote` is currently a configuration surface rather than a working install path

## Session Defaults And Portability

When plugins are enabled globally, new sessions are seeded from:

- `RuntimePluginConfig::enabled_by_default`
- each registered plugin's own `enabled_by_default`

Session-scoped plugin enablement lives on `DurableSession`:

- `set_plugin_enabled(...)`
- `is_plugin_enabled(...)`
- `list_enabled_plugins()`

The `AgentSession` facade does not currently mirror those helpers.

Plugin runtime state is intentionally runtime-local and is not part of handoff portability. Exported handoff bundles do not carry:

- registered plugin inventory
- plugin auth state or credentials
- session plugin-enablement decisions

The receiving runtime must re-register any plugins it wants to expose.

## Manifest Model

`PluginManifest` captures the structured metadata that clients and the runtime use to describe a plugin:

- identity: `id`, `name`, `version`
- publisher metadata
- presentation metadata such as description, icon, category, and keywords
- `network_policy`
- optional plugin-level OAuth requirements
- exported tool metadata
- `api_version`

Manifest validation currently checks:

- required identity/publisher/presentation fields
- `api_version == "1.0"`
- unique tool names within a plugin

## Auth Model

The v1 auth model is runtime-governed.

Plugins can declare requirements with `OAuthRequirements` and `ToolAuthRequirements`, but they do not define their own auth lifecycle semantics. `iron-core` is authoritative for:

- `AuthState`
- `AuthAvailability`
- `CredentialBinding`
- tool gating based on the current auth state

Relevant public interaction types:

- `AuthInteractionRequest`
- `AuthInteractionResponse`
- `AuthInteractionResult`

The API surface exists today, but the actual OAuth browser/code exchange flow is not implemented yet.

## Network Policy

Plugins declare one of these policies through `PluginNetworkPolicy`:

- `Allowlist(Vec<String>)`
- `Blocklist(Vec<String>)`
- `Wildcard`

The policy type and its allow/block matching helpers are implemented. Actual WASM-side outbound enforcement depends on the future runtime execution host.

## Tool Exposure Rules

`EffectivePluginToolView` only exposes a plugin tool when the plugin is:

- enabled for the session
- healthy
- backed by a loaded manifest
- authenticated when the tool requires auth and is not marked `available_unauthenticated`

Plugin tool names are namespaced as:

```text
plugin_{plugin_id}_{tool_name}
```

This avoids collisions with built-in tools, custom tools, and MCP tools.

## Status And Registry APIs

`PluginRegistry` exposes runtime-facing inspection helpers such as:

- `get(plugin_id)`
- `list()`
- `get_status(plugin_id)`
- `get_all_statuses()`

`PluginStatus` separates:

- plugin health via `PluginHealth`
- auth availability via `AuthAvailability`
- aggregate readiness via `PluginRuntimeStatus`

## Lifecycle API

`PluginLifecycle` models installation and uninstall operations and includes `InstallResult` for structured outcomes.

That API is not complete yet because successful installation depends on the unimplemented remote fetch and manifest-extraction steps.
