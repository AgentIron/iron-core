use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptInput {
    pub script: String,
    pub input: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptOutput {
    pub status: ScriptExecStatus,
    pub result: Option<Value>,
    pub error: Option<ScriptError>,
    pub child_outcomes: Vec<ChildCallOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptExecStatus {
    Completed,
    CompletedWithFailures,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptError {
    pub kind: ScriptErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptErrorKind {
    Timeout,
    SourceTooLarge,
    ResultTooLarge,
    Runtime,
    ChildCallLimitExceeded,
    Cancelled,
    SandboxViolation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildCallOutcome {
    pub call_id: String,
    pub tool_name: String,
    pub status: ChildCallStatus,
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildCallStatus {
    Completed,
    Failed,
    Denied,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptResult {
    pub output: Value,
}

impl ScriptOutput {
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

    pub fn failed(error: ScriptError) -> Self {
        Self {
            status: ScriptExecStatus::Failed,
            result: None,
            error: Some(error),
            child_outcomes: Vec::new(),
        }
    }

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
    pub fn timeout() -> Self {
        Self {
            kind: ScriptErrorKind::Timeout,
            message: "script execution timed out".into(),
        }
    }

    pub fn source_too_large(max: usize) -> Self {
        Self {
            kind: ScriptErrorKind::SourceTooLarge,
            message: format!("script source exceeds maximum size of {} bytes", max),
        }
    }

    pub fn result_too_large(max: usize) -> Self {
        Self {
            kind: ScriptErrorKind::ResultTooLarge,
            message: format!("script result exceeds maximum size of {} bytes", max),
        }
    }

    pub fn child_call_limit(max: usize) -> Self {
        Self {
            kind: ScriptErrorKind::ChildCallLimitExceeded,
            message: format!("script exceeded maximum of {} child tool calls", max),
        }
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self {
            kind: ScriptErrorKind::Runtime,
            message: message.into(),
        }
    }

    pub fn sandbox_violation(message: impl Into<String>) -> Self {
        Self {
            kind: ScriptErrorKind::SandboxViolation,
            message: message.into(),
        }
    }
}
