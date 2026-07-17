//! In-memory state for a single active session turn.
//!
//! Unlike [`crate::durable::DurableSession`], this state is not conversation
//! history. It owns transient permission requests, streamed chunks, phase
//! notifications, and a shareable cancellation flag for one in-flight turn.

use crate::durable::SessionId;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Current lifecycle phase of an in-flight prompt turn.
pub enum TurnPhase {
    /// The turn exists but has not started.
    Idle,
    /// Model or tool work is in progress.
    Running,
    /// At least one tool call is awaiting permission.
    WaitingPermission,
    /// Cancellation has begun but has not reached a terminal state.
    Cancelling,
    /// The turn completed normally.
    Completed,
    /// The turn was cancelled.
    Cancelled,
}

/// A permission decision still required by the current turn.
#[derive(Debug, Clone)]
pub enum PendingPermission {
    /// A tool call waiting for approval or denial.
    Waiting {
        /// Provider-assigned identifier of the tool call.
        call_id: String,
        /// Name of the requested tool.
        tool_name: String,
        /// Arguments supplied to the tool call.
        arguments: Value,
    },
}

/// Transient state owned by one in-flight turn.
///
/// The turn belongs to [`Self::session_id`], but it does not own the durable
/// session. Cloned phase watchers and cancellation tokens may outlive a lock
/// guard on this value and allow concurrent tasks to observe its lifecycle.
pub struct EphemeralTurn {
    /// Durable session to which this turn belongs.
    pub session_id: SessionId,
    /// Optional client- or protocol-supplied turn identifier.
    pub turn_id: Option<String>,
    /// Current lifecycle phase.
    pub phase: TurnPhase,
    /// Tool calls whose permission decisions are outstanding.
    pub pending_permissions: Vec<PendingPermission>,
    /// Response fragments buffered while the turn is active.
    pub partial_chunks: Vec<String>,
    phase_tx: watch::Sender<TurnPhase>,
    phase_rx: watch::Receiver<TurnPhase>,
    cancel_requested: Arc<std::sync::atomic::AtomicBool>,
}

impl EphemeralTurn {
    /// Creates an idle turn associated with `session_id`.
    pub fn new(session_id: SessionId, turn_id: Option<String>) -> Self {
        let (phase_tx, phase_rx) = watch::channel(TurnPhase::Idle);
        Self {
            session_id,
            turn_id,
            phase: TurnPhase::Idle,
            pending_permissions: Vec::new(),
            partial_chunks: Vec::new(),
            phase_tx,
            phase_rx,
            cancel_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Moves the turn to [`TurnPhase::Running`] and notifies phase watchers.
    pub fn start(&mut self) {
        self.phase = TurnPhase::Running;
        let _ = self.phase_tx.send(TurnPhase::Running);
    }

    /// Records a tool permission request and enters the waiting phase.
    pub fn request_permission(&mut self, call_id: String, tool_name: String, arguments: Value) {
        self.phase = TurnPhase::WaitingPermission;
        let _ = self.phase_tx.send(TurnPhase::WaitingPermission);
        self.pending_permissions.push(PendingPermission::Waiting {
            call_id,
            tool_name,
            arguments,
        });
    }

    /// Removes the pending request for `call_id`.
    ///
    /// Returns `true` when a request was found. The turn resumes
    /// [`TurnPhase::Running`] only after the final pending request is removed.
    pub fn resolve_permission(&mut self, call_id: &str) -> bool {
        let idx = self.pending_permissions.iter().position(|p| match p {
            PendingPermission::Waiting { call_id: cid, .. } => cid == call_id,
        });
        if let Some(i) = idx {
            self.pending_permissions.swap_remove(i);
            if self.pending_permissions.is_empty() {
                self.phase = TurnPhase::Running;
                let _ = self.phase_tx.send(TurnPhase::Running);
            }
            true
        } else {
            false
        }
    }

    /// Appends a streamed response fragment to the turn-local buffer.
    pub fn add_chunk(&mut self, chunk: String) {
        self.partial_chunks.push(chunk);
    }

    /// Completes the turn and clears permissions and buffered chunks.
    pub fn complete(&mut self) {
        self.phase = TurnPhase::Completed;
        let _ = self.phase_tx.send(TurnPhase::Completed);
        self.pending_permissions.clear();
        self.partial_chunks.clear();
    }

    /// Cancels the turn, publishes its terminal phase, and signals workers.
    ///
    /// Pending permissions are discarded, while buffered chunks are retained.
    pub fn cancel(&mut self) {
        self.phase = TurnPhase::Cancelled;
        let _ = self.phase_tx.send(TurnPhase::Cancelled);
        self.cancel_requested
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.pending_permissions.clear();
    }

    /// Returns whether cancellation has been requested through this turn.
    pub fn is_cancel_requested(&self) -> bool {
        self.cancel_requested
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Returns a shared cancellation flag for work spawned by this turn.
    ///
    /// The returned [`Arc`] observes the same atomic flag as [`Self::cancel`].
    pub fn cancel_token(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.cancel_requested.clone()
    }

    /// Subscribes to subsequent phase changes.
    pub fn phase_watcher(&self) -> watch::Receiver<TurnPhase> {
        self.phase_rx.clone()
    }

    /// Returns whether the turn is completed or cancelled.
    pub fn is_terminal(&self) -> bool {
        matches!(self.phase, TurnPhase::Completed | TurnPhase::Cancelled)
    }
}

impl std::fmt::Debug for EphemeralTurn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EphemeralTurn")
            .field("session_id", &self.session_id)
            .field("turn_id", &self.turn_id)
            .field("phase", &self.phase)
            .field("pending_permissions", &self.pending_permissions.len())
            .field("partial_chunks", &self.partial_chunks.len())
            .finish()
    }
}

/// Shared, asynchronously locked ownership of an [`EphemeralTurn`].
pub type SharedEphemeralTurn = Arc<tokio::sync::Mutex<EphemeralTurn>>;
