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
    /// A blocking interaction requires resolution before the turn can continue.
    ///
    /// This is the generalized interaction event. For approval interactions,
    /// existing callers may still listen for `ApprovalRequired` (which is
    /// emitted alongside this for compatibility), but new code should use
    /// `InteractionRequired`.
    InteractionRequired {
        interaction: PendingInteractionInfo,
    },
    /// A tool call requires explicit approval before execution.
    ///
    /// **Deprecated in favor of `InteractionRequired`**: retained for backward
    /// compatibility. Emitted alongside `InteractionRequired` when the pending
    /// interaction is an approval.
    #[deprecated(note = "Use InteractionRequired instead")]
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
    /// Paused waiting for a blocking interaction resolution.
    WaitingForInteraction { pending: PendingInteractionInfo },
    /// Turn has ended.
    Finished { outcome: TurnOutcome },
}

// ---------------------------------------------------------------------------
// Pending interaction envelope (shared by approval and choice)
// ---------------------------------------------------------------------------

/// Who initiated the blocking interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionSource {
    /// The model/runtime requested this interaction.
    Model,
    /// The runtime itself initiated this interaction (e.g. tool approval gating).
    Runtime,
}

/// A single blocking pending interaction envelope exposed through turn status
/// and event APIs. At most one of these may be active per turn in v1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingInteractionInfo {
    /// Unique identifier for this interaction within the turn.
    pub interaction_id: String,
    /// Who initiated the interaction.
    pub source: InteractionSource,
    /// Typed payload describing what is being requested.
    pub payload: PendingInteractionPayload,
}

/// Typed payload carried by a pending interaction envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingInteractionPayload {
    /// One or more tool calls awaiting approval.
    Approval(ApprovalInteractionInfo),
    /// A model-originated choice prompt awaiting user selection.
    Choice(ChoiceInteractionInfo),
}

// ---------------------------------------------------------------------------
// Approval interaction payload
// ---------------------------------------------------------------------------

/// Information about an approval interaction containing one or more tool calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalInteractionInfo {
    /// The tool calls that require approval.
    pub calls: Vec<ApprovalCallInfo>,
}

/// Information about a single tool call awaiting approval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalCallInfo {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
}

// ---------------------------------------------------------------------------
// Choice interaction payload
// ---------------------------------------------------------------------------

/// Information about a model-originated choice prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceInteractionInfo {
    /// The prompt text presented to the user.
    pub prompt: String,
    /// Whether the user may select one or multiple items.
    pub selection_mode: ChoiceSelectionMode,
    /// The available options.
    pub items: Vec<ChoiceItem>,
}

/// Whether a choice prompt allows single or multiple selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceSelectionMode {
    Single,
    Multiple,
}

/// A single selectable option in a choice prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceItem {
    /// Stable identifier for this option.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Optional longer description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
// Interaction resolution
// ---------------------------------------------------------------------------

/// A typed resolution for a pending interaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InteractionResolution {
    /// Resolution for an approval interaction.
    Approval(ApprovalInteractionResolution),
    /// Resolution for a choice interaction.
    Choice(ChoiceInteractionResolution),
}

/// Resolution for an approval interaction, containing per-call decisions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalInteractionResolution {
    /// Per-call approval decisions.
    pub decisions: Vec<ApprovalDecision>,
}

/// A single approval decision for one tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalDecision {
    /// The tool call this decision applies to.
    pub call_id: String,
    /// The verdict for this call.
    pub verdict: ApprovalVerdict,
}

/// Possible verdicts for an approval decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalVerdict {
    AllowOnce,
    Deny,
    Cancelled,
}

/// Resolution for a choice interaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ChoiceInteractionResolution {
    /// The user selected one or more options.
    Submitted { selected_ids: Vec<String> },
    /// The user cancelled the choice.
    Cancelled,
}

// ---------------------------------------------------------------------------
// Canonical model-facing choice-resolution record
// ---------------------------------------------------------------------------

/// The canonical structured record injected into continuation context when a
/// choice interaction is resolved. This is what the model sees after the user
/// responds to a choice prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceResolutionRecord {
    /// Always `choice_resolution`.
    pub kind: String,
    /// The interaction that was resolved.
    pub interaction_id: String,
    /// Whether the user submitted a selection or cancelled.
    pub status: ChoiceResolutionStatus,
    /// The original prompt text.
    pub prompt: String,
    /// The selection mode of the original prompt.
    pub selection_mode: ChoiceSelectionMode,
    /// The selected items (empty for cancelled).
    pub selected_items: Vec<ChoiceResolutionItem>,
}

/// Status of a resolved choice interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceResolutionStatus {
    Submitted,
    Cancelled,
}

/// A single selected item in a choice-resolution record, carrying both the
/// stable ID and the human-readable label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceResolutionItem {
    pub id: String,
    pub label: String,
}

impl ChoiceResolutionRecord {
    /// Create a submitted choice-resolution record.
    pub fn submitted(
        interaction_id: String,
        prompt: String,
        selection_mode: ChoiceSelectionMode,
        selected_items: Vec<ChoiceResolutionItem>,
    ) -> Self {
        Self {
            kind: "choice_resolution".to_string(),
            interaction_id,
            status: ChoiceResolutionStatus::Submitted,
            prompt,
            selection_mode,
            selected_items,
        }
    }

    /// Create a cancelled choice-resolution record.
    pub fn cancelled(
        interaction_id: String,
        prompt: String,
        selection_mode: ChoiceSelectionMode,
    ) -> Self {
        Self {
            kind: "choice_resolution".to_string(),
            interaction_id,
            status: ChoiceResolutionStatus::Cancelled,
            prompt,
            selection_mode,
            selected_items: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy compatibility aliases
// ---------------------------------------------------------------------------

/// Information about a single tool call awaiting approval.
///
/// **Deprecated**: Use [`ApprovalCallInfo`] instead. This type is retained for
/// backward compatibility with existing callers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingCallInfo {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
}

impl From<&ApprovalCallInfo> for PendingCallInfo {
    fn from(info: &ApprovalCallInfo) -> Self {
        Self {
            call_id: info.call_id.clone(),
            tool_name: info.tool_name.clone(),
            arguments: info.arguments.clone(),
        }
    }
}
