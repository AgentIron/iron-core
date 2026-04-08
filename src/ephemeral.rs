use crate::durable::SessionId;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnPhase {
    Idle,
    Running,
    WaitingPermission,
    Cancelling,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub enum PendingPermission {
    Waiting {
        call_id: String,
        tool_name: String,
        arguments: Value,
    },
}

pub struct EphemeralTurn {
    pub session_id: SessionId,
    pub phase: TurnPhase,
    pub pending_permissions: Vec<PendingPermission>,
    pub partial_chunks: Vec<String>,
    phase_tx: watch::Sender<TurnPhase>,
    phase_rx: watch::Receiver<TurnPhase>,
    cancel_requested: Arc<std::sync::atomic::AtomicBool>,
}

impl EphemeralTurn {
    pub fn new(session_id: SessionId) -> Self {
        let (phase_tx, phase_rx) = watch::channel(TurnPhase::Idle);
        Self {
            session_id,
            phase: TurnPhase::Idle,
            pending_permissions: Vec::new(),
            partial_chunks: Vec::new(),
            phase_tx,
            phase_rx,
            cancel_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn start(&mut self) {
        self.phase = TurnPhase::Running;
        let _ = self.phase_tx.send(TurnPhase::Running);
    }

    pub fn request_permission(&mut self, call_id: String, tool_name: String, arguments: Value) {
        self.phase = TurnPhase::WaitingPermission;
        let _ = self.phase_tx.send(TurnPhase::WaitingPermission);
        self.pending_permissions.push(PendingPermission::Waiting {
            call_id,
            tool_name,
            arguments,
        });
    }

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

    pub fn add_chunk(&mut self, chunk: String) {
        self.partial_chunks.push(chunk);
    }

    pub fn complete(&mut self) {
        self.phase = TurnPhase::Completed;
        let _ = self.phase_tx.send(TurnPhase::Completed);
        self.pending_permissions.clear();
        self.partial_chunks.clear();
    }

    pub fn cancel(&mut self) {
        self.phase = TurnPhase::Cancelled;
        let _ = self.phase_tx.send(TurnPhase::Cancelled);
        self.cancel_requested
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.pending_permissions.clear();
    }

    pub fn is_cancel_requested(&self) -> bool {
        self.cancel_requested
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn cancel_token(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.cancel_requested.clone()
    }

    pub fn phase_watcher(&self) -> watch::Receiver<TurnPhase> {
        self.phase_rx.clone()
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.phase, TurnPhase::Completed | TurnPhase::Cancelled)
    }
}

impl std::fmt::Debug for EphemeralTurn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EphemeralTurn")
            .field("session_id", &self.session_id)
            .field("phase", &self.phase)
            .field("pending_permissions", &self.pending_permissions.len())
            .field("partial_chunks", &self.partial_chunks.len())
            .finish()
    }
}

pub type SharedEphemeralTurn = Arc<tokio::sync::Mutex<EphemeralTurn>>;
