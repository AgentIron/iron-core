//! Shared runtime state for `IronAgent`.

use crate::{
    capability::{CapabilityBackend, CapabilityDescriptor, CapabilityRegistry},
    config::Config,
    durable::{DurableSession, SessionId},
    ephemeral::EphemeralTurn,
    error::RuntimeError,
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
    /// Create a new runtime with a privately owned Tokio runtime.
    pub fn new<P>(config: Config, provider: P) -> Self
    where
        P: Provider + 'static,
    {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
        let handle = runtime.handle().clone();
        let (shutdown_tx, _) = watch::channel(false);

        let inner = RuntimeInner {
            config,
            provider: Arc::new(provider),
            capabilities: RwLock::new(CapabilityRegistry::new()),
            tool_registry: RwLock::new(ToolRegistry::new()),
            sessions: RwLock::new(HashMap::new()),
            connections: RwLock::new(HashMap::new()),
            tokio_handle: handle,
            _owned_runtime: Some(runtime),
            is_shutdown: AtomicBool::new(false),
            shutdown_tx,
            active_tasks: Mutex::new(Vec::new()),
        };

        Self {
            inner: Arc::new(inner),
        }
    }

    /// Create a new runtime using an existing Tokio runtime handle.
    pub fn from_handle<P>(config: Config, provider: P, handle: tokio::runtime::Handle) -> Self
    where
        P: Provider + 'static,
    {
        let (shutdown_tx, _) = watch::channel(false);

        let inner = RuntimeInner {
            config,
            provider: Arc::new(provider),
            capabilities: RwLock::new(CapabilityRegistry::new()),
            tool_registry: RwLock::new(ToolRegistry::new()),
            sessions: RwLock::new(HashMap::new()),
            connections: RwLock::new(HashMap::new()),
            tokio_handle: handle,
            _owned_runtime: None,
            is_shutdown: AtomicBool::new(false),
            shutdown_tx,
            active_tasks: Mutex::new(Vec::new()),
        };

        Self {
            inner: Arc::new(inner),
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
        durable: DurableSession,
        connection_id: ConnectionId,
    ) -> Result<(), RuntimeError> {
        if self.is_shutdown() {
            return Err(RuntimeError::Connection("Runtime is shut down".into()));
        }
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
        self.inner.is_shutdown.store(true, Ordering::SeqCst);
        let _ = self.inner.shutdown_tx.send(true);

        let tasks = std::mem::take(&mut *self.inner.active_tasks.lock().unwrap());
        for handle in tasks {
            handle.abort();
        }

        self.inner.sessions.write().unwrap().clear();
        self.inner.connections.write().unwrap().clear();
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
