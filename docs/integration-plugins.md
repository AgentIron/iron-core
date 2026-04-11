# Integration Plugins

`iron-core` exposes a WASM-based integration-plugin surface powered by [Extism](https://extism.org). Plugins are a third tool source alongside built-in tools and MCP servers.

This document describes the fully implemented plugin system.

## Current Status

The plugin system is fully implemented. All 10 implementation phases are complete and 490 tests pass with 0 failures.

Available now:

- **Install lifecycle** — full pipeline: artifact fetch (local or HTTPS+checksum), cache, manifest extraction from WASM binaries, identity validation, and Extism-backed WASM host loading
- **WASM execution** — real Extism-backed `WasmHost` loads plugins and invokes `tool_{name}` exports with a JSON request/response envelope and a 30-second timeout
- **Session-scoped enablement** — runtime-level defaults materialised at session creation, with per-session overrides via `SessionPluginEnablement`
- **Canonical tool availability** — single-source-of-truth `compute_tool_availability()` gates tools on health, auth requirements, unauthenticated flags, authentication state, and scope satisfaction
- **Auth model** — runtime-governed auth state, credential bindings, and per-tool scope checks with `UnavailableReason::AuthRequired` and `UnavailableReason::ScopeMissing`
- **Tool diagnostics** — structured `UnavailableReason` variants (`PluginNotEnabled`, `PluginNotInstalled`, `ManifestMissing`, `PluginNotHealthy`, `AuthRequired`, `ScopeMissing`) for actionable error messages
- **Manifest extraction** — `iron_manifest` custom WASM section parsed and validated, with identity cross-check against the plugin configuration
- **Network policy** — allowlist, blocklist, and wildcard policies declared in plugin manifests
- **Registry inspection** — `PluginRegistry` with `get()`, `list()`, `get_status()`, `get_all_statuses()`, and `recompute_availability()`
- **Session catalog integration** — `SessionToolCatalog` unifies local, MCP, and plugin tools with a single `execute()` dispatch and precise per-source diagnostics
- **Handoff exclusion** — plugin inventory, auth state, and session enablement are intentionally runtime-local and excluded from handoff bundles

## v1 Plugin Entrypoint Contract

Iron-core plugins are WASM modules that export one or more functions. The v1 contract defines:

- **Manifest section**: A custom WASM section named `iron_manifest` containing a UTF-8 JSON payload (see `PluginManifest`).
- **Tool entrypoints**: Each tool declared in the manifest has a corresponding exported function named `tool_{tool_name}`. For example, a tool named `greet` is invoked via the `tool_greet` export.
- **Request envelope**: The host serializes arguments as JSON and passes them as a UTF-8 string via Extism's input buffer.
- **Response envelope**: The plugin returns a UTF-8 JSON string via Extism's output buffer. On success the plugin returns `{"ok": <result_value>}`. On failure the plugin returns `{"error": "<message>"}`.
- **Timeout**: The host sets a 30-second timeout on all plugin calls via the Extism manifest. Plugins that exceed this are interrupted.

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
- `Remote` fetches over HTTPS with mandatory checksum verification (SHA-256 or SHA-512)
- Remote fetch with a missing checksum returns `InstallResult::MissingChecksum`
- Checksum mismatch returns `InstallResult::InvalidChecksum` with both expected and computed values

## Install Lifecycle

The lifecycle manager (`PluginLifecycle`) handles the full install pipeline:

```text
Configured → Loading → (cache artifact) → (extract manifest) → (validate identity) → (load into WASM host) → Healthy
                                                       ↘ Error
```

Design principles:

- **Idempotent installs** — re-installing the same plugin ID replaces the cached artifact and manifest, resetting the plugin to `Healthy`.
- **Atomic state transitions** — the registry is updated in explicit steps (`Configured` → `Loading` → `Healthy` / `Error`), and stale state is cleared when a reinstall fails.
- **Checksum enforcement** — remote sources require a checksum. Missing checksum, checksum mismatch, HTTP failure, and filesystem failure are each represented as distinct `InstallResult` variants.
- **Identity validation** — the manifest's `id` must match the plugin configuration's `id` or the install fails with `InstallResult::IdentityMismatch`.

`InstallResult` variants provide structured, actionable diagnostics:

| Variant | Meaning |
|---------|---------|
| `Success` | Plugin installed and loaded successfully |
| `InvalidChecksum` | Computed checksum did not match the expected value |
| `MissingChecksum` | Remote source provided without a required checksum |
| `DownloadFailed` | Network or filesystem failure during fetch |
| `InvalidManifest` | Manifest could not be extracted or is structurally invalid |
| `CacheWriteFailed` | Artifact could not be written to the cache directory |
| `LoadFailed` | WASM host rejected the plugin during the load step |
| `IdentityMismatch` | Manifest identity does not match the plugin configuration |

## Session Defaults And Portability

When plugins are enabled globally, new sessions are seeded from:

- `RuntimePluginConfig::enabled_by_default`
- each registered plugin's own `enabled_by_default`

Runtime defaults are **materialised** at session creation time — every known plugin gets an explicit per-session enablement entry. The session-effective path never falls back to per-server metadata.

Session-scoped plugin enablement lives on `DurableSession`:

- `set_plugin_enabled(...)`
- `is_plugin_enabled(...)`
- `list_enabled_plugins()`

Plugin runtime state is intentionally runtime-local and is **not** part of handoff portability. Exported handoff bundles do not carry:

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

Manifest validation checks:

- required identity/publisher/presentation fields
- `api_version == "1.0"`
- unique tool names within a plugin

The manifest is embedded in the WASM artifact as a custom section named `iron_manifest` and is extracted during the install lifecycle. The lifecycle manager cross-checks the manifest identity against the plugin configuration ID.

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

The API surface exists today. The actual OAuth browser/code exchange flow is not implemented yet — `AuthState` is managed programmatically through the runtime.

## Network Policy

Plugins declare one of these policies through `PluginNetworkPolicy`:

- `Allowlist(Vec<String>)`
- `Blocklist(Vec<String>)`
- `Wildcard`

The policy type and its allow/block matching helpers are implemented.

## Tool Exposure Rules

`compute_tool_availability()` is the single source of truth for deciding whether a tool from a plugin is available. Every call-site that answers "is this tool usable right now?" delegates here.

Evaluation order:

1. Plugin health — must be `Healthy`.
2. Tool has no auth requirements → available.
3. Tool is `available_unauthenticated` → available.
4. Plugin must be in `Authenticated` state.
5. Granted scopes must cover required scopes.

`EffectivePluginToolView` aggregates the per-plugin results for a session, factoring in session enablement, health, manifest presence, and the canonical availability check.

Plugin tool names are namespaced as:

```text
plugin_{plugin_id}_{tool_name}
```

This avoids collisions with built-in tools, custom tools, and MCP tools.

## Unavailable-Reason Diagnostics

When a tool is not available, `UnavailableReason` provides a structured explanation:

| Variant | Meaning |
|---------|---------|
| `PluginNotEnabled` | Plugin is not enabled for the current session |
| `PluginNotInstalled` | Plugin has not been installed yet |
| `ManifestMissing` | Plugin manifest has not been loaded |
| `PluginNotHealthy(PluginHealth)` | Plugin runtime health is not `Healthy` |
| `AuthRequired` | Tool requires authentication but the plugin is not authenticated |
| `ScopeMissing { required, missing }` | Tool requires scopes not covered by granted credentials |

The session tool catalog (`SessionToolCatalog`) uses these diagnostics for both provider-facing tool listings and execution-time error messages.

## Status And Registry APIs

`PluginRegistry` exposes runtime-facing inspection helpers:

- `get(plugin_id)` — look up a specific plugin
- `list()` — enumerate all registered plugins
- `get_status(plugin_id)` — get aggregate status
- `get_all_statuses()` — status for all plugins
- `recompute_availability()` — re-evaluate per-tool availability after auth changes, returns `PluginAvailabilitySummary`

`PluginStatus` separates:

- plugin health via `PluginHealth`
- auth availability via `AuthAvailability`
- aggregate readiness via `PluginRuntimeStatus`

`PluginAvailabilitySummary` provides:

- `healthy` / `authenticated` flags
- `total_tools` / `available_tools` counts
- per-tool `PerToolAvailability` breakdown

## Session Tool Catalog Integration

`SessionToolCatalog` unifies three tool sources — local tools, MCP servers, and plugins — behind a single interface:

- Provider-facing tool definitions include all enabled, available tools from all three sources.
- `execute()` dispatches to the correct handler (local registry, MCP connection manager, or WASM host) based on the tool's origin.
- Unavailable tool calls return precise, actionable diagnostics from the canonical availability logic.
- `inspect_tools()` returns a diagnostic summary showing each tool's source and availability state.

## Thread Safety

`WasmHost` is `Clone` + `Send + Sync`. Internally, plugin instances are guarded by a `Mutex` because Extism's `Plugin::call()` requires `&mut self`. Tool execution is dispatched to Tokio's blocking thread pool so the async runtime is not blocked.

## Test Coverage

The plugin system has 490 passing tests across all modules, including:

- Install lifecycle edge cases (reinstall, rollback, identity mismatch)
- WasmHost load/unload/execute with Extism
- Canonical `compute_tool_availability()` with auth-gating and per-tool scope differences
- Session catalog integration with plugin tool filtering and execution error paths
- Session isolation and handoff boundary verification
