use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::sync::watch;
use tokio::task::JoinHandle;

struct RuntimeInner {
    handle: tokio::runtime::Handle,
    _owned_runtime: Option<tokio::runtime::Runtime>,
    is_shutdown: AtomicBool,
    shutdown_tx: watch::Sender<bool>,
    active_tasks: Mutex<Vec<JoinHandle<()>>>,
}

/// Shared runtime owner for advanced lifecycle control.
///
/// `SessionRuntime` is an **advanced** type. Most callers should use
/// [`SessionHandle::new`](crate::SessionHandle::new) or
/// [`SessionHandle::with_tools`](crate::SessionHandle::with_tools), which
/// create a private runtime internally.
///
/// Use `SessionRuntime` when you need:
/// - Multiple [`SessionHandle`](crate::SessionHandle) instances sharing one
///   runtime (e.g. multi-tenant or multi-session servers).
/// - Explicit control over when background execution stops via
///   [`SessionRuntime::shutdown`].
/// - Binding to a caller-managed Tokio runtime via
///   [`SessionRuntime::from_handle`].
///
/// # Lifecycle
///
/// 1. Create: [`SessionRuntime::new`] (owned) or
///    [`SessionRuntime::from_handle`] (borrowed).
/// 2. Share: pass `&SessionRuntime` to
///    [`SessionHandle::new_in`](crate::SessionHandle::new_in) or
///    [`SessionHandle::with_tools_in`](crate::SessionHandle::with_tools_in).
/// 3. Shutdown: call [`SessionRuntime::shutdown`] to cancel active turns and
///    reject new spawns. Dropping the last reference also performs cleanup.
pub struct SessionRuntime {
    inner: Arc<RuntimeInner>,
}

impl SessionRuntime {
    /// Create a runtime that owns a new Tokio runtime.
    pub fn new() -> Self {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
        let handle = runtime.handle().clone();
        let (shutdown_tx, _) = watch::channel(false);

        Self {
            inner: Arc::new(RuntimeInner {
                handle,
                _owned_runtime: Some(runtime),
                is_shutdown: AtomicBool::new(false),
                shutdown_tx,
                active_tasks: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Create a runtime that borrows a caller-managed Tokio handle.
    ///
    /// The caller is responsible for keeping the backing runtime alive.
    /// Shutting down this `SessionRuntime` cancels tracked tasks but does
    /// **not** tear down the underlying Tokio runtime.
    pub fn from_handle(handle: tokio::runtime::Handle) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            inner: Arc::new(RuntimeInner {
                handle,
                _owned_runtime: None,
                is_shutdown: AtomicBool::new(false),
                shutdown_tx,
                active_tasks: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Spawn a future on this runtime.
    ///
    /// Returns `false` if the runtime is already shut down (the future is
    /// dropped without being executed).
    pub fn spawn<F>(&self, future: F) -> bool
    where
        F: Future<Output = ()> + Send + 'static,
    {
        if self.inner.is_shutdown.load(Ordering::SeqCst) {
            return false;
        }

        let handle = self.inner.handle.spawn(future);
        self.inner.active_tasks.lock().unwrap().push(handle);
        true
    }

    pub fn is_shutdown(&self) -> bool {
        self.inner.is_shutdown.load(Ordering::SeqCst)
    }

    pub fn shutdown_token(&self) -> watch::Receiver<bool> {
        self.inner.shutdown_tx.subscribe()
    }

    /// Shut down this runtime.
    ///
    /// Sets the shutdown flag, signals all shutdown-token holders, and
    /// aborts tracked tasks. After this call, [`SessionRuntime::spawn`]
    /// returns `false` and active turns receive a cancellation outcome.
    pub fn shutdown(&self) {
        self.inner.is_shutdown.store(true, Ordering::SeqCst);
        let _ = self.inner.shutdown_tx.send(true);

        let tasks = std::mem::take(&mut *self.inner.active_tasks.lock().unwrap());
        for handle in tasks {
            handle.abort();
        }
    }
}

impl Clone for SessionRuntime {
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

impl std::fmt::Debug for SessionRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRuntime")
            .field("is_shutdown", &self.is_shutdown())
            .field("owned", &self.inner._owned_runtime.is_some())
            .finish()
    }
}

impl Default for SessionRuntime {
    fn default() -> Self {
        Self::new()
    }
}
