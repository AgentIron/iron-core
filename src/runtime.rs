//! Shared runtime state for `IronAgent`.

use crate::{
    capability::{CapabilityBackend, CapabilityDescriptor, CapabilityRegistry},
    config::Config,
    durable::{DurableSession, SessionId},
    ephemeral::EphemeralTurn,
    error::RuntimeError,
    mcp::{McpConnectionManager, McpServerRegistry, ReconnectConfig, SessionToolCatalog},
    plugin::auth::{AuthInteractionRequest, AuthInteractionResponse, AuthStatusTransition, CredentialBinding},
    plugin::effective_tools::{EffectivePluginToolView, SessionPluginToolSummary},
    plugin::registry::{PluginAvailabilitySummary, PluginRegistry},
    plugin::status::{PluginInfo, PluginStatus},
    plugin::wasm_host::WasmHost,
    tool::ToolRegistry,
};
use iron_providers::Provider;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};
use tokio::sync::watch;
use tokio::task::JoinHandle;

struct RuntimeInner {
    config: Config,
    provider: Arc<dyn Provider>,
    capabilities: RwLock<CapabilityRegistry>,
    tool_registry: RwLock<ToolRegistry>,
    mcp_registry: RwLock<McpServerRegistry>,
    mcp_connection_manager: Arc<McpConnectionManager>,
    plugin_registry: RwLock<PluginRegistry>,
    wasm_host: RwLock<WasmHost>,
    sessions: RwLock<HashMap<SessionId, Arc<RuntimeSession>>>,
    connections: RwLock<HashMap<ConnectionId, Arc<RuntimeConnection>>>,
    tokio_handle: tokio::runtime::Handle,
    _owned_runtime: Option<tokio::runtime::Runtime>,
    is_shutdown: AtomicBool,
    shutdown_tx: watch::Sender<bool>,
    active_tasks: Mutex<Vec<JoinHandle<()>>>,
}

struct ActivePrompt {
    ephemeral: Arc<std::sync::Mutex<EphemeralTurn>>,
}

struct RuntimeSession {
    session: Arc<Mutex<DurableSession>>,
    connection_id: ConnectionId,
    active_prompt: Mutex<Option<ActivePrompt>>,
}

impl RuntimeSession {
    fn new(session: Arc<Mutex<DurableSession>>, connection_id: ConnectionId) -> Self {
        Self {
            session,
            connection_id,
            active_prompt: Mutex::new(None),
        }
    }
}

struct RuntimeConnection {
    active: AtomicBool,
}

/// Stable identifier for a client connection registered with the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

/// Shared runtime backing one or more `IronAgent` facade values.
///
/// `IronRuntime` owns the provider, tool registry, capability registry, session
/// store, and Tokio runtime handle used for orchestration.
pub struct IronRuntime {
    inner: Arc<RuntimeInner>,
}

impl IronRuntime {
    fn apply_runtime_mcp_policy_to_session(&self, durable: &mut DurableSession) {
        if !self.inner.config.mcp.enabled {
            return;
        }

        let runtime_default = self.inner.config.mcp.enabled_by_default;
        let mcp_registry = self.mcp_registry();
        for server in mcp_registry.list_servers() {
            durable
                .mcp_server_enablement
                .entry(server.config.id)
                .or_insert(runtime_default);
        }
    }

    /// Apply the runtime-level plugin default policy to a session's
    /// enablement map.  Only adds entries for plugins that do not already
    /// have an explicit value (preserves admin/client overrides).
    fn apply_runtime_plugin_policy_to_session(&self, durable: &mut DurableSession) {
        if !self.inner.config.plugins.enabled {
            return;
        }

        let runtime_default = self.inner.config.plugins.enabled_by_default;
        let plugin_registry = self.plugin_registry();
        for plugin in plugin_registry.list() {
            let plugin_id = &plugin.config.id;
            // Only insert if not already set — imported sessions may carry
            // their own enablement choices.
            if durable.is_plugin_enabled(plugin_id).is_none() {
                durable.set_plugin_enabled(plugin_id, runtime_default);
            }
        }
    }

    fn initialize_existing_sessions_for_new_mcp_server(&self, server_id: &str) {
        if !self.inner.config.mcp.enabled {
            return;
        }

        let runtime_default = self.inner.config.mcp.enabled_by_default;
        let sessions = self.inner.sessions.read().unwrap();
        for runtime_session in sessions.values() {
            if let Ok(mut session) = runtime_session.session.lock() {
                session
                    .mcp_server_enablement
                    .entry(server_id.to_string())
                    .or_insert(runtime_default);
            }
        }
    }

    /// When a new plugin is registered, seed any existing sessions with
    /// the runtime-default enablement value (same pattern as MCP).
    fn initialize_existing_sessions_for_new_plugin(&self, plugin_id: &str) {
        if !self.inner.config.plugins.enabled {
            return;
        }

        let runtime_default = self.inner.config.plugins.enabled_by_default;
        let sessions = self.inner.sessions.read().unwrap();
        for runtime_session in sessions.values() {
            if let Ok(mut session) = runtime_session.session.lock() {
                if session.is_plugin_enabled(plugin_id).is_none() {
                    session.set_plugin_enabled(plugin_id, runtime_default);
                }
            }
        }
    }

    /// Create a new runtime with a privately owned Tokio runtime.
    pub fn new<P>(config: Config, provider: P) -> Self
    where
        P: Provider + 'static,
    {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
        let handle = runtime.handle().clone();
        let (shutdown_tx, _) = watch::channel(false);
        let mcp_registry = McpServerRegistry::new();
        let mcp_connection_manager = Arc::new(McpConnectionManager::new(mcp_registry.clone()));

        let inner = RuntimeInner {
            config,
            provider: Arc::new(provider),
            capabilities: RwLock::new(CapabilityRegistry::new()),
            tool_registry: RwLock::new(ToolRegistry::new()),
            mcp_registry: RwLock::new(mcp_registry),
            mcp_connection_manager,
            plugin_registry: RwLock::new(PluginRegistry::new()),
            wasm_host: RwLock::new(WasmHost::new()),
            sessions: RwLock::new(HashMap::new()),
            connections: RwLock::new(HashMap::new()),
            tokio_handle: handle,
            _owned_runtime: Some(runtime),
            is_shutdown: AtomicBool::new(false),
            shutdown_tx,
            active_tasks: Mutex::new(Vec::new()),
        };

        let this = Self {
            inner: Arc::new(inner),
        };

        if this.inner.config.mcp.enabled {
            let manager = this.inner.mcp_connection_manager.clone();
            let shutdown_rx = this.shutdown_token();
            let _ = this.spawn(async move {
                manager.start(ReconnectConfig::default(), shutdown_rx).await;
            });
        }

        this
    }

    /// Create a new runtime using an existing Tokio runtime handle.
    pub fn from_handle<P>(config: Config, provider: P, handle: tokio::runtime::Handle) -> Self
    where
        P: Provider + 'static,
    {
        let (shutdown_tx, _) = watch::channel(false);
        let mcp_registry = McpServerRegistry::new();
        let mcp_connection_manager = Arc::new(McpConnectionManager::new(mcp_registry.clone()));

        let inner = RuntimeInner {
            config,
            provider: Arc::new(provider),
            capabilities: RwLock::new(CapabilityRegistry::new()),
            tool_registry: RwLock::new(ToolRegistry::new()),
            mcp_registry: RwLock::new(mcp_registry),
            mcp_connection_manager,
            plugin_registry: RwLock::new(PluginRegistry::new()),
            wasm_host: RwLock::new(WasmHost::new()),
            sessions: RwLock::new(HashMap::new()),
            connections: RwLock::new(HashMap::new()),
            tokio_handle: handle,
            _owned_runtime: None,
            is_shutdown: AtomicBool::new(false),
            shutdown_tx,
            active_tasks: Mutex::new(Vec::new()),
        };

        let this = Self {
            inner: Arc::new(inner),
        };

        if this.inner.config.mcp.enabled {
            let manager = this.inner.mcp_connection_manager.clone();
            let shutdown_rx = this.shutdown_token();
            let _ = this.spawn(async move {
                manager.start(ReconnectConfig::default(), shutdown_rx).await;
            });
        }

        this
    }

    /// Borrow the validated runtime configuration.
    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    /// Borrow the provider implementation used for inference.
    pub fn provider(&self) -> &dyn Provider {
        self.inner.provider.as_ref()
    }

    /// Borrow the tool registry.
    pub fn tool_registry(&self) -> std::sync::RwLockReadGuard<'_, ToolRegistry> {
        self.inner.tool_registry.read().unwrap()
    }

    /// Register a custom tool with the runtime.
    pub fn register_tool<T: crate::tool::Tool + 'static>(&self, tool: T) {
        self.inner.tool_registry.write().unwrap().register(tool);
    }

    /// Register the built-in tool set using the supplied configuration.
    pub fn register_builtin_tools(&self, config: &crate::builtin::BuiltinToolConfig) {
        let mut registry = self.inner.tool_registry.write().unwrap();
        crate::builtin::register_builtin_tools(&mut registry, config);
    }

    #[cfg(feature = "embedded-python")]
    /// Register the embedded Python execution tool.
    pub fn register_python_exec_tool(&self) {
        self.register_tool(crate::embedded_python::PythonExecTool::new());
    }

    /// Borrow the capability registry.
    pub fn capabilities(&self) -> std::sync::RwLockReadGuard<'_, CapabilityRegistry> {
        self.inner.capabilities.read().unwrap()
    }

    /// Register a capability descriptor.
    pub fn register_capability(&self, descriptor: CapabilityDescriptor) {
        self.inner
            .capabilities
            .write()
            .unwrap()
            .register(descriptor);
    }

    /// Override the backend used for a capability.
    pub fn set_capability_backend(
        &self,
        capability_id: crate::capability::CapabilityId,
        backend: CapabilityBackend,
    ) {
        let mut caps = self.inner.capabilities.write().unwrap();
        if let Some(desc) = caps.get_mut(capability_id) {
            desc.backend = backend;
        }
    }

    /// Borrow the MCP server registry.
    pub fn mcp_registry(&self) -> std::sync::RwLockReadGuard<'_, McpServerRegistry> {
        self.inner.mcp_registry.read().unwrap()
    }

    /// Register an MCP server configuration.
    pub fn register_mcp_server(&self, config: crate::mcp::McpServerConfig) {
        let server_id = config.id.clone();
        self.inner
            .mcp_registry
            .write()
            .unwrap()
            .register_server(config);
        self.initialize_existing_sessions_for_new_mcp_server(&server_id);

        if self.inner.config.mcp.enabled {
            let manager = self.inner.mcp_connection_manager.clone();
            let _ = self.spawn(async move {
                manager.connect_server(&server_id).await;
            });
        }
    }

    pub fn mcp_connection_manager(&self) -> Arc<McpConnectionManager> {
        self.inner.mcp_connection_manager.clone()
    }

    /// Borrow the plugin registry.
    pub fn plugin_registry(&self) -> std::sync::RwLockReadGuard<'_, PluginRegistry> {
        self.inner.plugin_registry.read().unwrap()
    }

    /// Register a plugin configuration.
    pub fn register_plugin(&self, config: crate::plugin::config::PluginConfig) {
        let plugin_id = config.id.clone();
        self.inner.plugin_registry.write().unwrap().register(config);
        self.initialize_existing_sessions_for_new_plugin(&plugin_id);
    }

    /// Borrow the Tokio runtime handle used for orchestration.
    pub fn tokio_handle(&self) -> &tokio::runtime::Handle {
        &self.inner.tokio_handle
    }

    /// Return whether the runtime has started shutting down.
    pub fn is_shutdown(&self) -> bool {
        self.inner.is_shutdown.load(Ordering::SeqCst)
    }

    /// Subscribe to runtime shutdown notifications.
    pub fn shutdown_token(&self) -> watch::Receiver<bool> {
        self.inner.shutdown_tx.subscribe()
    }

    /// Spawn a task on the runtime if it is still active.
    pub fn spawn<F>(&self, future: F) -> bool
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        if self.inner.is_shutdown.load(Ordering::SeqCst) {
            return false;
        }
        let handle = self.inner.tokio_handle.spawn(future);
        self.inner.active_tasks.lock().unwrap().push(handle);
        true
    }

    pub fn register_connection(&self, id: ConnectionId) {
        let conn = Arc::new(RuntimeConnection {
            active: AtomicBool::new(true),
        });
        self.inner.connections.write().unwrap().insert(id, conn);
    }

    pub fn close_connection(&self, id: ConnectionId) {
        if let Some(conn) = self.inner.connections.write().unwrap().get(&id) {
            conn.active.store(false, Ordering::SeqCst);
        }
        self.inner.connections.write().unwrap().remove(&id);
        self.close_sessions_for_connection(id);
    }

    pub fn connection_count(&self) -> usize {
        self.inner.connections.read().unwrap().len()
    }

    pub fn create_session(
        &self,
        connection_id: ConnectionId,
    ) -> Result<(SessionId, Arc<Mutex<DurableSession>>), RuntimeError> {
        if self.is_shutdown() {
            return Err(RuntimeError::Connection("Runtime is shut down".into()));
        }

        let session_id = SessionId::new();
        let mut durable = DurableSession::new(session_id);

        let repo_payload = crate::prompt::RepoInstructionLoader::resolve(
            &self.inner.config.prompt_composition.repo_instructions,
        )
        .ok();

        if let Some(ref payload) = repo_payload {
            let mut payload = payload.clone();
            let _ = crate::prompt::RepoInstructionLoader::load_additional_files(
                &mut payload,
                &self.inner.config.prompt_composition.additional_files,
            );
            durable.repo_instruction_payload = Some(payload);
        }

        // Initialize MCP server enablement state for new session
        // Uses the single runtime-level default policy without per-server override
        self.apply_runtime_mcp_policy_to_session(&mut durable);

        // Initialize plugin enablement state for new session
        // Uses the single runtime-level default policy without per-plugin override
        if self.inner.config.plugins.enabled {
            let plugin_registry = self.plugin_registry();
            let runtime_default = self.inner.config.plugins.enabled_by_default;
            for plugin in plugin_registry.list() {
                durable.set_plugin_enabled(&plugin.config.id, runtime_default);
            }
        }

        let session = Arc::new(Mutex::new(durable));

        let runtime_session = RuntimeSession::new(session.clone(), connection_id);

        self.inner
            .sessions
            .write()
            .unwrap()
            .insert(session_id, Arc::new(runtime_session));

        Ok((session_id, session))
    }

    pub fn insert_session(
        &self,
        session_id: SessionId,
        mut durable: DurableSession,
        connection_id: ConnectionId,
    ) -> Result<(), RuntimeError> {
        if self.is_shutdown() {
            return Err(RuntimeError::Connection("Runtime is shut down".into()));
        }
        // Apply destination-runtime defaults for both MCP and plugin enablement.
        // Uses .entry() / is_none() guards so existing client choices are preserved.
        self.apply_runtime_mcp_policy_to_session(&mut durable);
        self.apply_runtime_plugin_policy_to_session(&mut durable);
        let session = Arc::new(Mutex::new(durable));
        let runtime_session = RuntimeSession::new(session, connection_id);
        self.inner
            .sessions
            .write()
            .unwrap()
            .insert(session_id, Arc::new(runtime_session));
        Ok(())
    }

    pub fn get_session(&self, id: SessionId) -> Option<Arc<Mutex<DurableSession>>> {
        self.inner
            .sessions
            .read()
            .unwrap()
            .get(&id)
            .map(|rs| rs.session.clone())
    }

    pub fn get_session_connection(&self, id: SessionId) -> Option<ConnectionId> {
        self.inner
            .sessions
            .read()
            .unwrap()
            .get(&id)
            .map(|rs| rs.connection_id)
    }

    pub fn close_session(&self, id: SessionId) {
        self.inner.sessions.write().unwrap().remove(&id);
    }

    pub fn close_sessions_for_connection(&self, connection_id: ConnectionId) {
        let mut sessions = self.inner.sessions.write().unwrap();
        let to_remove: Vec<SessionId> = sessions
            .iter()
            .filter(|(_, rs)| rs.connection_id == connection_id)
            .map(|(id, _)| *id)
            .collect();

        for id in to_remove {
            sessions.remove(&id);
        }
    }

    pub fn try_start_prompt(
        &self,
        session_id: SessionId,
    ) -> Result<Arc<std::sync::Mutex<EphemeralTurn>>, RuntimeError> {
        let sessions = self.inner.sessions.read().unwrap();
        let rs = sessions
            .get(&session_id)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.to_string()))?;
        let mut active = rs.active_prompt.lock().unwrap();
        if active.is_some() {
            return Err(RuntimeError::Turn(
                "session already has an active prompt".into(),
            ));
        }
        let ephemeral = Arc::new(std::sync::Mutex::new(EphemeralTurn::new(session_id)));
        ephemeral.lock().unwrap().start();
        *active = Some(ActivePrompt {
            ephemeral: ephemeral.clone(),
        });
        Ok(ephemeral)
    }

    pub fn finish_prompt(&self, session_id: SessionId) {
        let sessions = self.inner.sessions.read().unwrap();
        if let Some(rs) = sessions.get(&session_id) {
            let mut active = rs.active_prompt.lock().unwrap();
            *active = None;
        }
    }

    pub fn cancel_active_prompt(&self, session_id: SessionId) -> bool {
        let sessions = self.inner.sessions.read().unwrap();
        if let Some(rs) = sessions.get(&session_id) {
            let active = rs.active_prompt.lock().unwrap();
            if let Some(prompt) = active.as_ref() {
                prompt.ephemeral.lock().unwrap().cancel();
                return true;
            }
        }
        false
    }

    pub fn has_active_prompt(&self, session_id: SessionId) -> bool {
        let sessions = self.inner.sessions.read().unwrap();
        sessions
            .get(&session_id)
            .map(|rs| rs.active_prompt.lock().unwrap().is_some())
            .unwrap_or(false)
    }

    pub fn get_active_prompt_ephemeral(
        &self,
        session_id: SessionId,
    ) -> Option<Arc<std::sync::Mutex<EphemeralTurn>>> {
        let sessions = self.inner.sessions.read().unwrap();
        sessions.get(&session_id).and_then(|rs| {
            rs.active_prompt
                .lock()
                .unwrap()
                .as_ref()
                .map(|p| p.ephemeral.clone())
        })
    }

    pub fn session_count(&self) -> usize {
        self.inner.sessions.read().unwrap().len()
    }

    pub fn sessions_for_connection(&self, connection_id: ConnectionId) -> Vec<SessionId> {
        self.inner
            .sessions
            .read()
            .unwrap()
            .iter()
            .filter(|(_, rs)| rs.connection_id == connection_id)
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn shutdown(&self) {
        let manager = self.inner.mcp_connection_manager.clone();
        let _shutdown_handle = self.inner.tokio_handle.spawn(async move {
            manager.shutdown().await;
        });

        self.inner.is_shutdown.store(true, Ordering::SeqCst);
        let _ = self.inner.shutdown_tx.send(true);

        let tasks = std::mem::take(&mut *self.inner.active_tasks.lock().unwrap());
        for handle in tasks {
            handle.abort();
        }

        self.inner.sessions.write().unwrap().clear();
        self.inner.connections.write().unwrap().clear();
    }

    /// Get the session-effective tool definitions exposed by the runtime.
    /// This follows the same session-effective path used for prompt construction
    /// and execution.
    pub fn get_effective_tool_definitions(
        &self,
        session_id: SessionId,
    ) -> Vec<crate::tool::ToolDefinition> {
        if let Some(catalog) = self.get_session_tool_catalog(session_id) {
            return catalog.definitions().to_vec();
        }

        self.tool_registry().definitions()
    }

    /// Get a session-effective tool catalog that can be used for both
    /// provider request building and tool execution.
    pub fn get_session_tool_catalog(&self, session_id: SessionId) -> Option<SessionToolCatalog> {
        let session = self.get_session(session_id)?;
        let session_guard = session.lock().ok()?;

        let local_registry = Arc::new((*self.tool_registry()).clone());
        let mcp_registry = Arc::new((*self.mcp_registry()).clone());
        let plugin_registry = Arc::new((*self.plugin_registry()).clone());
        let wasm_host = Arc::new((*self.inner.wasm_host.read().ok()?).clone());
        let connection_manager = self.mcp_connection_manager();

        Some(SessionToolCatalog::new(
            local_registry,
            mcp_registry,
            plugin_registry,
            wasm_host,
            connection_manager,
            &session_guard,
        ))
    }

    // -----------------------------------------------------------------------
    // Session-scoped plugin controls (Phase 6)
    // -----------------------------------------------------------------------

    /// Enable or disable a plugin for a specific session.
    ///
    /// This is the runtime-level entry point for callers that have a
    /// [`SessionId`] but not an [`AgentSession`](crate::facade::AgentSession).
    pub fn set_session_plugin_enabled(
        &self,
        session_id: SessionId,
        plugin_id: impl Into<String>,
        enabled: bool,
    ) -> Result<(), RuntimeError> {
        let session = self
            .get_session(session_id)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.to_string()))?;
        let mut guard = session
            .lock()
            .map_err(|_| RuntimeError::Connection(session_id.to_string()))?;
        guard.set_plugin_enabled(plugin_id, enabled);
        Ok(())
    }

    /// Check whether a plugin is enabled for a specific session.
    ///
    /// Returns `None` if the session does not exist or the plugin has no
    /// explicit enablement state for the session.
    pub fn is_session_plugin_enabled(
        &self,
        session_id: SessionId,
        plugin_id: &str,
    ) -> Option<bool> {
        let session = self.get_session(session_id)?;
        let guard = session.lock().ok()?;
        guard.is_plugin_enabled(plugin_id)
    }

    /// Get a full inventory of all registered plugins.
    ///
    /// Returns a [`PluginInfo`] for every plugin in the registry, reflecting
    /// the current runtime state (health, auth, tool counts).
    pub fn get_plugin_inventory(&self) -> Vec<PluginInfo> {
        let registry = self.plugin_registry();
        registry
            .list()
            .iter()
            .filter_map(|p| registry.get_plugin_info(&p.config.id))
            .collect()
    }

    /// Get auth prompts for all plugins that require authentication.
    ///
    /// Returns a list of [`AuthPrompt`](crate::plugin::auth::AuthPrompt)
    /// values for every registered plugin that declares OAuth requirements.
    pub fn get_auth_prompts(&self) -> Vec<crate::plugin::auth::AuthPrompt> {
        self.inner.plugin_registry.read().unwrap().get_auth_prompts()
    }

    /// Get the runtime status of a single plugin.
    ///
    /// Returns `None` if the plugin is not registered.
    pub fn get_plugin_status(&self, plugin_id: &str) -> Option<PluginStatus> {
        self.plugin_registry().get_status(plugin_id)
    }

    /// Set credentials for a plugin and mark it as authenticated.
    ///
    /// This triggers a recomputation of per-tool availability.
    pub fn set_plugin_credentials(&self, plugin_id: &str, credentials: CredentialBinding) {
        self.inner
            .plugin_registry
            .write()
            .unwrap()
            .set_credentials(plugin_id, credentials);
    }

    /// Clear credentials for a plugin and reset its auth state.
    ///
    /// This triggers a recomputation of per-tool availability.
    pub fn clear_plugin_credentials(&self, plugin_id: &str) {
        self.inner
            .plugin_registry
            .write()
            .unwrap()
            .clear_credentials(plugin_id);
    }

    /// Start a direct client-initiated auth flow for a plugin.
    ///
    /// Validates that the plugin exists, requires auth, and is in a state
    /// that allows starting authentication.  Returns an
    /// [`AuthInteractionRequest`] that the client should act on (e.g. open
    /// a browser to the authorization URL).
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin is not found, does not require auth,
    /// is already authenticating, or is already authenticated.
    pub fn begin_plugin_auth_flow(
        &self,
        plugin_id: &str,
    ) -> Result<AuthInteractionRequest, String> {
        self.inner
            .plugin_registry
            .write()
            .unwrap()
            .begin_auth_flow(plugin_id)
    }

    /// Complete a direct client-initiated auth flow for a plugin.
    ///
    /// Processes the client's [`AuthInteractionResponse`].  On success,
    /// stores credentials and transitions to `Authenticated`.  On denial,
    /// failure, or cancellation, transitions back to `Unauthenticated`.
    ///
    /// Returns the [`AuthStatusTransition`] describing the state change.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin is not found or is not in the
    /// `Authenticating` state.
    pub fn complete_plugin_auth_flow(
        &self,
        plugin_id: &str,
        response: AuthInteractionResponse,
    ) -> Result<AuthStatusTransition, String> {
        self.inner
            .plugin_registry
            .write()
            .unwrap()
            .complete_auth_flow(plugin_id, response)
    }

    /// Get a session-scoped summary of plugin tool availability.
    ///
    /// Combines the runtime-level registry state with the session's plugin
    /// enablement to produce a per-plugin summary of how many tools are
    /// usable for the given session.
    ///
    /// Returns `None` if the session does not exist.
    pub fn get_session_plugin_summary(
        &self,
        session_id: SessionId,
    ) -> Option<SessionPluginToolSummary> {
        let session = self.get_session(session_id)?;
        let guard = session.lock().ok()?;
        let plugin_registry = Arc::new((*self.plugin_registry()).clone());
        let wasm_host = Arc::new((*self.inner.wasm_host.read().ok()?).clone());
        let view = EffectivePluginToolView::new(plugin_registry, wasm_host);
        Some(view.get_session_summary(&guard, &guard.plugin_enablement))
    }

    /// Get a recomputed availability summary for a single plugin.
    ///
    /// Returns `None` if the plugin is not registered.
    pub fn get_plugin_availability(&self, plugin_id: &str) -> Option<PluginAvailabilitySummary> {
        self.inner
            .plugin_registry
            .read()
            .unwrap()
            .recompute_availability(plugin_id)
    }

    /// Get unified tool diagnostics for a session.
    ///
    /// Returns `None` if the session does not exist.
    pub fn get_session_tool_diagnostics(
        &self,
        session_id: SessionId,
    ) -> Option<Vec<crate::mcp::session_catalog::ToolDiagnostic>> {
        let catalog = self.get_session_tool_catalog(session_id)?;
        let session = self.get_session(session_id)?;
        let guard = session.lock().ok()?;
        Some(catalog.inspect_tools(&guard))
    }
}

impl Clone for IronRuntime {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        let tasks = std::mem::take(&mut *self.active_tasks.lock().unwrap());
        for handle in tasks {
            handle.abort();
        }
        if let Some(runtime) = self._owned_runtime.take() {
            runtime.shutdown_background();
        }
    }
}

impl std::fmt::Debug for IronRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IronRuntime")
            .field("session_count", &self.session_count())
            .field("connection_count", &self.connection_count())
            .field("is_shutdown", &self.is_shutdown())
            .finish()
    }
}
