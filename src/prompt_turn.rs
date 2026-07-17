//! Standalone handle for observing and cancelling ephemeral prompt-turn state.

use crate::{
    durable::SessionId,
    ephemeral::{EphemeralTurn, SharedEphemeralTurn, TurnPhase},
    error::RuntimeError,
    runtime::IronRuntime,
};
use std::sync::Arc;

/// A locally observable prompt turn associated with one durable session.
///
/// This type owns ephemeral phase and cancellation state. Constructing it does
/// not register the turn as the runtime's active prompt; runtime-managed prompt
/// admission is performed by [`IronRuntime::try_start_prompt`].
pub struct PromptTurn {
    /// Durable session associated with the turn.
    pub session_id: SessionId,
    ephemeral: SharedEphemeralTurn,
    _runtime: IronRuntime,
}

impl PromptTurn {
    /// Creates a pending turn for `session_id` backed by `runtime`.
    pub fn new(session_id: SessionId, runtime: IronRuntime) -> Self {
        let ephemeral = Arc::new(tokio::sync::Mutex::new(EphemeralTurn::new(
            session_id, None,
        )));
        Self {
            session_id,
            ephemeral,
            _runtime: runtime,
        }
    }

    /// Returns the durable session associated with this turn.
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Returns a snapshot of the turn's current phase.
    pub async fn phase(&self) -> TurnPhase {
        self.ephemeral.lock().await.phase
    }

    /// Returns whether the turn has reached a terminal phase.
    pub async fn is_terminal(&self) -> bool {
        self.ephemeral.lock().await.is_terminal()
    }

    /// Returns a shared handle to the turn's ephemeral state.
    pub fn ephemeral(&self) -> SharedEphemeralTurn {
        self.ephemeral.clone()
    }

    /// Requests cancellation unless the turn is already terminal.
    ///
    /// Repeated cancellation and cancellation after termination are no-ops.
    ///
    /// # Errors
    ///
    /// The current implementation is infallible; the result preserves the
    /// cancellation API's fallible contract.
    pub async fn cancel(&self) -> Result<(), RuntimeError> {
        let mut turn = self.ephemeral.lock().await;
        if turn.is_terminal() {
            return Ok(());
        }
        turn.cancel();
        Ok(())
    }
}

impl std::fmt::Debug for PromptTurn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptTurn")
            .field("session_id", &self.session_id)
            .finish()
    }
}
