//! WASM execution host for plugins (Extism-backed).
//!
//! `WasmHost` manages loaded plugin instances in memory and provides the
//! bridge between the lifecycle manager's artifact cache and the actual WASM
//! runtime powered by [Extism](https://extism.org).
//!
//! ## v1 Plugin Entrypoint Contract
//!
//! Iron-core plugins are WASM modules that export one or more functions. The
//! v1 contract defines:
//!
//! * **Manifest section**: A custom WASM section named `iron_manifest` containing
//!   a UTF-8 JSON payload (see `PluginManifest`).
//!
//! * **Tool entrypoints**: Each tool declared in the manifest has a corresponding
//!   exported function named `tool_{tool_name}`. For example, a tool named
//!   `greet` is invoked via the `tool_greet` export.
//!
//! * **Request envelope**: The host serializes arguments as JSON and passes them
//!   as a UTF-8 string via Extism's input buffer.
//!
//! * **Response envelope**: The plugin returns a UTF-8 JSON string via Extism's
//!   output buffer. The response must be a JSON object. On success the plugin
//!   returns `{"ok": <result_value>}`. On failure the plugin returns
//!   `{"error": "<message>"}`.
//!
//! * **Timeout**: The host sets a 30-second timeout on all plugin calls via the
//!   Extism manifest. Plugins that exceed this are interrupted.
//!
//! * **Error mapping**: Host-side errors (plugin not found, load failure, invalid
//!   input, malformed output, timeout) are mapped to structured `WasmError`
//!   variants.
//!
//! ## Thread safety
//!
//! `extism::Plugin::call()` requires `&mut self`, so the internal plugin map
//! is guarded by a `Mutex` rather than an `RwLock`.  The outer `WasmHost` is
//! `Clone` (cloning only bumps the `Arc` ref-count) and `Send + Sync`.

use crate::plugin::lifecycle::PluginLoader;
use crate::plugin::manifest::PluginManifest;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

/// Result type for WASM operations.
pub type WasmResult<T> = Result<T, WasmError>;

/// Future type for async WASM tool execution.
pub type WasmExecutionFuture = Pin<Box<dyn Future<Output = WasmResult<Value>> + Send>>;

/// Bookkeeping for a single loaded plugin inside the WASM host.
struct LoadedPlugin {
    /// Path to the cached WASM artifact on disk.
    artifact_path: PathBuf,
    /// The Extism plugin instance.
    plugin: extism::Plugin,
    /// Manifest extracted during load (mirrors registry manifest).
    manifest: Option<PluginManifest>,
}

/// WASM execution host for running plugin code.
///
/// Thread-safe: internal state is guarded by a `Mutex` so that concurrent
/// tool execution (which requires `&mut` access to the Extism plugin) is
/// serialised correctly.  `WasmHost` is cheaply `Clone`-able — cloning only
/// bumps the `Arc` ref-count.
#[derive(Clone)]
pub struct WasmHost {
    inner: Arc<Mutex<WasmHostInner>>,
}

struct WasmHostInner {
    /// Plugins currently loaded into the host, keyed by plugin ID.
    loaded: HashMap<String, LoadedPlugin>,
}

impl WasmHost {
    /// Create a new WASM host with no loaded plugins.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(WasmHostInner {
                loaded: HashMap::new(),
            })),
        }
    }

    /// Load a plugin into the WASM runtime.
    ///
    /// Reads the artifact from `artifact_path`, creates an Extism manifest
    /// with a 30-second timeout and WASI enabled, then instantiates the
    /// plugin.
    pub fn load_plugin(&self, plugin_id: &str, artifact_path: &Path) -> WasmResult<()> {
        let wasm_bytes = std::fs::read(artifact_path).map_err(|e| {
            WasmError::LoadFailed(format!(
                "Failed to read artifact {}: {}",
                artifact_path.display(),
                e
            ))
        })?;

        let manifest = extism::Manifest::new([extism::Wasm::data(wasm_bytes)])
            .with_timeout(std::time::Duration::from_secs(30));

        let plugin = extism::Plugin::new(manifest, [], true)
            .map_err(|e| WasmError::LoadFailed(format!("Extism plugin creation failed: {}", e)))?;

        let mut inner = self.inner.lock().unwrap();
        inner.loaded.insert(
            plugin_id.to_string(),
            LoadedPlugin {
                artifact_path: artifact_path.to_path_buf(),
                plugin,
                manifest: None, // populated separately by lifecycle
            },
        );

        info!(plugin_id = %plugin_id, "Plugin loaded into WASM host");
        Ok(())
    }

    /// Unload a plugin from the WASM runtime.
    ///
    /// Removes the in-memory tracking.  Returns `Ok(())` even if the plugin
    /// was not loaded (idempotent).  Dropping the `LoadedPlugin` drops the
    /// underlying `extism::Plugin`.
    pub fn unload_plugin(&self, plugin_id: &str) -> WasmResult<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner.loaded.remove(plugin_id).is_some() {
            info!(plugin_id = %plugin_id, "Plugin unloaded from WASM host");
        } else {
            debug!(plugin_id = %plugin_id, "Unload requested but plugin was not loaded");
        }
        Ok(())
    }

    /// Execute a plugin tool (async version).
    ///
    /// Resolves the plugin, calls the Extism entrypoint `tool_{tool_name}`,
    /// and parses the response envelope `{"ok": ...}` / `{"error": ...}`.
    ///
    /// The actual Extism call is synchronous and requires `&mut self` on the
    /// plugin, so it is dispatched to a Tokio blocking thread.
    pub fn execute_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        arguments: Value,
    ) -> WasmExecutionFuture {
        let plugin_id = plugin_id.to_string();
        let tool_name = tool_name.to_string();
        let inner = self.inner.clone();

        Box::pin(async move {
            // Execute on a blocking thread since Extism::call is sync
            let result = tokio::task::spawn_blocking(move || {
                let mut guard = inner.lock().unwrap();
                let loaded = guard
                    .loaded
                    .get_mut(&plugin_id)
                    .ok_or_else(|| WasmError::NotFound(plugin_id.clone()))?;

                let entrypoint = format!("tool_{}", tool_name);

                // Check if the function exists
                if !loaded.plugin.function_exists(&entrypoint) {
                    return Err(WasmError::ExecutionFailed(format!(
                        "Plugin does not export function '{}'",
                        entrypoint
                    )));
                }

                // Serialize arguments
                let input = serde_json::to_string(&arguments).map_err(|e| {
                    WasmError::InvalidInput(format!("Failed to serialize arguments: {}", e))
                })?;

                // Call the plugin
                let output: &str = loaded.plugin.call(&entrypoint, &input).map_err(|e| {
                    let msg = e.to_string();
                    if msg.contains("timeout") || msg.contains("timed out") {
                        WasmError::Timeout
                    } else if msg.contains("trap") || msg.contains("panic") {
                        WasmError::PluginPanicked(msg)
                    } else {
                        WasmError::ExecutionFailed(msg)
                    }
                })?;

                // Parse the response envelope
                let response: Value = serde_json::from_str(output).map_err(|e| {
                    WasmError::ExecutionFailed(format!("Plugin returned invalid JSON: {}", e))
                })?;

                if let Some(error) = response.get("error") {
                    let msg = error.as_str().unwrap_or("Unknown plugin error");
                    return Err(WasmError::ExecutionFailed(msg.to_string()));
                }

                Ok(response.get("ok").cloned().unwrap_or(Value::Null))
            })
            .await;

            match result {
                Ok(inner_result) => inner_result,
                Err(join_error) => Err(WasmError::ExecutionFailed(format!(
                    "Task join error: {}",
                    join_error
                ))),
            }
        })
    }

    /// Execute a plugin tool synchronously (for non-async contexts).
    pub fn execute_tool_sync(
        &self,
        plugin_id: &str,
        tool_name: &str,
        arguments: Value,
    ) -> WasmResult<Value> {
        let mut guard = self.inner.lock().unwrap();
        let loaded = guard
            .loaded
            .get_mut(plugin_id)
            .ok_or_else(|| WasmError::NotFound(plugin_id.to_string()))?;

        let entrypoint = format!("tool_{}", tool_name);

        if !loaded.plugin.function_exists(&entrypoint) {
            return Err(WasmError::ExecutionFailed(format!(
                "Plugin does not export function '{}'",
                entrypoint
            )));
        }

        let input = serde_json::to_string(&arguments).map_err(|e| {
            WasmError::InvalidInput(format!("Failed to serialize arguments: {}", e))
        })?;

        let output: &str = loaded.plugin.call(&entrypoint, &input).map_err(|e| {
            let msg = e.to_string();
            if msg.contains("timeout") || msg.contains("timed out") {
                WasmError::Timeout
            } else if msg.contains("trap") || msg.contains("panic") {
                WasmError::PluginPanicked(msg)
            } else {
                WasmError::ExecutionFailed(msg)
            }
        })?;

        let response: Value = serde_json::from_str(output).map_err(|e| {
            WasmError::ExecutionFailed(format!("Plugin returned invalid JSON: {}", e))
        })?;

        if let Some(error) = response.get("error") {
            let msg = error.as_str().unwrap_or("Unknown plugin error");
            return Err(WasmError::ExecutionFailed(msg.to_string()));
        }

        Ok(response.get("ok").cloned().unwrap_or(Value::Null))
    }

    /// Check if a plugin is loaded in the WASM host.
    pub fn is_plugin_loaded(&self, plugin_id: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.loaded.contains_key(plugin_id)
    }

    /// Check if a plugin is loaded and considered healthy.
    ///
    /// A plugin is healthy if it is loaded and its artifact still exists on disk.
    pub fn is_plugin_healthy(&self, plugin_id: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        match inner.loaded.get(plugin_id) {
            Some(loaded) => loaded.artifact_path.exists(),
            None => false,
        }
    }

    /// Store the manifest for a loaded plugin.
    ///
    /// Called by the lifecycle manager after it extracts the manifest from the
    /// WASM binary.
    pub fn set_manifest(&self, plugin_id: &str, manifest: PluginManifest) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(loaded) = inner.loaded.get_mut(plugin_id) {
            loaded.manifest = Some(manifest);
        }
    }

    /// Get the manifest for a loaded plugin, if available.
    pub fn get_plugin_manifest(&self, plugin_id: &str) -> Option<PluginManifest> {
        let inner = self.inner.lock().unwrap();
        inner.loaded.get(plugin_id).and_then(|l| l.manifest.clone())
    }

    /// List all currently loaded plugin IDs.
    pub fn loaded_plugins(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.loaded.keys().cloned().collect()
    }
}

impl Default for WasmHost {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for WasmHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock().unwrap();
        let ids: Vec<&String> = inner.loaded.keys().collect();
        f.debug_struct("WasmHost")
            .field("loaded_plugins", &ids)
            .finish()
    }
}

/// Implement `PluginLoader` so the lifecycle manager can delegate the
/// host-load step to `WasmHost` without coupling.
impl PluginLoader for WasmHost {
    fn load(&self, plugin_id: &str, artifact_path: &Path) -> Result<(), String> {
        self.load_plugin(plugin_id, artifact_path)
            .map_err(|e| e.to_string())
    }

    fn unload(&self, plugin_id: &str) {
        if let Err(e) = self.unload_plugin(plugin_id) {
            warn!(
                plugin_id = %plugin_id,
                error = %e,
                "Failed to unload plugin from WASM host during uninstall"
            );
        }
    }
}

/// Errors that can occur during WASM operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WasmError {
    /// Plugin not found in the WASM host.
    NotFound(String),
    /// Plugin failed to load.
    LoadFailed(String),
    /// Tool execution failed.
    ExecutionFailed(String),
    /// Invalid input arguments.
    InvalidInput(String),
    /// Plugin panicked during execution.
    PluginPanicked(String),
    /// Timeout during execution.
    Timeout,
}

impl std::fmt::Display for WasmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "Plugin not found: {}", id),
            Self::LoadFailed(msg) => write!(f, "Failed to load plugin: {}", msg),
            Self::ExecutionFailed(msg) => write!(f, "Tool execution failed: {}", msg),
            Self::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            Self::PluginPanicked(msg) => write!(f, "Plugin panicked: {}", msg),
            Self::Timeout => write!(f, "Plugin execution timed out"),
        }
    }
}

impl std::error::Error for WasmError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::manifest::{
        ExportedTool, PluginIdentity, PluginManifest, PluginPublisher, PresentationMetadata,
    };
    use crate::plugin::network::NetworkPolicy;

    fn sample_manifest() -> PluginManifest {
        PluginManifest {
            identity: PluginIdentity {
                id: "com.example.test".to_string(),
                name: "Test".to_string(),
                version: "1.0.0".to_string(),
            },
            publisher: PluginPublisher {
                name: "Test".to_string(),
                url: None,
                contact: None,
            },
            presentation: PresentationMetadata {
                description: "A test plugin".to_string(),
                long_description: None,
                icon: None,
                category: None,
                keywords: vec![],
            },
            network_policy: NetworkPolicy::Wildcard,
            auth: None,
            tools: vec![ExportedTool {
                name: "greet".to_string(),
                description: "Say hello".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                requires_approval: false,
                auth_requirements: None,
            }],
            api_version: "1.0".to_string(),
        }
    }

    #[test]
    fn load_nonexistent_artifact_fails() {
        let host = WasmHost::new();
        let result = host.load_plugin("test", Path::new("/nonexistent/file.wasm"));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), WasmError::LoadFailed(_)));
    }

    #[test]
    fn load_minimal_wasm_succeeds_but_no_functions() {
        // A bare WASM header is a valid but empty module. Extism accepts it,
        // but any attempt to call a tool function will fail because the module
        // exports nothing.
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("minimal.wasm");
        std::fs::write(&artifact, b"\x00asm\x01\x00\x00\x00").unwrap();

        let host = WasmHost::new();
        let result = host.load_plugin("test", &artifact);
        assert!(
            result.is_ok(),
            "Extism should accept a minimal valid WASM module, got {:?}",
            result
        );
        assert!(host.is_plugin_loaded("test"));
    }

    #[test]
    fn unload_nonexistent_plugin_is_ok() {
        let host = WasmHost::new();
        assert!(host.unload_plugin("no-such-plugin").is_ok());
    }

    #[tokio::test]
    async fn execute_tool_on_unloaded_plugin_returns_not_found() {
        let host = WasmHost::new();
        let result = host
            .execute_tool("no-such-plugin", "tool", serde_json::json!({}))
            .await;
        assert!(matches!(result.unwrap_err(), WasmError::NotFound(_)));
    }

    #[test]
    fn execute_tool_sync_on_unloaded_plugin_returns_not_found() {
        let host = WasmHost::new();
        let result = host.execute_tool_sync("no-such-plugin", "tool", serde_json::json!({}));
        assert!(matches!(result.unwrap_err(), WasmError::NotFound(_)));
    }

    #[test]
    fn is_plugin_loaded_initially_false() {
        let host = WasmHost::new();
        assert!(!host.is_plugin_loaded("any"));
    }

    #[test]
    fn is_plugin_healthy_initially_false() {
        let host = WasmHost::new();
        assert!(!host.is_plugin_healthy("any"));
    }

    #[test]
    fn get_plugin_manifest_initially_none() {
        let host = WasmHost::new();
        assert!(host.get_plugin_manifest("any").is_none());
    }

    #[test]
    fn loaded_plugins_initially_empty() {
        let host = WasmHost::new();
        assert!(host.loaded_plugins().is_empty());
    }

    #[test]
    fn plugin_loader_trait_load_accepts_minimal_wasm() {
        // Extism accepts a minimal WASM header as a valid (but empty) module.
        // The module loads successfully but exports no functions.
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("real.wasm");
        std::fs::write(&artifact, b"\x00asm\x01\x00\x00\x00").unwrap();

        let host = WasmHost::new();
        let result = host.load("test", &artifact);
        assert!(
            result.is_ok(),
            "Extism should accept a minimal valid WASM module"
        );
        assert!(host.is_plugin_loaded("test"));
    }

    #[tokio::test]
    async fn execute_tool_on_empty_plugin_returns_execution_failed() {
        // Load a minimal WASM that has no exported functions.
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("empty.wasm");
        std::fs::write(&artifact, b"\x00asm\x01\x00\x00\x00").unwrap();

        let host = WasmHost::new();
        host.load_plugin("empty-plugin", &artifact).unwrap();

        let result = host
            .execute_tool("empty-plugin", "greet", serde_json::json!({}))
            .await;
        assert!(
            matches!(result, Err(WasmError::ExecutionFailed(_))),
            "Expected ExecutionFailed for missing function, got {:?}",
            result
        );
    }

    #[test]
    fn plugin_loader_trait_unload_noop_for_unknown() {
        let host = WasmHost::new();
        // Unloading a plugin that was never loaded should be a no-op.
        host.unload("no-such-plugin");
        assert!(!host.is_plugin_loaded("no-such-plugin"));
    }

    #[test]
    fn debug_impl_works() {
        let host = WasmHost::new();
        let debug_str = format!("{:?}", host);
        assert!(debug_str.contains("WasmHost"));
    }

    #[test]
    fn clone_shares_state() {
        let host = WasmHost::new();
        let host2 = host.clone();
        // Both point to the same inner state
        assert!(host2.loaded_plugins().is_empty());
        assert!(host.loaded_plugins().is_empty());
    }

    #[test]
    fn set_manifest_on_nonexistent_plugin_is_noop() {
        let host = WasmHost::new();
        // Should not panic
        host.set_manifest("no-such-plugin", sample_manifest());
        assert!(host.get_plugin_manifest("no-such-plugin").is_none());
    }

    // ---- Phase 9.2: Additional host-level tests ----

    #[test]
    fn load_and_unload_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("rt.wasm");
        std::fs::write(&artifact, b"\x00asm\x01\x00\x00\x00").unwrap();

        let host = WasmHost::new();
        host.load_plugin("rt", &artifact).unwrap();
        assert!(host.is_plugin_loaded("rt"));
        assert!(host.loaded_plugins().contains(&"rt".to_string()));

        host.unload_plugin("rt").unwrap();
        assert!(!host.is_plugin_loaded("rt"));
        assert!(!host.loaded_plugins().contains(&"rt".to_string()));
    }

    #[test]
    fn load_same_plugin_id_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("replace.wasm");
        std::fs::write(&artifact, b"\x00asm\x01\x00\x00\x00").unwrap();

        let host = WasmHost::new();
        host.load_plugin("dup", &artifact).unwrap();
        // Loading again with the same ID should succeed (replace).
        host.load_plugin("dup", &artifact).unwrap();
        assert!(host.is_plugin_loaded("dup"));
    }

    #[test]
    fn health_check_delegates_to_artifact_existence() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("health.wasm");
        std::fs::write(&artifact, b"\x00asm\x01\x00\x00\x00").unwrap();

        let host = WasmHost::new();
        host.load_plugin("hp", &artifact).unwrap();
        assert!(host.is_plugin_healthy("hp"));

        // Delete the artifact — plugin is loaded but no longer healthy.
        std::fs::remove_file(&artifact).unwrap();
        assert!(!host.is_plugin_healthy("hp"));
        // Still loaded though.
        assert!(host.is_plugin_loaded("hp"));
    }

    #[test]
    fn set_and_get_manifest_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("manifest.wasm");
        std::fs::write(&artifact, b"\x00asm\x01\x00\x00\x00").unwrap();

        let host = WasmHost::new();
        host.load_plugin("mp", &artifact).unwrap();

        assert!(host.get_plugin_manifest("mp").is_none());

        let m = sample_manifest();
        host.set_manifest("mp", m.clone());
        let retrieved = host.get_plugin_manifest("mp").unwrap();
        assert_eq!(retrieved.identity.id, m.identity.id);
        assert_eq!(retrieved.tools.len(), m.tools.len());
    }

    #[test]
    fn sync_execution_on_unloaded_returns_not_found() {
        let host = WasmHost::new();
        let err = host
            .execute_tool_sync("nope", "tool", serde_json::json!({}))
            .unwrap_err();
        assert!(matches!(err, WasmError::NotFound(_)));
    }

    #[test]
    fn sync_execution_on_empty_plugin_returns_execution_failed() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("empty_sync.wasm");
        std::fs::write(&artifact, b"\x00asm\x01\x00\x00\x00").unwrap();

        let host = WasmHost::new();
        host.load_plugin("empty-sync", &artifact).unwrap();

        let err = host
            .execute_tool_sync("empty-sync", "greet", serde_json::json!({}))
            .unwrap_err();
        assert!(
            matches!(err, WasmError::ExecutionFailed(ref msg) if msg.contains("does not export")),
            "expected ExecutionFailed for missing export, got: {:?}",
            err
        );
    }

    #[test]
    fn clone_independent_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("clone.wasm");
        std::fs::write(&artifact, b"\x00asm\x01\x00\x00\x00").unwrap();

        let host1 = WasmHost::new();
        let host2 = host1.clone();

        // Load via host1, verify visible via host2.
        host1.load_plugin("shared", &artifact).unwrap();
        assert!(host2.is_plugin_loaded("shared"));

        // Unload via host2, verify gone via host1.
        host2.unload_plugin("shared").unwrap();
        assert!(!host1.is_plugin_loaded("shared"));
    }

    #[test]
    fn error_display_contains_useful_info() {
        let err = WasmError::NotFound("my-plugin".to_string());
        assert!(err.to_string().contains("my-plugin"));

        let err = WasmError::LoadFailed("bad wasm".to_string());
        assert!(err.to_string().contains("bad wasm"));

        let err = WasmError::Timeout;
        assert!(err.to_string().contains("timed out"));

        let err = WasmError::InvalidInput("bad json".to_string());
        assert!(err.to_string().contains("bad json"));

        let err = WasmError::PluginPanicked("trap".to_string());
        assert!(err.to_string().contains("trap"));
    }

    #[test]
    fn wasm_error_is_std_error() {
        let err = WasmError::LoadFailed("test".to_string());
        let _: &dyn std::error::Error = &err;
    }
}
