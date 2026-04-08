use crate::{
    durable::SessionId,
    ephemeral::{EphemeralTurn, SharedEphemeralTurn, TurnPhase},
    error::RuntimeError,
    runtime::IronRuntime,
};
use std::sync::Arc;

pub struct PromptTurn {
    pub session_id: SessionId,
    ephemeral: SharedEphemeralTurn,
    _runtime: IronRuntime,
}

impl PromptTurn {
    pub fn new(session_id: SessionId, runtime: IronRuntime) -> Self {
        let ephemeral = Arc::new(tokio::sync::Mutex::new(EphemeralTurn::new(session_id)));
        Self {
            session_id,
            ephemeral,
            _runtime: runtime,
        }
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub async fn phase(&self) -> TurnPhase {
        self.ephemeral.lock().await.phase
    }

    pub async fn is_terminal(&self) -> bool {
        self.ephemeral.lock().await.is_terminal()
    }

    pub fn ephemeral(&self) -> SharedEphemeralTurn {
        self.ephemeral.clone()
    }

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
