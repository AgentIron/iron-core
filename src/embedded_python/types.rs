//! Serializable request, result, status, and error types for script execution.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Source code and structured input supplied to an embedded-Python run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptInput {
    /// Python source code to execute.
    pub script: String,
    /// JSON value exposed to the script as `input`.
    pub input: Value,
}

/// Terminal output from an embedded-Python run.
///
/// Successful runs set `result`; failed and cancelled runs set `error`.
/// Outcomes from child tool calls are retained when execution reaches a point
/// where they can be reported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptOutput {
    /// Overall terminal status of the script.
    pub status: ScriptExecStatus,
    /// JSON-compatible value produced by the script's last expression.
    pub result: Option<Value>,
    /// Error that terminated execution, if any.
    pub error: Option<ScriptError>,
    /// Outcomes of host tool calls initiated by the script.
    pub child_outcomes: Vec<ChildCallOutcome>,
}

/// Terminal status of an embedded-Python run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptExecStatus {
    /// The script and all child tool calls completed successfully.
    Completed,
    /// The script returned a result, but at least one child tool call did not complete.
    CompletedWithFailures,
    /// Script execution terminated with an error.
    Failed,
    /// Execution was cancelled before producing a result.
    Cancelled,
}

/// Structured error returned by a failed or cancelled script.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptError {
    /// Machine-readable category of the failure.
    pub kind: ScriptErrorKind,
    /// Human-readable failure description.
    pub message: String,
}

/// Categories of embedded-Python execution failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptErrorKind {
    /// Execution exceeded the configured wall-clock duration.
    Timeout,
    /// Python source exceeded the configured byte limit.
    SourceTooLarge,
    /// Serialized JSON output exceeded the configured byte limit.
    ResultTooLarge,
    /// Parsing or executing the Python source failed.
    Runtime,
    /// The script attempted more than the configured number of child tool calls.
    ChildCallLimitExceeded,
    /// The run's cancellation token was set.
    Cancelled,
    /// The script attempted unavailable host OS access.
    SandboxViolation,
}

/// Recorded outcome of one host tool call initiated by a script.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildCallOutcome {
    /// Runtime-generated identifier for the child call.
    pub call_id: String,
    /// Registered tool name passed to the host executor.
    pub tool_name: String,
    /// Terminal status reported by the host executor.
    pub status: ChildCallStatus,
    /// Optional JSON result or structured failure detail from the tool.
    pub result: Option<Value>,
}

/// Terminal status of a child tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildCallStatus {
    /// The tool returned successfully.
    Completed,
    /// The tool returned an execution failure.
    Failed,
    /// Permission or policy denied the tool call.
    Denied,
    /// The child call was cancelled.
    Cancelled,
}

/// JSON output wrapper for consumers that need a result-only value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptResult {
    /// JSON-compatible script output.
    pub output: Value,
}

impl ScriptOutput {
    /// Builds successful output and derives its status from child-call outcomes.
    ///
    /// Any child status other than [`ChildCallStatus::Completed`] produces
    /// [`ScriptExecStatus::CompletedWithFailures`].
    pub fn completed(result: Value, child_outcomes: Vec<ChildCallOutcome>) -> Self {
        let has_failure = child_outcomes
            .iter()
            .any(|o| o.status != ChildCallStatus::Completed);
        let status = if has_failure {
            ScriptExecStatus::CompletedWithFailures
        } else {
            ScriptExecStatus::Completed
        };
        Self {
            status,
            result: Some(result),
            error: None,
            child_outcomes,
        }
    }

    /// Builds failed output with no result or child-call outcomes.
    pub fn failed(error: ScriptError) -> Self {
        Self {
            status: ScriptExecStatus::Failed,
            result: None,
            error: Some(error),
            child_outcomes: Vec::new(),
        }
    }

    /// Builds cancelled output with the standard cancellation error.
    pub fn cancelled() -> Self {
        Self {
            status: ScriptExecStatus::Cancelled,
            result: None,
            error: Some(ScriptError {
                kind: ScriptErrorKind::Cancelled,
                message: "script execution was cancelled".into(),
            }),
            child_outcomes: Vec::new(),
        }
    }
}

impl ScriptError {
    /// Builds the standard execution-timeout error.
    pub fn timeout() -> Self {
        Self {
            kind: ScriptErrorKind::Timeout,
            message: "script execution timed out".into(),
        }
    }

    /// Builds an error reporting the maximum accepted source size in bytes.
    pub fn source_too_large(max: usize) -> Self {
        Self {
            kind: ScriptErrorKind::SourceTooLarge,
            message: format!("script source exceeds maximum size of {} bytes", max),
        }
    }

    /// Builds an error reporting the maximum accepted serialized result size in bytes.
    pub fn result_too_large(max: usize) -> Self {
        Self {
            kind: ScriptErrorKind::ResultTooLarge,
            message: format!("script result exceeds maximum size of {} bytes", max),
        }
    }

    /// Builds an error reporting the maximum number of child tool calls.
    pub fn child_call_limit(max: usize) -> Self {
        Self {
            kind: ScriptErrorKind::ChildCallLimitExceeded,
            message: format!("script exceeded maximum of {} child tool calls", max),
        }
    }

    /// Builds a runtime error with the supplied interpreter message.
    pub fn runtime(message: impl Into<String>) -> Self {
        Self {
            kind: ScriptErrorKind::Runtime,
            message: message.into(),
        }
    }

    /// Builds an error describing an attempted operation outside the sandbox.
    pub fn sandbox_violation(message: impl Into<String>) -> Self {
        Self {
            kind: ScriptErrorKind::SandboxViolation,
            message: message.into(),
        }
    }
}
