use crate::durable::DurableSession;
use crate::plugin::manifest::{ExportedTool, ToolAuthRequirements};
use crate::plugin::registry::{PluginRegistry, PluginState};
use crate::plugin::session::SessionPluginEnablement;
use crate::plugin::status::PluginHealth;
use crate::plugin::wasm_host::WasmHost;
use crate::tool::{Tool, ToolDefinition, ToolFuture};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// A tool that wraps a plugin-exported tool
pub struct PluginTool {
    definition: ToolDefinition,
    plugin_id: String,
    tool_name: String,
    requires_auth: bool,
    wasm_host: Option<Arc<WasmHost>>,
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
            wasm_host: None,
        }
    }

    pub fn with_auth_requirement(mut self, requires_auth: bool) -> Self {
        self.requires_auth = requires_auth;
        self
    }

    pub fn with_wasm_host(mut self, host: Arc<WasmHost>) -> Self {
        self.wasm_host = Some(host);
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

    fn execute(&self, _call_id: &str, arguments: Value) -> ToolFuture {
        match &self.wasm_host {
            Some(host) => {
                let host = host.clone();
                let plugin_id = self.plugin_id.clone();
                let tool_name = self.tool_name.clone();
                Box::pin(async move {
                    host.execute_tool(&plugin_id, &tool_name, arguments)
                        .await
                        .map_err(|e| {
                            crate::error::LoopError::tool_execution(format!(
                                "Plugin tool call failed for plugin_{}_{}: {}",
                                plugin_id, tool_name, e
                            ))
                        })
                })
            }
            None => {
                let result = serde_json::json!({
                    "error": "Plugin tool has no WASM host attached"
                });
                Box::pin(async move { Ok(result) })
            }
        }
    }

    fn requires_approval(&self) -> bool {
        self.definition.requires_approval
    }
}

/// Reason a tool is unavailable.
///
/// Each variant captures enough context for clients and logs to produce
/// actionable messages without needing to re-derive the cause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UnavailableReason {
    /// Plugin is not enabled for the current session.
    PluginNotEnabled,
    /// Plugin has not been installed yet (no artifact loaded).
    PluginNotInstalled,
    /// Plugin manifest has not been loaded.
    ManifestMissing,
    /// Plugin runtime health is not `Healthy`.
    PluginNotHealthy(PluginHealth),
    /// Tool requires authentication but the plugin is not authenticated.
    AuthRequired,
    /// Tool requires specific scopes that are not covered by the granted credentials.
    ScopeMissing {
        required: Vec<String>,
        missing: Vec<String>,
    },
}

/// Result of canonical per-tool availability computation.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolAvailabilityResult {
    pub available: bool,
    pub reason: Option<UnavailableReason>,
}

/// Canonical per-tool availability check.
///
/// This is the **single source of truth** for deciding whether a tool from a
/// plugin is available in the current state.  Every call-site that needs to
/// answer "is this tool usable right now?" must delegate here rather than
/// re-implementing the logic.
///
/// Evaluation order:
/// 1. Plugin health — must be `Healthy`.
/// 2. Tool has no auth requirements → available.
/// 3. Tool is `available_unauthenticated` → available.
/// 4. Plugin must be in `Authenticated` state.
/// 5. Granted scopes must cover required scopes.
pub fn compute_tool_availability(
    plugin: &PluginState,
    tool: &ExportedTool,
) -> ToolAvailabilityResult {
    // 1. Health gate
    if !plugin.health.is_healthy() {
        return ToolAvailabilityResult {
            available: false,
            reason: Some(UnavailableReason::PluginNotHealthy(plugin.health)),
        };
    }

    // 2. No auth requirements → available
    let auth_reqs: &ToolAuthRequirements = match &tool.auth_requirements {
        None => {
            return ToolAvailabilityResult {
                available: true,
                reason: None,
            }
        }
        Some(reqs) => reqs,
    };

    // 3. Explicitly available without authentication
    if auth_reqs.available_unauthenticated {
        return ToolAvailabilityResult {
            available: true,
            reason: None,
        };
    }

    // 4. Plugin must be authenticated
    if !plugin.auth_state.is_authenticated() {
        return ToolAvailabilityResult {
            available: false,
            reason: Some(UnavailableReason::AuthRequired),
        };
    }

    // 5. Scope satisfaction
    let required_scopes = &auth_reqs.scopes;
    if !required_scopes.is_empty() {
        let granted: Vec<&str> = plugin
            .credentials
            .as_ref()
            .map(|c| c.scopes.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();

        let missing: Vec<String> = required_scopes
            .iter()
            .filter(|scope| !granted.contains(&scope.as_str()))
            .cloned()
            .collect();

        if !missing.is_empty() {
            return ToolAvailabilityResult {
                available: false,
                reason: Some(UnavailableReason::ScopeMissing {
                    required: required_scopes.clone(),
                    missing,
                }),
            };
        }
    }

    ToolAvailabilityResult {
        available: true,
        reason: None,
    }
}

/// Computes effective tool sets that include plugin-backed tools
pub struct EffectivePluginToolView {
    plugin_registry: Arc<PluginRegistry>,
    wasm_host: Arc<WasmHost>,
}

impl EffectivePluginToolView {
    pub fn new(plugin_registry: Arc<PluginRegistry>, wasm_host: Arc<WasmHost>) -> Self {
        Self {
            plugin_registry,
            wasm_host,
        }
    }

    /// Get all available plugin tools for a session
    pub fn get_available_tools(
        &self,
        _session: &DurableSession,
        plugin_enablement: &SessionPluginEnablement,
    ) -> Vec<(String, ExportedTool)> {
        let mut available = Vec::new();

        for plugin in self.plugin_registry.list() {
            let plugin_id = &plugin.config.id;

            // Check session enablement.
            // Explicit session state is authoritative; absent means not
            // enabled (runtime defaults are materialised at session creation).
            let enabled = plugin_enablement.is_enabled(plugin_id).unwrap_or_default();

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

    /// Get a specific plugin tool by its namespaced name.
    ///
    /// Uses longest-match resolution against registered plugins so that
    /// plugin IDs containing underscores are handled unambiguously.
    pub fn get_tool(&self, namespaced_name: &str) -> Option<PluginTool> {
        let without_prefix = namespaced_name.strip_prefix("plugin_")?;

        // Longest-match: try each registered plugin prefix and pick the
        // longest match.  This mirrors the MCP longest-match logic in
        // SessionToolCatalog.
        let best: Option<(String, String)> = self
            .plugin_registry
            .list()
            .into_iter()
            .filter_map(|plugin| {
                let prefix = format!("{}_", plugin.config.id);
                without_prefix
                    .strip_prefix(&prefix)
                    .map(|tool_name| (plugin.config.id.clone(), tool_name.to_string()))
            })
            .max_by_key(|(plugin_id, _)| plugin_id.len());

        let (plugin_id, tool_name) = best?;
        let plugin = self.plugin_registry.get(&plugin_id)?;
        let manifest = plugin.manifest.as_ref()?;
        let tool = manifest.tools.iter().find(|t| t.name == tool_name)?;

        if !self.is_tool_available(&plugin, tool) {
            return None;
        }

        Some(
            PluginTool::new(
                plugin_id,
                tool_name,
                tool.description.clone(),
                tool.input_schema.clone(),
                tool.requires_approval,
            )
            .with_auth_requirement(tool.auth_requirements.is_some())
            .with_wasm_host(self.wasm_host.clone()),
        )
    }

    /// Check if a specific tool is available given the plugin state.
    ///
    /// Delegates to the canonical [`compute_tool_availability`] function.
    fn is_tool_available(&self, plugin: &PluginState, tool: &ExportedTool) -> bool {
        compute_tool_availability(plugin, tool).available
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
            let enabled = plugin_enablement.is_enabled(&plugin_id).unwrap_or_default();

            let healthy = plugin.health.is_healthy();
            let usable = enabled && healthy;

            let tool_count = if usable {
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
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SessionPluginToolSummary {
    pub plugins: Vec<PluginToolSummary>,
}

/// Summary for a single plugin
#[derive(Debug, Serialize, Deserialize)]
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
    use crate::durable::SessionId;
    use crate::plugin::auth::{AuthState, CredentialBinding};
    use crate::plugin::config::{PluginConfig, PluginSource};
    use crate::plugin::manifest::{
        PluginIdentity, PluginManifest, PluginPublisher, PresentationMetadata, ToolAuthRequirements,
    };
    use crate::plugin::network::NetworkPolicy;
    use crate::plugin::status::PluginHealth;
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

    fn make_tool(auth: Option<ToolAuthRequirements>) -> ExportedTool {
        ExportedTool {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            requires_approval: false,
            auth_requirements: auth,
        }
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

    #[test]
    fn test_plugin_tool_without_wasm_host_returns_error_json() {
        let tool = PluginTool::new(
            "my-plugin".to_string(),
            "my-tool".to_string(),
            "Does something".to_string(),
            serde_json::json!({"type": "object"}),
            false,
        );

        // Without a wasm_host, execute should return an error JSON result
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(tool.execute("call-1", serde_json::json!({})));
        assert!(result.is_ok());
        let value = result.unwrap();
        assert!(value.get("error").is_some());
    }

    // ---- compute_tool_availability tests ----

    #[test]
    fn test_available_when_no_auth_requirements() {
        let plugin = create_test_plugin();
        let tool = make_tool(None);

        let result = compute_tool_availability(&plugin, &tool);
        assert!(result.available);
        assert!(result.reason.is_none());
    }

    #[test]
    fn test_available_when_unauthenticated_allowed() {
        let plugin = create_test_plugin();
        let tool = make_tool(Some(ToolAuthRequirements {
            scopes: vec![],
            available_unauthenticated: true,
        }));

        let result = compute_tool_availability(&plugin, &tool);
        assert!(result.available);
        assert!(result.reason.is_none());
    }

    #[test]
    fn test_unavailable_when_not_healthy() {
        let mut plugin = create_test_plugin();
        plugin.health = PluginHealth::Error;
        let tool = make_tool(None);

        let result = compute_tool_availability(&plugin, &tool);
        assert!(!result.available);
        assert_eq!(
            result.reason,
            Some(UnavailableReason::PluginNotHealthy(PluginHealth::Error))
        );
    }

    #[test]
    fn test_unavailable_for_all_non_healthy_states() {
        let tool = make_tool(None);
        for health in [
            PluginHealth::Configured,
            PluginHealth::Loading,
            PluginHealth::Error,
            PluginHealth::Disabled,
        ] {
            let mut plugin = create_test_plugin();
            plugin.health = health;
            let result = compute_tool_availability(&plugin, &tool);
            assert!(
                !result.available,
                "Expected unavailable for health {:?}",
                health
            );
            assert!(
                matches!(result.reason, Some(UnavailableReason::PluginNotHealthy(h)) if h == health),
                "Expected PluginNotHealthy({:?}), got {:?}",
                health,
                result.reason
            );
        }
    }

    #[test]
    fn test_unavailable_auth_required_when_not_authenticated() {
        let plugin = create_test_plugin();
        assert_eq!(plugin.auth_state, AuthState::Unauthenticated);

        let tool = make_tool(Some(ToolAuthRequirements {
            scopes: vec![],
            available_unauthenticated: false,
        }));

        let result = compute_tool_availability(&plugin, &tool);
        assert!(!result.available);
        assert_eq!(result.reason, Some(UnavailableReason::AuthRequired));
    }

    #[test]
    fn test_available_when_authenticated_and_no_scopes_required() {
        let mut plugin = create_test_plugin();
        plugin.auth_state = AuthState::Authenticated;

        let tool = make_tool(Some(ToolAuthRequirements {
            scopes: vec![],
            available_unauthenticated: false,
        }));

        let result = compute_tool_availability(&plugin, &tool);
        assert!(result.available);
        assert!(result.reason.is_none());
    }

    #[test]
    fn test_available_when_scopes_granted() {
        let mut plugin = create_test_plugin();
        plugin.auth_state = AuthState::Authenticated;
        plugin.credentials = Some(CredentialBinding {
            plugin_id: "test-plugin".to_string(),
            provider: "github".to_string(),
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: None,
            scopes: vec!["repo".to_string(), "read:org".to_string()],
        });

        let tool = make_tool(Some(ToolAuthRequirements {
            scopes: vec!["repo".to_string()],
            available_unauthenticated: false,
        }));

        let result = compute_tool_availability(&plugin, &tool);
        assert!(result.available);
        assert!(result.reason.is_none());
    }

    #[test]
    fn test_scope_missing_when_granted_scopes_insufficient() {
        let mut plugin = create_test_plugin();
        plugin.auth_state = AuthState::Authenticated;
        plugin.credentials = Some(CredentialBinding {
            plugin_id: "test-plugin".to_string(),
            provider: "github".to_string(),
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: None,
            scopes: vec!["repo".to_string()],
        });

        let tool = make_tool(Some(ToolAuthRequirements {
            scopes: vec!["repo".to_string(), "admin:org".to_string()],
            available_unauthenticated: false,
        }));

        let result = compute_tool_availability(&plugin, &tool);
        assert!(!result.available);
        match result.reason {
            Some(UnavailableReason::ScopeMissing { required, missing }) => {
                assert_eq!(required, vec!["repo", "admin:org"]);
                assert_eq!(missing, vec!["admin:org"]);
            }
            other => panic!("Expected ScopeMissing, got {:?}", other),
        }
    }

    #[test]
    fn test_scope_missing_when_no_credentials() {
        let mut plugin = create_test_plugin();
        plugin.auth_state = AuthState::Authenticated;
        // No credentials set — granted scopes are empty
        assert!(plugin.credentials.is_none());

        let tool = make_tool(Some(ToolAuthRequirements {
            scopes: vec!["read".to_string()],
            available_unauthenticated: false,
        }));

        let result = compute_tool_availability(&plugin, &tool);
        assert!(!result.available);
        match result.reason {
            Some(UnavailableReason::ScopeMissing { missing, .. }) => {
                assert_eq!(missing, vec!["read"]);
            }
            other => panic!("Expected ScopeMissing, got {:?}", other),
        }
    }

    #[test]
    fn test_health_checked_before_auth() {
        // Even if auth would fail, health reason takes priority
        let mut plugin = create_test_plugin();
        plugin.health = PluginHealth::Error;
        // Auth state is Unauthenticated, tool requires auth
        let tool = make_tool(Some(ToolAuthRequirements {
            scopes: vec![],
            available_unauthenticated: false,
        }));

        let result = compute_tool_availability(&plugin, &tool);
        assert!(matches!(
            result.reason,
            Some(UnavailableReason::PluginNotHealthy(PluginHealth::Error))
        ));
    }

    #[test]
    fn test_expired_auth_is_not_authenticated() {
        let mut plugin = create_test_plugin();
        plugin.auth_state = AuthState::Expired;
        let tool = make_tool(Some(ToolAuthRequirements {
            scopes: vec![],
            available_unauthenticated: false,
        }));

        let result = compute_tool_availability(&plugin, &tool);
        assert!(!result.available);
        assert_eq!(result.reason, Some(UnavailableReason::AuthRequired));
    }

    #[test]
    fn test_revoked_auth_is_not_authenticated() {
        let mut plugin = create_test_plugin();
        plugin.auth_state = AuthState::Revoked;
        let tool = make_tool(Some(ToolAuthRequirements {
            scopes: vec![],
            available_unauthenticated: false,
        }));

        let result = compute_tool_availability(&plugin, &tool);
        assert!(!result.available);
        assert_eq!(result.reason, Some(UnavailableReason::AuthRequired));
    }

    #[test]
    fn test_authenticating_state_is_not_authenticated() {
        let mut plugin = create_test_plugin();
        plugin.auth_state = AuthState::Authenticating;
        let tool = make_tool(Some(ToolAuthRequirements {
            scopes: vec![],
            available_unauthenticated: false,
        }));

        let result = compute_tool_availability(&plugin, &tool);
        assert!(!result.available);
        assert_eq!(result.reason, Some(UnavailableReason::AuthRequired));
    }

    #[test]
    fn test_partial_scope_coverage() {
        let mut plugin = create_test_plugin();
        plugin.auth_state = AuthState::Authenticated;
        plugin.credentials = Some(CredentialBinding {
            plugin_id: "test-plugin".to_string(),
            provider: "github".to_string(),
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: None,
            scopes: vec!["repo".to_string()],
        });

        // Tool requires 3 scopes, plugin only has 1
        let tool = make_tool(Some(ToolAuthRequirements {
            scopes: vec![
                "repo".to_string(),
                "admin:org".to_string(),
                "user:email".to_string(),
            ],
            available_unauthenticated: false,
        }));

        let result = compute_tool_availability(&plugin, &tool);
        assert!(!result.available);
        match result.reason {
            Some(UnavailableReason::ScopeMissing { required, missing }) => {
                assert_eq!(required.len(), 3);
                assert_eq!(missing, vec!["admin:org", "user:email"]);
            }
            other => panic!("Expected ScopeMissing, got {:?}", other),
        }
    }

    // ---- Session isolation tests (Phase 6.4) ----

    /// Helper: build a registry with one healthy plugin that declares two
    /// no-auth tools.
    fn registry_with_healthy_plugin() -> PluginRegistry {
        let registry = PluginRegistry::new();
        registry.register(PluginConfig {
            id: "iso-plugin".to_string(),
            source: PluginSource::LocalPath {
                path: PathBuf::from("/dev/null"),
            },
            enabled_by_default: true,
        });
        registry.update_health("iso-plugin", PluginHealth::Healthy);
        registry.set_manifest(
            "iso-plugin",
            PluginManifest {
                identity: PluginIdentity {
                    id: "com.test.iso".to_string(),
                    name: "ISO Plugin".to_string(),
                    version: "1.0.0".to_string(),
                },
                publisher: PluginPublisher {
                    name: "Test".to_string(),
                    url: None,
                    contact: None,
                },
                presentation: PresentationMetadata {
                    description: "Isolation test plugin".to_string(),
                    long_description: None,
                    icon: None,
                    category: None,
                    keywords: vec![],
                },
                network_policy: NetworkPolicy::Wildcard,
                auth: None,
                tools: vec![make_tool_named("read"), make_tool_named("write")],
                api_version: "1.0".to_string(),
            },
        );
        registry
    }

    fn make_tool_named(name: &str) -> ExportedTool {
        ExportedTool {
            name: name.to_string(),
            description: format!("Tool {}", name),
            input_schema: serde_json::json!({"type": "object"}),
            requires_approval: false,
            auth_requirements: None,
        }
    }

    /// Test: enabling a plugin in session A while it is disabled in session B
    /// means `get_available_tools` returns tools only for session A.
    #[test]
    fn test_session_isolation_enabled_vs_disabled() {
        let registry = registry_with_healthy_plugin();
        let wasm_host = WasmHost::new();
        let view = EffectivePluginToolView::new(Arc::new(registry), Arc::new(wasm_host));

        // Create two independent sessions.
        let mut session_a = DurableSession::new(SessionId::new());
        let mut session_b = DurableSession::new(SessionId::new());

        // Enable the plugin in session A, disable in session B.
        session_a.set_plugin_enabled("iso-plugin", true);
        session_b.set_plugin_enabled("iso-plugin", false);

        let tools_a = view.get_available_tools(&session_a, &session_a.plugin_enablement);
        let tools_b = view.get_available_tools(&session_b, &session_b.plugin_enablement);

        assert_eq!(
            tools_a.len(),
            2,
            "session A should see both tools from the enabled plugin"
        );
        assert_eq!(
            tools_a
                .iter()
                .map(|(pid, t)| (pid.clone(), t.name.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("iso-plugin".to_string(), "read".to_string()),
                ("iso-plugin".to_string(), "write".to_string()),
            ]
        );

        assert!(
            tools_b.is_empty(),
            "session B should see zero tools from the disabled plugin"
        );
    }

    /// Test: runtime inventory (`PluginRegistry::get_status` /
    /// `PluginRegistry::get_plugin_info`) reflects runtime state only and is
    /// independent of per-session enablement toggles.
    #[test]
    fn test_runtime_inventory_independent_of_session_enablement() {
        let registry = registry_with_healthy_plugin();

        // Create a session that disables the plugin.
        let mut session = DurableSession::new(SessionId::new());
        session.set_plugin_enabled("iso-plugin", false);

        // Runtime status should still show the plugin as healthy and ready.
        let status = registry.get_status("iso-plugin").unwrap();
        assert!(status.health.is_healthy());
        assert!(status.ready);
        assert_eq!(status.available_tool_count, 2);

        // Runtime info should reflect the same.
        let info = registry.get_plugin_info("iso-plugin").unwrap();
        assert!(info.ready);
        assert_eq!(info.available_tool_count, 2);
        assert_eq!(info.declared_tool_count, 2);
    }

    /// Test: changing plugin enablement in one session does not affect another
    /// session's tool catalog.
    #[test]
    fn test_cross_session_enablement_independence() {
        let registry = registry_with_healthy_plugin();
        let wasm_host = WasmHost::new();
        let view = EffectivePluginToolView::new(Arc::new(registry), Arc::new(wasm_host));

        let mut session_a = DurableSession::new(SessionId::new());
        let mut session_b = DurableSession::new(SessionId::new());

        // Enable plugin in both sessions initially.
        session_a.set_plugin_enabled("iso-plugin", true);
        session_b.set_plugin_enabled("iso-plugin", true);

        // Both should see 2 tools.
        let tools_a = view.get_available_tools(&session_a, &session_a.plugin_enablement);
        let tools_b = view.get_available_tools(&session_b, &session_b.plugin_enablement);
        assert_eq!(tools_a.len(), 2);
        assert_eq!(tools_b.len(), 2);

        // Disable in session A only.
        session_a.set_plugin_enabled("iso-plugin", false);

        // Session A should now see 0 tools.
        let tools_a_after = view.get_available_tools(&session_a, &session_a.plugin_enablement);
        assert!(
            tools_a_after.is_empty(),
            "session A should see no tools after disabling"
        );

        // Session B should still see 2 tools (unchanged).
        let tools_b_after = view.get_available_tools(&session_b, &session_b.plugin_enablement);
        assert_eq!(
            tools_b_after.len(),
            2,
            "session B must be unaffected by session A's change"
        );
    }

    // ---- Phase 9.4: Auth-gating tests with per-tool scope differences ----

    /// Build a PluginState with three tools that have different auth
    /// requirements:
    ///   - "free":  no auth required
    ///   - "token": auth required, no scopes needed
    ///   - "scoped_read": auth required, scope "read" needed
    ///   - "scoped_admin": auth required, scopes "read" + "admin" needed
    fn plugin_with_mixed_auth() -> PluginState {
        let mut state = PluginState::new(PluginConfig {
            id: "mixed-auth".to_string(),
            source: PluginSource::LocalPath {
                path: PathBuf::from("/dev/null"),
            },
            enabled_by_default: true,
        });
        state.health = PluginHealth::Healthy;
        state.manifest = Some(PluginManifest {
            identity: PluginIdentity {
                id: "com.test.mixed".to_string(),
                name: "Mixed Auth".to_string(),
                version: "1.0.0".to_string(),
            },
            publisher: PluginPublisher {
                name: "Test".to_string(),
                url: None,
                contact: None,
            },
            presentation: PresentationMetadata {
                description: "Mixed auth test".to_string(),
                long_description: None,
                icon: None,
                category: None,
                keywords: vec![],
            },
            network_policy: NetworkPolicy::Wildcard,
            auth: None,
            tools: vec![
                make_tool_named_with_auth("free", None),
                make_tool_named_with_auth(
                    "token",
                    Some(ToolAuthRequirements {
                        scopes: vec![],
                        available_unauthenticated: false,
                    }),
                ),
                make_tool_named_with_auth(
                    "scoped_read",
                    Some(ToolAuthRequirements {
                        scopes: vec!["read".to_string()],
                        available_unauthenticated: false,
                    }),
                ),
                make_tool_named_with_auth(
                    "scoped_admin",
                    Some(ToolAuthRequirements {
                        scopes: vec!["read".to_string(), "admin".to_string()],
                        available_unauthenticated: false,
                    }),
                ),
            ],
            api_version: "1.0".to_string(),
        });
        state
    }

    fn make_tool_named_with_auth(name: &str, auth: Option<ToolAuthRequirements>) -> ExportedTool {
        ExportedTool {
            name: name.to_string(),
            description: format!("Tool {}", name),
            input_schema: serde_json::json!({"type": "object"}),
            requires_approval: false,
            auth_requirements: auth,
        }
    }

    #[test]
    fn test_unauthenticated_sees_only_free_and_public_tools() {
        let plugin = plugin_with_mixed_auth();
        // Default: Unauthenticated

        let tools = &plugin.manifest.as_ref().unwrap().tools;

        let free_result = compute_tool_availability(&plugin, &tools[0]);
        assert!(free_result.available);

        let token_result = compute_tool_availability(&plugin, &tools[1]);
        assert!(!token_result.available);
        assert_eq!(token_result.reason, Some(UnavailableReason::AuthRequired));

        let scoped_read_result = compute_tool_availability(&plugin, &tools[2]);
        assert!(!scoped_read_result.available);
        assert_eq!(
            scoped_read_result.reason,
            Some(UnavailableReason::AuthRequired)
        );

        let scoped_admin_result = compute_tool_availability(&plugin, &tools[3]);
        assert!(!scoped_admin_result.available);
        assert_eq!(
            scoped_admin_result.reason,
            Some(UnavailableReason::AuthRequired)
        );
    }

    #[test]
    fn test_authenticated_no_scopes_sees_token_but_not_scoped_tools() {
        let mut plugin = plugin_with_mixed_auth();
        plugin.auth_state = AuthState::Authenticated;
        plugin.credentials = Some(CredentialBinding {
            plugin_id: "mixed-auth".to_string(),
            provider: "test".to_string(),
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: None,
            scopes: vec![], // no scopes
        });

        let tools = &plugin.manifest.as_ref().unwrap().tools;

        // free: available
        assert!(compute_tool_availability(&plugin, &tools[0]).available);

        // token (no scopes required): available
        assert!(compute_tool_availability(&plugin, &tools[1]).available);

        // scoped_read: requires "read" → ScopeMissing
        let result = compute_tool_availability(&plugin, &tools[2]);
        assert!(!result.available);
        assert!(matches!(
            &result.reason,
            Some(UnavailableReason::ScopeMissing { missing, .. }) if missing == &vec!["read".to_string()]
        ));

        // scoped_admin: requires "read" + "admin" → ScopeMissing
        let result = compute_tool_availability(&plugin, &tools[3]);
        assert!(!result.available);
        assert!(matches!(
            &result.reason,
            Some(UnavailableReason::ScopeMissing { missing, .. })
                if missing.contains(&"read".to_string()) && missing.contains(&"admin".to_string())
        ));
    }

    #[test]
    fn test_authenticated_partial_scopes_unlocks_matching_tools_only() {
        let mut plugin = plugin_with_mixed_auth();
        plugin.auth_state = AuthState::Authenticated;
        plugin.credentials = Some(CredentialBinding {
            plugin_id: "mixed-auth".to_string(),
            provider: "test".to_string(),
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: None,
            scopes: vec!["read".to_string()], // has "read" but not "admin"
        });

        let tools = &plugin.manifest.as_ref().unwrap().tools;

        // free: available
        assert!(compute_tool_availability(&plugin, &tools[0]).available);
        // token: available
        assert!(compute_tool_availability(&plugin, &tools[1]).available);
        // scoped_read: available (scope satisfied)
        assert!(compute_tool_availability(&plugin, &tools[2]).available);
        // scoped_admin: unavailable (missing "admin" scope)
        let result = compute_tool_availability(&plugin, &tools[3]);
        assert!(!result.available);
        assert!(matches!(
            &result.reason,
            Some(UnavailableReason::ScopeMissing { missing, .. }) if missing == &vec!["admin".to_string()]
        ));
    }

    #[test]
    fn test_authenticated_full_scopes_unlocks_everything() {
        let mut plugin = plugin_with_mixed_auth();
        plugin.auth_state = AuthState::Authenticated;
        plugin.credentials = Some(CredentialBinding {
            plugin_id: "mixed-auth".to_string(),
            provider: "test".to_string(),
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: None,
            scopes: vec!["read".to_string(), "admin".to_string()],
        });

        let tools = &plugin.manifest.as_ref().unwrap().tools;
        for (i, tool) in tools.iter().enumerate() {
            let result = compute_tool_availability(&plugin, tool);
            assert!(
                result.available,
                "tool '{}' (index {}) should be available with full scopes, got {:?}",
                tool.name, i, result.reason
            );
        }
    }

    #[test]
    fn test_expired_auth_scopes_dont_count() {
        let mut plugin = plugin_with_mixed_auth();
        plugin.auth_state = AuthState::Expired;
        // Credentials still present but expired
        plugin.credentials = Some(CredentialBinding {
            plugin_id: "mixed-auth".to_string(),
            provider: "test".to_string(),
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: None,
            scopes: vec!["read".to_string(), "admin".to_string()],
        });

        let tools = &plugin.manifest.as_ref().unwrap().tools;

        // free: available (no auth required)
        assert!(compute_tool_availability(&plugin, &tools[0]).available);

        // All others should be AuthRequired since expired is not authenticated
        for tool in &tools[1..] {
            let result = compute_tool_availability(&plugin, tool);
            assert!(
                !result.available,
                "tool '{}' should be unavailable with expired auth",
                tool.name
            );
            assert_eq!(result.reason, Some(UnavailableReason::AuthRequired));
        }
    }

    #[test]
    fn test_unhealthy_overrides_auth_gating() {
        let mut plugin = plugin_with_mixed_auth();
        plugin.health = PluginHealth::Error;
        plugin.auth_state = AuthState::Authenticated;
        plugin.credentials = Some(CredentialBinding {
            plugin_id: "mixed-auth".to_string(),
            provider: "test".to_string(),
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: None,
            scopes: vec!["read".to_string(), "admin".to_string()],
        });

        let tools = &plugin.manifest.as_ref().unwrap().tools;
        for tool in tools {
            let result = compute_tool_availability(&plugin, tool);
            assert!(!result.available);
            assert_eq!(
                result.reason,
                Some(UnavailableReason::PluginNotHealthy(PluginHealth::Error))
            );
        }
    }
}
