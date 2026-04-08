//! Event types for the AgentIron streaming API
//!
//! Contains both the legacy `StreamEvent` (used by `AgentLoop`) and the new
//! `TurnEvent` / `TurnOutcome` / `TurnStatus` types used by `SessionHandle`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Legacy events (AgentLoop)
// ---------------------------------------------------------------------------

/// Events emitted by the AgentLoop
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    Status {
        message: String,
    },
    Output {
        content: String,
    },
    ToolCall {
        call_id: String,
        tool_name: String,
        arguments: Value,
    },
    ApprovalRequest {
        call_id: String,
        tool_name: String,
        arguments: Value,
    },
    ToolResult {
        call_id: String,
        tool_name: String,
        result: Value,
    },
    Complete,
    Error {
        message: String,
    },
    MaxIterationsReached {
        count: u32,
    },
}

impl StreamEvent {
    pub fn status<S: Into<String>>(message: S) -> Self {
        Self::Status {
            message: message.into(),
        }
    }

    pub fn output<S: Into<String>>(content: S) -> Self {
        Self::Output {
            content: content.into(),
        }
    }

    pub fn tool_call<S1: Into<String>, S2: Into<String>>(
        call_id: S1,
        tool_name: S2,
        arguments: Value,
    ) -> Self {
        Self::ToolCall {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            arguments,
        }
    }

    pub fn approval_request<S1: Into<String>, S2: Into<String>>(
        call_id: S1,
        tool_name: S2,
        arguments: Value,
    ) -> Self {
        Self::ApprovalRequest {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            arguments,
        }
    }

    pub fn tool_result<S1: Into<String>, S2: Into<String>>(
        call_id: S1,
        tool_name: S2,
        result: Value,
    ) -> Self {
        Self::ToolResult {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            result,
        }
    }

    pub fn complete() -> Self {
        Self::Complete
    }

    pub fn error<S: Into<String>>(message: S) -> Self {
        Self::Error {
            message: message.into(),
        }
    }

    pub fn max_iterations(count: u32) -> Self {
        Self::MaxIterationsReached { count }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Error { .. } | Self::MaxIterationsReached { .. }
        )
    }
}

// ---------------------------------------------------------------------------
// New streaming API types (SessionHandle / TurnHandle / TurnEvents)
// ---------------------------------------------------------------------------

/// Stable identifier for a turn within a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnId(pub u64);

/// Operational events emitted during a live turn.
///
/// The event stream always ends with exactly one `Finished` variant
/// (which carries the terminal `TurnOutcome`). After that, `next_event()`
/// returns `Ok(None)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnEvent {
    /// Transient status update (e.g. "Thinking...").
    Status { message: String },
    /// Incremental assistant text.
    OutputDelta { content: String },
    /// The model requested a tool call.
    ToolCall {
        call_id: String,
        tool_name: String,
        arguments: Value,
    },
    /// A tool call requires explicit approval before execution.
    ApprovalRequired {
        call_id: String,
        tool_name: String,
        arguments: Value,
    },
    /// A tool has been executed (or denied) and produced a result.
    ToolResult {
        call_id: String,
        tool_name: String,
        result: Value,
    },
    /// Terminal event — the turn has ended.
    Finished { outcome: TurnOutcome },
}

/// How a turn ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnOutcome {
    /// Turn completed normally (model finished or no more tool calls).
    Completed,
    /// Turn was interrupted by the caller (soft stop).
    Interrupted,
    /// Turn was cancelled by the caller (hard stop).
    Cancelled,
    /// Loop exceeded the configured maximum iterations.
    MaxIterationsReached { count: u32 },
    /// Provider or internal error.
    Failed { message: String },
}

/// Snapshot of a turn's current state, observable through `TurnHandle::status()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TurnStatus {
    /// Provider streaming / tool execution in progress.
    Running,
    /// Paused waiting for per-call approval decisions.
    WaitingForApproval { pending: Vec<PendingCallInfo> },
    /// Turn has ended.
    Finished { outcome: TurnOutcome },
}

/// Information about a single tool call awaiting approval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingCallInfo {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
}
