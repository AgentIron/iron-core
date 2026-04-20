//! Shared runtime state for `IronAgent`.

use crate::{
    capability::{CapabilityBackend, CapabilityDescriptor, CapabilityRegistry},
    config::Config,
    durable::{DurableSession, SessionId},
    ephemeral::EphemeralTurn,
    error::RuntimeError,
    mcp::{McpConnectionManager, McpServerRegistry, ReconnectConfig, SessionToolCatalog},
    plugin::auth::{
        AuthInteractionRequest, AuthInteractionResponse, AuthStatusTransition, CredentialBinding,
    },
    plugin::effective_tools::{EffectivePluginToolView, SessionPluginToolSummary},
    plugin::registry::{PluginAvailabilitySummary, PluginRegistry},
    plugin::status::{PluginInfo, PluginStatus},
    plugin::wasm_host::WasmHost,
    skill::{LoadedSkill, SkillCatalog, SkillDiagnostic},
    skill::source::FilesystemSkillSource,
    skill::SkillOrigin,
    tool::ToolRegistry,
};
use iron_providers::Provider;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
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
    skill_catalog: RwLock<SkillCatalog>,
    sessions: RwLock<HashMap<SessionId, Arc<RuntimeSession>>>,
    connections: RwLock<HashMap<ConnectionId, Arc<RuntimeConnection>>>,
    tokio_handle: tokio::runtime::Handle,
    _owned_runtime: Option<tokio::runtime::Runtime>,
    is_shutdown: AtomicBool,
    shutdown_tx: watch::Sender<bool>,
    active_tasks: Mutex<Vec<JoinHandle<()>>>,
}

struct ActivePrompt {
    ephemeral: Arc<Mutex<EphemeralTurn>>,
}

struct RuntimeSession {
    session: Arc<Mutex<DurableSession>>,
    connection_id: ConnectionId,
    active_prompt: Mutex<Option<ActivePrompt>>,
    tool_catalog_cache: Mutex<Option<CachedSessionToolCatalog>>,
}

struct CachedSessionToolCatalog {
    tool_registry_version: u64,
    mcp_registry_version: u64,
    plugin_registry_version: u64,
    mcp_server_enablement: std::collections::HashMap<String, bool>,
    plugin_enablement: crate::plugin::session::SessionPluginEnablement,
    available_skills: Vec<(String, String)>,
    catalog: Arc<SessionToolCatalog>,
}

impl RuntimeSession {
    fn new(session: Arc<Mutex<DurableSession>>, connection_id: ConnectionId) -> Self {
        Self {
            session,
            connection_id,
            active_prompt: Mutex::new(None),
            tool_catalog_cache: Mutex::new(None),
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
        let sessions = self.inner.sessions.read();
        for runtime_session in sessions.values() {
            let mut session = runtime_session.session.lock();
            session
                .mcp_server_enablement
                .entry(server_id.to_string())
                .or_insert(runtime_default);
        }
    }

    /// When a new plugin is registered, seed any existing sessions with
    /// the runtime-default enablement value (same pattern as MCP).
    fn initialize_existing_sessions_for_new_plugin(&self, plugin_id: &str) {
        if !self.inner.config.plugins.enabled {
            return;
        }

        let runtime_default = self.inner.config.plugins.enabled_by_default;
        let sessions = self.inner.sessions.read();
        for runtime_session in sessions.values() {
            let mut session = runtime_session.session.lock();
            if session.is_plugin_enabled(plugin_id).is_none() {
                session.set_plugin_enabled(plugin_id, runtime_default);
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
        let plugin_max_memory = config.plugins.max_memory_bytes;

        let skill_catalog = RwLock::new(SkillCatalog::new());

        let inner = RuntimeInner {
            config,
            provider: Arc::new(provider),
            capabilities: RwLock::new(CapabilityRegistry::new()),
            tool_registry: RwLock::new(ToolRegistry::new()),
            mcp_registry: RwLock::new(mcp_registry),
            mcp_connection_manager,
            plugin_registry: RwLock::new(PluginRegistry::new()),
            wasm_host: RwLock::new(WasmHost::with_max_memory_bytes(plugin_max_memory)),
            skill_catalog,
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

        if this.inner.config.skills.enabled {
            this.register_activate_skill_tool();
            this.refresh_skill_catalog();
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
        let plugin_max_memory = config.plugins.max_memory_bytes;

        let skill_catalog = RwLock::new(SkillCatalog::new());

        let inner = RuntimeInner {
            config,
            provider: Arc::new(provider),
            capabilities: RwLock::new(CapabilityRegistry::new()),
            tool_registry: RwLock::new(ToolRegistry::new()),
            mcp_registry: RwLock::new(mcp_registry),
            mcp_connection_manager,
            plugin_registry: RwLock::new(PluginRegistry::new()),
            wasm_host: RwLock::new(WasmHost::with_max_memory_bytes(plugin_max_memory)),
            skill_catalog,
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

        if this.inner.config.skills.enabled {
            this.register_activate_skill_tool();
            this.refresh_skill_catalog();
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
    pub fn tool_registry(&self) -> parking_lot::RwLockReadGuard<'_, ToolRegistry> {
        self.inner.tool_registry.read()
    }

    /// Register a custom tool with the runtime.
    pub fn register_tool<T: crate::tool::Tool + 'static>(&self, tool: T) {
        self.inner.tool_registry.write().register(tool);
    }

    /// Register the built-in tool set using the supplied configuration.
    pub fn register_builtin_tools(&self, config: &crate::builtin::BuiltinToolConfig) {
        let mut registry = self.inner.tool_registry.write();
        crate::builtin::register_builtin_tools(&mut registry, config);
    }

    #[cfg(feature = "embedded-python")]
    /// Register the embedded Python execution tool.
    pub fn register_python_exec_tool(&self) {
        self.register_tool(crate::embedded_python::PythonExecTool::new());
    }

    /// Register the `activate_skill` model-facing tool.
    pub fn register_activate_skill_tool(&self) {
        use crate::tool::{ToolDefinition, FunctionTool};
        let definition = ToolDefinition::new(
            "activate_skill",
            "Activate a skill by name to receive its instructions. The skill will be loaded into the session context.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "skill_name": {
                        "type": "string",
                        "description": "The name of the skill to activate"
                    }
                },
                "required": ["skill_name"]
            }),
        );
        let tool = FunctionTool::new(definition, |_args| {
            // Actual activation is handled by the orchestrator; this dummy
            // handler should never be called.
            Err(crate::error::RuntimeError::tool_execution(
                "activate_skill must be handled by the orchestrator".to_string(),
            ))
        });
        self.register_tool(tool);
    }

    /// Borrow the capability registry.
    pub fn capabilities(&self) -> parking_lot::RwLockReadGuard<'_, CapabilityRegistry> {
        self.inner.capabilities.read()
    }

    /// Register a capability descriptor.
    pub fn register_capability(&self, descriptor: CapabilityDescriptor) {
        self.inner.capabilities.write().register(descriptor);
    }

    /// Override the backend used for a capability.
    pub fn set_capability_backend(
        &self,
        capability_id: crate::capability::CapabilityId,
        backend: CapabilityBackend,
    ) {
        let mut caps = self.inner.capabilities.write();
        if let Some(desc) = caps.get_mut(capability_id) {
            desc.backend = backend;
        }
    }

    /// Borrow the MCP server registry.
    pub fn mcp_registry(&self) -> parking_lot::RwLockReadGuard<'_, McpServerRegistry> {
        self.inner.mcp_registry.read()
    }

    /// Register an MCP server configuration.
    pub fn register_mcp_server(&self, config: crate::mcp::McpServerConfig) {
        let server_id = config.id.clone();
        self.inner.mcp_registry.write().register_server(config);
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
    pub fn plugin_registry(&self) -> parking_lot::RwLockReadGuard<'_, PluginRegistry> {
        self.inner.plugin_registry.read()
    }

    /// Register a plugin configuration.
    pub fn register_plugin(&self, config: crate::plugin::config::PluginConfig) {
        let plugin_id = config.id.clone();
        self.inner.plugin_registry.write().register(config);
        self.initialize_existing_sessions_for_new_plugin(&plugin_id);
    }

    /// Borrow the skill catalog.
    pub fn skill_catalog(&self) -> parking_lot::RwLockReadGuard<'_, SkillCatalog> {
        self.inner.skill_catalog.read()
    }

    /// Register a skill into the catalog.
    pub fn register_skill(&self, skill: crate::skill::LoadedSkill) {
        self.inner.skill_catalog.write().register(skill);
    }

    /// Discover skills from all configured sources and merge into the catalog.
    pub fn discover_skills(&self, sources: &[Box<dyn crate::skill::source::SkillSource>]) {
        let mut catalog = self.inner.skill_catalog.write();
        *catalog = SkillCatalog::discover(sources);
    }

    /// Refresh the skill catalog by re-scanning all configured sources.
    ///
    /// Returns diagnostics about the discovery process (skipped skills,
    /// collisions, parse errors, etc.).
    pub fn refresh_skill_catalog(&self) -> Vec<SkillDiagnostic> {
        let mut sources: Vec<Box<dyn crate::skill::source::SkillSource>> = Vec::new();
        let mut diagnostics = Vec::new();
        let config = &self.inner.config;

        if !config.skills.enabled {
            *self.inner.skill_catalog.write() = SkillCatalog::new();
            return diagnostics;
        }

        let roots = if config.workspace_roots.is_empty() {
            vec![std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))]
        } else {
            config.workspace_roots.clone()
        };

        // Project-level skills: .agents/skills/ in each workspace root
        for root in roots {
            let project_skills_dir = root.join(".agents").join("skills");
            if project_skills_dir.exists() && project_skills_dir.is_dir() {
                if config.skills.trust_project_skills {
                    sources.push(Box::new(FilesystemSkillSource::new(
                        project_skills_dir,
                        SkillOrigin::ProjectFilesystem,
                    )));
                } else {
                    diagnostics.push(SkillDiagnostic {
                        level: crate::skill::DiagnosticLevel::Warning,
                        message: format!(
                            "Project skills in '{}' were hidden because trust_project_skills is disabled",
                            project_skills_dir.display()
                        ),
                        skill_name: None,
                    });
                }
            }
        }

        // User-level skills: ~/.agents/skills/
        let home_dir = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(std::path::PathBuf::from)
            .ok();
        if let Some(home) = home_dir {
            let user_skills_dir = home.join(".agents").join("skills");
            if user_skills_dir.exists() && user_skills_dir.is_dir() {
                sources.push(Box::new(FilesystemSkillSource::new(
                    user_skills_dir,
                    SkillOrigin::UserFilesystem,
                )));
            }
        }

        // Additional configured skill directories
        for dir in &config.skills.additional_skill_dirs {
            if dir.exists() && dir.is_dir() {
                sources.push(Box::new(FilesystemSkillSource::new(
                    dir.clone(),
                    SkillOrigin::UserFilesystem,
                )));
            }
        }

        let mut catalog = SkillCatalog::discover(&sources);
        catalog.extend_diagnostics(diagnostics);
        let diagnostics = catalog.diagnostics().to_vec();
        *self.inner.skill_catalog.write() = catalog;
        diagnostics
    }

    pub(crate) fn available_skill_snapshot(&self) -> Vec<LoadedSkill> {
        self.skill_catalog().list_all().into_iter().cloned().collect()
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
        self.inner.active_tasks.lock().push(handle);
        true
    }

    pub fn register_connection(&self, id: ConnectionId) {
        let conn = Arc::new(RuntimeConnection {
            active: AtomicBool::new(true),
        });
        self.inner.connections.write().insert(id, conn);
    }

    pub fn close_connection(&self, id: ConnectionId) {
        if let Some(conn) = self.inner.connections.write().get(&id) {
            conn.active.store(false, Ordering::SeqCst);
        }
        self.inner.connections.write().remove(&id);
        self.close_sessions_for_connection(id);
    }

    pub fn connection_count(&self) -> usize {
        self.inner.connections.read().len()
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

        // Initialize active skills from runtime skill catalog
        if self.inner.config.skills.enabled {
            let available_skills = self.available_skill_snapshot();
            durable.set_available_skills(available_skills.clone());
            for skill in &available_skills {
                if skill.metadata.auto_activate && !skill.metadata.requires_trust {
                    durable.activate_skill(
                        &skill.metadata.id,
                        &skill.body,
                        skill.resources.clone(),
                    );
                }
            }
        }

        let session = Arc::new(Mutex::new(durable));

        let runtime_session = RuntimeSession::new(session.clone(), connection_id);

        self.inner
            .sessions
            .write()
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
            .insert(session_id, Arc::new(runtime_session));
        Ok(())
    }

    pub fn get_session(&self, id: SessionId) -> Option<Arc<Mutex<DurableSession>>> {
        self.inner
            .sessions
            .read()
            .get(&id)
            .map(|rs| rs.session.clone())
    }

    pub fn get_session_connection(&self, id: SessionId) -> Option<ConnectionId> {
        self.inner
            .sessions
            .read()
            .get(&id)
            .map(|rs| rs.connection_id)
    }

    pub fn close_session(&self, id: SessionId) {
        self.inner.sessions.write().remove(&id);
    }

    pub fn close_sessions_for_connection(&self, connection_id: ConnectionId) {
        let mut sessions = self.inner.sessions.write();
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
    ) -> Result<Arc<Mutex<EphemeralTurn>>, RuntimeError> {
        let sessions = self.inner.sessions.read();
        let rs = sessions
            .get(&session_id)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.to_string()))?;
        let mut active = rs.active_prompt.lock();
        if active.is_some() {
            return Err(RuntimeError::Turn(
                "session already has an active prompt".into(),
            ));
        }
        let ephemeral = Arc::new(Mutex::new(EphemeralTurn::new(session_id)));
        ephemeral.lock().start();
        *active = Some(ActivePrompt {
            ephemeral: ephemeral.clone(),
        });
        Ok(ephemeral)
    }

    pub fn finish_prompt(&self, session_id: SessionId) {
        let sessions = self.inner.sessions.read();
        if let Some(rs) = sessions.get(&session_id) {
            let mut active = rs.active_prompt.lock();
            *active = None;
        }
    }

    pub fn cancel_active_prompt(&self, session_id: SessionId) -> bool {
        let sessions = self.inner.sessions.read();
        if let Some(rs) = sessions.get(&session_id) {
            let active = rs.active_prompt.lock();
            if let Some(prompt) = active.as_ref() {
                prompt.ephemeral.lock().cancel();
                return true;
            }
        }
        false
    }

    pub fn has_active_prompt(&self, session_id: SessionId) -> bool {
        let sessions = self.inner.sessions.read();
        sessions
            .get(&session_id)
            .map(|rs| rs.active_prompt.lock().is_some())
            .unwrap_or(false)
    }

    pub fn get_active_prompt_ephemeral(
        &self,
        session_id: SessionId,
    ) -> Option<Arc<Mutex<EphemeralTurn>>> {
        let sessions = self.inner.sessions.read();
        sessions.get(&session_id).and_then(|rs| {
            rs.active_prompt
                .lock()
                .as_ref()
                .map(|p| p.ephemeral.clone())
        })
    }

    pub fn session_count(&self) -> usize {
        self.inner.sessions.read().len()
    }

    pub fn sessions_for_connection(&self, connection_id: ConnectionId) -> Vec<SessionId> {
        self.inner
            .sessions
            .read()
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

        let tasks = std::mem::take(&mut *self.inner.active_tasks.lock());
        for handle in tasks {
            handle.abort();
        }

        self.inner.sessions.write().clear();
        self.inner.connections.write().clear();
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
        let runtime_session = self.inner.sessions.read().get(&session_id).cloned()?;
        let session_guard = runtime_session.session.lock();

        let tool_registry_version = self.inner.tool_registry.read().version();
        let mcp_registry_snapshot = self.inner.mcp_registry.read().clone();
        let mcp_registry_version = mcp_registry_snapshot.version();
        let plugin_registry_snapshot = self.inner.plugin_registry.read().clone();
        let plugin_registry_version = plugin_registry_snapshot.version();

        {
            let available_skills: Vec<(String, String)> = session_guard
                .list_available_skills()
                .iter()
                .map(|skill| {
                    (
                        skill.metadata.id.clone(),
                        skill.metadata.description.clone(),
                    )
                })
                .collect();
            let cache_guard = runtime_session.tool_catalog_cache.lock();
            if let Some(cached) = cache_guard.as_ref() {
                if cached.tool_registry_version == tool_registry_version
                    && cached.mcp_registry_version == mcp_registry_version
                    && cached.plugin_registry_version == plugin_registry_version
                    && cached.mcp_server_enablement == session_guard.mcp_server_enablement
                    && cached.plugin_enablement == session_guard.plugin_enablement
                    && cached.available_skills == available_skills
                {
                    return Some((*cached.catalog).clone());
                }
            }
        }

        let local_registry = Arc::new(self.inner.tool_registry.read().clone());
        let mcp_registry = Arc::new(mcp_registry_snapshot);
        let plugin_registry = Arc::new(plugin_registry_snapshot);
        let wasm_host = Arc::new(self.inner.wasm_host.read().clone());
        let connection_manager = self.mcp_connection_manager();

        let catalog = Arc::new(SessionToolCatalog::new(
            local_registry,
            mcp_registry,
            plugin_registry,
            wasm_host,
            connection_manager,
            &session_guard,
        ));

        {
            let available_skills = session_guard
                .list_available_skills()
                .iter()
                .map(|skill| {
                    (
                        skill.metadata.id.clone(),
                        skill.metadata.description.clone(),
                    )
                })
                .collect();
            let mut cache_guard = runtime_session.tool_catalog_cache.lock();
            *cache_guard = Some(CachedSessionToolCatalog {
                tool_registry_version,
                mcp_registry_version,
                plugin_registry_version,
                mcp_server_enablement: session_guard.mcp_server_enablement.clone(),
                plugin_enablement: session_guard.plugin_enablement.clone(),
                available_skills,
                catalog: catalog.clone(),
            });
        }

        Some((*catalog).clone())
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
        let mut guard = session.lock();
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
        let guard = session.lock();
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
        self.inner.plugin_registry.read().get_auth_prompts()
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
            .set_credentials(plugin_id, credentials);
    }

    /// Clear credentials for a plugin and reset its auth state.
    ///
    /// This triggers a recomputation of per-tool availability.
    pub fn clear_plugin_credentials(&self, plugin_id: &str) {
        self.inner
            .plugin_registry
            .write()
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
        let guard = session.lock();
        let plugin_registry = Arc::new((*self.plugin_registry()).clone());
        let wasm_host = Arc::new((*self.inner.wasm_host.read()).clone());
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
        let guard = session.lock();
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
        let tasks = std::mem::take(&mut *self.active_tasks.lock());
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

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::{self, BoxStream};
    use futures::StreamExt;
    use crate::skill::source::StaticSkillSource;
    use crate::skill::{LoadedSkill, SkillMetadata};

    #[derive(Clone, Default)]
    struct MockProvider;

    impl Provider for MockProvider {
        fn infer(
            &self,
            _request: iron_providers::InferenceRequest,
        ) -> iron_providers::ProviderFuture<'_, Vec<iron_providers::ProviderEvent>> {
            Box::pin(async move { Ok(vec![iron_providers::ProviderEvent::Complete]) })
        }

        fn infer_stream(
            &self,
            _request: iron_providers::InferenceRequest,
        ) -> iron_providers::ProviderFuture<
            '_,
            BoxStream<'static, iron_providers::ProviderResult<iron_providers::ProviderEvent>>,
        > {
            Box::pin(async move {
                Ok(stream::iter(vec![Ok(iron_providers::ProviderEvent::Complete)]).boxed())
            })
        }
    }

    fn make_skill(name: &str, description: &str) -> LoadedSkill {
        LoadedSkill {
            metadata: SkillMetadata {
                id: name.to_string(),
                display_name: name.to_string(),
                description: description.to_string(),
                origin: SkillOrigin::ClientProvided,
                auto_activate: false,
                tags: vec![],
                requires_tools: vec![],
                requires_capabilities: vec![],
                requires_trust: false,
            },
            location: None,
            body: format!("# {}\nInstructions", name),
            resources: vec![],
        }
    }

    #[test]
    fn refresh_skill_catalog_records_diagnostic_for_untrusted_project_skills() {
        let temp_root = std::env::temp_dir().join(format!(
            "iron-core-skill-trust-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project_skill_dir = temp_root.join(".agents").join("skills").join("review");
        std::fs::create_dir_all(&project_skill_dir).unwrap();
        std::fs::write(
            project_skill_dir.join("SKILL.md"),
            "---\nid: review\nname: review\ndescription: Review code\n---\n# review\nUse this skill.",
        )
        .unwrap();

        let config = Config::default()
            .with_workspace_roots(vec![temp_root.clone()])
            .with_skills(crate::config::SkillConfig::default().with_trust_project_skills(false));
        let runtime = IronRuntime::new(config, MockProvider);

        let diagnostics = runtime.refresh_skill_catalog();
        assert!(diagnostics.iter().any(|diag| {
            diag.message.contains("hidden because trust_project_skills is disabled")
        }));
        assert!(runtime.skill_catalog().get("review").is_none());

        let _ = std::fs::remove_dir_all(temp_root);
    }

    #[test]
    fn sessions_keep_independent_skill_snapshots() {
        let runtime = IronRuntime::new(Config::default(), MockProvider);
        let conn = ConnectionId(1);
        runtime.register_connection(conn);

        let mut source_a = StaticSkillSource::new();
        source_a.register(make_skill("review", "Review code changes"));
        runtime.discover_skills(&[Box::new(source_a)]);
        let (_session_a_id, session_a) = runtime.create_session(conn).unwrap();

        let mut source_b = StaticSkillSource::new();
        source_b.register(make_skill("docs", "Write technical docs"));
        runtime.discover_skills(&[Box::new(source_b)]);
        let (_session_b_id, session_b) = runtime.create_session(conn).unwrap();

        let session_a_names: Vec<_> = session_a
            .lock()
            .list_available_skills()
            .iter()
            .map(|skill| skill.metadata.id.clone())
            .collect();
        let session_b_names: Vec<_> = session_b
            .lock()
            .list_available_skills()
            .iter()
            .map(|skill| skill.metadata.id.clone())
            .collect();

        assert_eq!(session_a_names, vec!["review".to_string()]);
        assert_eq!(session_b_names, vec!["docs".to_string()]);
    }

    #[test]
    fn additional_skill_dirs_are_loaded_as_trusted_user_scope() {
        let temp_root = std::env::temp_dir().join(format!(
            "iron-core-skill-additional-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let skill_dir = temp_root.join("review");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nid: review\nname: review\ndescription: Review code\n---\n# review\nUse this skill.",
        )
        .unwrap();

        let config = Config::default().with_skills(
            crate::config::SkillConfig::default()
                .with_trust_project_skills(false)
                .with_additional_skill_dir(temp_root.clone()),
        );
        let runtime = IronRuntime::new(config, MockProvider);

        let catalog = runtime.skill_catalog();
        let skill = catalog
            .get("review")
            .expect("configured skill dir should load even when project trust is disabled");
        assert_eq!(skill.origin, SkillOrigin::UserFilesystem);

        let _ = std::fs::remove_dir_all(temp_root);
    }
}
