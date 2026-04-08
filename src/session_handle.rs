#![allow(deprecated)]
use crate::{
    config::{Config, ConfigSource},
    error::LoopError,
    events::{TurnId, TurnStatus},
    session::Session,
    session_runtime::SessionRuntime,
    tool::ToolRegistry,
    turn::{self, TurnEvents, TurnHandle},
};
use iron_providers::Provider;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};

/// Primary public API for session and turn execution.
///
/// A `SessionHandle` owns conversation state, configuration, a provider,
/// and a tool registry. It manages turn lifecycle (one active turn at a time)
/// and provides streaming events for each turn.
///
/// # Runtime ownership
///
/// The default constructors ([`SessionHandle::new`] and
/// [`SessionHandle::with_tools`]) create a **private** Tokio runtime
/// internally. Callers do **not** need an ambient Tokio runtime — starting a
/// turn works from plain `#[test]` functions, `main()`, or any synchronous
/// context.
///
/// Advanced Rust callers that already manage a Tokio runtime can bind
/// sessions to a shared [`SessionRuntime`] via [`SessionHandle::new_in`]
/// and [`SessionHandle::with_tools_in`]. See [`SessionRuntime`] for details.
///
/// # Direct construction
///
/// ```ignore
/// use iron_core::{Config, SessionHandle, Session, OpenAiProvider, OpenAiConfig};
///
/// let handle = SessionHandle::new(
///     Config::default(),
///     OpenAiProvider::new(OpenAiConfig::new("sk-...".into())),
///     Session::new(),
/// );
/// let (turn, events) = handle.start_turn("hello").unwrap();
/// ```
///
/// # Caller-owned config (bridge pattern)
///
/// For applications that keep their own config type, implement
/// [`ConfigSource`] on your config and use the `from_source` constructors.
/// The bridge snapshots projected values at construction time — later
/// mutations to your config do not affect running sessions.
///
/// ```ignore
/// use iron_core::{ConfigSource, Config, SessionHandle, Session, LoopError};
///
/// struct AppConfig {
///     model: String,
///     temperature: f32,
/// }
///
/// impl ConfigSource for AppConfig {
///     fn to_config(&self) -> Result<Config, LoopError> {
///         Ok(Config::new()
///             .with_model(&self.model)
///             .with_default_generation(
///                 GenerationConfig::new().with_temperature(self.temperature),
///             ))
///     }
/// }
///
/// let app_config = AppConfig { model: "gpt-4o".into(), temperature: 0.7 };
/// let handle = SessionHandle::from_source(
///     &app_config,
///     provider,
///     Session::new(),
/// )?;
/// ```
///
/// # Lifecycle
///
/// - **Start a turn**: [`SessionHandle::start_turn`] returns a
///   [`TurnHandle`] and a
///   [`TurnEvents`] stream.
/// - **Close**: [`SessionHandle::close`] cancels the active turn, rejects
///   future turns, and releases the private runtime (if owned).
/// - **Drop**: performs the same best-effort cleanup as `close()`.
///
/// # Example (default, private runtime)
///
/// ```ignore
/// use iron_core::{Config, SessionHandle, Session, OpenAiProvider, OpenAiConfig};
///
/// let handle = SessionHandle::new(
///     Config::default(),
///     OpenAiProvider::new(OpenAiConfig::new("sk-...".into())),
///     Session::new(),
/// );
/// let (turn, events) = handle.start_turn("hello").unwrap();
/// ```
///
/// # Example (shared runtime)
///
/// ```ignore
/// use iron_core::{Config, SessionHandle, Session, SessionRuntime, OpenAiProvider, OpenAiConfig};
///
/// let runtime = SessionRuntime::new();
/// let handle = SessionHandle::new_in(
///     &runtime,
///     Config::default(),
///     OpenAiProvider::new(OpenAiConfig::new("sk-...".into())),
///     Session::new(),
/// );
/// ```
pub struct SessionHandle {
    session: Arc<Mutex<Session>>,
    config: Config,
    provider: Arc<dyn Provider>,
    tools: Arc<ToolRegistry>,
    next_turn_id: Arc<AtomicU64>,
    active_turn: Mutex<Option<TurnHandle>>,
    runtime: SessionRuntime,
    owns_runtime: bool,
    closed: AtomicBool,
}

impl SessionHandle {
    /// Create a new session handle with a private internal runtime.
    ///
    /// No ambient Tokio runtime is required from the caller.
    pub fn new<P>(config: Config, provider: P, session: Session) -> Self
    where
        P: Provider + 'static,
    {
        Self {
            session: Arc::new(Mutex::new(session)),
            config,
            provider: Arc::new(provider),
            tools: Arc::new(ToolRegistry::new()),
            next_turn_id: Arc::new(AtomicU64::new(1)),
            active_turn: Mutex::new(None),
            runtime: SessionRuntime::new(),
            owns_runtime: true,
            closed: AtomicBool::new(false),
        }
    }

    /// Create a new session handle with tools and a private internal runtime.
    ///
    /// No ambient Tokio runtime is required from the caller.
    pub fn with_tools<P>(config: Config, provider: P, session: Session, tools: ToolRegistry) -> Self
    where
        P: Provider + 'static,
    {
        Self {
            session: Arc::new(Mutex::new(session)),
            config,
            provider: Arc::new(provider),
            tools: Arc::new(tools),
            next_turn_id: Arc::new(AtomicU64::new(1)),
            active_turn: Mutex::new(None),
            runtime: SessionRuntime::new(),
            owns_runtime: true,
            closed: AtomicBool::new(false),
        }
    }

    /// Create a session handle bound to a shared [`SessionRuntime`].
    ///
    /// The runtime's lifecycle is managed externally; dropping this handle
    /// does **not** shut down the runtime.
    pub fn new_in<P>(
        runtime: &SessionRuntime,
        config: Config,
        provider: P,
        session: Session,
    ) -> Self
    where
        P: Provider + 'static,
    {
        Self {
            session: Arc::new(Mutex::new(session)),
            config,
            provider: Arc::new(provider),
            tools: Arc::new(ToolRegistry::new()),
            next_turn_id: Arc::new(AtomicU64::new(1)),
            active_turn: Mutex::new(None),
            runtime: runtime.clone(),
            owns_runtime: false,
            closed: AtomicBool::new(false),
        }
    }

    /// Create a session handle with tools, bound to a shared [`SessionRuntime`].
    ///
    /// The runtime's lifecycle is managed externally; dropping this handle
    /// does **not** shut down the runtime.
    pub fn with_tools_in<P>(
        runtime: &SessionRuntime,
        config: Config,
        provider: P,
        session: Session,
        tools: ToolRegistry,
    ) -> Self
    where
        P: Provider + 'static,
    {
        Self {
            session: Arc::new(Mutex::new(session)),
            config,
            provider: Arc::new(provider),
            tools: Arc::new(tools),
            next_turn_id: Arc::new(AtomicU64::new(1)),
            active_turn: Mutex::new(None),
            runtime: runtime.clone(),
            owns_runtime: false,
            closed: AtomicBool::new(false),
        }
    }

    pub fn from_shared(
        config: Config,
        provider: Arc<dyn Provider>,
        session: Session,
        tools: ToolRegistry,
    ) -> Self {
        Self {
            session: Arc::new(Mutex::new(session)),
            config,
            provider,
            tools: Arc::new(tools),
            next_turn_id: Arc::new(AtomicU64::new(1)),
            active_turn: Mutex::new(None),
            runtime: SessionRuntime::new(),
            owns_runtime: true,
            closed: AtomicBool::new(false),
        }
    }

    /// Create a session handle from a caller-owned config source.
    ///
    /// The source is projected into a validated `Config` snapshot at
    /// construction time. Later mutations to the source do not affect
    /// the session.
    pub fn from_source<S, P>(source: &S, provider: P, session: Session) -> Result<Self, LoopError>
    where
        S: ConfigSource,
        P: Provider + 'static,
    {
        let config = source.to_config()?;
        config.validate()?;
        Ok(Self::new(config, provider, session))
    }

    /// Create a session handle with tools from a caller-owned config source.
    pub fn from_source_with_tools<S, P>(
        source: &S,
        provider: P,
        session: Session,
        tools: ToolRegistry,
    ) -> Result<Self, LoopError>
    where
        S: ConfigSource,
        P: Provider + 'static,
    {
        let config = source.to_config()?;
        config.validate()?;
        Ok(Self::with_tools(config, provider, session, tools))
    }

    /// Create a session handle from a source, bound to a shared runtime.
    pub fn from_source_in<S, P>(
        runtime: &SessionRuntime,
        source: &S,
        provider: P,
        session: Session,
    ) -> Result<Self, LoopError>
    where
        S: ConfigSource,
        P: Provider + 'static,
    {
        let config = source.to_config()?;
        config.validate()?;
        Ok(Self::new_in(runtime, config, provider, session))
    }

    /// Create a session handle with tools from a source, bound to a shared runtime.
    pub fn from_source_with_tools_in<S, P>(
        runtime: &SessionRuntime,
        source: &S,
        provider: P,
        session: Session,
        tools: ToolRegistry,
    ) -> Result<Self, LoopError>
    where
        S: ConfigSource,
        P: Provider + 'static,
    {
        let config = source.to_config()?;
        config.validate()?;
        Ok(Self::with_tools_in(
            runtime, config, provider, session, tools,
        ))
    }

    /// Start a new turn on this session.
    ///
    /// Returns an error if the session is closed, the runtime is shut down,
    /// or another turn is already active.
    pub fn start_turn(&self, user_input: &str) -> Result<(TurnHandle, TurnEvents), LoopError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(LoopError::SessionClosed);
        }

        if self.runtime.is_shutdown() {
            return Err(LoopError::RuntimeShutdown);
        }

        let mut active = self.active_turn.lock().unwrap();

        if let Some(ref handle) = *active {
            match handle.status() {
                TurnStatus::Finished { .. } => {}
                _ => {
                    return Err(LoopError::TurnAlreadyActive {
                        turn_id: handle.id().0,
                    })
                }
            }
        }

        self.session.lock().unwrap().add_user_message(user_input);

        let turn_id = TurnId(self.next_turn_id.fetch_add(1, Ordering::SeqCst));
        let (handle, events) = turn::create_turn(
            turn_id,
            self.provider.clone(),
            self.config.clone(),
            self.tools.clone(),
            self.session.clone(),
            &self.runtime,
        )?;

        *active = Some(handle.clone());

        Ok((handle, events))
    }

    /// Close this session handle.
    ///
    /// Cancels the active turn, rejects future turns, and releases the
    /// private runtime if this handle owns one. For shared-runtime handles,
    /// only the per-session cleanup is performed (the runtime keeps running).
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);

        if let Some(turn) = self.active_turn.lock().unwrap().take() {
            let _ = turn.cancel();
        }

        if self.owns_runtime {
            self.runtime.shutdown();
        }
    }

    pub fn active_turn(&self) -> Option<TurnHandle> {
        let active = self.active_turn.lock().unwrap();
        active
            .as_ref()
            .filter(|h| !matches!(h.status(), TurnStatus::Finished { .. }))
            .cloned()
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn session(&self) -> std::sync::MutexGuard<'_, Session> {
        self.session.lock().unwrap()
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        if !self.closed.load(Ordering::SeqCst) {
            self.closed.store(true, Ordering::SeqCst);
            if let Some(turn) = self.active_turn.lock().unwrap().take() {
                let _ = turn.cancel();
            }
            if self.owns_runtime {
                self.runtime.shutdown();
            }
        }
    }
}

impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandle")
            .field("config", &self.config)
            .finish()
    }
}
