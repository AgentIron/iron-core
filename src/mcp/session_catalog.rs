use crate::durable::DurableSession;
use crate::mcp::server::{McpServerHealth, McpServerRegistry};
use crate::mcp::McpConnectionManager;
use crate::plugin::effective_tools::compute_tool_availability;
use crate::plugin::effective_tools::PluginToolSummary;
use crate::plugin::effective_tools::UnavailableReason;
use crate::plugin::registry::PluginRegistry;
use crate::plugin::wasm_host::WasmHost;
use crate::tool::{ToolDefinition, ToolFuture, ToolRegistry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Information about a tool in the session catalog.
#[derive(Clone)]
enum ToolInfo {
    Local {
        name: String,
    },
    Mcp {
        server_id: String,
        tool_name: String,
        definition: ToolDefinition,
    },
    Plugin {
        plugin_id: String,
        tool_name: String,
        definition: ToolDefinition,
    },
}

/// A session-scoped tool catalog that provides both provider-facing definitions
/// and executable handles for all visible tools (local + MCP + plugins).
#[derive(Clone)]
pub struct SessionToolCatalog {
    /// Local tool registry
    local_registry: Arc<ToolRegistry>,
    /// MCP server registry
    mcp_registry: Arc<McpServerRegistry>,
    /// Plugin registry
    plugin_registry: Arc<PluginRegistry>,
    /// WASM host for plugin execution
    wasm_host: Arc<WasmHost>,
    /// Connection manager for executing MCP tools
    connection_manager: Arc<McpConnectionManager>,
    /// Cache of tool definitions for this session
    definitions: Vec<ToolDefinition>,
    /// Map from tool name to tool info for lookup
    tool_map: Arc<HashMap<String, ToolInfo>>,
}

/// Summary of an MCP server's status for a session.
#[derive(Debug)]
pub struct McpServerSummary {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub health: McpServerHealth,
    pub usable: bool,
    pub tool_count: usize,
}

impl SessionToolCatalog {
    fn is_mcp_server_enabled_for_session(session: &DurableSession, server_id: &str) -> bool {
        session.is_mcp_server_enabled(server_id).unwrap_or(false)
    }

    /// Resolve an MCP provider-visible tool name using the longest matching
    /// registered server id. This avoids ambiguous first-underscore parsing.
    pub(crate) fn resolve_mcp_tool_name(
        name: &str,
        registry: &McpServerRegistry,
    ) -> Option<(String, String)> {
        let without_prefix = name.strip_prefix("mcp_")?;
        registry
            .list_servers()
            .into_iter()
            .filter_map(|server| {
                let prefix = format!("{}_", server.config.id);
                without_prefix
                    .strip_prefix(&prefix)
                    .map(|tool_name| (server.config.id, tool_name.to_string()))
            })
            .max_by_key(|(server_id, _)| server_id.len())
    }

    /// Resolve a plugin tool name using the longest matching registered plugin id.
    pub(crate) fn resolve_plugin_tool_name(
        name: &str,
        registry: &PluginRegistry,
    ) -> Option<(String, String)> {
        let without_prefix = name.strip_prefix("plugin_")?;
        registry
            .list()
            .into_iter()
            .filter_map(|plugin| {
                let prefix = format!("{}_", plugin.config.id);
                without_prefix
                    .strip_prefix(&prefix)
                    .map(|tool_name| (plugin.config.id, tool_name.to_string()))
            })
            .max_by_key(|(plugin_id, _)| plugin_id.len())
    }

    /// Create a new session tool catalog for the given session.
    pub fn new(
        local_registry: Arc<ToolRegistry>,
        mcp_registry: Arc<McpServerRegistry>,
        plugin_registry: Arc<PluginRegistry>,
        wasm_host: Arc<WasmHost>,
        connection_manager: Arc<McpConnectionManager>,
        session: &DurableSession,
    ) -> Self {
        let mut definitions = Vec::new();
        let mut tool_map = HashMap::new();

        // Add local tools
        for def in local_registry.definitions() {
            let name = def.name.clone();
            tool_map.insert(name.clone(), ToolInfo::Local { name: name.clone() });
            definitions.push(def);
        }

        // Add MCP tools for enabled and usable servers
        for server in mcp_registry.list_servers() {
            let server_id = server.config.id.clone();

            // Check if server is enabled for this session
            let enabled = Self::is_mcp_server_enabled_for_session(session, &server_id);

            if !enabled {
                continue;
            }

            // Check if server is usable (connected and healthy)
            if !server.health.is_usable() {
                continue;
            }

            // Add discovered tools from this server
            for tool_info in &server.discovered_tools {
                let namespaced_name = format!("mcp_{}_{}", server_id, tool_info.name);
                let description = format!("[MCP:{}] {}", server_id, tool_info.description);

                let definition = ToolDefinition::new(
                    &namespaced_name,
                    &description,
                    tool_info.input_schema.clone(),
                )
                .with_approval(true); // MCP tools require approval by default

                tool_map.insert(
                    namespaced_name.clone(),
                    ToolInfo::Mcp {
                        server_id: server_id.clone(),
                        tool_name: tool_info.name.clone(),
                        definition: definition.clone(),
                    },
                );

                definitions.push(definition);
            }
        }

        // Add plugin tools for enabled, healthy, and auth-satisfied plugins
        for plugin in plugin_registry.list() {
            let plugin_id = plugin.config.id.clone();

            // Check if plugin is enabled for this session.
            // Explicit session state is authoritative; if absent the plugin
            // is considered not enabled (runtime defaults should have been
            // materialised at session creation time).
            let enabled = session.is_plugin_enabled(&plugin_id).unwrap_or(false);

            if !enabled {
                continue;
            }

            // Check if plugin is healthy
            if !plugin.health.is_healthy() {
                continue;
            }

            // Get the manifest to check tools
            let manifest = match &plugin.manifest {
                Some(m) => m,
                None => continue,
            };

            // Add each tool that is available (considering auth requirements + scopes)
            for tool in &manifest.tools {
                if !compute_tool_availability(&plugin, tool).available {
                    continue;
                }

                let namespaced_name = format!("plugin_{}_{}", plugin_id, tool.name);
                let description = format!("[Plugin:{}] {}", plugin_id, tool.description);

                let definition =
                    ToolDefinition::new(&namespaced_name, &description, tool.input_schema.clone())
                        .with_approval(tool.requires_approval);

                tool_map.insert(
                    namespaced_name.clone(),
                    ToolInfo::Plugin {
                        plugin_id: plugin_id.clone(),
                        tool_name: tool.name.clone(),
                        definition: definition.clone(),
                    },
                );

                definitions.push(definition);
            }
        }

        Self {
            local_registry,
            mcp_registry,
            plugin_registry,
            wasm_host,
            connection_manager,
            definitions,
            tool_map: Arc::new(tool_map),
        }
    }

    /// Get all tool definitions visible to the model for this session.
    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    /// Get a specific tool definition by name.
    pub fn get_definition(&self, name: &str) -> Option<&ToolDefinition> {
        self.definitions.iter().find(|d| d.name == name)
    }

    /// Check if a tool with the given name exists in this session's catalog.
    pub fn contains(&self, name: &str) -> bool {
        self.tool_map.contains_key(name)
    }

    /// Get the number of tools visible in this session.
    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    /// Check if the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }

    /// Check if a tool requires approval before execution.
    pub fn requires_approval(&self, name: &str) -> bool {
        match self.tool_map.get(name) {
            Some(ToolInfo::Local { .. }) => {
                // For local tools, check the registry
                self.local_registry
                    .get(name)
                    .map(|t| t.requires_approval())
                    .unwrap_or(false)
            }
            Some(ToolInfo::Mcp { .. }) => true, // MCP tools require approval by default
            Some(ToolInfo::Plugin { definition, .. }) => definition.requires_approval,
            None => false,
        }
    }

    /// Execute a tool by name with the given arguments.
    ///
    /// This is the **canonical execution path** for all tool calls, whether
    /// they originate from the model, embedded Python child-tool calls, or
    /// any other source.  Plugin-backed tools follow the same checks as
    /// model-issued calls: session enablement, health, auth-gating, and
    /// WASM host execution via Extism.
    ///
    /// Returns a future that resolves to the tool result.
    pub fn execute(
        &self,
        call_id: &str,
        name: &str,
        arguments: Value,
        session: &DurableSession,
    ) -> ToolFuture {
        let tool_info = self.tool_map.get(name).cloned();
        let local_registry = Arc::clone(&self.local_registry);
        let connection_manager = Arc::clone(&self.connection_manager);
        let mcp_registry = Arc::clone(&self.mcp_registry);
        let plugin_registry = Arc::clone(&self.plugin_registry);
        let wasm_host = Arc::clone(&self.wasm_host);
        let call_id_owned = call_id.to_owned();

        match tool_info {
            Some(ToolInfo::Local { name: tool_name }) => {
                // Execute local tool
                Box::pin(async move {
                    if let Some(tool) = local_registry.get(&tool_name) {
                        tool.execute(&call_id_owned, arguments).await
                    } else {
                        Err(crate::error::LoopError::tool_execution(format!(
                            "Tool '{}' is registered in the session catalog but could not be found in the tool registry. This may indicate an internal error.",
                            tool_name
                        )))
                    }
                })
            }
            Some(ToolInfo::Mcp {
                server_id,
                tool_name,
                ..
            }) => {
                // Execute MCP tool
                Box::pin(async move {
                    connection_manager
                        .call_tool(&server_id, &tool_name, arguments)
                        .await
                        .map_err(|e| {
                            crate::error::LoopError::tool_execution(format!(
                                "MCP tool call failed: {}",
                                e
                            ))
                        })
                })
            }
            Some(ToolInfo::Plugin {
                plugin_id,
                tool_name,
                ..
            }) => {
                // Execute plugin tool via WASM host
                Box::pin(async move {
                    // Check plugin state before executing
                    if let Some(plugin) = plugin_registry.get(&plugin_id) {
                        // Check health
                        if !plugin.health.is_healthy() {
                            return Err(crate::error::LoopError::tool_execution(format!(
                                "Plugin '{}' is not healthy (status: {:?}). Cannot execute tool '{}'.",
                                plugin_id, plugin.health, tool_name
                            )));
                        }

                        // Check auth requirements for this specific tool
                        if let Some(manifest) = &plugin.manifest {
                            if let Some(tool) = manifest.tools.iter().find(|t| t.name == tool_name)
                            {
                                if let Some(auth_reqs) = &tool.auth_requirements {
                                    if !auth_reqs.available_unauthenticated
                                        && !plugin.auth_state.is_authenticated()
                                    {
                                        return Err(crate::error::LoopError::tool_execution(format!(
                                            "Tool '{}' from plugin '{}' requires authentication. Please authenticate first.",
                                            tool_name, plugin_id
                                        )));
                                    }
                                }
                            }
                        }

                        // Execute via WASM host
                        wasm_host
                            .execute_tool(&plugin_id, &tool_name, arguments)
                            .await
                            .map_err(|e| {
                                crate::error::LoopError::tool_execution(format!(
                                    "Plugin tool call failed: {}",
                                    e
                                ))
                            })
                    } else {
                        Err(crate::error::LoopError::tool_execution(format!(
                            "Plugin '{}' not found in registry.",
                            plugin_id
                        )))
                    }
                })
            }
            None => {
                let name_owned = name.to_owned();
                // Clone the session's enablement states for use in the async block
                let session_mcp_enablement: HashMap<String, bool> = session
                    .mcp_server_enablement
                    .iter()
                    .map(|(k, v)| (k.clone(), *v))
                    .collect();
                let session_plugin_enablement: HashMap<String, bool> = session
                    .plugin_enablement
                    .list_all()
                    .iter()
                    .map(|(k, v)| (k.clone(), *v))
                    .collect();
                Box::pin(async move {
                    // Check if this looks like an MCP tool
                    if let Some((server_id, tool_name)) =
                        Self::resolve_mcp_tool_name(&name_owned, &mcp_registry)
                    {
                        // Check if the server exists
                        if let Some(server) = mcp_registry.get_server(&server_id) {
                            // Check session enablement state FIRST using the captured enablement
                            let is_enabled = session_mcp_enablement
                                .get(&server_id)
                                .copied()
                                .unwrap_or(false);

                            if !is_enabled {
                                return Err(crate::error::LoopError::tool_execution(format!(
                                    "MCP server '{}' is disabled for this session. Enable it to use tool '{}'.",
                                    server_id, tool_name
                                )));
                            }

                            // Server is enabled - now check health
                            if !server.health.is_usable() {
                                return Err(crate::error::LoopError::tool_execution(format!(
                                    "MCP server '{}' is not healthy (status: {:?}). Cannot execute tool '{}'.",
                                    server_id, server.health, tool_name
                                )));
                            }

                            // Server is enabled and healthy - now check if the tool exists
                            let tool_exists =
                                server.discovered_tools.iter().any(|t| t.name == tool_name);

                            if !tool_exists {
                                // Tool doesn't exist on this server
                                let available_tools: Vec<_> = server
                                    .discovered_tools
                                    .iter()
                                    .map(|t| t.name.clone())
                                    .collect();
                                return Err(crate::error::LoopError::tool_execution(format!(
                                    "Tool '{}' not found on MCP server '{}'. Available tools: {}",
                                    tool_name,
                                    server_id,
                                    if available_tools.is_empty() {
                                        "(none)".to_string()
                                    } else {
                                        available_tools.join(", ")
                                    }
                                )));
                            }

                            // Shouldn't reach here if server is enabled, healthy, and tool exists
                            // but tool not in catalog - this is an internal error
                            Err(crate::error::LoopError::tool_execution(format!(
                                "Tool '{}' from MCP server '{}' is unexpectedly unavailable. This may indicate an internal consistency error.",
                                tool_name, server_id
                            )))
                        } else {
                            // Server doesn't exist
                            Err(crate::error::LoopError::tool_execution(format!(
                                "Tool '{}' references MCP server '{}' which is not configured.",
                                name_owned, server_id
                            )))
                        }
                    } else if name_owned.starts_with("mcp_") {
                        Err(crate::error::LoopError::tool_execution(format!(
                            "Tool '{}' references an unknown or ambiguous MCP server identifier.",
                            name_owned
                        )))
                    } else if let Some((plugin_id, tool_name)) =
                        Self::resolve_plugin_tool_name(&name_owned, &plugin_registry)
                    {
                        // Check if the plugin exists
                        if let Some(plugin) = plugin_registry.get(&plugin_id) {
                            // Check session enablement state FIRST.
                            // Absent explicit state means not enabled (runtime
                            // defaults should have been materialised at session
                            // creation).
                            let is_enabled = session_plugin_enablement
                                .get(&plugin_id)
                                .copied()
                                .unwrap_or(false);

                            if !is_enabled {
                                return Err(crate::error::LoopError::tool_execution(format!(
                                    "Plugin '{}' is disabled for this session. Enable it to use tool '{}'.",
                                    plugin_id, tool_name
                                )));
                            }

                            // Plugin is enabled - now check health
                            if !plugin.health.is_healthy() {
                                return Err(crate::error::LoopError::tool_execution(format!(
                                    "Plugin '{}' is not healthy (status: {:?}). Cannot execute tool '{}'.",
                                    plugin_id, plugin.health, tool_name
                                )));
                            }

                            // Check auth requirements
                            if let Some(manifest) = &plugin.manifest {
                                if let Some(tool) =
                                    manifest.tools.iter().find(|t| t.name == tool_name)
                                {
                                    if let Some(auth_reqs) = &tool.auth_requirements {
                                        if !auth_reqs.available_unauthenticated
                                            && !plugin.auth_state.is_authenticated()
                                        {
                                            return Err(crate::error::LoopError::tool_execution(format!(
                                                "Tool '{}' from plugin '{}' requires authentication. Please authenticate first.",
                                                tool_name, plugin_id
                                            )));
                                        }
                                    }
                                } else {
                                    // Tool doesn't exist in this plugin
                                    let available_tools: Vec<_> =
                                        manifest.tools.iter().map(|t| t.name.clone()).collect();
                                    return Err(crate::error::LoopError::tool_execution(format!(
                                        "Tool '{}' not found in plugin '{}'. Available tools: {}",
                                        tool_name,
                                        plugin_id,
                                        if available_tools.is_empty() {
                                            "(none)".to_string()
                                        } else {
                                            available_tools.join(", ")
                                        }
                                    )));
                                }
                            } else {
                                return Err(crate::error::LoopError::tool_execution(format!(
                                    "Plugin '{}' has no manifest loaded. Cannot execute tool '{}'.",
                                    plugin_id, tool_name
                                )));
                            }

                            // Shouldn't reach here if plugin is enabled, healthy, auth satisfied, and tool exists
                            Err(crate::error::LoopError::tool_execution(format!(
                                "Tool '{}' from plugin '{}' is unexpectedly unavailable. This may indicate an internal consistency error.",
                                tool_name, plugin_id
                            )))
                        } else {
                            // Plugin doesn't exist
                            Err(crate::error::LoopError::tool_execution(format!(
                                "Tool '{}' references plugin '{}' which is not installed.",
                                name_owned, plugin_id
                            )))
                        }
                    } else if name_owned.starts_with("plugin_") {
                        Err(crate::error::LoopError::tool_execution(format!(
                            "Tool '{}' references an unknown or ambiguous plugin identifier.",
                            name_owned
                        )))
                    } else {
                        // Not an MCP tool, not a plugin tool, and not in local registry
                        Err(crate::error::LoopError::tool_execution(format!(
                            "Tool '{}' is not available. It may not exist or may not be enabled for this session.",
                            name_owned
                        )))
                    }
                })
            }
        }
    }

    /// Get provider-facing tool definitions for building inference requests.
    pub fn provider_definitions(&self) -> Vec<iron_providers::ToolDefinition> {
        self.definitions
            .iter()
            .map(|d| d.to_provider_definition())
            .collect()
    }

    /// Get a summary of MCP server status for this session.
    pub fn mcp_server_summary(&self, session: &DurableSession) -> Vec<McpServerSummary> {
        let mut summaries = Vec::new();

        for server in self.mcp_registry.list_servers() {
            let server_id = server.config.id.clone();
            let enabled = Self::is_mcp_server_enabled_for_session(session, &server_id);
            let usable = server.health.is_usable();

            summaries.push(McpServerSummary {
                id: server_id.clone(),
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

        summaries
    }

    /// Get a summary of plugin status for this session.
    pub fn plugin_summary(&self, session: &DurableSession) -> Vec<PluginToolSummary> {
        let mut summaries = Vec::new();

        for plugin in self.plugin_registry.list() {
            let plugin_id = plugin.config.id.clone();
            // Session enablement is authoritative; absent means not enabled.
            let enabled = session.is_plugin_enabled(&plugin_id).unwrap_or(false);
            let healthy = plugin.health.is_healthy();
            let usable = enabled && healthy;

            // Count available tools using canonical computation
            let available_tool_count = if usable {
                plugin
                    .manifest
                    .as_ref()
                    .map(|m| {
                        m.tools
                            .iter()
                            .filter(|t| compute_tool_availability(&plugin, t).available)
                            .count()
                    })
                    .unwrap_or(0)
            } else {
                0
            };

            summaries.push(PluginToolSummary {
                id: plugin_id,
                enabled,
                healthy,
                usable,
                tool_count: available_tool_count,
                requires_auth: plugin.requires_auth(),
                authenticated: plugin.auth_state.is_authenticated(),
            });
        }

        summaries
    }

    /// Produce unified diagnostics for every tool visible (or potentially
    /// visible) to the session.
    ///
    /// Tools currently in the catalog are reported as `available: true`.
    /// Plugin tools that are *not* available (disabled, unhealthy, or
    /// auth-gated) are also included so that callers can understand *why*
    /// a tool is absent.
    pub fn inspect_tools(&self, session: &DurableSession) -> Vec<ToolDiagnostic> {
        let mut diagnostics = Vec::new();

        // --- Phase 1: tools already in the catalog (available by construction) ---

        for (name, info) in self.tool_map.iter() {
            let (source, description) = match info {
                ToolInfo::Local { .. } => (ToolSource::Local, String::new()),
                ToolInfo::Mcp {
                    server_id,
                    definition,
                    ..
                } => (
                    ToolSource::Mcp {
                        server_id: server_id.clone(),
                    },
                    definition.description.clone(),
                ),
                ToolInfo::Plugin {
                    plugin_id,
                    definition,
                    ..
                } => (
                    ToolSource::Plugin {
                        plugin_id: plugin_id.clone(),
                    },
                    definition.description.clone(),
                ),
            };

            // For local tools, pull the description from definitions list.
            let description = if matches!(info, ToolInfo::Local { .. }) {
                self.definitions
                    .iter()
                    .find(|d| d.name == *name)
                    .map(|d| d.description.clone())
                    .unwrap_or_default()
            } else {
                description
            };

            diagnostics.push(ToolDiagnostic {
                name: name.clone(),
                source,
                available: true,
                unavailable_reason: None,
                requires_approval: self.requires_approval(name),
                description,
            });
        }

        // --- Phase 2: plugin tools NOT in the catalog (unavailable) ---

        // Collect namespaced names already emitted so we don't duplicate.
        let emitted: std::collections::HashSet<String> =
            diagnostics.iter().map(|d| d.name.clone()).collect();

        for plugin in self.plugin_registry.list() {
            let plugin_id = &plugin.config.id;

            // Session enablement check.
            let enabled = session.is_plugin_enabled(plugin_id).unwrap_or(false);
            if !enabled {
                // Plugin not enabled — all its tools are unavailable with
                // PluginNotEnabled. We only know about them if a manifest
                // exists.
                if let Some(manifest) = &plugin.manifest {
                    for tool in &manifest.tools {
                        let namespaced = format!("plugin_{}_{}", plugin_id, tool.name);
                        if !emitted.contains(namespaced.as_str()) {
                            diagnostics.push(ToolDiagnostic {
                                name: namespaced,
                                source: ToolSource::Plugin {
                                    plugin_id: plugin_id.clone(),
                                },
                                available: false,
                                unavailable_reason: Some(UnavailableReason::PluginNotEnabled),
                                requires_approval: tool.requires_approval,
                                description: tool.description.clone(),
                            });
                        }
                    }
                }
                continue;
            }

            // Plugin is enabled but may not be healthy.
            if !plugin.health.is_healthy() {
                if let Some(manifest) = &plugin.manifest {
                    for tool in &manifest.tools {
                        let namespaced = format!("plugin_{}_{}", plugin_id, tool.name);
                        if !emitted.contains(namespaced.as_str()) {
                            diagnostics.push(ToolDiagnostic {
                                name: namespaced,
                                source: ToolSource::Plugin {
                                    plugin_id: plugin_id.clone(),
                                },
                                available: false,
                                unavailable_reason: Some(UnavailableReason::PluginNotHealthy(
                                    plugin.health,
                                )),
                                requires_approval: tool.requires_approval,
                                description: tool.description.clone(),
                            });
                        }
                    }
                }
                continue;
            }

            // Plugin is enabled + healthy, but individual tools may be
            // auth-gated or scope-missing.
            if let Some(manifest) = &plugin.manifest {
                for tool in &manifest.tools {
                    let namespaced = format!("plugin_{}_{}", plugin_id, tool.name);
                    if emitted.contains(namespaced.as_str()) {
                        continue;
                    }

                    let result = compute_tool_availability(&plugin, tool);
                    diagnostics.push(ToolDiagnostic {
                        name: namespaced,
                        source: ToolSource::Plugin {
                            plugin_id: plugin_id.clone(),
                        },
                        available: result.available,
                        unavailable_reason: result.reason,
                        requires_approval: tool.requires_approval,
                        description: tool.description.clone(),
                    });
                }
            }
        }

        diagnostics
    }
}

/// Origin of a tool in the session catalog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    /// A tool registered directly in the local tool registry.
    Local,
    /// A tool provided by an MCP server.
    Mcp { server_id: String },
    /// A tool provided by a WASM plugin.
    Plugin { plugin_id: String },
}

/// Diagnostic information about a tool's availability in the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDiagnostic {
    /// The tool's namespaced name (e.g. "plugin_my-plugin_my-tool").
    pub name: String,
    /// Where this tool comes from.
    pub source: ToolSource,
    /// Whether this tool is currently available for use.
    pub available: bool,
    /// If not available, the reason why.
    pub unavailable_reason: Option<UnavailableReason>,
    /// Whether this tool requires approval before execution.
    pub requires_approval: bool,
    /// Human-readable description.
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable::SessionId;
    use crate::plugin::auth::CredentialBinding;
    use crate::plugin::config::{PluginConfig, PluginSource};
    use crate::plugin::manifest::{
        ExportedTool, PluginIdentity, PluginManifest, PluginPublisher, PresentationMetadata,
        ToolAuthRequirements,
    };
    use crate::plugin::network::NetworkPolicy;
    use crate::plugin::status::PluginHealth;
    use std::path::PathBuf;
    use std::sync::Arc;

    // ---- Test helpers ----

    fn make_plugin_config(id: &str) -> PluginConfig {
        PluginConfig {
            id: id.to_string(),
            source: PluginSource::LocalPath {
                path: PathBuf::from("/dev/null"),
            },
            enabled_by_default: true,
        }
    }

    fn make_manifest(id: &str, tools: Vec<ExportedTool>) -> PluginManifest {
        PluginManifest {
            identity: PluginIdentity {
                id: id.to_string(),
                name: format!("Plugin {}", id),
                version: "1.0.0".to_string(),
            },
            publisher: PluginPublisher {
                name: "Test".to_string(),
                url: None,
                contact: None,
            },
            presentation: PresentationMetadata {
                description: "Test plugin".to_string(),
                long_description: None,
                icon: None,
                category: None,
                keywords: vec![],
            },
            network_policy: NetworkPolicy::Wildcard,
            auth: None,
            tools,
            api_version: "1.0".to_string(),
        }
    }

    fn make_tool(name: &str, auth: Option<ToolAuthRequirements>) -> ExportedTool {
        ExportedTool {
            name: name.to_string(),
            description: format!("Tool {}", name),
            input_schema: serde_json::json!({"type": "object"}),
            requires_approval: false,
            auth_requirements: auth,
        }
    }

    fn make_approval_tool(name: &str) -> ExportedTool {
        ExportedTool {
            name: name.to_string(),
            description: format!("Approval tool {}", name),
            input_schema: serde_json::json!({"type": "object"}),
            requires_approval: true,
            auth_requirements: None,
        }
    }

    /// Helper: build a SessionToolCatalog with only local + plugin registries
    /// (no MCP).  Uses empty MCP registries.
    fn build_catalog(session: &DurableSession) -> SessionToolCatalog {
        let local = Arc::new(crate::tool::ToolRegistry::new());
        let mcp = Arc::new(crate::mcp::server::McpServerRegistry::new());
        let plugin = Arc::new(crate::plugin::registry::PluginRegistry::new());
        let wasm = Arc::new(crate::plugin::wasm_host::WasmHost::new());
        let conn = Arc::new(crate::mcp::McpConnectionManager::new(
            crate::mcp::server::McpServerRegistry::new(),
        ));

        SessionToolCatalog::new(local, mcp, plugin, wasm, conn, session)
    }

    /// Helper: build a catalog with a pre-configured plugin registry.
    fn build_catalog_with_plugins(
        session: &DurableSession,
        plugin_registry: Arc<crate::plugin::registry::PluginRegistry>,
    ) -> SessionToolCatalog {
        let local = Arc::new(crate::tool::ToolRegistry::new());
        let mcp = Arc::new(crate::mcp::server::McpServerRegistry::new());
        let wasm = Arc::new(crate::plugin::wasm_host::WasmHost::new());
        let conn = Arc::new(crate::mcp::McpConnectionManager::new(
            crate::mcp::server::McpServerRegistry::new(),
        ));

        SessionToolCatalog::new(local, mcp, plugin_registry, wasm, conn, session)
    }

    fn register_healthy_plugin(
        registry: &crate::plugin::registry::PluginRegistry,
        plugin_id: &str,
        tools: Vec<ExportedTool>,
    ) {
        registry.register(make_plugin_config(plugin_id));
        registry.update_health(plugin_id, PluginHealth::Healthy);
        registry.set_manifest(plugin_id, make_manifest(plugin_id, tools));
    }

    // ---- Tests: plugin tool visibility in session catalog ----

    #[test]
    fn catalog_empty_when_no_plugins_registered() {
        let session = DurableSession::new(SessionId::new());
        let catalog = build_catalog(&session);
        assert!(catalog.is_empty());
    }

    #[test]
    fn catalog_includes_enabled_healthy_plugin_tools() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(
            &registry,
            "myplug",
            vec![make_tool("read", None), make_tool("write", None)],
        );

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("myplug", true);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        assert_eq!(catalog.len(), 2);

        let names: Vec<&str> = catalog
            .definitions()
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        assert!(names.contains(&"plugin_myplug_read"));
        assert!(names.contains(&"plugin_myplug_write"));
    }

    #[test]
    fn catalog_excludes_disabled_plugin_tools() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(&registry, "offplug", vec![make_tool("x", None)]);

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("offplug", false);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        assert!(
            catalog.is_empty(),
            "disabled plugin should contribute no tools"
        );
    }

    #[test]
    fn catalog_excludes_unhealthy_plugin_tools_even_when_enabled() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        registry.register(make_plugin_config("sick"));
        // Not promoted to Healthy — stays Configured
        registry.set_manifest("sick", make_manifest("sick", vec![make_tool("a", None)]));

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("sick", true);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        assert!(
            catalog.is_empty(),
            "unhealthy plugin should contribute no tools"
        );
    }

    #[test]
    fn catalog_excludes_plugin_without_manifest() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        registry.register(make_plugin_config("nomanifest"));
        registry.update_health("nomanifest", PluginHealth::Healthy);
        // No manifest set.

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("nomanifest", true);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        assert!(catalog.is_empty());
    }

    #[test]
    fn catalog_excludes_auth_gated_tools_when_not_authenticated() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(
            &registry,
            "authplug",
            vec![
                make_tool(
                    "free",
                    Some(ToolAuthRequirements {
                        scopes: vec![],
                        available_unauthenticated: true,
                    }),
                ),
                make_tool(
                    "gated",
                    Some(ToolAuthRequirements {
                        scopes: vec![],
                        available_unauthenticated: false,
                    }),
                ),
            ],
        );

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("authplug", true);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        assert_eq!(catalog.len(), 1, "only the free tool should appear");

        let name = &catalog.definitions()[0].name;
        assert_eq!(name, "plugin_authplug_free");
    }

    #[test]
    fn catalog_includes_auth_gated_tools_when_authenticated() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(
            &registry,
            "authplug2",
            vec![
                make_tool(
                    "free",
                    Some(ToolAuthRequirements {
                        scopes: vec![],
                        available_unauthenticated: true,
                    }),
                ),
                make_tool(
                    "gated",
                    Some(ToolAuthRequirements {
                        scopes: vec![],
                        available_unauthenticated: false,
                    }),
                ),
            ],
        );
        registry.set_credentials(
            "authplug2",
            CredentialBinding {
                plugin_id: "authplug2".to_string(),
                provider: "test".to_string(),
                access_token: "tok".to_string(),
                refresh_token: None,
                expires_at: None,
                scopes: vec![],
            },
        );

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("authplug2", true);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        assert_eq!(catalog.len(), 2, "both tools should appear after auth");
    }

    #[test]
    fn catalog_reflects_approval_from_manifest() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(
            &registry,
            "approval-plug",
            vec![
                make_tool("no_approval", None),
                make_approval_tool("needs_approval"),
            ],
        );

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("approval-plug", true);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        assert!(!catalog.requires_approval("plugin_approval-plug_no_approval"));
        assert!(catalog.requires_approval("plugin_approval-plug_needs_approval"));
    }

    #[test]
    fn catalog_contains_and_get_definition() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(&registry, "lookup", vec![make_tool("find", None)]);

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("lookup", true);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        assert!(catalog.contains("plugin_lookup_find"));
        assert!(!catalog.contains("plugin_lookup_missing"));

        let def = catalog.get_definition("plugin_lookup_find").unwrap();
        assert_eq!(def.name, "plugin_lookup_find");
    }

    // ---- Tests: plugin_summary ----

    #[test]
    fn plugin_summary_reports_enabled_healthy_usable() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(&registry, "sum-plug", vec![make_tool("a", None)]);

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("sum-plug", true);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        let summaries = catalog.plugin_summary(&session);

        let s = summaries.iter().find(|s| s.id == "sum-plug").unwrap();
        assert!(s.enabled);
        assert!(s.healthy);
        assert!(s.usable);
        assert_eq!(s.tool_count, 1);
    }

    #[test]
    fn plugin_summary_reports_disabled_as_not_usable() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(&registry, "dis-plug", vec![make_tool("a", None)]);

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("dis-plug", false);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        let summaries = catalog.plugin_summary(&session);

        let s = summaries.iter().find(|s| s.id == "dis-plug").unwrap();
        assert!(!s.enabled);
        assert!(s.healthy);
        assert!(!s.usable);
        assert_eq!(s.tool_count, 0);
    }

    // ---- Tests: inspect_tools ----

    #[test]
    fn inspect_tools_reports_available_and_unavailable() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(
            &registry,
            "diag",
            vec![
                make_tool("visible", None),
                make_tool(
                    "hidden",
                    Some(ToolAuthRequirements {
                        scopes: vec![],
                        available_unauthenticated: false,
                    }),
                ),
            ],
        );

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("diag", true);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        let diags = catalog.inspect_tools(&session);

        let visible = diags
            .iter()
            .find(|d| d.name == "plugin_diag_visible")
            .unwrap();
        assert!(visible.available);
        assert_eq!(
            visible.source,
            ToolSource::Plugin {
                plugin_id: "diag".to_string()
            }
        );

        let hidden = diags
            .iter()
            .find(|d| d.name == "plugin_diag_hidden")
            .unwrap();
        assert!(!hidden.available);
        assert_eq!(
            hidden.unavailable_reason,
            Some(UnavailableReason::AuthRequired)
        );
    }

    #[test]
    fn inspect_tools_reports_disabled_plugin_as_not_enabled() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(&registry, "off-diag", vec![make_tool("a", None)]);

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("off-diag", false);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        let diags = catalog.inspect_tools(&session);

        let tool = diags
            .iter()
            .find(|d| d.name == "plugin_off-diag_a")
            .unwrap();
        assert!(!tool.available);
        assert_eq!(
            tool.unavailable_reason,
            Some(UnavailableReason::PluginNotEnabled)
        );
    }

    #[test]
    fn inspect_tools_reports_unhealthy_plugin() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        registry.register(make_plugin_config("sick-diag"));
        registry.update_health("sick-diag", PluginHealth::Error);
        registry.set_manifest(
            "sick-diag",
            make_manifest("sick-diag", vec![make_tool("a", None)]),
        );

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("sick-diag", true);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        let diags = catalog.inspect_tools(&session);

        let tool = diags
            .iter()
            .find(|d| d.name == "plugin_sick-diag_a")
            .unwrap();
        assert!(!tool.available);
        assert_eq!(
            tool.unavailable_reason,
            Some(UnavailableReason::PluginNotHealthy(PluginHealth::Error))
        );
    }

    // ---- Tests: resolve_plugin_tool_name ----

    #[test]
    fn resolve_plugin_tool_name_basic() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        registry.register(make_plugin_config("myplug"));

        let result = SessionToolCatalog::resolve_plugin_tool_name("plugin_myplug_read", &registry);
        assert_eq!(result, Some(("myplug".to_string(), "read".to_string())));
    }

    #[test]
    fn resolve_plugin_tool_name_longest_match() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        registry.register(make_plugin_config("my"));
        registry.register(make_plugin_config("my_plugin"));

        // "my_plugin_read" should match "my_plugin" + "read", not "my" + "plugin_read"
        let result =
            SessionToolCatalog::resolve_plugin_tool_name("plugin_my_plugin_read", &registry);
        assert_eq!(result, Some(("my_plugin".to_string(), "read".to_string())));
    }

    #[test]
    fn resolve_plugin_tool_name_no_match() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        assert!(
            SessionToolCatalog::resolve_plugin_tool_name("plugin_nonexistent_tool", &registry)
                .is_none()
        );
    }

    // ---- Tests: execution error paths ----

    #[tokio::test]
    async fn execute_plugin_tool_not_enabled_returns_error() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(&registry, "exec-off", vec![make_tool("x", None)]);

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("exec-off", false);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        let result = catalog
            .execute(
                "call-1",
                "plugin_exec-off_x",
                serde_json::json!({}),
                &session,
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("disabled"),
            "error should mention disabled: {}",
            msg
        );
    }

    #[tokio::test]
    async fn execute_plugin_tool_unhealthy_returns_error() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        registry.register(make_plugin_config("exec-sick"));
        registry.update_health("exec-sick", PluginHealth::Error);
        registry.set_manifest(
            "exec-sick",
            make_manifest("exec-sick", vec![make_tool("x", None)]),
        );

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("exec-sick", true);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        let result = catalog
            .execute(
                "call-2",
                "plugin_exec-sick_x",
                serde_json::json!({}),
                &session,
            )
            .await;

        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("not healthy"),
            "error should mention unhealthy: {}",
            msg
        );
    }

    #[tokio::test]
    async fn execute_plugin_tool_requires_auth_returns_error() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(
            &registry,
            "exec-auth",
            vec![make_tool(
                "gated",
                Some(ToolAuthRequirements {
                    scopes: vec![],
                    available_unauthenticated: false,
                }),
            )],
        );

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("exec-auth", true);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        let result = catalog
            .execute(
                "call-3",
                "plugin_exec-auth_gated",
                serde_json::json!({}),
                &session,
            )
            .await;

        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("requires authentication"),
            "error should mention authentication: {}",
            msg
        );
    }

    #[tokio::test]
    async fn execute_unknown_plugin_tool_returns_error() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        let session = DurableSession::new(SessionId::new());
        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));

        let result = catalog
            .execute(
                "call-4",
                "plugin_nonexistent_x",
                serde_json::json!({}),
                &session,
            )
            .await;

        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("not installed") || msg.contains("unknown"),
            "error should mention plugin status: {}",
            msg
        );
    }

    #[tokio::test]
    async fn execute_completely_unknown_tool_returns_error() {
        let session = DurableSession::new(SessionId::new());
        let catalog = build_catalog(&session);

        let result = catalog
            .execute(
                "call-5",
                "some_random_tool",
                serde_json::json!({}),
                &session,
            )
            .await;

        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("not available"));
    }

    // ---- Tests: multi-plugin isolation ----

    #[test]
    fn multiple_plugins_contribute_tools_independently() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(&registry, "plug-a", vec![make_tool("alpha", None)]);
        register_healthy_plugin(&registry, "plug-b", vec![make_tool("beta", None)]);

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("plug-a", true);
        session.set_plugin_enabled("plug-b", true);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        assert_eq!(catalog.len(), 2);
        assert!(catalog.contains("plugin_plug-a_alpha"));
        assert!(catalog.contains("plugin_plug-b_beta"));
    }

    #[test]
    fn enabling_one_plugin_does_not_affect_another() {
        let registry = crate::plugin::registry::PluginRegistry::new();
        register_healthy_plugin(&registry, "plug-a", vec![make_tool("alpha", None)]);
        register_healthy_plugin(&registry, "plug-b", vec![make_tool("beta", None)]);

        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("plug-a", true);
        session.set_plugin_enabled("plug-b", false);

        let catalog = build_catalog_with_plugins(&session, Arc::new(registry));
        assert_eq!(catalog.len(), 1);
        assert!(catalog.contains("plugin_plug-a_alpha"));
        assert!(!catalog.contains("plugin_plug-b_beta"));
    }
}
