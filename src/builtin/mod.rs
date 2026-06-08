pub mod config;
pub mod error;
pub mod file_ops;
pub mod helpers;
pub mod policy;
pub mod registration;
pub mod render;
pub mod search;
pub mod shell;
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

/// Instantiate a builtin tool by name with the given configuration.
///
/// Returns `None` if the name does not correspond to a builtin tool.
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
