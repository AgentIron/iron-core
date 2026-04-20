//! Runtime error types.

use thiserror::Error;

pub type RuntimeResult<T> = Result<T, RuntimeError>;

#[derive(Error, Debug, Clone, PartialEq)]
pub enum RuntimeError {
    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Maximum iterations ({max}) exceeded")]
    MaxIterationsExceeded { max: u32 },

    #[error("Tool execution error: {message}")]
    ToolExecution { message: String },

    #[error("Tool not found: {name}")]
    ToolNotFound { name: String },

    #[error("Invalid configuration: {message}")]
    InvalidConfig { message: String },

    #[error("Session error: {message}")]
    Session { message: String },

    #[error("Approval required for tool: {tool_name}")]
    ApprovalRequired { tool_name: String },

    #[error("Turn already active (turn_id: {turn_id})")]
    TurnAlreadyActive { turn_id: u64 },

    #[error("Turn has already finished")]
    TurnFinished,

    #[error("Turn is not waiting for approval")]
    NotWaitingForApproval,

    #[error("No pending approval for call_id: {call_id}")]
    ApprovalNotFound { call_id: String },

    #[error("Turn is not waiting for an interaction")]
    NotWaitingForInteraction,

    #[error("No pending interaction for interaction_id: {interaction_id}")]
    InteractionNotFound { interaction_id: String },

    #[error("Interaction resolution kind mismatch for interaction_id: {interaction_id}")]
    InteractionKindMismatch { interaction_id: String },

    #[error("Invalid interaction resolution: {message}")]
    InvalidInteractionResolution { message: String },

    #[error("Session has been closed")]
    SessionClosed,

    #[error("Session runtime has been shut down")]
    RuntimeShutdown,

    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Capability error: {0}")]
    Capability(String),

    #[error("Turn error: {0}")]
    Turn(String),

    #[error("Configuration error: {0}")]
    Config(String),
}

impl RuntimeError {
    pub fn provider<S: Into<String>>(message: S) -> Self {
        Self::Provider(message.into())
    }

    pub fn max_iterations(max: u32) -> Self {
        Self::MaxIterationsExceeded { max }
    }

    pub fn tool_execution<S: Into<String>>(message: S) -> Self {
        Self::ToolExecution {
            message: message.into(),
        }
    }

    pub fn tool_not_found<S: Into<String>>(name: S) -> Self {
        Self::ToolNotFound { name: name.into() }
    }

    pub fn invalid_config<S: Into<String>>(message: S) -> Self {
        Self::InvalidConfig {
            message: message.into(),
        }
    }

    pub fn session<S: Into<String>>(message: S) -> Self {
        Self::Session {
            message: message.into(),
        }
    }

    pub fn approval_required<S: Into<String>>(tool_name: S) -> Self {
        Self::ApprovalRequired {
            tool_name: tool_name.into(),
        }
    }

    pub fn transport<S: Into<String>>(msg: S) -> Self {
        Self::Transport(msg.into())
    }

    pub fn session_not_found<S: Into<String>>(id: S) -> Self {
        Self::SessionNotFound(id.into())
    }

    pub fn connection<S: Into<String>>(msg: S) -> Self {
        Self::Connection(msg.into())
    }

    pub fn capability<S: Into<String>>(msg: S) -> Self {
        Self::Capability(msg.into())
    }

    pub fn is_max_iterations(&self) -> bool {
        matches!(self, Self::MaxIterationsExceeded { .. })
    }
}
