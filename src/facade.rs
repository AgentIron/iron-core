use crate::plugin::rich_output::{transcript_text as plugin_transcript_text, view as plugin_view};
use crate::{
    capability::{CapabilityBackend, CapabilityDescriptor, CapabilityId},
    config::Config,
    connection::{ClientChannel, IronConnection},
    context::compaction::{CompactionCheckpoint, CompactionEngine, CompactionReason},
    context::handoff::{HandoffExporter, HandoffImporter},
    durable::{
        ContentBlock, DurableSession, DurableToolRecord, SessionId, StructuredMessage,
        TimelineEntry,
    },
    error::RuntimeError,
    runtime::{ConnectionId, IronRuntime},
    tool::Tool,
};
use agent_client_protocol::schema as acp;
use futures::Stream;
use iron_providers::Provider;
use parking_lot::Mutex;
use std::{
    cell::RefCell,
    collections::HashMap,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
};

// Permission option ID constants to avoid magic strings
const PERMISSION_ALLOW_ONCE: &str = "allow_once";
const PERMISSION_REJECT_ONCE: &str = "reject_once";

/// Outcome of a prompt/turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptOutcome {
    /// The turn completed normally.
    EndTurn,
    /// The turn was cancelled by the user.
    Cancelled,
    /// The maximum number of turn requests was reached.
    MaxTurnRequests,
}

impl From<acp::StopReason> for PromptOutcome {
    fn from(reason: acp::StopReason) -> Self {
        match reason {
            acp::StopReason::EndTurn => PromptOutcome::EndTurn,
            acp::StopReason::Cancelled => PromptOutcome::Cancelled,
            acp::StopReason::MaxTurnRequests => PromptOutcome::MaxTurnRequests,
            _ => PromptOutcome::EndTurn,
        }
    }
}

/// Verdict for a permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionVerdict {
    /// Allow this tool call to proceed.
    AllowOnce,
    /// Deny this tool call.
    Deny,
    /// Cancel the entire prompt.
    Cancel,
}

/// A request for user approval of a tool call.
#[derive(Debug, Clone)]
pub struct PermissionRequest {
    /// Unique identifier for this tool call.
    pub call_id: String,
    /// Name of the tool being called.
    pub tool_name: String,
    /// Arguments passed to the tool.
    pub arguments: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Stream-first prompt event types
// ---------------------------------------------------------------------------

/// Events emitted during a streaming prompt session.
///
/// These events are produced by the [`PromptEvents`] stream returned from
/// [`AgentSession::prompt_stream`] and [`AgentSession::prompt_stream_with_blocks`].
/// They provide real-time visibility into the agent's progress, including model
/// output, tool calls, and user approval requests.
///
/// # Event Ordering
///
/// Events are emitted in the order they occur:
/// 1. Zero or more [`Status`](PromptEvent::Status) events
/// 2. Zero or more [`Output`](PromptEvent::Output) events (text from the model)
/// 3. Zero or more [`ToolCall`](PromptEvent::ToolCall) events
/// 4. For each tool requiring approval: an [`ApprovalRequest`](PromptEvent::ApprovalRequest)
/// 5. For each tool call: a [`ToolResult`](PromptEvent::ToolResult)
/// 6. Finally, a [`Complete`](PromptEvent::Complete) event
///
/// # Example
///
/// ```ignore
/// let (handle, mut events) = session.prompt_stream("Hello");
/// while let Some(event) = events.next().await {
///     match event {
///         PromptEvent::Output { text } => print!("{}", text),
///         PromptEvent::ApprovalRequest { call_id, tool_name, .. } => {
///             println!("\nTool {} requires approval", tool_name);
///             handle.approve(&call_id).unwrap();
///         }
///         PromptEvent::Complete { outcome } => break,
///         _ => {}
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub enum PromptEvent {
    /// A status update from the agent.
    Status {
        /// The status message.
        message: String,
    },
    /// Text output from the language model.
    Output {
        /// The text content.
        text: String,
    },
    /// A tool call has been initiated.
    ToolCall {
        /// Unique identifier for this tool call.
        call_id: String,
        /// Name of the tool being called.
        tool_name: String,
        /// Arguments passed to the tool.
        arguments: serde_json::Value,
    },
    /// A tool call requires user approval.
    ApprovalRequest {
        /// Unique identifier for this tool call.
        call_id: String,
        /// Name of the tool being called.
        tool_name: String,
        /// Arguments passed to the tool.
        arguments: serde_json::Value,
    },
    /// A tool call has completed.
    ToolResult {
        /// Unique identifier for this tool call.
        call_id: String,
        /// Name of the tool that was called.
        tool_name: String,
        /// Status of the tool execution.
        status: ToolResultStatus,
        /// The result value (if successful).
        result: Option<serde_json::Value>,
        /// Normalized transcript-safe text for plugin rich output, when present.
        transcript_text: Option<String>,
        /// Normalized rich view payload for plugin rich output, when present.
        view: Option<serde_json::Value>,
    },
    /// Activity from an embedded Python script.
    ScriptActivity {
        /// Unique identifier for the script.
        script_id: String,
        /// The parent tool call that started the script.
        parent_call_id: String,
        /// Type of activity.
        activity_type: ScriptActivityType,
        /// Current status of the activity.
        status: ScriptActivityStatus,
        /// Additional details about the activity.
        detail: Option<serde_json::Value>,
    },
    /// An authentication state transition occurred for a plugin.
    ///
    /// Emitted when a plugin's auth state changes (e.g. from `Unauthenticated`
    /// to `Authenticating`, or from `Authenticating` to `Authenticated`).
    /// Clients can use this to update their auth UX without polling.
    AuthStateChange {
        /// The plugin whose auth state changed.
        auth_id: String,
        /// Previous auth state.
        previous_state: crate::plugin::auth::AuthState,
        /// New auth state.
        new_state: crate::plugin::auth::AuthState,
    },
    /// The prompt has completed.
    Complete {
        /// The final outcome of the prompt.
        outcome: PromptOutcome,
    },
}

/// Types of activities that can occur during embedded Python script execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptActivityType {
    /// The script has started.
    ScriptStarted,
    /// A phase update during script execution.
    ScriptPhase,
    /// The script has completed.
    ScriptCompleted,
    /// A child tool call started within the script.
    ChildToolCallStarted,
    /// A child tool call completed within the script.
    ChildToolCallCompleted,
    /// A child tool call failed within the script.
    ChildToolCallFailed,
}

/// Status of a script activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptActivityStatus {
    /// The activity is currently running.
    Running,
    /// The activity completed successfully.
    Completed,
    /// The activity completed but with some failures.
    CompletedWithFailures,
    /// The activity failed.
    Failed,
    /// The activity was cancelled.
    Cancelled,
}

/// Status of a tool call result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolResultStatus {
    /// The tool call completed successfully.
    Completed,
    /// The tool call failed.
    Failed,
    /// The tool call was denied by the user.
    Denied,
    /// The tool call was cancelled.
    Cancelled,
}

/// Status of an active prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptStatus {
    /// The prompt is pending (not yet started).
    Pending,
    /// The prompt is currently running.
    Running,
    /// The prompt has completed.
    Completed,
    /// The prompt was cancelled.
    Cancelled,
}

/// Handle to control an active streaming prompt.
///
/// Returned by [`AgentSession::prompt_stream`] and
/// [`AgentSession::prompt_stream_with_blocks`], this handle allows you to
/// approve or deny tool calls that require permission, or cancel the entire
/// prompt.
///
/// # Example
///
/// ```ignore
/// let (handle, mut events) = session.prompt_stream("Hello");
///
/// // Later, when an approval request is received...
/// handle.approve("call-123").unwrap();
///
/// // Or cancel the entire prompt
/// handle.cancel().await;
/// ```
pub struct PromptHandle {
    approval_resolvers:
        Rc<RefCell<HashMap<String, tokio::sync::oneshot::Sender<PermissionVerdict>>>>,
    session: AgentSession,
    status: Rc<RefCell<PromptStatus>>,
}

impl PromptHandle {
    /// Approve a tool call that is waiting for permission.
    ///
    /// # Arguments
    ///
    /// * `call_id` - The unique identifier of the tool call to approve.
    ///
    /// # Errors
    ///
    /// Returns an error if no pending approval exists for the given `call_id`.
    pub fn approve(&self, call_id: &str) -> Result<(), String> {
        let mut resolvers = self.approval_resolvers.borrow_mut();
        match resolvers.remove(call_id) {
            Some(tx) => {
                let _ = tx.send(PermissionVerdict::AllowOnce);
                Ok(())
            }
            None => Err(format!("no pending approval for call_id: {}", call_id)),
        }
    }

    /// Deny a tool call that is waiting for permission.
    ///
    /// # Arguments
    ///
    /// * `call_id` - The unique identifier of the tool call to deny.
    ///
    /// # Errors
    ///
    /// Returns an error if no pending approval exists for the given `call_id`.
    pub fn deny(&self, call_id: &str) -> Result<(), String> {
        let mut resolvers = self.approval_resolvers.borrow_mut();
        match resolvers.remove(call_id) {
            Some(tx) => {
                let _ = tx.send(PermissionVerdict::Deny);
                Ok(())
            }
            None => Err(format!("no pending approval for call_id: {}", call_id)),
        }
    }

    /// Cancel the active prompt.
    ///
    /// This will stop any in-progress model inference and tool execution.
    /// All pending approvals will be resolved with a cancel verdict.
    pub async fn cancel(&self) {
        {
            let mut resolvers = self.approval_resolvers.borrow_mut();
            for (_, tx) in resolvers.drain() {
                let _ = tx.send(PermissionVerdict::Cancel);
            }
        }
        *self.status.borrow_mut() = PromptStatus::Cancelled;
        self.session.cancel().await;
    }

    /// Get the current status of the prompt.
    pub fn status(&self) -> PromptStatus {
        *self.status.borrow()
    }
}

impl std::fmt::Debug for PromptHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptHandle")
            .field("status", &*self.status.borrow())
            .finish()
    }
}

/// A stream of prompt events.
///
/// Returned by [`AgentSession::prompt_stream`] and
/// [`AgentSession::prompt_stream_with_blocks`], this struct provides
/// asynchronous access to events emitted during a prompt session.
/// It implements [`Stream`] and can be polled for events.
///
/// # Example
///
/// ```ignore
/// let (handle, mut events) = session.prompt_stream("Hello");
///
/// while let Some(event) = events.next().await {
///     match event {
///         PromptEvent::Output { text } => print!("{}", text),
///         PromptEvent::Complete { .. } => break,
///         _ => {}
///     }
/// }
/// ```
pub struct PromptEvents {
    rx: tokio::sync::mpsc::UnboundedReceiver<PromptEvent>,
}

impl PromptEvents {
    /// Wait for the next event from the stream.
    ///
    /// Returns `None` if the stream has ended.
    pub async fn next(&mut self) -> Option<PromptEvent> {
        self.rx.recv().await
    }

    /// Try to get the next event without blocking.
    ///
    /// Returns `None` if no event is available or if the stream has ended.
    pub fn try_next(&mut self) -> Option<PromptEvent> {
        self.rx.try_recv().ok()
    }

    /// Convert this into a [`Stream`] for use with stream combinators.
    pub fn into_stream(self) -> Pin<Box<dyn Stream<Item = PromptEvent>>> {
        Box::pin(self)
    }
}

impl Stream for PromptEvents {
    type Item = PromptEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

impl std::fmt::Debug for PromptEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptEvents").finish()
    }
}

// ---------------------------------------------------------------------------
// Stream prompt state (internal)
// ---------------------------------------------------------------------------

struct StreamPromptState {
    event_tx: tokio::sync::mpsc::UnboundedSender<PromptEvent>,
    approval_resolvers:
        Rc<RefCell<HashMap<String, tokio::sync::oneshot::Sender<PermissionVerdict>>>>,
    tool_name_index: Rc<RefCell<HashMap<String, String>>>,
}

// ---------------------------------------------------------------------------
// IronAgent
// ---------------------------------------------------------------------------

/// The main entry point for interacting with an Iron agent.
///
/// `IronAgent` is the top-level type for creating and managing agent connections.
/// It owns the runtime, provider, and tool registry. Use [`IronAgent::connect`]
/// to establish a connection and begin interacting with the agent.
///
/// # Example
///
/// ```ignore
/// use iron_core::{IronAgent, Config};
/// use iron_providers::{OpenAiProvider, OpenAiConfig};
///
/// let config = Config::default();
/// let provider = OpenAiProvider::new(OpenAiConfig::new("sk-...".into()));
/// let agent = IronAgent::new(config, provider);
///
/// // Register custom tools
/// agent.register_tool(my_custom_tool);
///
/// // Connect and create a session
/// let conn = agent.connect();
/// let session = conn.create_session().unwrap();
/// ```
pub struct IronAgent {
    runtime: IronRuntime,
}

impl IronAgent {
    /// Create a new agent with the given configuration and provider.
    ///
    /// This creates an agent with its own private Tokio runtime. For integration
    /// with an existing Tokio runtime, use [`IronAgent::with_tokio_handle`].
    pub fn new<P: Provider + 'static>(config: Config, provider: P) -> Self {
        Self {
            runtime: IronRuntime::new(config, provider),
        }
    }

    /// Create a new agent using an existing Tokio runtime handle.
    ///
    /// This is useful when integrating Iron into an existing async application
    /// that already manages its own Tokio runtime.
    pub fn with_tokio_handle<P: Provider + 'static>(
        config: Config,
        provider: P,
        handle: tokio::runtime::Handle,
    ) -> Self {
        Self {
            runtime: IronRuntime::from_handle(config, provider, handle),
        }
    }

    /// Get a reference to the underlying runtime.
    pub fn runtime(&self) -> &IronRuntime {
        &self.runtime
    }

    /// Register a custom tool with the agent.
    ///
    /// Tools must implement the [`Tool`] trait. Once registered,
    /// the tool becomes available for the model to call during prompts.
    pub fn register_tool<T: Tool + 'static>(&self, tool: T) {
        self.runtime.register_tool(tool);
    }

    /// Register all built-in tools with the agent.
    ///
    /// Built-in tools include file operations (read, write, edit), shell commands,
    /// web fetching, and search capabilities. Use the config to control which
    /// tools are enabled.
    pub fn register_builtin_tools(&self, config: &crate::builtin::BuiltinToolConfig) {
        self.runtime.register_builtin_tools(config);
    }

    /// Register the Python execution tool (requires `embedded-python` feature).
    #[cfg(feature = "embedded-python")]
    pub fn register_python_exec_tool(&self) {
        self.runtime
            .register_tool(crate::embedded_python::PythonExecTool::new());
    }

    /// Register the `activate_skill` model-facing tool.
    pub fn register_activate_skill_tool(&self) {
        self.runtime.register_activate_skill_tool();
    }

    /// Get a reference to the Tokio runtime handle.
    pub fn tokio_handle(&self) -> &tokio::runtime::Handle {
        self.runtime.tokio_handle()
    }

    /// Register a capability with the agent.
    ///
    /// Capabilities extend the agent with additional functionality beyond tools.
    pub fn register_capability(&self, descriptor: CapabilityDescriptor) {
        self.runtime.register_capability(descriptor);
    }

    /// Set the backend implementation for a capability.
    pub fn set_capability_backend(&self, id: CapabilityId, backend: CapabilityBackend) {
        self.runtime.set_capability_backend(id, backend);
    }

    /// Register an MCP server with the agent.
    ///
    /// This adds a configured MCP server to the runtime inventory.
    /// Servers must be registered before they can be enabled for sessions.
    pub fn register_mcp_server(&self, config: crate::mcp::McpServerConfig) {
        self.runtime.register_mcp_server(config);
    }

    /// Get the MCP server registry for inspection.
    pub fn mcp_registry(&self) -> parking_lot::RwLockReadGuard<'_, crate::mcp::McpServerRegistry> {
        self.runtime.mcp_registry()
    }

    /// Register a plugin with the agent.
    ///
    /// This adds a configured plugin to the runtime inventory.
    /// Plugins must be registered before they can be enabled for sessions.
    pub fn register_plugin(&self, config: crate::plugin::config::PluginConfig) {
        self.runtime.register_plugin(config);
    }

    /// Get the plugin registry for inspection.
    pub fn plugin_registry(
        &self,
    ) -> parking_lot::RwLockReadGuard<'_, crate::plugin::registry::PluginRegistry> {
        self.runtime.plugin_registry()
    }

    /// Get effective tool definitions for a session, including MCP and plugin tools.
    pub fn get_effective_tools(&self, session_id: SessionId) -> Vec<crate::tool::ToolDefinition> {
        self.runtime.get_effective_tool_definitions(session_id)
    }

    /// Get a full inventory of all registered plugins.
    ///
    /// Returns a [`PluginInfo`](crate::plugin::status::PluginInfo) for every
    /// plugin in the registry.
    pub fn get_plugin_inventory(&self) -> Vec<crate::plugin::status::PluginInfo> {
        self.runtime.get_plugin_inventory()
    }

    /// Get auth prompts for all plugins that require authentication.
    ///
    /// Returns a list of [`AuthPrompt`](crate::plugin::auth::AuthPrompt)
    /// values for every registered plugin that declares OAuth requirements.
    /// Clients can use this to render auth UX without polling individual
    /// plugin statuses.
    pub fn get_auth_prompts(&self) -> Vec<crate::plugin::auth::AuthPrompt> {
        self.runtime.get_auth_prompts()
    }

    /// Get the runtime status of a single plugin.
    ///
    /// Returns `None` if the plugin is not registered.
    pub fn get_plugin_status(
        &self,
        plugin_id: &str,
    ) -> Option<crate::plugin::status::PluginStatus> {
        self.runtime.get_plugin_status(plugin_id)
    }

    /// Set credentials for a plugin and mark it as authenticated.
    pub fn set_plugin_credentials(
        &self,
        plugin_id: &str,
        credentials: crate::plugin::auth::CredentialBinding,
    ) {
        self.runtime.set_plugin_credentials(plugin_id, credentials);
    }

    /// Clear credentials for a plugin and reset its auth state.
    pub fn clear_plugin_credentials(&self, plugin_id: &str) {
        self.runtime.clear_plugin_credentials(plugin_id);
    }

    /// Start a direct client-initiated auth flow for a plugin.
    ///
    /// Returns an [`AuthInteractionRequest`](crate::plugin::auth::AuthInteractionRequest)
    /// that the client should act on (e.g. open a browser to the authorization URL).
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin is not found, does not require auth,
    /// is already authenticating, or is already authenticated.
    pub fn start_auth_flow(
        &self,
        plugin_id: &str,
    ) -> Result<crate::plugin::auth::AuthInteractionRequest, String> {
        self.runtime.begin_plugin_auth_flow(plugin_id)
    }

    /// Complete a direct client-initiated auth flow for a plugin.
    ///
    /// Processes the client's response.  On success, stores credentials and
    /// transitions to `Authenticated`.  On denial, failure, or cancellation,
    /// transitions back to `Unauthenticated`.
    ///
    /// Returns the [`AuthStatusTransition`](crate::plugin::auth::AuthStatusTransition)
    /// describing the state change.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin is not found or is not in the
    /// `Authenticating` state.
    pub fn complete_auth_flow(
        &self,
        plugin_id: &str,
        response: crate::plugin::auth::AuthInteractionResponse,
    ) -> Result<crate::plugin::auth::AuthStatusTransition, String> {
        self.runtime.complete_plugin_auth_flow(plugin_id, response)
    }

    /// Get a recomputed availability summary for a single plugin.
    ///
    /// Returns `None` if the plugin is not registered.
    pub fn get_plugin_availability(
        &self,
        plugin_id: &str,
    ) -> Option<crate::plugin::registry::PluginAvailabilitySummary> {
        self.runtime.get_plugin_availability(plugin_id)
    }

    /// Get unified tool diagnostics for a session.
    ///
    /// Returns `None` if the session does not exist.
    pub fn get_session_tool_diagnostics(
        &self,
        session_id: SessionId,
    ) -> Option<Vec<crate::mcp::session_catalog::ToolDiagnostic>> {
        self.runtime.get_session_tool_diagnostics(session_id)
    }

    /// Establish a new connection to the agent.
    ///
    /// Returns an [`AgentConnection`] which can be used to create sessions
    /// and interact with the agent. Multiple connections can exist simultaneously.
    pub fn connect(&self) -> AgentConnection {
        AgentConnection::new(self.runtime.clone())
    }

    /// Shut down the agent runtime.
    ///
    /// This will cancel all active prompts and close all connections.
    /// After shutdown, no new connections can be established.
    pub fn shutdown(&self) {
        self.runtime.shutdown();
    }
}

impl Clone for IronAgent {
    fn clone(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// AgentConnection
// ---------------------------------------------------------------------------

type AsyncPermissionHandler =
    Box<dyn Fn(PermissionRequest) -> Pin<Box<dyn std::future::Future<Output = PermissionVerdict>>>>;

type SyncPermissionHandler = Rc<RefCell<Option<Box<dyn Fn(&str) -> PermissionVerdict>>>>;

/// A connection to an Iron agent.
///
/// Connections are the primary interface for creating and managing sessions.
/// Each connection has its own event queue and can set up permission handlers
/// for tool approval. Multiple connections can coexist, each with their own
/// set of sessions.
///
/// # Session Ownership
///
/// Sessions created through a connection are owned by that connection. Only
/// the owning connection can prompt, cancel, or close its sessions. This is
/// enforced at the runtime level.
///
/// # Example
///
/// ```ignore
/// let conn = agent.connect();
///
/// // Set up a permission handler
/// conn.on_permission(|call_id| {
///     println!("Tool call {} requires approval", call_id);
///     PermissionVerdict::AllowOnce
/// });
///
/// // Create a session
/// let session = conn.create_session().unwrap();
/// ```
pub struct AgentConnection {
    inner: Rc<IronConnection>,
    permission_handler: SyncPermissionHandler,
    async_permission_handler: Rc<RefCell<Option<AsyncPermissionHandler>>>,
    active_streams: Rc<RefCell<HashMap<String, StreamPromptState>>>,
}

impl AgentConnection {
    fn emit_auth_transition_to_all_streams(
        &self,
        auth_id: &str,
        previous_state: crate::plugin::auth::AuthState,
        new_state: crate::plugin::auth::AuthState,
    ) {
        let streams = self.active_streams.borrow();
        for (_, state) in streams.iter() {
            let _ = state.event_tx.send(PromptEvent::AuthStateChange {
                auth_id: auth_id.to_string(),
                previous_state,
                new_state,
            });
        }
    }

    fn new(runtime: IronRuntime) -> Self {
        let inner = Rc::new(IronConnection::new(runtime));
        let permission_handler = Rc::new(RefCell::new(None));
        let async_permission_handler = Rc::new(RefCell::new(None));
        let active_streams = Rc::new(RefCell::new(HashMap::new()));

        let client: Rc<dyn ClientChannel> = Rc::new(FacadeClientChannel {
            permission_handler: permission_handler.clone(),
            async_permission_handler: async_permission_handler.clone(),
            active_streams: active_streams.clone(),
        });
        inner.set_client(client);

        Self {
            inner,
            permission_handler,
            async_permission_handler,
            active_streams,
        }
    }

    /// Get the unique identifier for this connection.
    pub fn id(&self) -> ConnectionId {
        self.inner.id()
    }

    /// Set a synchronous permission handler for tool approval.
    ///
    /// The handler receives the call ID and should return a [`PermissionVerdict`].
    /// This is called when a tool requiring approval is invoked.
    pub fn on_permission(&self, handler: impl Fn(&str) -> PermissionVerdict + 'static) {
        *self.permission_handler.borrow_mut() = Some(Box::new(handler));
    }

    /// Set an asynchronous permission handler for tool approval.
    ///
    /// The handler receives a [`PermissionRequest`] and returns a future that
    /// resolves to a [`PermissionVerdict`]. This is useful when approval
    /// requires user interaction or external confirmation.
    pub fn on_permission_async(
        &self,
        handler: impl Fn(PermissionRequest) -> Pin<Box<dyn std::future::Future<Output = PermissionVerdict>>>
            + 'static,
    ) {
        *self.async_permission_handler.borrow_mut() = Some(Box::new(handler));
    }

    /// Create a new session on this connection.
    ///
    /// Returns an [`AgentSession`] which can be used to send prompts and
    /// receive responses. Each session maintains its own conversation history.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime has been shut down.
    pub fn create_session(&self) -> Result<AgentSession, RuntimeError> {
        let connection_id = self.inner.id();
        let (session_id, durable) = self.inner.runtime().create_session(connection_id)?;
        Ok(AgentSession {
            id: session_id,
            durable,
            connection: self.inner.clone(),
            active_streams: self.active_streams.clone(),
        })
    }

    /// Close a session, releasing its resources.
    ///
    /// # Errors
    ///
    /// Returns an error if the session is not owned by this connection.
    pub fn close_session(&self, session: &AgentSession) -> Result<(), RuntimeError> {
        let owner = self.inner.runtime().get_session_connection(session.id);
        if owner != Some(self.inner.id()) {
            return Err(RuntimeError::Connection(
                "session not owned by this connection".into(),
            ));
        }
        self.inner.runtime().close_session(session.id);
        Ok(())
    }

    /// Get the list of active session IDs on this connection.
    pub fn active_sessions(&self) -> Vec<SessionId> {
        self.inner
            .runtime()
            .sessions_for_connection(self.inner.id())
    }

    /// Start a direct client-initiated auth flow for a plugin.
    ///
    /// Returns an [`AuthInteractionRequest`](crate::plugin::auth::AuthInteractionRequest)
    /// that the client should act on (e.g. open a browser to the authorization URL).
    /// This does not require model mediation — the client triggers the flow directly.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin is not found, does not require auth,
    /// is already authenticating, or is already authenticated.
    pub fn start_auth_flow(
        &self,
        plugin_id: &str,
    ) -> Result<crate::plugin::auth::AuthInteractionRequest, String> {
        let previous_state = self
            .inner
            .runtime()
            .get_plugin_status(plugin_id)
            .map(|status| status.auth.state)
            .unwrap_or(crate::plugin::auth::AuthState::Unauthenticated);
        let request = self.inner.runtime().begin_plugin_auth_flow(plugin_id)?;
        self.emit_auth_transition_to_all_streams(
            plugin_id,
            previous_state,
            crate::plugin::auth::AuthState::Authenticating,
        );
        Ok(request)
    }

    /// Complete a direct client-initiated auth flow for a plugin.
    ///
    /// Processes the client's response.  On success, stores credentials and
    /// transitions to `Authenticated`.  On denial, failure, or cancellation,
    /// transitions back to `Unauthenticated`.
    ///
    /// Returns the [`AuthStatusTransition`](crate::plugin::auth::AuthStatusTransition)
    /// describing the state change.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin is not found or is not in the
    /// `Authenticating` state.
    pub fn complete_auth_flow(
        &self,
        plugin_id: &str,
        response: crate::plugin::auth::AuthInteractionResponse,
    ) -> Result<crate::plugin::auth::AuthStatusTransition, String> {
        let transition = self
            .inner
            .runtime()
            .complete_plugin_auth_flow(plugin_id, response)?;
        self.emit_auth_transition_to_all_streams(
            &transition.auth_id,
            transition.previous_state,
            transition.new_state,
        );
        Ok(transition)
    }

    /// Get auth prompts for all plugins that require authentication.
    ///
    /// Returns a list of [`AuthPrompt`](crate::plugin::auth::AuthPrompt)
    /// values for every registered plugin that declares OAuth requirements.
    pub fn get_auth_prompts(&self) -> Vec<crate::plugin::auth::AuthPrompt> {
        self.inner.runtime().get_auth_prompts()
    }

    /// Create a new session from a handoff bundle.
    ///
    /// This restores a session that was previously exported via
    /// [`AgentSession::export_handoff`]. The session will have the
    /// conversation history and context from the bundle.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime has been shut down or if the
    /// session ID conflicts with an existing session.
    pub fn create_session_from_handoff(
        &self,
        bundle: crate::context::HandoffBundle,
    ) -> Result<AgentSession, RuntimeError> {
        let connection_id = self.inner.id();
        let durable = crate::context::HandoffImporter::hydrate_into_new(bundle);
        let session_id = durable.id;

        self.inner
            .runtime()
            .insert_session(session_id, durable, connection_id)?;

        let durable = self
            .inner
            .runtime()
            .get_session(session_id)
            .ok_or_else(|| RuntimeError::Connection("Failed to retrieve created session".into()))?;

        Ok(AgentSession {
            id: session_id,
            durable,
            connection: self.inner.clone(),
            active_streams: self.active_streams.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// FacadeClientChannel (routes ACP notifications and permission requests)
// ---------------------------------------------------------------------------

struct FacadeClientChannel {
    permission_handler: SyncPermissionHandler,
    async_permission_handler: Rc<RefCell<Option<AsyncPermissionHandler>>>,
    active_streams: Rc<RefCell<HashMap<String, StreamPromptState>>>,
}

impl FacadeClientChannel {
    fn emit_stream_event(&self, event: PromptEvent) {
        let streams = self.active_streams.borrow();
        for (_, state) in streams.iter() {
            let _ = state.event_tx.send(event.clone());
        }
    }
}

impl ClientChannel for FacadeClientChannel {
    fn send_notification(
        &self,
        notification: acp::SessionNotification,
    ) -> Pin<Box<dyn std::future::Future<Output = agent_client_protocol::Result<()>>>> {
        let session_key = notification.session_id.to_string();
        let streams = self.active_streams.borrow();
        if let Some(state) = streams.get(&session_key) {
            if let Some(prompt_event) = convert_notification_to_prompt_event_with_index(
                &notification,
                &state.tool_name_index,
            ) {
                let _ = state.event_tx.send(prompt_event);
            }
        }
        drop(streams);
        Box::pin(async { Ok(()) })
    }

    fn emit_script_activity(
        &self,
        script_id: &str,
        parent_call_id: &str,
        activity_type: &str,
        status: &str,
        detail: Option<serde_json::Value>,
    ) -> Pin<Box<dyn std::future::Future<Output = ()>>> {
        let activity = match activity_type {
            "script_started" => ScriptActivityType::ScriptStarted,
            "script_phase" => ScriptActivityType::ScriptPhase,
            "script_completed" => ScriptActivityType::ScriptCompleted,
            "child_tool_call_started" => ScriptActivityType::ChildToolCallStarted,
            "child_tool_call_completed" => ScriptActivityType::ChildToolCallCompleted,
            "child_tool_call_failed" => ScriptActivityType::ChildToolCallFailed,
            _ => return Box::pin(async {}),
        };
        let act_status = match status {
            "running" => ScriptActivityStatus::Running,
            "completed" => ScriptActivityStatus::Completed,
            "completed_with_failures" => ScriptActivityStatus::CompletedWithFailures,
            "failed" => ScriptActivityStatus::Failed,
            "cancelled" => ScriptActivityStatus::Cancelled,
            _ => return Box::pin(async {}),
        };
        self.emit_stream_event(PromptEvent::ScriptActivity {
            script_id: script_id.to_string(),
            parent_call_id: parent_call_id.to_string(),
            activity_type: activity,
            status: act_status,
            detail,
        });
        Box::pin(async {})
    }

    fn request_permission(
        &self,
        request: acp::RequestPermissionRequest,
    ) -> Pin<
        Box<
            dyn std::future::Future<
                Output = agent_client_protocol::Result<acp::RequestPermissionResponse>,
            >,
        >,
    > {
        let call_id = request.tool_call.tool_call_id.to_string();
        let tool_name = request.tool_call.fields.title.clone().unwrap_or_default();
        let arguments = request
            .tool_call
            .fields
            .raw_input
            .clone()
            .unwrap_or_default();

        let session_key = request.session_id.to_string();
        let stream_state = self.active_streams.borrow();
        if let Some(state) = stream_state.get(&session_key) {
            let (tx, rx) = tokio::sync::oneshot::channel();
            state
                .approval_resolvers
                .borrow_mut()
                .insert(call_id.clone(), tx);

            let event_tx = state.event_tx.clone();
            drop(stream_state);

            return Box::pin(async move {
                let _ = event_tx.send(PromptEvent::ApprovalRequest {
                    call_id,
                    tool_name,
                    arguments,
                });
                match rx.await {
                    Ok(verdict) => Ok(acp::RequestPermissionResponse::new(verdict_to_outcome(
                        verdict,
                    ))),
                    Err(_) => Ok(acp::RequestPermissionResponse::new(verdict_to_outcome(
                        PermissionVerdict::Deny,
                    ))),
                }
            });
        }
        drop(stream_state);

        let async_handler = self.async_permission_handler.borrow();
        if let Some(ref handler) = *async_handler {
            let perm_req = PermissionRequest {
                call_id,
                tool_name,
                arguments,
            };
            let future = handler(perm_req);
            drop(async_handler);
            return Box::pin(async move {
                let verdict = future.await;
                Ok(acp::RequestPermissionResponse::new(verdict_to_outcome(
                    verdict,
                )))
            });
        }
        drop(async_handler);

        let handler = self.permission_handler.borrow();
        let verdict = handler
            .as_ref()
            .map(|h| h(&call_id))
            .unwrap_or(PermissionVerdict::AllowOnce);
        drop(handler);

        Box::pin(async move {
            Ok(acp::RequestPermissionResponse::new(verdict_to_outcome(
                verdict,
            )))
        })
    }
}

fn verdict_to_outcome(verdict: PermissionVerdict) -> acp::RequestPermissionOutcome {
    match verdict {
        PermissionVerdict::AllowOnce => {
            acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                acp::PermissionOptionId::new(PERMISSION_ALLOW_ONCE),
            ))
        }
        PermissionVerdict::Deny => {
            acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                acp::PermissionOptionId::new(PERMISSION_REJECT_ONCE),
            ))
        }
        PermissionVerdict::Cancel => acp::RequestPermissionOutcome::Cancelled,
    }
}

fn convert_notification_to_prompt_event_with_index(
    notification: &acp::SessionNotification,
    tool_name_index: &Rc<RefCell<HashMap<String, String>>>,
) -> Option<PromptEvent> {
    match &notification.update {
        acp::SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
            acp::ContentBlock::Text(tc) => Some(PromptEvent::Output {
                text: tc.text.clone(),
            }),
            _ => None,
        },
        acp::SessionUpdate::ToolCall(tc) => {
            let call_id = tc.tool_call_id.to_string();
            let tool_name = tc.title.clone();
            let arguments = tc.raw_input.clone().unwrap_or_default();
            tool_name_index
                .borrow_mut()
                .insert(call_id.clone(), tool_name.clone());
            Some(PromptEvent::ToolCall {
                call_id,
                tool_name,
                arguments,
            })
        }
        acp::SessionUpdate::ToolCallUpdate(update) => {
            let call_id = update.tool_call_id.to_string();
            let tool_name = update
                .fields
                .title
                .clone()
                .or_else(|| tool_name_index.borrow().get(&call_id).cloned())
                .unwrap_or_default();
            let result = update.fields.raw_output.clone();
            let transcript_text = result
                .as_ref()
                .and_then(plugin_transcript_text)
                .map(str::to_string);
            let view = result.as_ref().and_then(plugin_view).cloned();
            let status = match update.fields.status {
                Some(acp::ToolCallStatus::Completed) => ToolResultStatus::Completed,
                Some(acp::ToolCallStatus::Failed) => {
                    let is_denied = result.as_ref().is_some_and(|r| {
                        r.get("error")
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| s.contains("denied"))
                    });
                    if is_denied {
                        ToolResultStatus::Denied
                    } else {
                        ToolResultStatus::Failed
                    }
                }
                Some(acp::ToolCallStatus::Pending) => {
                    return None;
                }
                Some(acp::ToolCallStatus::InProgress) => {
                    return None;
                }
                _ => return None,
            };
            Some(PromptEvent::ToolResult {
                call_id,
                tool_name,
                status,
                result,
                transcript_text,
                view,
            })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// AgentSession
// ---------------------------------------------------------------------------

/// A session for interacting with the agent.
///
/// Sessions maintain conversation history and context. Each session is owned
/// by the connection that created it. Use the session to send prompts and
/// receive responses from the agent.
///
/// # Prompt Methods
///
/// Two streaming methods are available for sending prompts:
///
/// - [`prompt_stream`](AgentSession::prompt_stream) - Streaming text-only
///   method. Returns a stream of events and a handle for real-time interaction.
///   This is a convenience wrapper over the shared streaming path.
///
/// - [`prompt_stream_with_blocks`](AgentSession::prompt_stream_with_blocks) -
///   Streaming multimodal method. Accepts structured [`ContentBlock`] values
///   (text, images, resources) and returns the same
///   `(PromptHandle, PromptEvents)` contract. This is the preferred method for
///   interactive applications that need multimodal input.
///
/// # Example
///
/// ```ignore
/// let session = conn.create_session().unwrap();
///
/// // Using the streaming API (text-only convenience)
/// let (handle, mut events) = session.prompt_stream("Hello again");
/// while let Some(event) = events.next().await {
///     match event {
///         PromptEvent::Output { text } => print!("{}", text),
///         PromptEvent::Complete { outcome } => break,
///         _ => {}
///     }
/// }
///
/// // Using the streaming API (multimodal)
/// use iron_core::ContentBlock;
/// let blocks = vec![
///     ContentBlock::text("Describe this image:"),
///     ContentBlock::Image { data: base64_data, mime_type: "image/png".into() },
/// ];
/// let (handle, mut events) = session.prompt_stream_with_blocks(&blocks);
/// while let Some(event) = events.next().await {
///     match event {
///         PromptEvent::Output { text } => print!("{}", text),
///         PromptEvent::Complete { .. } => break,
///         _ => {}
///     }
/// }
/// ```
pub struct AgentSession {
    id: SessionId,
    durable: Arc<Mutex<DurableSession>>,
    connection: Rc<IronConnection>,
    active_streams: Rc<RefCell<HashMap<String, StreamPromptState>>>,
}

impl AgentSession {
    fn emit_auth_transition_to_stream(
        &self,
        auth_id: &str,
        previous_state: crate::plugin::auth::AuthState,
        new_state: crate::plugin::auth::AuthState,
    ) {
        let session_key = self.id.to_string();
        let streams = self.active_streams.borrow();
        if let Some(state) = streams.get(&session_key) {
            let _ = state.event_tx.send(PromptEvent::AuthStateChange {
                auth_id: auth_id.to_string(),
                previous_state,
                new_state,
            });
        }
    }

    /// Get the unique identifier for this session.
    pub fn id(&self) -> SessionId {
        self.id
    }

    /// Send a text prompt and await completion.
    ///
    /// Returns the terminal [`PromptOutcome`]. For incremental event access,
    /// use [`prompt_stream`](AgentSession::prompt_stream).
    pub async fn prompt(&self, text: &str) -> PromptOutcome {
        let acp_session_id = acp::SessionId::new(self.id.to_string());
        let request = acp::PromptRequest::new(
            acp_session_id,
            vec![acp::ContentBlock::Text(acp::TextContent::new(text))],
        );
        match self.connection.handle_prompt(request).await {
            Ok(response) => response.stop_reason.into(),
            Err(_) => PromptOutcome::EndTurn,
        }
    }

    /// Send a multimodal prompt and await completion.
    pub async fn prompt_with_blocks(&self, blocks: &[ContentBlock]) -> PromptOutcome {
        let acp_session_id = acp::SessionId::new(self.id.to_string());
        let acp_blocks: Vec<_> = blocks.iter().map(to_acp_content_block).collect();
        let request = acp::PromptRequest::new(acp_session_id, acp_blocks);
        match self.connection.handle_prompt(request).await {
            Ok(response) => response.stop_reason.into(),
            Err(_) => PromptOutcome::EndTurn,
        }
    }

    /// Send a text prompt and return a stream of events.
    ///
    /// This is a convenience wrapper that wraps the text as a single text
    /// [`ContentBlock`] and delegates to the shared streaming path used by
    /// [`prompt_stream_with_blocks`](AgentSession::prompt_stream_with_blocks).
    ///
    /// See [`prompt_stream_with_blocks`](AgentSession::prompt_stream_with_blocks) for the full multimodal streaming API.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (handle, mut events) = session.prompt_stream("Hello");
    ///
    /// while let Some(event) = events.next().await {
    ///     match event {
    ///         PromptEvent::Output { text } => print!("{}", text),
    ///         PromptEvent::ApprovalRequest { call_id, .. } => {
    ///             handle.approve(&call_id).unwrap();
    ///         }
    ///         PromptEvent::Complete { outcome } => break,
    ///         _ => {}
    ///     }
    /// }
    /// ```
    pub fn prompt_stream(&self, text: &str) -> (PromptHandle, PromptEvents) {
        let acp_blocks = vec![acp::ContentBlock::Text(acp::TextContent::new(text))];
        self.prompt_stream_with_acp_blocks(acp_blocks)
    }

    /// Send a multimodal prompt and return a stream of events.
    ///
    /// This is the streaming API for structured content. It accepts a slice of
    /// [`ContentBlock`] values (text, images, resources) and returns the same
    /// `(PromptHandle, PromptEvents)` contract as [`prompt_stream`](AgentSession::prompt_stream).
    ///
    /// Multimodal streaming preserves the same event-ordering guarantees as
    /// text-only streaming: incremental output may arrive before completion,
    /// `ToolCall` precedes `ToolResult`, approval requests are emitted before
    /// resolution, and exactly one terminal `Complete` is emitted last.
    ///
    /// An empty slice is accepted and follows the same semantics as
    /// [`prompt_stream_with_blocks`](AgentSession::prompt_stream_with_blocks) for empty input.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use iron_core::ContentBlock;
    ///
    /// let blocks = vec![
    ///     ContentBlock::text("Describe this image:"),
    ///     ContentBlock::Image {
    ///         data: base64_data,
    ///         mime_type: "image/png".into(),
    ///     },
    /// ];
    /// let (handle, mut events) = session.prompt_stream_with_blocks(&blocks);
    ///
    /// while let Some(event) = events.next().await {
    ///     match event {
    ///         PromptEvent::Output { text } => print!("{}", text),
    ///         PromptEvent::Complete { outcome } => break,
    ///         _ => {}
    ///     }
    /// }
    /// ```
    pub fn prompt_stream_with_blocks(
        &self,
        blocks: &[ContentBlock],
    ) -> (PromptHandle, PromptEvents) {
        let acp_blocks: Vec<_> = blocks.iter().map(to_acp_content_block).collect();
        self.prompt_stream_with_acp_blocks(acp_blocks)
    }

    /// Shared internal streaming path that accepts pre-converted ACP content
    /// blocks, sets up the active-stream state, and spawns the background
    /// prompt task.
    ///
    /// Both [`prompt_stream`] (text convenience) and
    /// [`prompt_stream_with_blocks`] (multimodal) delegate here so that
    /// ACP block request construction, active-stream registration/removal,
    /// and completion handling are unified in one place.
    fn prompt_stream_with_acp_blocks(
        &self,
        acp_blocks: Vec<acp::ContentBlock>,
    ) -> (PromptHandle, PromptEvents) {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let approval_resolvers: Rc<
            RefCell<HashMap<String, tokio::sync::oneshot::Sender<PermissionVerdict>>>,
        > = Rc::new(RefCell::new(HashMap::new()));

        let prompt_key = self.id.to_string();
        self.active_streams.borrow_mut().insert(
            prompt_key.clone(),
            StreamPromptState {
                event_tx: event_tx.clone(),
                approval_resolvers: approval_resolvers.clone(),
                tool_name_index: Rc::new(RefCell::new(HashMap::new())),
            },
        );

        let status = Rc::new(RefCell::new(PromptStatus::Running));
        let handle = PromptHandle {
            approval_resolvers,
            session: self.clone(),
            status: status.clone(),
        };
        let events = PromptEvents { rx: event_rx };

        let acp_session_id = acp::SessionId::new(self.id.to_string());
        let request = acp::PromptRequest::new(acp_session_id, acp_blocks);

        let connection = self.connection.clone();
        let active_streams = self.active_streams.clone();
        let status_cell = status.clone();

        tokio::task::spawn_local(async move {
            let outcome = match connection.handle_prompt(request).await {
                Ok(response) => response.stop_reason.into(),
                Err(_) => PromptOutcome::EndTurn,
            };

            {
                let mut streams = active_streams.borrow_mut();
                if let Some(state) = streams.remove(&prompt_key) {
                    let _ = state.event_tx.send(PromptEvent::Complete { outcome });
                }
            }

            *status_cell.borrow_mut() = match outcome {
                PromptOutcome::Cancelled => PromptStatus::Cancelled,
                _ => PromptStatus::Completed,
            };
        });

        (handle, events)
    }

    /// Cancel any active prompt on this session.
    pub async fn cancel(&self) {
        let acp_session_id = acp::SessionId::new(self.id.to_string());
        let notification = acp::CancelNotification::new(acp_session_id);
        let _ = self.connection.handle_cancel(notification).await;
    }

    /// Get the conversation timeline.
    ///
    /// Returns a list of timeline entries representing the conversation history.
    pub fn timeline(&self) -> Vec<TimelineEntry> {
        self.durable.lock().timeline.clone()
    }

    /// Get the conversation messages.
    ///
    /// Returns the structured messages in the conversation.
    pub fn messages(&self) -> Vec<StructuredMessage> {
        self.durable.lock().messages.clone()
    }

    /// Get the tool call records.
    ///
    /// Returns a list of all tool calls made during this session.
    pub fn tool_records(&self) -> Vec<DurableToolRecord> {
        self.durable.lock().tool_records.clone()
    }

    /// Check if the session is empty (has no messages).
    pub fn is_empty(&self) -> bool {
        self.durable.lock().is_empty()
    }

    /// Set the system instructions for this session.
    pub fn set_instructions(&self, instructions: impl Into<String>) {
        self.durable.lock().set_instructions(instructions);
    }

    // -- Skill APIs --

    /// Refresh the skill catalog by re-scanning all configured sources.
    ///
    /// Returns diagnostics from the discovery process.
    pub fn refresh_skill_catalog(&self) -> Vec<crate::skill::SkillDiagnostic> {
        let diagnostics = self.connection.runtime().refresh_skill_catalog();
        let available_skills = self.connection.runtime().available_skill_snapshot();
        self.durable.lock().set_available_skills(available_skills);
        diagnostics
    }

    /// List skills available in the runtime catalog.
    pub fn list_available_skills(&self) -> Vec<crate::skill::SkillMetadata> {
        self.durable
            .lock()
            .list_available_skills()
            .iter()
            .map(|skill| skill.metadata.clone())
            .collect()
    }

    /// Activate a skill for this session.
    pub fn activate_skill(&self, name: &str) -> Result<(), String> {
        let skill = self
            .durable
            .lock()
            .load_available_skill(name)
            .ok_or_else(|| format!("Skill '{}' not found", name))?;
        self.durable.lock().activate_skill(
            &skill.metadata.id,
            &skill.body,
            skill.resources.clone(),
        );
        Ok(())
    }

    /// Deactivate a skill for this session.
    pub fn deactivate_skill(&self, name: &str) {
        self.durable.lock().deactivate_skill(name);
    }

    /// List the names of skills currently active in this session.
    pub fn list_active_skills(&self) -> Vec<String> {
        self.durable
            .lock()
            .list_active_skills()
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// Get a snapshot of the active context.
    ///
    /// This provides telemetry about the current context window usage.
    pub fn active_context(
        &self,
        tool_registry: &crate::tool::ToolRegistry,
        current_prompt: Option<&str>,
        context_window_hint: Option<usize>,
    ) -> crate::context::ActiveContextSnapshot {
        let session = self.durable.lock();
        let tail = session.to_transcript();
        crate::context::ContextTelemetry::for_session(
            session.instructions.as_deref(),
            session.compacted_context.as_ref(),
            &tail.messages,
            tool_registry,
            current_prompt,
            context_window_hint,
        )
    }

    /// Check if the session is idle (no active prompts or tool calls).
    pub fn is_idle(&self) -> bool {
        let durable_idle = self.durable.lock().is_idle();
        let has_active_prompt = self.connection.runtime().has_active_prompt(self.id);
        durable_idle && !has_active_prompt
    }

    /// Get the number of uncompacted tokens in the session.
    pub fn uncompacted_tokens(&self) -> usize {
        self.durable.lock().uncompacted_tokens
    }

    /// Get the compacted context, if any.
    pub fn compacted_context(&self) -> Option<crate::context::models::CompactedContext> {
        self.durable.lock().compacted_context.clone()
    }

    /// Create a checkpoint by compacting the session context.
    ///
    /// This uses the provider to generate a summary of older messages,
    /// reducing the token count while preserving conversation context.
    ///
    /// # Errors
    ///
    /// Returns an error if the session is not idle or if context management
    /// is not enabled.
    pub async fn checkpoint(&self, _checkpoint: CompactionCheckpoint) -> Result<(), String> {
        if !self.is_idle() {
            return Err("Cannot checkpoint: session is not idle".into());
        }

        let config = self.connection.runtime().config().clone();
        if !config.context_management.enabled {
            return Err("Context management is not enabled".into());
        }

        let input = {
            let session = self.durable.lock();
            CompactionEngine::prepare(
                &session,
                &config.context_management.tail_retention,
                CompactionReason::Checkpoint,
            )
        };

        let provider = self.connection.runtime().provider();
        let (compacted, tail) = CompactionEngine::execute(input, provider, &config.model).await?;

        {
            let mut session = self.durable.lock();
            session.apply_compaction(compacted, tail);
        }

        Ok(())
    }

    /// Export a handoff bundle for transferring this session to another agent.
    ///
    /// The bundle contains the session's conversation history and context,
    /// allowing it to be resumed elsewhere via
    /// [`AgentConnection::create_session_from_handoff`].
    ///
    /// # Errors
    ///
    /// Returns an error if the session is not idle.
    pub async fn export_handoff(
        &self,
        model: &str,
        provider_name: Option<&str>,
    ) -> Result<crate::context::HandoffBundle, String> {
        if !self.is_idle() {
            return Err("Cannot export handoff: session is not idle".into());
        }

        let config = self.connection.runtime().config().clone();

        let (compacted, tail) = {
            let session = self.durable.lock();
            let (_older, tail) = CompactionEngine::split_session(
                &session,
                &config.context_management.tail_retention,
            );
            (session.compacted_context.clone(), tail)
        };

        let session = self.durable.lock();
        HandoffExporter::export(
            &session,
            model,
            compacted.as_ref(),
            tail,
            &config.context_management,
            provider_name,
        )
    }

    /// Enable or disable an MCP server for this session.
    pub fn set_mcp_server_enabled(&self, server_id: impl Into<String>, enabled: bool) {
        self.durable
            .lock()
            .set_mcp_server_enabled(server_id, enabled);
    }

    /// Check if an MCP server is enabled for this session.
    pub fn is_mcp_server_enabled(&self, server_id: &str) -> Option<bool> {
        self.durable.lock().is_mcp_server_enabled(server_id)
    }

    /// Get list of MCP servers enabled for this session.
    pub fn list_enabled_mcp_servers(&self) -> Vec<String> {
        self.durable.lock().list_enabled_mcp_servers()
    }

    /// Enable or disable a plugin for this session.
    pub fn set_plugin_enabled(&self, plugin_id: impl Into<String>, enabled: bool) {
        self.durable.lock().set_plugin_enabled(plugin_id, enabled);
    }

    /// Check if a plugin is enabled for this session.
    pub fn is_plugin_enabled(&self, plugin_id: &str) -> Option<bool> {
        self.durable.lock().is_plugin_enabled(plugin_id)
    }

    /// Get list of plugins enabled for this session.
    pub fn list_enabled_plugins(&self) -> Vec<String> {
        self.durable.lock().list_enabled_plugins()
    }

    /// Start a direct client-initiated auth flow for a plugin.
    ///
    /// Convenience wrapper that delegates to the connection's
    /// [`AgentConnection::start_auth_flow`].
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin is not found, does not require auth,
    /// is already authenticating, or is already authenticated.
    pub fn start_auth_flow(
        &self,
        plugin_id: &str,
    ) -> Result<crate::plugin::auth::AuthInteractionRequest, String> {
        let previous_state = self
            .connection
            .runtime()
            .get_plugin_status(plugin_id)
            .map(|status| status.auth.state)
            .unwrap_or(crate::plugin::auth::AuthState::Unauthenticated);
        let request = self
            .connection
            .runtime()
            .begin_plugin_auth_flow(plugin_id)?;
        self.emit_auth_transition_to_stream(
            plugin_id,
            previous_state,
            crate::plugin::auth::AuthState::Authenticating,
        );
        Ok(request)
    }

    /// Complete a direct client-initiated auth flow for a plugin.
    ///
    /// Convenience wrapper that delegates to the connection's
    /// [`AgentConnection::complete_auth_flow`].
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin is not found or is not in the
    /// `Authenticating` state.
    pub fn complete_auth_flow(
        &self,
        plugin_id: &str,
        response: crate::plugin::auth::AuthInteractionResponse,
    ) -> Result<crate::plugin::auth::AuthStatusTransition, String> {
        let transition = self
            .connection
            .runtime()
            .complete_plugin_auth_flow(plugin_id, response)?;
        self.emit_auth_transition_to_stream(
            &transition.auth_id,
            transition.previous_state,
            transition.new_state,
        );
        Ok(transition)
    }

    /// Get auth prompts for all plugins that require authentication.
    ///
    /// Convenience wrapper that delegates to the runtime.
    pub fn get_auth_prompts(&self) -> Vec<crate::plugin::auth::AuthPrompt> {
        self.connection.runtime().get_auth_prompts()
    }

    /// Get a session-scoped summary of plugin tool availability.
    ///
    /// Combines the runtime-level registry state with this session's plugin
    /// enablement to produce a per-plugin summary of usable tool counts.
    pub fn get_plugin_tool_summary(
        &self,
    ) -> crate::plugin::effective_tools::SessionPluginToolSummary {
        self.connection
            .runtime()
            .get_session_plugin_summary(self.id)
            .unwrap_or_default()
    }

    /// Get unified tool diagnostics for this session.
    ///
    /// Returns diagnostics for every tool visible (or potentially visible)
    /// to this session, including unavailable plugin tools with reasons.
    pub fn get_tool_diagnostics(&self) -> Vec<crate::mcp::session_catalog::ToolDiagnostic> {
        self.connection
            .runtime()
            .get_session_tool_diagnostics(self.id)
            .unwrap_or_default()
    }

    /// Import a handoff bundle into this session.
    ///
    /// # Errors
    ///
    /// Returns an error if the session is not idle or is not empty.
    /// To import into a new session, use [`AgentConnection::create_session_from_handoff`].
    pub fn import_handoff(&self, bundle: crate::context::HandoffBundle) -> Result<(), String> {
        if !self.is_idle() {
            return Err("Cannot import handoff: session is not idle".into());
        }
        if !self.is_empty() {
            return Err(
                "Cannot import handoff: session must be empty. Use create_session_from_handoff instead.".into(),
            );
        }
        let mut session = self.durable.lock();
        HandoffImporter::hydrate(&mut session, bundle)
    }
}

fn to_acp_content_block(block: &ContentBlock) -> acp::ContentBlock {
    match block {
        ContentBlock::Text { text } => acp::ContentBlock::Text(acp::TextContent::new(text)),
        ContentBlock::Image { data, mime_type } => {
            acp::ContentBlock::Image(acp::ImageContent::new(data, mime_type))
        }
        ContentBlock::Resource { uri, name } => acp::ContentBlock::ResourceLink(
            acp::ResourceLink::new(name.as_deref().unwrap_or("resource"), uri),
        ),
    }
}

impl Clone for AgentSession {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            durable: self.durable.clone(),
            connection: self.connection.clone(),
            active_streams: self.active_streams.clone(),
        }
    }
}

impl std::fmt::Debug for IronAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IronAgent").finish()
    }
}

impl std::fmt::Debug for AgentConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentConnection")
            .field("id", &self.inner.id())
            .finish()
    }
}

impl std::fmt::Debug for AgentSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentSession")
            .field("id", &self.id)
            .finish()
    }
}
