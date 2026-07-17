//! Built-in filesystem, search, shell, and web tools.
//!
//! Tools are configured through [`BuiltinToolConfig`] and enforce its path,
//! network, approval, size, and timeout policies when executed.

/// Configuration and limits shared by built-in tools.
pub mod config;
/// Structured errors produced by built-in tools.
pub mod error;
/// Built-in file reading, writing, and editing tools.
pub mod file_ops;
/// Shared result metadata and JSON response helpers.
pub mod helpers;
/// Approval, network, shell, and path-access policies.
pub mod policy;
/// Registration of enabled built-in tools in a tool registry.
pub mod registration;
/// Compact rendering helpers for model-facing tool output.
pub mod render;
/// Built-in file-name and content search tools.
pub mod search;
/// Built-in Bash and PowerShell execution tools.
pub mod shell;
/// Built-in HTTP content retrieval.
pub mod web;

pub use config::BuiltinToolConfig;
pub use error::{BuiltinErrorCode, BuiltinToolError};
pub use helpers::{BuiltinMeta, BuiltinResult};
pub use policy::{
    ApprovalScope, ApprovalScopeMatch, BuiltinToolPolicy, NetworkPolicy, ShellAvailability,
};
pub use registration::register_builtin_tools;

use crate::tool::Tool;

/// Check if a name corresponds to a builtin tool.
///
/// Returns `true` if the name is a recognized builtin tool name.
pub fn is_builtin_name(name: &str) -> bool {
    matches!(
        name,
        "read"
            | "write"
            | "edit"
            | "multiedit"
            | "glob"
            | "grep"
            | "webfetch"
            | "bash"
            | "powershell"
    )
}

/// Instantiate an enabled built-in tool by name with the given configuration.
///
/// Returns `None` if the name is unknown, disabled, or names a shell that is
/// unavailable according to [`BuiltinToolConfig::shell_availability`].
pub fn instantiate_builtin(name: &str, config: &BuiltinToolConfig) -> Option<Box<dyn Tool>> {
    match name {
        "read" => config
            .is_tool_enabled("read")
            .then(|| Box::new(file_ops::ReadTool::new(config.clone())) as Box<dyn Tool>),
        "write" => config
            .is_tool_enabled("write")
            .then(|| Box::new(file_ops::WriteTool::new(config.clone())) as Box<dyn Tool>),
        "edit" => config
            .is_tool_enabled("edit")
            .then(|| Box::new(file_ops::EditTool::new(config.clone())) as Box<dyn Tool>),
        "multiedit" => config
            .is_tool_enabled("multiedit")
            .then(|| Box::new(file_ops::MultieditTool::new(config.clone())) as Box<dyn Tool>),
        "glob" => config
            .is_tool_enabled("glob")
            .then(|| Box::new(search::GlobTool::new(config.clone())) as Box<dyn Tool>),
        "grep" => config
            .is_tool_enabled("grep")
            .then(|| Box::new(search::GrepTool::new(config.clone())) as Box<dyn Tool>),
        "webfetch" => config
            .is_tool_enabled("webfetch")
            .then(|| Box::new(web::WebFetchTool::new(config.clone())) as Box<dyn Tool>),
        "bash" => {
            if config.is_tool_enabled("bash")
                && matches!(config.shell_availability, policy::ShellAvailability::Bash)
            {
                Some(Box::new(shell::BashTool::new(config.clone())))
            } else {
                None
            }
        }
        "powershell" => {
            if config.is_tool_enabled("powershell")
                && matches!(
                    config.shell_availability,
                    policy::ShellAvailability::PowerShell
                )
            {
                Some(Box::new(shell::PowerShellTool::new(config.clone())))
            } else {
                None
            }
        }
        _ => None,
    }
}
