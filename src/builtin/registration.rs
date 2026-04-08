use super::config::BuiltinToolConfig;
use super::policy::ShellAvailability;
use crate::tool::ToolRegistry;

pub fn register_builtin_tools(registry: &mut ToolRegistry, config: &BuiltinToolConfig) {
    if config.is_tool_enabled("read") {
        registry.register(super::file_ops::ReadTool::new(config.clone()));
    }
    if config.is_tool_enabled("write") {
        registry.register(super::file_ops::WriteTool::new(config.clone()));
    }
    if config.is_tool_enabled("edit") {
        registry.register(super::file_ops::EditTool::new(config.clone()));
    }
    if config.is_tool_enabled("glob") {
        registry.register(super::search::GlobTool::new(config.clone()));
    }
    if config.is_tool_enabled("grep") {
        registry.register(super::search::GrepTool::new(config.clone()));
    }
    if config.is_tool_enabled("webfetch") {
        registry.register(super::web::WebFetchTool::new(config.clone()));
    }

    match config.shell_availability {
        ShellAvailability::Bash => {
            if config.is_tool_enabled("bash") {
                registry.register(super::shell::BashTool::new(config.clone()));
            }
        }
        ShellAvailability::PowerShell => {
            if config.is_tool_enabled("powershell") {
                registry.register(super::shell::PowerShellTool::new(config.clone()));
            }
        }
        ShellAvailability::None => {}
    }
}
