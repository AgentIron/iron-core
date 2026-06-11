//! Shared runtime state for `IronAgent`.

use crate::{
    capability::{CapabilityBackend, CapabilityDescriptor, CapabilityRegistry},
    config::Config,
    context::model_switch::PendingModelSwitch,
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
    provider_credential::domain::{
        ProviderAuthError, ProviderAuthResult, ProviderAuthStatus, ProviderPromptContext,
        ProviderSlug,
    },
    provider_credential::resolver::CredentialResolver,
    provider_credential::store::DynCredentialStore,
    skill::source::FilesystemSkillSource,
    skill::SkillOrigin,
    skill::{LoadedSkill, SkillCatalog, SkillDiagnostic},
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
    resolver: Option<Arc<CredentialResolver>>,
    debug_sink: RwLock<Option<Arc<dyn crate::debug::DebugSink>>>,
    debug_sequence: crate::debug::SequenceGenerator,
    builtin_tool_config: Mutex<Option<crate::builtin::BuiltinToolConfig>>,
    model_capability_registry: RwLock<crate::context::model_switch::ModelCapabilityRegistry>,
}

struct ActivePrompt {
    ephemeral: Arc<Mutex<EphemeralTurn>>,
}

struct RuntimeSession {
    session: Arc<Mutex<DurableSession>>,
    connection_id: ConnectionId,
    active_prompt: Mutex<Option<ActivePrompt>>,
    tool_catalog_cache: Mutex<Option<CachedSessionToolCatalog>>,
    pending_model_switch: Mutex<Option<PendingModelSwitch>>,
    turn_counter: std::sync::atomic::AtomicU64,
}

struct CachedSessionToolCatalog {
    tool_registry_version: u64,
    mcp_registry_version: u64,
    plugin_registry_version: u64,
    mcp_server_enablement: std::collections::HashMap<String, bool>,
    plugin_enablement: crate::plugin::session::SessionPluginEnablement,
    available_skills: Vec<(String, String)>,
    hidden_tools: Vec<String>,
    catalog: Arc<SessionToolCatalog>,
}

impl RuntimeSession {
    fn new(session: Arc<Mutex<DurableSession>>, connection_id: ConnectionId) -> Self {
        Self {
            session,
            connection_id,
            active_prompt: Mutex::new(None),
            tool_catalog_cache: Mutex::new(None),
            pending_model_switch: Mutex::new(None),
            turn_counter: std::sync::atomic::AtomicU64::new(1),
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
            resolver: None,
            debug_sink: RwLock::new(None),
            debug_sequence: crate::debug::SequenceGenerator::new(),
            builtin_tool_config: Mutex::new(None),
            model_capability_registry: RwLock::new(
                crate::context::model_switch::ModelCapabilityRegistry::new(),
            ),
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

        this.register_compress_tool();
        this.hydrate_builtin_model_capability_registry();

        // Emit runtime configuration debug event (new())
        let config_event = crate::debug::redact_config(&this.inner.config);
        this.emit_debug(crate::debug::DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: this.inner.debug_sequence.next(),
            severity: crate::debug::DebugSeverity::Info,
            scope: crate::debug::DebugScope::default(),
            payload: crate::debug::DebugPayload::Config(config_event),
        });

        this
    }

    /// Create a new runtime with a credential store for managed provider resolution.
    pub fn new_with_credential_store<P>(
        config: Config,
        provider: P,
        credential_store: DynCredentialStore,
    ) -> Self
    where
        P: Provider + 'static,
    {
        let mut runtime = Self::new(config, provider);
        if let Some(inner) = Arc::get_mut(&mut runtime.inner) {
            inner.resolver = Some(Arc::new(CredentialResolver::new(credential_store)));
        }
        runtime
    }

    /// Create a new runtime with both infallible and fallible credential stores.
    ///
    /// When a fallible store is provided, the resolver surfaces durable store
    /// errors (encryption failures, database busy, etc.) as actionable errors
    /// rather than silently returning missing credentials.
    pub fn new_with_fallible_credential_store<P>(
        config: Config,
        provider: P,
        credential_store: DynCredentialStore,
        fallible_store: Arc<dyn crate::provider_credential::FallibleCredentialStore>,
    ) -> Self
    where
        P: Provider + 'static,
    {
        let mut runtime = Self::new(config, provider);
        if let Some(inner) = Arc::get_mut(&mut runtime.inner) {
            inner.resolver = Some(Arc::new(CredentialResolver::with_fallible_store(
                credential_store,
                fallible_store,
            )));
        }
        runtime
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
            resolver: None,
            debug_sink: RwLock::new(None),
            debug_sequence: crate::debug::SequenceGenerator::new(),
            builtin_tool_config: Mutex::new(None),
            model_capability_registry: RwLock::new(
                crate::context::model_switch::ModelCapabilityRegistry::new(),
            ),
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

        this.register_compress_tool();
        this.hydrate_builtin_model_capability_registry();

        // Emit runtime configuration debug event
        let config_event = crate::debug::redact_config(&this.inner.config);
        this.emit_debug(crate::debug::DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: this.inner.debug_sequence.next(),
            severity: crate::debug::DebugSeverity::Info,
            scope: crate::debug::DebugScope::default(),
            payload: crate::debug::DebugPayload::Config(config_event),
        });

        this
    }

    /// Create a new runtime using an existing Tokio runtime handle, with a
    /// credential store for managed provider resolution.
    pub fn from_handle_with_credential_store<P>(
        config: Config,
        provider: P,
        handle: tokio::runtime::Handle,
        credential_store: DynCredentialStore,
    ) -> Self
    where
        P: Provider + 'static,
    {
        let mut runtime = Self::from_handle(config, provider, handle);
        if let Some(inner) = Arc::get_mut(&mut runtime.inner) {
            inner.resolver = Some(Arc::new(CredentialResolver::new(credential_store)));
        }
        runtime
    }

    /// Create a new runtime using an existing Tokio runtime handle, with both
    /// infallible and fallible credential stores.
    pub fn from_handle_with_fallible_credential_store<P>(
        config: Config,
        provider: P,
        handle: tokio::runtime::Handle,
        credential_store: DynCredentialStore,
        fallible_store: Arc<dyn crate::provider_credential::FallibleCredentialStore>,
    ) -> Self
    where
        P: Provider + 'static,
    {
        let mut runtime = Self::from_handle(config, provider, handle);
        if let Some(inner) = Arc::get_mut(&mut runtime.inner) {
            inner.resolver = Some(Arc::new(CredentialResolver::with_fallible_store(
                credential_store,
                fallible_store,
            )));
        }
        runtime
    }

    /// Set the debug observation sink for this runtime.
    ///
    /// The sink receives typed debug events emitted at semantic runtime
    /// transitions. Setting `None` restores the default no-op sink.
    pub fn set_debug_sink(&self, sink: Option<Arc<dyn crate::debug::DebugSink>>) {
        let mut guard = self.inner.debug_sink.write();
        *guard = sink.clone();
        // Emit config event when sink is registered so embedders can observe current configuration
        if let Some(ref s) = sink {
            let config_event = crate::debug::redact_config(&self.inner.config);
            crate::debug::emit_debug(
                s.as_ref(),
                crate::debug::DebugEvent {
                    timestamp: chrono::Utc::now(),
                    sequence: self.inner.debug_sequence.next(),
                    severity: crate::debug::DebugSeverity::Info,
                    scope: crate::debug::DebugScope::default(),
                    payload: crate::debug::DebugPayload::Config(config_event),
                },
            );
        }
    }

    /// Access the current debug sink (read-only).
    pub(crate) fn debug_sink(
        &self,
    ) -> parking_lot::MappedRwLockReadGuard<'_, Option<Arc<dyn crate::debug::DebugSink>>> {
        parking_lot::RwLockReadGuard::map(self.inner.debug_sink.read(), |s| s)
    }

    /// Get the next debug event sequence number.
    pub(crate) fn next_debug_sequence(&self) -> u64 {
        self.inner.debug_sequence.next()
    }

    /// Emit a debug event through the runtime's sink if one is registered.
    ///
    /// This is a no-op if no sink is configured. Events are best-effort
    /// and must never affect runtime behavior.
    pub(crate) fn emit_debug(&self, event: crate::debug::DebugEvent) {
        if let Some(ref sink) = *self.debug_sink() {
            crate::debug::emit_debug(sink.as_ref(), event);
        }
    }

    /// Borrow the validated runtime configuration.
    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    /// Borrow the provider implementation used for inference.
    pub fn provider(&self) -> &dyn Provider {
        self.inner.provider.as_ref()
    }

    /// Resolve a managed provider for the given prompt context.
    ///
    /// This looks up credentials, refreshes if needed, and constructs a
    /// provider from the built-in registry. Returns an error if no resolver
    /// is configured or if credential resolution fails.
    pub async fn resolve_managed_provider(
        &self,
        context: &ProviderPromptContext,
    ) -> ProviderAuthResult<Box<dyn Provider>> {
        let resolver = self.inner.resolver.as_ref().ok_or_else(|| {
            ProviderAuthError::NotConfigured(context.provider_slug.as_str().to_string())
        })?;

        let resolved = resolver.resolve(context, context.api_key.clone()).await?;
        let runtime_config =
            iron_providers::RuntimeConfig::from_credential(resolved.provider_credential);

        let registry = iron_providers::ProviderRegistry::default();
        let provider = registry
            .get(context.provider_slug.as_str(), runtime_config)
            .map_err(|_e| ProviderAuthError::UnsupportedCredential {
                provider: context.provider_slug.as_str().to_string(),
                mode: resolved.mode,
            })?;

        Ok(provider)
    }

    /// Get the client-visible auth status for a provider.
    pub async fn provider_auth_status(&self, slug: &ProviderSlug) -> Option<ProviderAuthStatus> {
        self.provider_auth_status_with_api_key(slug, None).await
    }

    /// Get the client-visible auth status for a provider, considering an
    /// app-owned API key when present.
    pub async fn provider_auth_status_with_api_key(
        &self,
        slug: &ProviderSlug,
        api_key: Option<&str>,
    ) -> Option<ProviderAuthStatus> {
        let resolver = self.inner.resolver.as_ref()?;
        Some(resolver.status(slug, api_key).await)
    }

    /// Get the client-visible auth status for a managed prompt context.
    pub async fn provider_auth_status_for_context(
        &self,
        context: &ProviderPromptContext,
    ) -> Option<ProviderAuthStatus> {
        self.provider_auth_status_with_api_key(&context.provider_slug, context.api_key.as_deref())
            .await
    }

    /// Disconnect OAuth for a provider without removing API keys.
    pub async fn disconnect_provider_oauth(&self, slug: &ProviderSlug) {
        if let Some(resolver) = self.inner.resolver.as_ref() {
            resolver.disconnect_oauth(slug).await;
        }
    }

    /// Force-refresh and resolve a managed provider for the given prompt context.
    ///
    /// This always refreshes OAuth tokens (if present) before constructing the
    /// provider, regardless of expiry. Used for auth-failure retry.
    pub async fn force_refresh_managed_provider(
        &self,
        context: &ProviderPromptContext,
    ) -> ProviderAuthResult<Box<dyn Provider>> {
        let resolver = self.inner.resolver.as_ref().ok_or_else(|| {
            ProviderAuthError::NotConfigured(context.provider_slug.as_str().to_string())
        })?;

        let resolved = resolver.force_refresh(&context.provider_slug).await?;
        let runtime_config =
            iron_providers::RuntimeConfig::from_credential(resolved.provider_credential);

        let registry = iron_providers::ProviderRegistry::default();
        let provider = registry
            .get(context.provider_slug.as_str(), runtime_config)
            .map_err(|_e| ProviderAuthError::UnsupportedCredential {
                provider: context.provider_slug.as_str().to_string(),
                mode: resolved.mode,
            })?;

        Ok(provider)
    }

    /// Borrow the credential resolver, if configured.
    pub fn credential_resolver(&self) -> Option<&CredentialResolver> {
        self.inner.resolver.as_ref().map(|arc| arc.as_ref())
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
        *self.inner.builtin_tool_config.lock() = Some(config.clone());
    }

    /// Create a session-configured BuiltinToolConfig with the given workspace roots.
    ///
    /// This preserves all other builtin policy/settings from the base config.
    pub fn session_builtin_tool_config(
        &self,
        roots: &[std::path::PathBuf],
    ) -> Option<crate::builtin::BuiltinToolConfig> {
        let base = self.inner.builtin_tool_config.lock().clone()?;
        Some(crate::builtin::BuiltinToolConfig {
            allowed_roots: roots.to_vec(),
            ..base
        })
    }

    #[cfg(feature = "embedded-python")]
    /// Register the embedded Python execution tool.
    pub fn register_python_exec_tool(&self) {
        self.register_tool(crate::embedded_python::PythonExecTool::new());
    }

    /// Register the `activate_skill` model-facing tool.
    pub fn register_activate_skill_tool(&self) {
        use crate::tool::{FunctionTool, ToolDefinition};
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

    /// Register the `compress` model-facing tool.
    pub fn register_compress_tool(&self) {
        let definition = crate::context::CompressTool::definition();
        let tool = crate::tool::FunctionTool::new(definition, |_args| {
            Err(crate::error::RuntimeError::tool_execution(
                "compress must be handled by the orchestrator".to_string(),
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
        let discovered_count = catalog.len();
        let all_skills = catalog.list_all();
        let trusted_count = all_skills
            .iter()
            .filter(|s| !s.metadata.requires_trust)
            .count();
        let untrusted_count = discovered_count - trusted_count;
        let source_kinds: Vec<String> = sources
            .iter()
            .map(|s| std::any::type_name_of_val(s.as_ref()).to_string())
            .collect();
        *self.inner.skill_catalog.write() = catalog;

        self.emit_debug(crate::debug::DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: self.next_debug_sequence(),
            severity: crate::debug::DebugSeverity::Info,
            scope: crate::debug::DebugScope::default(),
            payload: crate::debug::DebugPayload::Skill(
                crate::debug::SkillDebugEvent::CatalogRefreshed {
                    sources: source_kinds,
                    discovered_count,
                    trusted_count,
                    untrusted_count,
                    diagnostic_count: diagnostics.len(),
                },
            ),
        });

        diagnostics
    }

    pub(crate) fn available_skill_snapshot(&self) -> Vec<LoadedSkill> {
        self.skill_catalog()
            .list_all()
            .into_iter()
            .cloned()
            .collect()
    }

    /// Discover and refresh available skills for a specific session's workspace roots.
    ///
    /// This updates the session's available skill snapshot while preserving
    /// already-active skill instructions.
    pub fn refresh_session_skills(
        &self,
        session_id: SessionId,
        roots: &[std::path::PathBuf],
    ) -> Result<(), RuntimeError> {
        if !self.inner.config.skills.enabled {
            return Ok(());
        }

        let session = self
            .get_session(session_id)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.to_string()))?;

        let available_skills = self.discover_available_skills_for_roots(roots);

        let mut guard = session.lock();
        let previously_active: Vec<String> = guard
            .list_active_skills()
            .into_iter()
            .map(|s| s.to_string())
            .collect();

        guard.set_available_skills(available_skills.clone());

        // Preserve already-active skills by re-activating them if they exist
        // in the new available set
        for skill in &available_skills {
            if previously_active.contains(&skill.metadata.id)
                || (skill.metadata.auto_activate && !skill.metadata.requires_trust)
            {
                guard.activate_skill(&skill.metadata.id, &skill.body, skill.resources.clone());
            }
        }

        drop(guard);

        // Invalidate tool catalog cache since available skills changed
        let sessions = self.inner.sessions.read();
        if let Some(rs) = sessions.get(&session_id) {
            let mut cache = rs.tool_catalog_cache.lock();
            *cache = None;
        }

        Ok(())
    }

    /// Discover available skills for the given workspace roots.
    ///
    /// Returns a list of LoadedSkill values discovered from project directories
    /// under the provided roots, plus user-level and additional configured skills.
    pub fn discover_available_skills_for_roots(
        &self,
        roots: &[std::path::PathBuf],
    ) -> Vec<LoadedSkill> {
        let mut sources: Vec<Box<dyn crate::skill::source::SkillSource>> = Vec::new();
        let config = &self.inner.config;

        if !config.skills.enabled {
            return Vec::new();
        }

        // Project-level skills: .agents/skills/ in each workspace root
        for root in roots {
            let project_skills_dir = root.join(".agents").join("skills");
            if project_skills_dir.exists()
                && project_skills_dir.is_dir()
                && config.skills.trust_project_skills
            {
                sources.push(Box::new(FilesystemSkillSource::new(
                    project_skills_dir,
                    SkillOrigin::ProjectFilesystem,
                )));
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

        // Runtime-registered skills (client-provided only)
        let mut static_source = crate::skill::source::StaticSkillSource::new();
        for skill in self.skill_catalog().list_all() {
            if skill.metadata.origin == SkillOrigin::ClientProvided {
                static_source.register(skill.clone());
            }
        }
        sources.push(Box::new(static_source));

        let catalog = SkillCatalog::discover(&sources);
        catalog.list_all().into_iter().cloned().collect()
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

        // Seed workspace roots from config or current directory fallback
        let roots = if self.inner.config.workspace_roots.is_empty() {
            vec![std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))]
        } else {
            self.inner.config.workspace_roots.clone()
        };
        durable.workspace_roots = roots.clone();

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
            let source_categories: Vec<String> = available_skills
                .iter()
                .map(|s| format!("{:?}", s.metadata.origin))
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            durable.set_available_skills(available_skills.clone());

            self.emit_debug(crate::debug::DebugEvent {
                timestamp: chrono::Utc::now(),
                sequence: self.next_debug_sequence(),
                severity: crate::debug::DebugSeverity::Info,
                scope: crate::debug::DebugScope {
                    session_id: Some(session_id),
                    ..crate::debug::DebugScope::default()
                },
                payload: crate::debug::DebugPayload::Skill(
                    crate::debug::SkillDebugEvent::AvailableToSession {
                        count: available_skills.len(),
                        source_categories,
                    },
                ),
            });

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

        // Backfill workspace roots for legacy sessions imported before this feature.
        if durable.workspace_roots.is_empty() {
            durable.workspace_roots = if self.inner.config.workspace_roots.is_empty() {
                vec![std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))]
            } else {
                self.inner.config.workspace_roots.clone()
            };
        }
        let roots = durable.workspace_roots.clone();

        let session = Arc::new(Mutex::new(durable));
        let runtime_session = RuntimeSession::new(session, connection_id);
        self.inner
            .sessions
            .write()
            .insert(session_id, Arc::new(runtime_session));

        // Refresh skills for the backfilled roots so the session’s skill catalog
        // and builtin allowed_roots reflect the effective workspace.
        let _ = self.refresh_session_skills(session_id, &roots);

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
        let turn_id = rs
            .turn_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let turn_id_str = format!("{}-{}", session_id, turn_id);
        let ephemeral = Arc::new(Mutex::new(EphemeralTurn::new(
            session_id,
            Some(turn_id_str),
        )));
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

    /// Queue a model switch for an active session.
    ///
    /// The switch will be applied at the next turn boundary when the
    /// current prompt completes.
    pub fn queue_model_switch(
        &self,
        session_id: SessionId,
        request: crate::context::model_switch::ModelSwitchRequest,
    ) -> Result<(), RuntimeError> {
        if self.is_shutdown() {
            return Err(RuntimeError::Connection("Runtime is shut down".into()));
        }

        let sessions = self.inner.sessions.read();
        let rs = sessions
            .get(&session_id)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.to_string()))?;

        let mut pending = rs.pending_model_switch.lock();
        *pending = Some(PendingModelSwitch {
            request: request.clone(),
            requested_at: chrono::Utc::now(),
        });

        // Emit model switch queued debug event
        let (target_model, target_provider) = match &request {
            crate::context::model_switch::ModelSwitchRequest::Managed {
                provider_slug,
                model,
                ..
            } => (model.clone(), provider_slug.clone()),
            crate::context::model_switch::ModelSwitchRequest::Unmanaged {
                model,
                provider_name,
            } => (model.clone(), provider_name.clone()),
        };
        self.emit_debug(crate::debug::DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: self.inner.debug_sequence.next(),
            severity: crate::debug::DebugSeverity::Info,
            scope: crate::debug::DebugScope {
                session_id: Some(session_id),
                ..crate::debug::DebugScope::default()
            },
            payload: crate::debug::DebugPayload::Provider(
                crate::debug::ProviderDebugEvent::ModelSwitchQueued {
                    target_model,
                    target_provider,
                },
            ),
        });

        Ok(())
    }

    /// Apply a model switch to an idle session.
    ///
    /// This updates the session's current model and records the switch
    /// in the timeline and history.
    ///
    /// Returns the capability diff so callers can emit client events.
    pub fn apply_model_switch(
        &self,
        session_id: SessionId,
        request: crate::context::model_switch::ModelSwitchRequest,
    ) -> Result<
        (
            crate::context::CapabilityDiff,
            Option<crate::context::model_switch::CompactionInfo>,
        ),
        RuntimeError,
    > {
        if self.is_shutdown() {
            return Err(RuntimeError::Connection("Runtime is shut down".into()));
        }

        let sessions = self.inner.sessions.read();
        let rs = sessions
            .get(&session_id)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.to_string()))?;

        let mut session = rs.session.lock();
        let from_model = session.current_model.clone();
        let from_provider = session.current_provider_slug.clone();

        let (to_model, to_provider, to_provider_slug, to_api_key) = match &request {
            crate::context::model_switch::ModelSwitchRequest::Managed {
                provider_slug,
                model,
                api_key,
            } => (
                model.clone(),
                Some(provider_slug.clone()),
                Some(provider_slug.clone()),
                api_key.clone(),
            ),
            crate::context::model_switch::ModelSwitchRequest::Unmanaged {
                model,
                provider_name,
            } => (model.clone(), Some(provider_name.clone()), None, None),
        };

        // Create adaptation plan
        let config = self.config();
        let current_tokens = session
            .token_tracker
            .estimate_current_context()
            .unwrap_or_else(|| {
                crate::context::model_switch::ModelSwitchPlanner::estimate_session_tokens(
                    session.uncompacted_tokens,
                    &session.compressed_blocks,
                )
            });
        let target_window = config.context_management.context_window_hint;
        let cap_registry = self.inner.model_capability_registry.read();
        let source_model_id = from_model.as_deref().unwrap_or("unknown");
        let source_provider_id = session
            .current_provider_slug
            .as_deref()
            .unwrap_or("unknown");
        let target_provider_id = to_provider.as_deref().unwrap_or("unknown");
        let source_caps = cap_registry
            .get(source_provider_id, source_model_id)
            .cloned();
        let target_caps = cap_registry.get(target_provider_id, &to_model).cloned();
        if target_caps.is_none() {
            return Err(RuntimeError::Connection(format!(
                "Unknown target model '{}/{}': not present in the model capability registry",
                target_provider_id, to_model
            )));
        }
        drop(cap_registry);

        let plan = crate::context::model_switch::ModelSwitchPlanner::create_plan_with_capabilities(
            source_model_id,
            &to_model,
            target_window,
            current_tokens,
            source_caps.as_ref(),
            target_caps.as_ref(),
        );

        // Reject switch if even minimal tail cannot fit target window
        if !plan.context_adaptation.tail_fits {
            return Err(RuntimeError::Connection(format!(
                "Context too large for target model '{}'. Even a minimal tail of {} messages (~{} tokens) exceeds the target window of {} tokens. Consider starting a new session or manually compacting context.",
                to_model,
                plan.context_adaptation.tail_messages,
                plan.context_adaptation.retained_tokens,
                target_window.unwrap_or(0)
            )));
        }

        // Trigger compaction if needed and enabled
        let compaction_info: Option<crate::context::model_switch::CompactionInfo> = if plan
            .context_adaptation
            .needs_compaction
            && config
                .context_management
                .model_switch
                .compact_on_window_shrink
        {
            let tail_count = plan.context_adaptation.tail_messages;
            let user_agent_count = session
                .timeline
                .iter()
                .filter(|e| {
                    matches!(
                        e,
                        crate::durable::TimelineEntry::UserMessage { .. }
                            | crate::durable::TimelineEntry::AgentMessage { .. }
                    )
                })
                .count();

            if user_agent_count > tail_count {
                let cutoff = user_agent_count - tail_count;
                let mut user_agent_seen = 0usize;
                let positions_to_remove: std::collections::BTreeSet<usize> = session
                    .timeline
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, entry)| {
                        let is_user_agent = matches!(
                            entry,
                            crate::durable::TimelineEntry::UserMessage { .. }
                                | crate::durable::TimelineEntry::AgentMessage { .. }
                        );
                        let is_tool = matches!(
                            entry,
                            crate::durable::TimelineEntry::ToolCallStarted { .. }
                                | crate::durable::TimelineEntry::ToolCallTerminal { .. }
                        );
                        if !is_user_agent && !is_tool {
                            return None;
                        }
                        if user_agent_seen >= cutoff {
                            return None;
                        }
                        if is_user_agent {
                            user_agent_seen += 1;
                        }
                        Some(idx)
                    })
                    .collect();

                let mut visible_ids = Vec::new();
                let mut content_parts = Vec::new();
                for (idx, entry) in session.timeline.iter().enumerate() {
                    if !positions_to_remove.contains(&idx) {
                        continue;
                    }
                    if let crate::durable::TimelineEntry::UserMessage {
                        message_index,
                        visible_id: Some(vid),
                        ..
                    }
                    | crate::durable::TimelineEntry::AgentMessage {
                        message_index,
                        visible_id: Some(vid),
                        ..
                    } = entry
                    {
                        visible_ids.push(vid.clone());
                        if let Some(msg) = session.messages.get(*message_index) {
                            content_parts.push(format!("[{}] {}", vid, msg.text_content()));
                        }
                    }
                }

                if !positions_to_remove.is_empty() {
                    let source_range = if visible_ids.len() >= 2 {
                        format!(
                            "{}-{}",
                            visible_ids.first().unwrap(),
                            visible_ids.last().unwrap()
                        )
                    } else if let Some(id) = visible_ids.first() {
                        id.clone()
                    } else {
                        "unknown".to_string()
                    };

                    let block_id = format!("c{:04}", session.compressed_blocks.len() + 1);
                    let tokens_before: usize = session
                        .token_tracker
                        .estimate_current_context()
                        .unwrap_or_else(|| content_parts.iter().map(|s| s.len() / 4).sum());

                    const MAX_AUTO_SUMMARY_LEN: usize = 1500;
                    let joined = content_parts.join("\n\n");
                    let truncated_summary = {
                        let truncated: String = joined.chars().take(MAX_AUTO_SUMMARY_LEN).collect();
                        if truncated.len() < joined.len() {
                            format!("{}…", truncated)
                        } else {
                            truncated
                        }
                    };
                    let summary_text = format!(
                        "[Auto-compressed during model switch to {}.]\n\n{}",
                        to_model, truncated_summary
                    );
                    let tokens_after = summary_text.len() / 4;

                    let block = crate::context::models::CompressedBlock {
                        id: block_id,
                        topic: format!("Auto-compressed during model switch to {}", to_model),
                        source_range,
                        summary: summary_text,
                        created_at: chrono::Utc::now(),
                        token_estimate_before: Some(tokens_before as u32),
                        token_estimate_after: Some(tokens_after as u32),
                    };

                    session.remove_timeline_positions(&positions_to_remove);
                    session.compressed_blocks.push(block);
                    session.uncompacted_tokens = plan.context_adaptation.retained_tokens;
                    session.token_tracker.invalidate_baseline();
                    Some(crate::context::model_switch::CompactionInfo {
                        tokens_before: tokens_before as u32,
                        tokens_after: tokens_after as u32,
                        method: "auto_compaction".to_string(),
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let adapted = compaction_info.is_some();

        // Emit model switch plan created debug event
        self.emit_debug(crate::debug::DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: self.inner.debug_sequence.next(),
            severity: crate::debug::DebugSeverity::Info,
            scope: crate::debug::DebugScope {
                session_id: Some(session_id),
                ..crate::debug::DebugScope::default()
            },
            payload: crate::debug::DebugPayload::Provider(
                crate::debug::ProviderDebugEvent::ModelSwitchPlanCreated {
                    current_tokens,
                    target_window,
                    adaptation_needed: plan.context_adaptation.needs_compaction,
                    estimate_quality: "estimated".to_string(),
                },
            ),
        });

        // Update current model and capability restrictions
        session.current_model = Some(to_model.clone());
        session.current_provider_slug = to_provider_slug;
        session.current_provider_api_key = to_api_key;
        session.hidden_tools = plan.capability_diff.hidden_tools.clone();

        // Emit model switch applied debug event
        self.emit_debug(crate::debug::DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: self.inner.debug_sequence.next(),
            severity: crate::debug::DebugSeverity::Info,
            scope: crate::debug::DebugScope {
                session_id: Some(session_id),
                ..crate::debug::DebugScope::default()
            },
            payload: crate::debug::DebugPayload::Provider(
                crate::debug::ProviderDebugEvent::ModelSwitchApplied {
                    from_model: from_model.clone().unwrap_or_else(|| "unknown".to_string()),
                    from_provider: from_provider
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                    to_model: to_model.clone(),
                    to_provider: to_provider.clone().unwrap_or_else(|| "unknown".to_string()),
                    capability_diff: Some(format!("{:?}", plan.capability_diff)),
                },
            ),
        });

        // Record in timeline
        let timeline_index = session.timeline.len() as u64;
        let visible_id = session.next_visible_id();
        session
            .timeline
            .push(crate::durable::TimelineEntry::ModelSwitched {
                index: timeline_index,
                from_model: from_model.clone().unwrap_or_else(|| "unknown".to_string()),
                to_model: to_model.clone(),
                from_provider: from_provider.clone(),
                to_provider: to_provider.clone(),
                adapted,
                visible_id: Some(visible_id),
            });

        // Record in history
        let capability_diff = plan.capability_diff.clone();
        let record = crate::context::model_switch::ModelSwitchRecord {
            from_model: from_model.unwrap_or_else(|| "unknown".to_string()),
            to_model,
            from_provider,
            to_provider,
            adapted,
            capability_diff: capability_diff.clone(),
            timestamp: chrono::Utc::now(),
        };
        session.model_switch_history.push(record);

        Ok((capability_diff, compaction_info))
    }

    /// Check and apply any pending model switch for a session.
    ///
    /// This should be called at turn boundaries after a prompt completes.
    /// Returns `Ok(Some(...))` when a pending switch was applied successfully,
    /// `Ok(None)` when no switch was pending, and `Err(...)` when a pending
    /// switch could not be applied so callers can surface the failure.
    pub fn check_and_apply_pending_model_switch(
        &self,
        session_id: SessionId,
    ) -> Result<
        Option<(
            crate::context::CapabilityDiff,
            Option<crate::context::model_switch::CompactionInfo>,
        )>,
        RuntimeError,
    > {
        let sessions = self.inner.sessions.read();
        let Some(rs) = sessions.get(&session_id) else {
            return Err(RuntimeError::SessionNotFound(session_id.to_string()));
        };

        let pending_request = {
            let pending = rs.pending_model_switch.lock();
            pending.as_ref().map(|p| p.request.clone())
        };

        if let Some(request) = pending_request {
            match self.apply_model_switch(session_id, request.clone()) {
                Ok(outcome) => {
                    let mut pending = rs.pending_model_switch.lock();
                    pending.take();
                    Ok(Some(outcome))
                }
                Err(e) => {
                    let (target_model, target_provider) = match &request {
                        crate::context::model_switch::ModelSwitchRequest::Managed {
                            provider_slug,
                            model,
                            ..
                        } => (model.clone(), provider_slug.clone()),
                        crate::context::model_switch::ModelSwitchRequest::Unmanaged {
                            model,
                            provider_name,
                        } => (model.clone(), provider_name.clone()),
                    };
                    self.emit_debug(crate::debug::DebugEvent {
                        timestamp: chrono::Utc::now(),
                        sequence: self.inner.debug_sequence.next(),
                        severity: crate::debug::DebugSeverity::Error,
                        scope: crate::debug::DebugScope {
                            session_id: Some(session_id),
                            ..crate::debug::DebugScope::default()
                        },
                        payload: crate::debug::DebugPayload::Provider(
                            crate::debug::ProviderDebugEvent::ModelSwitchFailed {
                                target_model,
                                target_provider,
                                reason: e.to_string(),
                            },
                        ),
                    });
                    Err(e)
                }
            }
        } else {
            Ok(None)
        }
    }

    /// Register capability metadata for a model in the capability registry.
    pub fn register_model_capability(
        &self,
        metadata: crate::context::model_switch::ModelCapabilityMetadata,
    ) {
        self.inner
            .model_capability_registry
            .write()
            .register(metadata);
    }

    /// Hydrate the model capability registry with compiled-in built-in model metadata.
    fn hydrate_builtin_model_capability_registry(&self) {
        use crate::config::builtin_models::builtin_model_catalog;

        let mut registry = self.inner.model_capability_registry.write();
        for entry in builtin_model_catalog() {
            let metadata = crate::context::model_switch::ModelCapabilityMetadata {
                model: entry.model_id,
                provider: entry.provider_slug,
                context_window: entry.context_window.unwrap_or(0),
                supports_tools: entry.supports_tool_calls,
                supports_streaming: entry.supports_streaming,
                supports_reasoning_effort: entry.supports_reasoning,
                reasoning_effort_values: entry.reasoning_effort_values,
                supported_modalities: entry.supported_modalities,
                unsupported_tools: entry.unsupported_tools,
            };
            registry.register(metadata);
        }
    }

    /// Hydrate the model capability registry from a runtime settings snapshot.
    ///
    /// This builds the effective model catalog from built-in provider metadata
    /// and ConfigStore custom model records, then registers every entry into
    /// the in-memory capability registry so that model switches and capability
    /// diffs can reason about both built-in and custom models.
    pub fn hydrate_model_capability_registry(
        &self,
        settings: &crate::config::RuntimeSettingsSnapshot,
    ) -> Result<(), crate::config::ConfigError> {
        use crate::config::builtin_models::builtin_model_catalog;
        use crate::config::effective_catalog::build_effective_catalog;

        let catalog = build_effective_catalog(&builtin_model_catalog(), &settings.custom_models)?;

        let mut registry = self.inner.model_capability_registry.write();
        *registry = crate::context::model_switch::ModelCapabilityRegistry::new();
        for entry in catalog.all_entries() {
            let metadata = crate::context::model_switch::ModelCapabilityMetadata::from(entry);
            registry.register(metadata);
        }

        Ok(())
    }

    /// Return a read lock on the model capability registry.
    ///
    /// Exposed primarily for tests and diagnostics.
    pub fn model_capability_registry(
        &self,
    ) -> parking_lot::RwLockReadGuard<'_, crate::context::model_switch::ModelCapabilityRegistry>
    {
        self.inner.model_capability_registry.read()
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

    /// Set workspace roots for a session.
    ///
    /// If the session is idle, roots are applied immediately and skills are refreshed.
    /// If a prompt is active, roots are deferred until the prompt completes.
    /// Returns whether the roots were applied immediately.
    pub fn set_session_workspace_roots(
        &self,
        session_id: SessionId,
        roots: Vec<std::path::PathBuf>,
    ) -> Result<bool, RuntimeError> {
        if self.is_shutdown() {
            return Err(RuntimeError::Connection("Runtime is shut down".into()));
        }

        let sessions = self.inner.sessions.read();
        let rs = sessions
            .get(&session_id)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.to_string()))?;

        if rs.active_prompt.lock().is_some() {
            // Defer: store pending roots in DurableSession (latest wins)
            let mut guard = rs.session.lock();
            guard.set_pending_workspace_roots(roots);
            Ok(false)
        } else {
            // Apply immediately: lock DurableSession while still holding sessions read lock
            // to prevent a prompt from starting between the check and the mutation.
            let mut guard = rs.session.lock();
            guard.workspace_roots = roots.clone();
            guard.token_tracker.invalidate_baseline();
            guard.clear_pending_workspace_roots();
            drop(guard);
            drop(sessions);

            // Refresh skills for the new roots
            self.refresh_session_skills(session_id, &roots)?;

            Ok(true)
        }
    }

    /// Check and apply any pending workspace roots for a session.
    ///
    /// This should be called at turn boundaries after a prompt completes.
    pub fn check_and_apply_pending_workspace_roots(&self, session_id: SessionId) {
        let sessions = self.inner.sessions.read();
        let Some(rs) = sessions.get(&session_id) else {
            return;
        };

        let mut guard = rs.session.lock();
        let pending = guard.apply_pending_workspace_roots();
        if !pending {
            return;
        }
        guard.token_tracker.invalidate_baseline();
        let roots = guard.workspace_roots.clone();
        drop(guard);
        drop(sessions);

        // Refresh skills for the new roots
        let _ = self.refresh_session_skills(session_id, &roots);
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
                    && cached.hidden_tools == session_guard.hidden_tools
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
                hidden_tools: session_guard.hidden_tools.clone(),
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
    use crate::provider_credential::store::ProviderCredentialStore;
    use crate::skill::source::StaticSkillSource;
    use crate::skill::{LoadedSkill, SkillMetadata};
    use futures::stream::{self, BoxStream};
    use futures::StreamExt;

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
            diag.message
                .contains("hidden because trust_project_skills is disabled")
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

    // -----------------------------------------------------------------------
    // Managed provider execution path tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn new_with_credential_store_creates_resolver() {
        use crate::provider_credential::store::InMemoryCredentialStore;
        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let runtime =
            IronRuntime::new_with_credential_store(Config::default(), MockProvider, store);
        assert!(runtime.credential_resolver().is_some());
    }

    #[tokio::test]
    async fn provider_auth_status_not_configured() {
        use crate::provider_credential::store::InMemoryCredentialStore;
        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let runtime =
            IronRuntime::new_with_credential_store(Config::default(), MockProvider, store);
        let status = runtime
            .provider_auth_status(&ProviderSlug::new("codex"))
            .await;
        assert_eq!(status, Some(ProviderAuthStatus::NotConfigured));
    }

    #[tokio::test]
    async fn provider_auth_status_api_key() {
        use crate::provider_credential::store::InMemoryCredentialStore;
        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("openai"),
                crate::provider_credential::StoredCredential::ApiKey("sk-test".into()),
            )
            .await;
        let runtime =
            IronRuntime::new_with_credential_store(Config::default(), MockProvider, store);
        let status = runtime
            .provider_auth_status(&ProviderSlug::new("openai"))
            .await;
        assert_eq!(status, Some(ProviderAuthStatus::ConfiguredApiKey));
    }

    #[tokio::test]
    async fn provider_auth_status_for_context_uses_api_key() {
        use crate::provider_credential::store::InMemoryCredentialStore;
        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let runtime =
            IronRuntime::new_with_credential_store(Config::default(), MockProvider, store);
        let context = ProviderPromptContext {
            provider_slug: ProviderSlug::new("kimi-code"),
            model: "kimi-for-coding".into(),
            api_key: Some("sk-test".into()),
        };

        let status = runtime.provider_auth_status_for_context(&context).await;

        assert_eq!(status, Some(ProviderAuthStatus::ConfiguredApiKey));
    }

    #[tokio::test]
    async fn provider_auth_status_for_context_reports_unsupported_api_key() {
        use crate::provider_credential::store::InMemoryCredentialStore;
        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let runtime =
            IronRuntime::new_with_credential_store(Config::default(), MockProvider, store);
        let context = ProviderPromptContext {
            provider_slug: ProviderSlug::new("codex"),
            model: "codex-model".into(),
            api_key: Some("sk-test".into()),
        };

        let status = runtime.provider_auth_status_for_context(&context).await;

        assert_eq!(status, Some(ProviderAuthStatus::UnsupportedCredential));
    }

    #[tokio::test]
    async fn disconnect_provider_oauth_removes_oauth_only() {
        use crate::provider_credential::store::InMemoryCredentialStore;
        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let slug = ProviderSlug::new("kimi-code");
        store
            .set(
                &slug,
                crate::provider_credential::StoredCredential::OAuthBearer(
                    crate::provider_credential::OAuthTokenSet {
                        access_token: "at".into(),
                        refresh_token: "rt".into(),
                        expires_at: None,
                        id_token: None,
                    },
                ),
            )
            .await;
        let runtime =
            IronRuntime::new_with_credential_store(Config::default(), MockProvider, store.clone());
        runtime.disconnect_provider_oauth(&slug).await;
        let stored = store.get(&slug).await;
        assert!(stored.is_none());
    }

    #[tokio::test]
    async fn resolve_managed_provider_api_key() {
        use crate::provider_credential::store::InMemoryCredentialStore;
        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        // kimi-code is a dual-mode provider in the built-in registry
        let slug = ProviderSlug::new("kimi-code");
        store
            .set(
                &slug,
                crate::provider_credential::StoredCredential::ApiKey("sk-test".into()),
            )
            .await;
        let runtime =
            IronRuntime::new_with_credential_store(Config::default(), MockProvider, store);
        let context = ProviderPromptContext {
            provider_slug: slug,
            model: "kimi-for-coding".into(),
            api_key: None,
        };
        let result = runtime.resolve_managed_provider(&context).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn resolve_managed_provider_kimi_code_oauth() {
        use crate::provider_credential::store::InMemoryCredentialStore;
        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let slug = ProviderSlug::new("kimi-code");
        store
            .set(
                &slug,
                crate::provider_credential::StoredCredential::OAuthBearer(
                    crate::provider_credential::OAuthTokenSet {
                        access_token: "access-token".into(),
                        refresh_token: "refresh-token".into(),
                        expires_at: None,
                        id_token: None,
                    },
                ),
            )
            .await;
        let runtime =
            IronRuntime::new_with_credential_store(Config::default(), MockProvider, store);
        let context = ProviderPromptContext {
            provider_slug: slug,
            model: "kimi-for-coding".into(),
            api_key: None,
        };

        let result = runtime.resolve_managed_provider(&context).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn resolve_managed_provider_codex_oauth() {
        use crate::provider_credential::store::InMemoryCredentialStore;
        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let slug = ProviderSlug::new("codex");
        store
            .set(
                &slug,
                crate::provider_credential::StoredCredential::OAuthBearer(
                    crate::provider_credential::OAuthTokenSet {
                        access_token: "access-token".into(),
                        refresh_token: "refresh-token".into(),
                        expires_at: None,
                        id_token: Some("id-token".into()),
                    },
                ),
            )
            .await;
        let runtime =
            IronRuntime::new_with_credential_store(Config::default(), MockProvider, store);
        let context = ProviderPromptContext {
            provider_slug: slug,
            model: "codex-model".into(),
            api_key: None,
        };

        let result = runtime.resolve_managed_provider(&context).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn resolve_managed_provider_missing() {
        use crate::provider_credential::store::InMemoryCredentialStore;
        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let runtime =
            IronRuntime::new_with_credential_store(Config::default(), MockProvider, store);
        let context = ProviderPromptContext {
            provider_slug: ProviderSlug::new("codex"),
            model: "codex-model".into(),
            api_key: None,
        };
        let result = runtime.resolve_managed_provider(&context).await;
        assert!(matches!(result, Err(ProviderAuthError::NotConfigured(ref s)) if s == "codex"),);
    }

    #[tokio::test]
    async fn resolve_managed_provider_with_context_api_key() {
        use crate::provider_credential::store::InMemoryCredentialStore;
        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let runtime =
            IronRuntime::new_with_credential_store(Config::default(), MockProvider, store);
        let context = ProviderPromptContext {
            provider_slug: ProviderSlug::new("kimi-code"),
            model: "kimi-for-coding".into(),
            api_key: Some("sk-test-key".into()),
        };
        let result = runtime.resolve_managed_provider(&context).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn resolve_managed_provider_context_api_key_unsupported() {
        use crate::provider_credential::store::InMemoryCredentialStore;
        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let runtime =
            IronRuntime::new_with_credential_store(Config::default(), MockProvider, store);
        let context = ProviderPromptContext {
            provider_slug: ProviderSlug::new("codex"),
            model: "codex-model".into(),
            api_key: Some("sk-test".into()),
        };
        let result = runtime.resolve_managed_provider(&context).await;
        assert!(
            matches!(result, Err(ProviderAuthError::UnsupportedCredential { ref provider, .. }) if provider == "codex"),
        );
    }

    #[test]
    fn debug_sink_receives_config_event_on_registration() {
        let runtime = IronRuntime::new(Config::default(), MockProvider);
        let sink = std::sync::Arc::new(crate::debug::test_helpers::RecordingDebugSink::new());
        runtime.set_debug_sink(Some(sink.clone()));

        let events = sink.events();
        assert!(
            events
                .iter()
                .any(|e| matches!(e.payload, crate::debug::DebugPayload::Config(_))),
            "Config event should be emitted when sink is registered"
        );
    }

    #[test]
    fn debug_sink_receives_skill_catalog_event_on_refresh() {
        let runtime = IronRuntime::new(Config::default(), MockProvider);
        let sink = std::sync::Arc::new(crate::debug::test_helpers::RecordingDebugSink::new());
        runtime.set_debug_sink(Some(sink.clone()));

        // Clear events from config emission
        let _ = sink.take_events();

        runtime.refresh_skill_catalog();

        let events = sink.events();
        assert!(
            events
                .iter()
                .any(|e| matches!(e.payload, crate::debug::DebugPayload::Skill(_))),
            "Skill catalog event should be emitted on refresh"
        );
    }

    #[test]
    fn debug_sink_receives_session_skill_events_on_create() {
        let runtime = IronRuntime::new(
            Config::default().with_skills(crate::config::SkillConfig::default().with_enabled(true)),
            MockProvider,
        );
        let sink = std::sync::Arc::new(crate::debug::test_helpers::RecordingDebugSink::new());
        runtime.set_debug_sink(Some(sink.clone()));

        // Clear events from config emission
        let _ = sink.take_events();

        let (_session_id, _) = runtime.create_session(crate::ConnectionId(1)).unwrap();

        let events = sink.events();
        assert!(
            events.iter().any(|e| matches!(
                e.payload,
                crate::debug::DebugPayload::Skill(
                    crate::debug::SkillDebugEvent::AvailableToSession { .. }
                )
            )),
            "Skill available event should be emitted when session is created"
        );
    }

    #[test]
    fn debug_events_include_turn_id_when_prompt_started() {
        let runtime = IronRuntime::new(Config::default(), MockProvider);
        let sink = std::sync::Arc::new(crate::debug::test_helpers::RecordingDebugSink::new());
        runtime.set_debug_sink(Some(sink.clone()));

        let (session_id, _) = runtime.create_session(crate::ConnectionId(1)).unwrap();

        // Clear events from config and session emissions
        let _ = sink.take_events();

        let ephemeral = runtime.try_start_prompt(session_id).unwrap();
        let turn_id = ephemeral.lock().turn_id.clone();

        assert!(turn_id.is_some(), "Turn should have a turn_id assigned");

        // Finish the prompt to clean up
        runtime.finish_prompt(session_id);
    }

    // -----------------------------------------------------------------------
    // Session workspace roots tests
    // -----------------------------------------------------------------------

    #[test]
    fn new_session_inherits_config_workspace_roots() {
        let roots = vec![std::path::PathBuf::from("/project/a")];
        let config = Config::default().with_workspace_roots(roots.clone());
        let runtime = IronRuntime::new(config, MockProvider);
        let conn = ConnectionId(1);
        runtime.register_connection(conn);

        let (_session_id, session) = runtime.create_session(conn).unwrap();
        let guard = session.lock();
        assert_eq!(guard.workspace_roots, roots);
    }

    #[test]
    fn new_session_falls_back_to_current_dir_when_config_empty() {
        let config = Config::default();
        let runtime = IronRuntime::new(config, MockProvider);
        let conn = ConnectionId(1);
        runtime.register_connection(conn);

        let (_session_id, session) = runtime.create_session(conn).unwrap();
        let guard = session.lock();
        assert!(!guard.workspace_roots.is_empty());
        // Should be current dir, not empty
        assert_eq!(
            guard.workspace_roots[0],
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        );
    }

    #[test]
    fn set_workspace_roots_on_idle_session_applies_immediately() {
        let config =
            Config::default().with_workspace_roots(vec![std::path::PathBuf::from("/original")]);
        let runtime = IronRuntime::new(config, MockProvider);
        let conn = ConnectionId(1);
        runtime.register_connection(conn);

        let (session_id, session) = runtime.create_session(conn).unwrap();
        let new_roots = vec![std::path::PathBuf::from("/new")];
        let applied = runtime
            .set_session_workspace_roots(session_id, new_roots.clone())
            .unwrap();

        assert!(
            applied,
            "Roots should be applied immediately for idle session"
        );
        let guard = session.lock();
        assert_eq!(guard.workspace_roots, new_roots);
        assert!(guard.pending_workspace_roots.is_none());
    }

    #[test]
    fn set_workspace_roots_during_active_prompt_is_deferred() {
        let config =
            Config::default().with_workspace_roots(vec![std::path::PathBuf::from("/original")]);
        let runtime = IronRuntime::new(config, MockProvider);
        let conn = ConnectionId(1);
        runtime.register_connection(conn);

        let (session_id, session) = runtime.create_session(conn).unwrap();

        // Start a prompt
        let _ephemeral = runtime.try_start_prompt(session_id).unwrap();

        let new_roots = vec![std::path::PathBuf::from("/new")];
        let applied = runtime
            .set_session_workspace_roots(session_id, new_roots.clone())
            .unwrap();

        assert!(!applied, "Roots should be deferred when a prompt is active");

        // Active roots should still be the original
        let guard = session.lock();
        assert_eq!(
            guard.workspace_roots,
            vec![std::path::PathBuf::from("/original")]
        );
        // But pending should be set
        drop(guard);

        // After finishing prompt, pending roots should be applied
        runtime.finish_prompt(session_id);
        runtime.check_and_apply_pending_workspace_roots(session_id);

        let guard = session.lock();
        assert_eq!(guard.workspace_roots, new_roots);
        assert!(guard.pending_workspace_roots.is_none());
    }

    #[test]
    fn multiple_deferred_updates_keep_only_latest() {
        let config =
            Config::default().with_workspace_roots(vec![std::path::PathBuf::from("/original")]);
        let runtime = IronRuntime::new(config, MockProvider);
        let conn = ConnectionId(1);
        runtime.register_connection(conn);

        let (session_id, _session) = runtime.create_session(conn).unwrap();

        // Start a prompt
        let _ephemeral = runtime.try_start_prompt(session_id).unwrap();

        // Set roots multiple times while prompt is active
        runtime
            .set_session_workspace_roots(session_id, vec![std::path::PathBuf::from("/first")])
            .unwrap();
        runtime
            .set_session_workspace_roots(session_id, vec![std::path::PathBuf::from("/second")])
            .unwrap();
        runtime
            .set_session_workspace_roots(session_id, vec![std::path::PathBuf::from("/third")])
            .unwrap();

        // Finish prompt and apply pending
        runtime.finish_prompt(session_id);
        runtime.check_and_apply_pending_workspace_roots(session_id);

        let session = runtime.get_session(session_id).unwrap();
        let guard = session.lock();
        assert_eq!(
            guard.workspace_roots,
            vec![std::path::PathBuf::from("/third")]
        );
    }

    #[test]
    fn workspace_root_changes_are_session_isolated() {
        let config =
            Config::default().with_workspace_roots(vec![std::path::PathBuf::from("/shared")]);
        let runtime = IronRuntime::new(config, MockProvider);
        let conn = ConnectionId(1);
        runtime.register_connection(conn);

        let (session_a_id, session_a) = runtime.create_session(conn).unwrap();
        let (_session_b_id, session_b) = runtime.create_session(conn).unwrap();

        // Change roots for session A only
        runtime
            .set_session_workspace_roots(session_a_id, vec![std::path::PathBuf::from("/a-only")])
            .unwrap();

        // Session A should have new roots
        let guard_a = session_a.lock();
        assert_eq!(
            guard_a.workspace_roots,
            vec![std::path::PathBuf::from("/a-only")]
        );
        drop(guard_a);

        // Session B should still have original roots
        let guard_b = session_b.lock();
        assert_eq!(
            guard_b.workspace_roots,
            vec![std::path::PathBuf::from("/shared")]
        );
    }

    #[test]
    fn session_workspace_roots_serialization_compatibility() {
        // Simulate loading an old session JSON that doesn't have workspace_roots fields
        let old_session_json = r#"{
            "id": 1,
            "messages": [],
            "tool_records": [],
            "timeline": [],
            "script_records": [],
            "instructions": null,
            "workspace_scope": null,
            "compressed_blocks": [],
            "uncompacted_tokens": 0,
            "repo_instruction_payload": null,
            "mcp_server_enablement": {},
            "plugin_enablement": {"enabled": {}},
            "skill_state": {"active": []},
            "available_skills": [],
            "next_visible_id": 1,
            "current_model": null,
            "model_switch_history": []
        }"#;

        let session: DurableSession = serde_json::from_str(old_session_json).unwrap();
        assert!(session.workspace_roots.is_empty());
        assert!(session.pending_workspace_roots.is_none());
    }

    #[test]
    fn session_builtin_tool_config_preserves_base_policy() {
        let base_config = crate::builtin::BuiltinToolConfig {
            allowed_roots: vec![std::path::PathBuf::from("/base")],
            ..Default::default()
        };
        let runtime = IronRuntime::new(Config::default(), MockProvider);
        runtime.register_builtin_tools(&base_config);

        let session_roots = vec![std::path::PathBuf::from("/session")];
        let session_config = runtime.session_builtin_tool_config(&session_roots);

        assert!(session_config.is_some());
        let config = session_config.unwrap();
        assert_eq!(config.allowed_roots, session_roots);
        // Other fields should be preserved from base
        assert_eq!(config.max_output_bytes, base_config.max_output_bytes);
    }

    // ============================================================================
    // Model capability registry hydration tests (issue #68)
    // ============================================================================

    #[test]
    fn runtime_startup_registers_builtin_model_capabilities() {
        let runtime = IronRuntime::new(Config::default(), MockProvider);

        let registry = runtime.model_capability_registry();
        assert!(registry.get("openai", "gpt-4o").is_some());
        assert!(registry
            .get("anthropic", "claude-3-7-sonnet-20250219")
            .is_some());
    }

    #[test]
    fn hydrate_model_capability_registry_loads_builtin_models() {
        let runtime = IronRuntime::new(Config::default(), MockProvider);
        let settings = crate::config::RuntimeSettingsSnapshot {
            provider_configs: vec![],
            custom_models: vec![],
            default_model: None,
            mcp_servers: vec![],
            skill_settings: crate::config::SkillSettingsRecord {
                trust_project_skills: false,
                additional_skill_dirs: vec![],
                updated_at: chrono::Utc::now(),
            },
        };

        runtime
            .hydrate_model_capability_registry(&settings)
            .unwrap();

        let registry = runtime.model_capability_registry();
        assert!(
            registry.get("openai", "gpt-4o").is_some(),
            "Built-in openai/gpt-4o should be registered"
        );
        assert!(
            registry
                .get("anthropic", "claude-3-7-sonnet-20250219")
                .is_some(),
            "Built-in anthropic/sonnet should be registered"
        );
    }

    #[test]
    fn hydrate_model_capability_registry_loads_custom_models() {
        use chrono::Utc;

        let runtime = IronRuntime::new(Config::default(), MockProvider);
        let settings = crate::config::RuntimeSettingsSnapshot {
            provider_configs: vec![],
            custom_models: vec![crate::config::CustomModelRecord {
                provider_slug: "openai".to_string(),
                model_id: "custom-model".to_string(),
                display_name: "Custom Model".to_string(),
                context_window: Some(256_000),
                output_limit: Some(8_192),
                supports_tool_calls: true,
                supports_reasoning: true,
                supports_vision: true,
                supports_streaming: false,
                reasoning_effort_values: vec!["low".to_string(), "high".to_string()],
                cost_input_per_million: Some(5.0),
                cost_output_per_million: Some(15.0),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            default_model: None,
            mcp_servers: vec![],
            skill_settings: crate::config::SkillSettingsRecord {
                trust_project_skills: false,
                additional_skill_dirs: vec![],
                updated_at: chrono::Utc::now(),
            },
        };

        runtime
            .hydrate_model_capability_registry(&settings)
            .unwrap();

        let registry = runtime.model_capability_registry();
        let custom = registry.get("openai", "custom-model").unwrap();
        assert_eq!(custom.context_window, 256_000);
        assert!(custom.supports_tools);
        assert!(!custom.supports_streaming);
        assert!(custom.supports_reasoning_effort);
        assert_eq!(custom.reasoning_effort_values, vec!["low", "high"]);
    }

    #[test]
    fn hydrate_model_capability_registry_enables_capability_diffs() {
        use chrono::Utc;

        let runtime = IronRuntime::new(Config::default(), MockProvider);
        let settings = crate::config::RuntimeSettingsSnapshot {
            provider_configs: vec![],
            custom_models: vec![crate::config::CustomModelRecord {
                provider_slug: "custom-provider".to_string(),
                model_id: "custom-model".to_string(),
                display_name: "Custom".to_string(),
                context_window: Some(50_000),
                output_limit: Some(4_096),
                supports_tool_calls: false,
                supports_reasoning: false,
                supports_vision: false,
                supports_streaming: true,
                reasoning_effort_values: vec![],
                cost_input_per_million: None,
                cost_output_per_million: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            default_model: None,
            mcp_servers: vec![],
            skill_settings: crate::config::SkillSettingsRecord {
                trust_project_skills: false,
                additional_skill_dirs: vec![],
                updated_at: chrono::Utc::now(),
            },
        };

        runtime
            .hydrate_model_capability_registry(&settings)
            .unwrap();

        let registry = runtime.model_capability_registry();
        let diff = registry
            .compare("openai", "gpt-4o", "custom-provider", "custom-model")
            .expect("Both models should be present in the hydrated registry");

        assert!(
            diff.window_shrink.is_some(),
            "Expected window shrink from 128k to 50k"
        );
        assert!(
            !diff.tools_supported,
            "Expected custom model to be marked as lacking tool support"
        );
    }
}
