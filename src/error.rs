//! Loop error types
//!
//! Errors that can occur during AgentLoop execution.

use thiserror::Error;

/// Result type alias for loop operations
pub type LoopResult<T> = Result<T, LoopError>;

/// Errors that can occur during loop execution
#[derive(Error, Debug, Clone, PartialEq)]
pub enum LoopError {
    /// Provider error during inference
    #[error("Provider error: {0}")]
    Provider(String),

    /// Maximum iterations exceeded
    #[error("Maximum iterations ({max}) exceeded")]
    MaxIterationsExceeded { max: u32 },

    /// Tool execution error
    #[error("Tool execution error: {message}")]
    ToolExecution { message: String },

    /// Tool not found
    #[error("Tool not found: {name}")]
    ToolNotFound { name: String },

    /// Invalid configuration
    #[error("Invalid configuration: {message}")]
    InvalidConfig { message: String },

    /// Session error
    #[error("Session error: {message}")]
    Session { message: String },

    /// Approval required but not provided
    #[error("Approval required for tool: {tool_name}")]
    ApprovalRequired { tool_name: String },

    /// A turn is already active on this session
    #[error("Turn already active (turn_id: {turn_id})")]
    TurnAlreadyActive { turn_id: u64 },

    /// Operation attempted on a finished turn
    #[error("Turn has already finished")]
    TurnFinished,

    /// Approval command issued while turn is not waiting
    #[error("Turn is not waiting for approval")]
    NotWaitingForApproval,

    /// Approval command referenced an unknown call_id
    #[error("No pending approval for call_id: {call_id}")]
    ApprovalNotFound { call_id: String },

    /// The session has been closed
    #[error("Session has been closed")]
    SessionClosed,

    /// The session runtime has been shut down
    #[error("Session runtime has been shut down")]
    RuntimeShutdown,
}

impl LoopError {
    /// Create a provider error
    pub fn provider<S: Into<String>>(message: S) -> Self {
        Self::Provider(message.into())
    }

    /// Create a max iterations error
    pub fn max_iterations(max: u32) -> Self {
        Self::MaxIterationsExceeded { max }
    }

    /// Create a tool execution error
    pub fn tool_execution<S: Into<String>>(message: S) -> Self {
        Self::ToolExecution {
            message: message.into(),
        }
    }

    /// Create a tool not found error
    pub fn tool_not_found<S: Into<String>>(name: S) -> Self {
        Self::ToolNotFound { name: name.into() }
    }

    /// Create an invalid config error
    pub fn invalid_config<S: Into<String>>(message: S) -> Self {
        Self::InvalidConfig {
            message: message.into(),
        }
    }

    /// Create a session error
    pub fn session<S: Into<String>>(message: S) -> Self {
        Self::Session {
            message: message.into(),
        }
    }

    /// Create an approval required error
    pub fn approval_required<S: Into<String>>(tool_name: S) -> Self {
        Self::ApprovalRequired {
            tool_name: tool_name.into(),
        }
    }

    /// Check if this is a max iterations error
    pub fn is_max_iterations(&self) -> bool {
        matches!(self, Self::MaxIterationsExceeded { .. })
    }
}

pub type RuntimeResult<T> = Result<T, RuntimeError>;

#[derive(Error, Debug)]
pub enum RuntimeError {
    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Capability error: {0}")]
    Capability(String),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Turn error: {0}")]
    Turn(String),

    #[error("Configuration error: {0}")]
    Config(String),
}

impl RuntimeError {
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

    pub fn provider<S: Into<String>>(msg: S) -> Self {
        Self::Provider(msg.into())
    }
}
