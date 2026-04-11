use crate::durable::DurableSession;
use crate::mcp::server::McpServerHealth;
use crate::mcp::McpServerRegistry;
use crate::tool::{Tool, ToolDefinition, ToolRegistry};
use std::sync::Arc;

/// A tool that wraps an MCP tool for use in the runtime.
///
/// **Deprecated**: `McpTool::execute` is not wired to the runtime-owned MCP
/// connection manager. Use [`SessionToolCatalog`] for MCP tool execution.
#[deprecated(
    since = "0.2.0",
    note = "McpTool::execute is not wired to the runtime MCP connection manager; use SessionToolCatalog instead"
)]
pub struct McpTool {
    definition: ToolDefinition,
    server_id: String,
    tool_name: String,
}

impl McpTool {
    pub fn new(
        server_id: String,
        tool_name: String,
        description: String,
        input_schema: serde_json::Value,
    ) -> Self {
        // Create a namespaced tool name to avoid collisions
        let namespaced_name = format!("mcp_{}_{}", server_id, tool_name);

        Self {
            definition: ToolDefinition::new(
                &namespaced_name,
                format!("[MCP:{}] {}", server_id, description),
                input_schema,
            ),
            server_id,
            tool_name,
        }
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub fn original_tool_name(&self) -> &str {
        &self.tool_name
    }
}

impl Tool for McpTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    fn execute(&self, _call_id: &str, _arguments: serde_json::Value) -> crate::tool::ToolFuture {
        // McpTool is a legacy helper type whose execution is not wired to the
        // runtime-owned MCP connection manager. The canonical MCP execution
        // path is through SessionToolCatalog + McpConnectionManager.
        // Return a clear error so callers cannot mistake this for a real result.
        let tool_name = self.definition.name.clone();
        Box::pin(async move {
            Err(crate::error::LoopError::tool_execution(format!(
                "McpTool::execute is not wired to the runtime MCP connection manager; \
                 use SessionToolCatalog for MCP tool execution (tool: {})",
                tool_name
            )))
        })
    }

    fn requires_approval(&self) -> bool {
        // MCP tools require approval by default
        true
    }
}

/// Legacy helper for computing MCP-visible tools for a session.
/// Public runtime inspection should prefer `SessionToolCatalog` so inspection,
/// prompt construction, and execution all use the same source of truth.
///
/// **Deprecated**: Prefer [`SessionToolCatalog`] which is the single source of
/// truth for prompt construction, execution, and public inspection.
#[deprecated(
    since = "0.2.0",
    note = "Prefer SessionToolCatalog, which is the single source of truth for MCP tool visibility"
)]
pub struct EffectiveToolView {
    local_registry: Arc<ToolRegistry>,
    mcp_registry: Arc<McpServerRegistry>,
}

impl EffectiveToolView {
    pub fn new(local_registry: Arc<ToolRegistry>, mcp_registry: Arc<McpServerRegistry>) -> Self {
        Self {
            local_registry,
            mcp_registry,
        }
    }

    /// Get the effective tool definitions for a session
    pub fn get_tool_definitions(&self, session: &DurableSession) -> Vec<ToolDefinition> {
        let mut definitions = Vec::new();

        // Add local tools
        for tool in self.local_registry.definitions() {
            definitions.push(tool);
        }

        // Add MCP tools for enabled and usable servers
        if let Ok(enabled_servers) = self.get_enabled_and_usable_servers(session) {
            for server_id in enabled_servers {
                if let Some(server) = self.mcp_registry.get_server(&server_id) {
                    if server.health.is_usable() {
                        for tool_info in &server.discovered_tools {
                            let mcp_tool = McpTool::new(
                                server_id.clone(),
                                tool_info.name.clone(),
                                tool_info.description.clone(),
                                tool_info.input_schema.clone(),
                            );
                            definitions.push(mcp_tool.definition());
                        }
                    }
                }
            }
        }

        definitions
    }

    /// Get an MCP tool by its namespaced name
    pub fn get_mcp_tool(&self, namespaced_name: &str) -> Option<McpTool> {
        if let Some((server_id, tool_name)) =
            crate::mcp::session_catalog::SessionToolCatalog::resolve_mcp_tool_name(
                namespaced_name,
                &self.mcp_registry,
            )
        {
            if let Some(server) = self.mcp_registry.get_server(&server_id) {
                for tool_info in &server.discovered_tools {
                    if tool_info.name == tool_name {
                        return Some(McpTool::new(
                            server_id,
                            tool_info.name.clone(),
                            tool_info.description.clone(),
                            tool_info.input_schema.clone(),
                        ));
                    }
                }
            }
        }

        None
    }

    /// Get all MCP tool definitions for a session
    pub fn get_mcp_tool_definitions(&self, session: &DurableSession) -> Vec<ToolDefinition> {
        let mut definitions = Vec::new();

        if let Ok(enabled_servers) = self.get_enabled_and_usable_servers(session) {
            for server_id in enabled_servers {
                if let Some(server) = self.mcp_registry.get_server(&server_id) {
                    if server.health.is_usable() {
                        for tool_info in &server.discovered_tools {
                            let mcp_tool = McpTool::new(
                                server_id.clone(),
                                tool_info.name.clone(),
                                tool_info.description.clone(),
                                tool_info.input_schema.clone(),
                            );
                            definitions.push(mcp_tool.definition());
                        }
                    }
                }
            }
        }

        definitions
    }

    /// Check if a server is enabled for a session and currently usable
    fn get_enabled_and_usable_servers(&self, session: &DurableSession) -> Result<Vec<String>, ()> {
        let mut enabled_servers = Vec::new();

        for server in self.mcp_registry.list_servers() {
            let server_id = &server.config.id;

            // Check session enablement
            match session.is_mcp_server_enabled(server_id) {
                Some(true) => {
                    // Explicitly enabled in session
                    enabled_servers.push(server_id.clone());
                }
                Some(false) => {
                    // Explicitly disabled in session
                    continue;
                }
                None => {
                    // Not set in session - use default from server config
                    if server.config.enabled_by_default {
                        enabled_servers.push(server_id.clone());
                    }
                }
            }
        }

        Ok(enabled_servers)
    }

    /// Get summary of MCP server status for a session
    pub fn get_session_mcp_summary(&self, session: &DurableSession) -> SessionMcpSummary {
        let mut summary = SessionMcpSummary::default();

        for server in self.mcp_registry.list_servers() {
            let server_id = server.config.id.clone();
            let enabled = session
                .is_mcp_server_enabled(&server_id)
                .unwrap_or(server.config.enabled_by_default);
            let usable = server.health.is_usable();

            summary.servers.push(ServerStatus {
                id: server_id,
                label: server.config.label.clone(),
                enabled,
                health: server.health,
                usable: enabled && usable,
                tool_count: if enabled && usable {
                    server.discovered_tools.len()
                } else {
                    0
                },
            });
        }

        summary
    }
}

/// Summary of MCP server status for a session
#[derive(Debug, Default)]
pub struct SessionMcpSummary {
    pub servers: Vec<ServerStatus>,
}

/// Status of a single MCP server for a session
#[derive(Debug)]
pub struct ServerStatus {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub health: McpServerHealth,
    pub usable: bool,
    pub tool_count: usize,
}
