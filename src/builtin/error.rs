use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinToolError {
    pub code: BuiltinErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinErrorCode {
    PathOutOfScope,
    PathConflict,
    PathNotFound,
    BinaryContent,
    EditMismatch,
    EditAmbiguous,
    InvalidInput,
    InvalidUrl,
    NetworkDenied,
    FetchFailed,
    Timeout,
    ShellNotAvailable,
    IoError,
    ConfigError,
}

impl BuiltinErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PathOutOfScope => "path_out_of_scope",
            Self::PathConflict => "path_conflict",
            Self::PathNotFound => "path_not_found",
            Self::BinaryContent => "binary_content",
            Self::EditMismatch => "edit_mismatch",
            Self::EditAmbiguous => "edit_ambiguous",
            Self::InvalidInput => "invalid_input",
            Self::InvalidUrl => "invalid_url",
            Self::NetworkDenied => "network_denied",
            Self::FetchFailed => "fetch_failed",
            Self::Timeout => "timeout",
            Self::ShellNotAvailable => "shell_not_available",
            Self::IoError => "io_error",
            Self::ConfigError => "config_error",
        }
    }
}

impl fmt::Display for BuiltinErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl BuiltinToolError {
    pub fn new(code: BuiltinErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn out_of_scope(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::PathOutOfScope, msg)
    }

    pub fn path_conflict(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::PathConflict, msg)
    }

    pub fn path_not_found(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::PathNotFound, msg)
    }

    pub fn binary_content(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::BinaryContent, msg)
    }

    pub fn edit_mismatch(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::EditMismatch, msg)
    }

    pub fn edit_ambiguous(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::EditAmbiguous, msg)
    }

    pub fn invalid_input(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::InvalidInput, msg)
    }

    pub fn invalid_url(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::InvalidUrl, msg)
    }

    pub fn network_denied(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::NetworkDenied, msg)
    }

    pub fn fetch_failed(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::FetchFailed, msg)
    }

    pub fn timeout(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::Timeout, msg)
    }

    pub fn shell_not_available() -> Self {
        Self::new(
            BuiltinErrorCode::ShellNotAvailable,
            "no shell backend is available",
        )
    }

    pub fn io(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::IoError, msg)
    }

    pub fn config(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::ConfigError, msg)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "error": {
                "code": self.code.as_str(),
                "message": self.message,
            }
        })
    }
}

impl std::fmt::Display for BuiltinToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BuiltinToolError {}

impl From<BuiltinToolError> for crate::error::LoopError {
    fn from(e: BuiltinToolError) -> Self {
        Self::tool_execution(e.to_string())
    }
}
