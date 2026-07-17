//! Runtime error types.

use thiserror::Error;

/// Result type returned by fallible runtime operations.
pub type RuntimeResult<T> = Result<T, RuntimeError>;

#[derive(Error, Debug, Clone, PartialEq)]
/// Failures produced by runtime orchestration and tool execution.
pub enum RuntimeError {
    /// A model provider request or response failed.
    #[error("Provider error: {0}")]
    Provider(
        /// Provider failure details.
        String,
    ),

    /// An agent turn exhausted its configured iteration limit.
    #[error("Maximum iterations ({max}) exceeded")]
    MaxIterationsExceeded {
        /// Configured maximum number of iterations.
        max: u32,
    },

    /// A tool failed while executing.
    #[error("Tool execution error: {message}")]
    ToolExecution {
        /// Tool-provided failure details.
        message: String,
    },

    /// No registered tool has the requested name.
    #[error("Tool not found: {name}")]
    ToolNotFound {
        /// Requested tool name.
        name: String,
    },

    /// Runtime configuration is invalid.
    #[error("Invalid configuration: {message}")]
    InvalidConfig {
        /// Description of the violated configuration invariant.
        message: String,
    },

    /// A session operation failed.
    #[error("Session error: {message}")]
    Session {
        /// Session failure details.
        message: String,
    },

    /// A tool call cannot proceed without approval.
    #[error("Approval required for tool: {tool_name}")]
    ApprovalRequired {
        /// Name of the tool awaiting approval.
        tool_name: String,
    },

    /// A turn was started while another turn remained active.
    #[error("Turn already active (turn_id: {turn_id})")]
    TurnAlreadyActive {
        /// Identifier of the currently active turn.
        turn_id: u64,
    },

    /// An operation targeted a turn that has already completed.
    #[error("Turn has already finished")]
    TurnFinished,

    /// Approval was supplied while the turn was in another state.
    #[error("Turn is not waiting for approval")]
    NotWaitingForApproval,

    /// No pending approval matches the supplied call identifier.
    #[error("No pending approval for call_id: {call_id}")]
    ApprovalNotFound {
        /// Tool call identifier that was not pending.
        call_id: String,
    },

    /// Interaction input was supplied while the turn was in another state.
    #[error("Turn is not waiting for an interaction")]
    NotWaitingForInteraction,

    /// No pending interaction matches the supplied identifier.
    #[error("No pending interaction for interaction_id: {interaction_id}")]
    InteractionNotFound {
        /// Interaction identifier that was not pending.
        interaction_id: String,
    },

    /// A resolution has a different kind than its pending interaction.
    #[error("Interaction resolution kind mismatch for interaction_id: {interaction_id}")]
    InteractionKindMismatch {
        /// Identifier of the interaction with the mismatched resolution.
        interaction_id: String,
    },

    /// An interaction resolution is malformed or semantically invalid.
    #[error("Invalid interaction resolution: {message}")]
    InvalidInteractionResolution {
        /// Description of the invalid resolution.
        message: String,
    },

    /// An operation targeted a closed session.
    #[error("Session has been closed")]
    SessionClosed,

    /// The session's runtime executor is no longer available.
    #[error("Session runtime has been shut down")]
    RuntimeShutdown,

    /// Communication with an external transport failed.
    #[error("Transport error: {0}")]
    Transport(
        /// Transport failure details.
        String,
    ),

    /// A session identifier did not resolve to a known session.
    #[error("Session not found: {0}")]
    SessionNotFound(
        /// Session identifier that was not found.
        String,
    ),

    /// Establishing or using a connection failed.
    #[error("Connection error: {0}")]
    Connection(
        /// Connection failure details.
        String,
    ),

    /// Capability negotiation or invocation failed.
    #[error("Capability error: {0}")]
    Capability(
        /// Capability failure details.
        String,
    ),

    /// A turn-level operation failed.
    #[error("Turn error: {0}")]
    Turn(
        /// Turn failure details.
        String,
    ),

    /// Loading or applying configuration failed.
    #[error("Configuration error: {0}")]
    Config(
        /// Configuration failure details.
        String,
    ),
}

impl RuntimeError {
    /// Construct a provider failure from a message.
    pub fn provider<S: Into<String>>(message: S) -> Self {
        Self::Provider(message.into())
    }

    /// Construct an iteration-limit failure for the configured maximum.
    pub fn max_iterations(max: u32) -> Self {
        Self::MaxIterationsExceeded { max }
    }

    /// Construct a tool-execution failure from a message.
    pub fn tool_execution<S: Into<String>>(message: S) -> Self {
        Self::ToolExecution {
            message: message.into(),
        }
    }

    /// Construct a missing-tool failure from the requested name.
    pub fn tool_not_found<S: Into<String>>(name: S) -> Self {
        Self::ToolNotFound { name: name.into() }
    }

    /// Construct an invalid-configuration failure from a message.
    pub fn invalid_config<S: Into<String>>(message: S) -> Self {
        Self::InvalidConfig {
            message: message.into(),
        }
    }

    /// Construct a session failure from a message.
    pub fn session<S: Into<String>>(message: S) -> Self {
        Self::Session {
            message: message.into(),
        }
    }

    /// Construct an approval-required failure for a tool name.
    pub fn approval_required<S: Into<String>>(tool_name: S) -> Self {
        Self::ApprovalRequired {
            tool_name: tool_name.into(),
        }
    }

    /// Construct a transport failure from a message.
    pub fn transport<S: Into<String>>(msg: S) -> Self {
        Self::Transport(msg.into())
    }

    /// Construct a missing-session failure from an identifier.
    pub fn session_not_found<S: Into<String>>(id: S) -> Self {
        Self::SessionNotFound(id.into())
    }

    /// Construct a connection failure from a message.
    pub fn connection<S: Into<String>>(msg: S) -> Self {
        Self::Connection(msg.into())
    }

    /// Construct a capability failure from a message.
    pub fn capability<S: Into<String>>(msg: S) -> Self {
        Self::Capability(msg.into())
    }

    /// Return whether this error represents exhaustion of the iteration limit.
    pub fn is_max_iterations(&self) -> bool {
        matches!(self, Self::MaxIterationsExceeded { .. })
    }
}
