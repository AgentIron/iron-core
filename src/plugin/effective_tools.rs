use crate::plugin::auth::AuthState;
use crate::plugin::manifest::{ExportedTool, ToolAuthRequirements};
use crate::plugin::registry::{PluginRegistry, PluginState};
use crate::plugin::session::SessionPluginEnablement;
use crate::plugin::status::PluginHealth;
use crate::tool::{Tool, ToolDefinition, ToolFuture};
use crate::durable::DurableSession;
use serde_json::Value;
use std::sync::Arc;

/// A tool that wraps a plugin-exported tool
pub struct PluginTool {
    definition: ToolDefinition,
    plugin_id: String,
    tool_name: String,
    requires_auth: bool,
}

impl PluginTool {
    pub fn new(
        plugin_id: String,
        tool_name: String,
        description: String,
        input_schema: Value,
        requires_approval: bool,
    ) -> Self {
        let namespaced_name = format!("plugin_{}_{}", plugin_id, tool_name);

        Self {
            definition: ToolDefinition::new(
                &namespaced_name,
                format!("[Plugin:{}] {}", plugin_id, description),
                input_schema,
            )
            .with_approval(requires_approval),
            plugin_id,
            tool_name,
            requires_auth: false,
        }
    }

    pub fn with_auth_requirement(mut self, requires_auth: bool) -> Self {
        self.requires_auth = requires_auth;
        self
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn original_tool_name(&self) -> &str {
        &self.tool_name
    }

    pub fn requires_auth(&self) -> bool {
        self.requires_auth
    }
}

impl Tool for PluginTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    fn execute(&self, _call_id: &str, _arguments: Value) -> ToolFuture {
        // TODO: Execute via WASM host
        let result = serde_json::json!({
            "error": "Plugin tool execution not yet implemented"
        });
        Box::pin(async move { Ok(result) })
    }

    fn requires_approval(&self) -> bool {
        self.definition.requires_approval
    }
}

/// Per-tool availability check result
#[derive(Debug, Clone, PartialEq)]
pub struct ToolAvailability {
    pub tool: ExportedTool,
    pub available: bool,
    pub reason: Option<String>,
}

/// Computes effective tool sets that include plugin-backed tools
pub struct EffectivePluginToolView {
    plugin_registry: Arc<PluginRegistry>,
}

impl EffectivePluginToolView {
    pub fn new(plugin_registry: Arc<PluginRegistry>) -> Self {
        Self { plugin_registry }
    }

    /// Get all available plugin tools for a session
    pub fn get_available_tools(
        &self,
        session: &DurableSession,
        plugin_enablement: &SessionPluginEnablement,
    ) -> Vec<(String, ExportedTool)> {
        let mut available = Vec::new();

        for plugin in self.plugin_registry.list() {
            let plugin_id = &plugin.config.id;

            // Check session enablement
            let enabled = match plugin_enablement.is_enabled(plugin_id) {
                Some(e) => e,
                None => plugin.config.enabled_by_default,
            };

            if !enabled {
                continue;
            }

            // Check plugin health
            if !plugin.health.is_healthy() {
                continue;
            }

            // Get the manifest to check tools
            let manifest = match &plugin.manifest {
                Some(m) => m,
                None => continue,
            };

            // Check each tool's auth requirements
            for tool in &manifest.tools {
                if self.is_tool_available(&plugin, tool) {
                    available.push((plugin_id.clone(), tool.clone()));
                }
            }
        }

        available
    }

    /// Get tool definitions for available plugin tools
    pub fn get_tool_definitions(
        &self,
        session: &DurableSession,
        plugin_enablement: &SessionPluginEnablement,
    ) -> Vec<ToolDefinition> {
        let available = self.get_available_tools(session, plugin_enablement);

        available
            .into_iter()
            .map(|(plugin_id, tool)| {
                PluginTool::new(
                    plugin_id,
                    tool.name,
                    tool.description,
                    tool.input_schema,
                    tool.requires_approval,
                )
                .with_auth_requirement(tool.auth_requirements.is_some())
                .definition()
            })
            .collect()
    }

    /// Get a specific plugin tool by its namespaced name
    pub fn get_tool(&self, namespaced_name: &str) -> Option<PluginTool> {
        let prefix = "plugin_";
        if !namespaced_name.starts_with(prefix) {
            return None;
        }

        let rest = &namespaced_name[prefix.len()..];
        if let Some(first_underscore) = rest.find('_') {
            let plugin_id = &rest[..first_underscore];
            let tool_name = &rest[first_underscore + 1..];

            if let Some(plugin) = self.plugin_registry.get(plugin_id) {
                if let Some(manifest) = &plugin.manifest {
                    if let Some(tool) = manifest.tools.iter().find(|t| t.name == tool_name) {
                        if self.is_tool_available(&plugin, tool) {
                            return Some(
                                PluginTool::new(
                                    plugin_id.to_string(),
                                    tool_name.to_string(),
                                    tool.description.clone(),
                                    tool.input_schema.clone(),
                                    tool.requires_approval,
                                )
                                .with_auth_requirement(tool.auth_requirements.is_some()),
                            );
                        }
                    }
                }
            }
        }

        None
    }

    /// Check if a specific tool is available given the plugin state
    fn is_tool_available(&self, plugin: &PluginState, tool: &ExportedTool) -> bool {
        // If tool has no auth requirements, it's available when plugin is healthy
        let tool_auth_reqs = match &tool.auth_requirements {
            Some(reqs) => reqs,
            None => return true,
        };

        // If tool is available unauthenticated, it's available
        if tool_auth_reqs.available_unauthenticated {
            return true;
        }

        // Check if plugin is authenticated
        plugin.auth_state.is_authenticated()
    }

    /// Get summary of plugin tool status for a session
    pub fn get_session_summary(
        &self,
        _session: &DurableSession,
        plugin_enablement: &SessionPluginEnablement,
    ) -> SessionPluginToolSummary {
        let mut summary = SessionPluginToolSummary::default();

        for plugin in self.plugin_registry.list() {
            let plugin_id = plugin.config.id.clone();
            let enabled = match plugin_enablement.is_enabled(&plugin_id) {
                Some(e) => e,
                None => plugin.config.enabled_by_default,
            };

            let healthy = plugin.health.is_healthy();
            let usable = enabled && healthy;

            let tool_count = if usable {
                plugin
                    .manifest
                    .as_ref()
                    .map(|m| {
                        m.tools
                            .iter()
                            .filter(|t| self.is_tool_available(&plugin, t))
                            .count()
                    })
                    .unwrap_or(0)
            } else {
                0
            };

            summary.plugins.push(PluginToolSummary {
                id: plugin_id,
                enabled,
                healthy,
                usable,
                tool_count,
                requires_auth: plugin.requires_auth(),
                authenticated: plugin.auth_state.is_authenticated(),
            });
        }

        summary
    }
}

/// Summary of plugin tool availability for a session
#[derive(Debug, Default)]
pub struct SessionPluginToolSummary {
    pub plugins: Vec<PluginToolSummary>,
}

/// Summary for a single plugin
#[derive(Debug)]
pub struct PluginToolSummary {
    pub id: String,
    pub enabled: bool,
    pub healthy: bool,
    pub usable: bool,
    pub tool_count: usize,
    pub requires_auth: bool,
    pub authenticated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::config::{PluginConfig, PluginSource};
    use crate::plugin::manifest::{
        PluginIdentity, PluginPublisher, PresentationMetadata, PluginManifest,
    };
    use crate::plugin::network::NetworkPolicy;
    use std::path::PathBuf;

    fn create_test_plugin() -> PluginState {
        let mut state = PluginState::new(PluginConfig {
            id: "test-plugin".to_string(),
            source: PluginSource::LocalPath {
                path: PathBuf::from("/dev/null"),
            },
            enabled_by_default: true,
        });

        state.manifest = Some(PluginManifest {
            identity: PluginIdentity {
                id: "com.test.plugin".to_string(),
                name: "Test Plugin".to_string(),
                version: "1.0.0".to_string(),
            },
            publisher: PluginPublisher {
                name: "Test".to_string(),
                url: None,
                contact: None,
            },
            presentation: PresentationMetadata {
                description: "Test".to_string(),
                long_description: None,
                icon: None,
                category: None,
                keywords: vec![],
            },
            network_policy: NetworkPolicy::Wildcard,
            auth: None,
            tools: vec![],
            api_version: "1.0".to_string(),
        });

        state.health = PluginHealth::Healthy;
        state
    }

    #[test]
    fn test_plugin_tool_namespacing() {
        let tool = PluginTool::new(
            "my-plugin".to_string(),
            "my-tool".to_string(),
            "Does something".to_string(),
            serde_json::json!({"type": "object"}),
            false,
        );

        assert_eq!(tool.definition().name, "plugin_my-plugin_my-tool");
    }
}
