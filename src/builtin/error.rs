//! Stable error codes and messages returned by built-in tools.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Error produced while validating or executing a built-in tool operation.
pub struct BuiltinToolError {
    /// Machine-readable category for the failure.
    pub code: BuiltinErrorCode,
    /// Human-readable details about the failure.
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Stable machine-readable categories for built-in tool failures.
pub enum BuiltinErrorCode {
    /// A requested path falls outside every configured root.
    PathOutOfScope,
    /// A path component conflicts with the requested operation.
    PathConflict,
    /// A requested filesystem path does not exist.
    PathNotFound,
    /// An operation requiring text encountered binary content.
    BinaryContent,
    /// Text selected for replacement was not found.
    EditMismatch,
    /// A single replacement matched more than one location.
    EditAmbiguous,
    /// Tool arguments are missing, malformed, or inconsistent.
    InvalidInput,
    /// A URL is malformed or uses an unsupported scheme.
    InvalidUrl,
    /// Network policy denied the request or resolved address.
    NetworkDenied,
    /// An HTTP request or response operation failed.
    FetchFailed,
    /// An operation exceeded its configured deadline.
    Timeout,
    /// No supported command shell is available.
    ShellNotAvailable,
    /// A filesystem or process I/O operation failed.
    IoError,
    /// Built-in tool configuration violates a required invariant.
    ConfigError,
}

impl BuiltinErrorCode {
    /// Return the stable snake-case representation used in JSON responses.
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
    /// Construct an error from its stable `code` and descriptive `message`.
    pub fn new(code: BuiltinErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Construct a [`BuiltinErrorCode::PathOutOfScope`] error.
    pub fn out_of_scope(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::PathOutOfScope, msg)
    }

    /// Construct a [`BuiltinErrorCode::PathConflict`] error.
    pub fn path_conflict(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::PathConflict, msg)
    }

    /// Construct a [`BuiltinErrorCode::PathNotFound`] error.
    pub fn path_not_found(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::PathNotFound, msg)
    }

    /// Construct a [`BuiltinErrorCode::BinaryContent`] error.
    pub fn binary_content(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::BinaryContent, msg)
    }

    /// Construct a [`BuiltinErrorCode::EditMismatch`] error.
    pub fn edit_mismatch(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::EditMismatch, msg)
    }

    /// Construct a [`BuiltinErrorCode::EditAmbiguous`] error.
    pub fn edit_ambiguous(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::EditAmbiguous, msg)
    }

    /// Construct a [`BuiltinErrorCode::InvalidInput`] error.
    pub fn invalid_input(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::InvalidInput, msg)
    }

    /// Construct a [`BuiltinErrorCode::InvalidUrl`] error.
    pub fn invalid_url(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::InvalidUrl, msg)
    }

    /// Construct a [`BuiltinErrorCode::NetworkDenied`] error.
    pub fn network_denied(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::NetworkDenied, msg)
    }

    /// Construct a [`BuiltinErrorCode::FetchFailed`] error.
    pub fn fetch_failed(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::FetchFailed, msg)
    }

    /// Construct a [`BuiltinErrorCode::Timeout`] error.
    pub fn timeout(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::Timeout, msg)
    }

    /// Construct the standard error for an unavailable shell backend.
    pub fn shell_not_available() -> Self {
        Self::new(
            BuiltinErrorCode::ShellNotAvailable,
            "no shell backend is available",
        )
    }

    /// Construct a [`BuiltinErrorCode::IoError`] error.
    pub fn io(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::IoError, msg)
    }

    /// Construct a [`BuiltinErrorCode::ConfigError`] error.
    pub fn config(msg: impl Into<String>) -> Self {
        Self::new(BuiltinErrorCode::ConfigError, msg)
    }

    /// Serialize this error as an object containing `error.code` and `error.message`.
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

impl From<BuiltinToolError> for crate::error::RuntimeError {
    fn from(e: BuiltinToolError) -> Self {
        Self::tool_execution(e.to_string())
    }
}
