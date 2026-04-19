use crate::plugin::auth::{
    AuthActionHint, AuthAvailability, AuthPrompt, AuthState, CredentialBinding,
};
use crate::plugin::config::PluginConfig;
use crate::plugin::effective_tools::compute_tool_availability;
use crate::plugin::manifest::PluginManifest;
use crate::plugin::status::{PerToolAvailability, PluginHealth, PluginInfo, PluginStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

/// Stable identifier for a plugin
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PluginId(pub String);

impl std::fmt::Display for PluginId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for PluginId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for PluginId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Summary of per-tool availability for a plugin after recomputation.
///
/// Returned by [`PluginRegistry::recompute_availability`] so callers can
/// inspect the effect of an auth state change without pulling the full
/// `PluginState`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginAvailabilitySummary {
    /// Plugin identifier.
    pub plugin_id: String,
    /// Whether the plugin is currently healthy.
    pub healthy: bool,
    /// Whether the plugin is currently authenticated.
    pub authenticated: bool,
    /// Total number of tools declared in the manifest.
    pub total_tools: usize,
    /// Number of tools that are available given current health + auth + scopes.
    pub available_tools: usize,
    /// Per-tool breakdown.
    pub per_tool: Vec<PerToolAvailability>,
}

/// Runtime-owned metadata about a plugin installation.
///
/// These fields are set by the lifecycle manager during install and are
/// **trusted** because they are derived from the runtime's own state rather
/// than from the plugin artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallMetadata {
    /// When the plugin was installed (or last re-installed).
    pub installed_at: DateTime<Utc>,
    /// The origin of the artifact: local path or remote URL.
    pub source_description: String,
    /// Whether the artifact was fetched over HTTPS with checksum verification.
    pub checksum_verified: bool,
}

/// Runtime state for an installed plugin
#[derive(Debug, Clone)]
pub struct PluginState {
    /// Plugin configuration
    pub config: PluginConfig,
    /// Plugin manifest (loaded after successful installation)
    pub manifest: Option<PluginManifest>,
    /// Current health state
    pub health: PluginHealth,
    /// Last error message (if any)
    pub last_error: Option<String>,
    /// Plugin artifact path (downloaded or local)
    pub artifact_path: Option<PathBuf>,
    /// Authentication state for this plugin
    pub auth_state: AuthState,
    /// Stored credentials (if authenticated)
    pub credentials: Option<CredentialBinding>,
    /// Trusted runtime metadata — set by the install pipeline, not by the
    /// plugin author.  This is kept separate from `manifest` because manifest
    /// values are embedded in the WASM artifact and are *untrusted* until
    /// the runtime validates them.
    pub install_metadata: Option<InstallMetadata>,
}

impl PluginState {
    pub fn new(config: PluginConfig) -> Self {
        // Runtime health starts as Configured regardless of session-level
        // enablement defaults.  Session enablement is a separate concern
        // managed by SessionPluginEnablement, not by PluginHealth.
        Self {
            config,
            manifest: None,
            health: PluginHealth::Configured,
            last_error: None,
            artifact_path: None,
            auth_state: AuthState::Unauthenticated,
            credentials: None,
            install_metadata: None,
        }
    }

    /// Get the plugin ID
    pub fn id(&self) -> &str {
        &self.config.id
    }

    /// Check if the plugin requires authentication
    pub fn requires_auth(&self) -> bool {
        self.manifest
            .as_ref()
            .map(|m| m.auth.is_some())
            .unwrap_or(false)
    }

    /// Get auth availability for this plugin
    pub fn auth_availability(&self) -> AuthAvailability {
        let requires_auth = self.requires_auth();

        if !requires_auth {
            return AuthAvailability {
                state: AuthState::Unauthenticated,
                auth_required: false,
                can_authenticate: false,
                message: "No authentication required".to_string(),
                action_hint: AuthActionHint::None,
            };
        }

        let (can_authenticate, action_hint, message) = match self.auth_state {
            AuthState::Unauthenticated => (
                true,
                AuthActionHint::StartOAuth,
                "Authentication required".to_string(),
            ),
            AuthState::Authenticating => (
                false,
                AuthActionHint::None,
                "Authentication in progress".to_string(),
            ),
            AuthState::Authenticated => (false, AuthActionHint::None, "Authenticated".to_string()),
            AuthState::Expired => (
                true,
                AuthActionHint::RefreshToken,
                "Authentication expired".to_string(),
            ),
            AuthState::Revoked => (
                true,
                AuthActionHint::Reauthenticate,
                "Authentication revoked".to_string(),
            ),
        };

        AuthAvailability {
            state: self.auth_state,
            auth_required: true,
            can_authenticate,
            message,
            action_hint,
        }
    }

    /// Build a structured auth prompt for client rendering.
    ///
    /// Returns `None` if the plugin does not require authentication.
    pub fn auth_prompt(&self) -> Option<AuthPrompt> {
        let oauth = self.manifest.as_ref()?.auth.as_ref()?;

        let title = format!("Connect {}", oauth.provider_name);
        let description = format!(
            "This plugin needs {} access before it can continue.",
            oauth.provider_name
        );

        Some(AuthPrompt {
            auth_id: self.config.id.clone(),
            state: self.auth_state,
            title,
            description,
        })
    }

    /// Check if the plugin is usable (healthy and auth satisfied)
    pub fn is_usable(&self) -> bool {
        if !self.health.is_healthy() {
            return false;
        }

        if self.requires_auth() && !self.auth_state.is_authenticated() {
            return false;
        }

        true
    }
}

/// Registry for managing installed plugins
#[derive(Debug, Clone, Default)]
pub struct PluginRegistry {
    plugins: Arc<RwLock<HashMap<String, PluginState>>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new plugin configuration
    pub fn register(&self, config: PluginConfig) {
        let mut plugins = self.plugins.write().unwrap();
        let id = config.id.clone();
        let state = PluginState::new(config);
        plugins.insert(id.clone(), state);
        info!("Registered plugin: {}", id);
    }

    /// Unregister a plugin
    pub fn unregister(&self, plugin_id: &str) -> Option<PluginState> {
        let mut plugins = self.plugins.write().unwrap();
        let removed = plugins.remove(plugin_id);
        if removed.is_some() {
            info!("Unregistered plugin: {}", plugin_id);
        }
        removed
    }

    /// Get a plugin by ID
    pub fn get(&self, plugin_id: &str) -> Option<PluginState> {
        let plugins = self.plugins.read().unwrap();
        plugins.get(plugin_id).cloned()
    }

    /// List all registered plugins
    pub fn list(&self) -> Vec<PluginState> {
        let plugins = self.plugins.read().unwrap();
        plugins.values().cloned().collect()
    }

    /// Update plugin health
    pub fn update_health(&self, plugin_id: &str, health: PluginHealth) {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(state) = plugins.get_mut(plugin_id) {
            state.health = health;
            if health.is_healthy() {
                state.last_error = None;
            }
        }
    }

    /// Set plugin error state
    pub fn set_error(&self, plugin_id: &str, error: String) {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(state) = plugins.get_mut(plugin_id) {
            state.health = PluginHealth::Error;
            state.last_error = Some(error.clone());
            warn!("Plugin {} entered error state: {}", plugin_id, error);
        }
    }

    /// Update plugin manifest after successful load
    pub fn set_manifest(&self, plugin_id: &str, manifest: PluginManifest) {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(state) = plugins.get_mut(plugin_id) {
            state.manifest = Some(manifest);
        }
    }

    /// Update plugin artifact path
    pub fn set_artifact_path(&self, plugin_id: &str, path: PathBuf) {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(state) = plugins.get_mut(plugin_id) {
            state.artifact_path = Some(path);
        }
    }

    /// Record trusted install metadata for a plugin.
    pub fn set_install_metadata(&self, plugin_id: &str, metadata: InstallMetadata) {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(state) = plugins.get_mut(plugin_id) {
            state.install_metadata = Some(metadata);
        }
    }

    /// Update plugin auth state
    pub fn update_auth_state(&self, plugin_id: &str, auth_state: AuthState) {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(state) = plugins.get_mut(plugin_id) {
            state.auth_state = auth_state;
        }
    }

    /// Set credentials for a plugin and mark as authenticated
    pub fn set_credentials(&self, plugin_id: &str, credentials: CredentialBinding) {
        {
            let mut plugins = self.plugins.write().unwrap();
            if let Some(state) = plugins.get_mut(plugin_id) {
                state.credentials = Some(credentials);
                state.auth_state = AuthState::Authenticated;
            }
        }
        if let Some(summary) = self.recompute_availability(plugin_id) {
            info!(
                plugin_id = %summary.plugin_id,
                available = summary.available_tools,
                total = summary.total_tools,
                "Credentials set; recomputed availability"
            );
        }
    }

    /// Clear credentials for a plugin and reset auth state
    pub fn clear_credentials(&self, plugin_id: &str) {
        {
            let mut plugins = self.plugins.write().unwrap();
            if let Some(state) = plugins.get_mut(plugin_id) {
                state.credentials = None;
                state.auth_state = AuthState::Unauthenticated;
            }
        }
        if let Some(summary) = self.recompute_availability(plugin_id) {
            info!(
                plugin_id = %summary.plugin_id,
                available = summary.available_tools,
                total = summary.total_tools,
                "Credentials cleared; recomputed availability"
            );
        }
    }

    /// Mark authentication as expired
    pub fn mark_auth_expired(&self, plugin_id: &str) {
        {
            let mut plugins = self.plugins.write().unwrap();
            if let Some(state) = plugins.get_mut(plugin_id) {
                if state.auth_state.is_authenticated() {
                    state.auth_state = AuthState::Expired;
                }
            }
        }
        if let Some(summary) = self.recompute_availability(plugin_id) {
            info!(
                plugin_id = %summary.plugin_id,
                available = summary.available_tools,
                total = summary.total_tools,
                "Auth expired; recomputed availability"
            );
        }
    }

    /// Mark authentication as revoked
    pub fn mark_auth_revoked(&self, plugin_id: &str) {
        {
            let mut plugins = self.plugins.write().unwrap();
            if let Some(state) = plugins.get_mut(plugin_id) {
                state.credentials = None;
                state.auth_state = AuthState::Revoked;
            }
        }
        if let Some(summary) = self.recompute_availability(plugin_id) {
            info!(
                plugin_id = %summary.plugin_id,
                available = summary.available_tools,
                total = summary.total_tools,
                "Auth revoked; recomputed availability"
            );
        }
    }

    /// Start authentication flow
    pub fn start_authentication(&self, plugin_id: &str) -> Result<(), String> {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(state) = plugins.get_mut(plugin_id) {
            match state.auth_state {
                AuthState::Authenticating => Err("Authentication already in progress".to_string()),
                AuthState::Authenticated => Err("Already authenticated".to_string()),
                _ => {
                    state.auth_state = AuthState::Authenticating;
                    Ok(())
                }
            }
        } else {
            Err(format!("Plugin '{}' not found", plugin_id))
        }
    }

    /// Start an auth flow and produce an [`AuthInteractionRequest`] for the client.
    ///
    /// Validates that the plugin exists, requires auth, and is in a state that
    /// allows starting authentication.  Transitions the plugin to
    /// `Authenticating` and returns the request the client should act on
    /// (e.g. open a browser to the authorization URL).
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin is not found, does not require auth,
    /// is already authenticating, or is already authenticated.
    pub fn begin_auth_flow(
        &self,
        plugin_id: &str,
    ) -> Result<crate::plugin::auth::AuthInteractionRequest, String> {
        let mut plugins = self.plugins.write().unwrap();
        let state = plugins
            .get_mut(plugin_id)
            .ok_or_else(|| format!("Plugin '{}' not found", plugin_id))?;

        match state.auth_state {
            AuthState::Authenticating => {
                return Err("Authentication already in progress".to_string());
            }
            AuthState::Authenticated => {
                return Err("Already authenticated".to_string());
            }
            _ => {}
        }

        let oauth = state
            .manifest
            .as_ref()
            .and_then(|m| m.auth.as_ref())
            .ok_or_else(|| {
                format!(
                    "Plugin '{}' does not require authentication or has no manifest",
                    plugin_id
                )
            })?;

        state.auth_state = AuthState::Authenticating;

        let request_id = format!("auth-{}-{}", plugin_id, uuid::Uuid::new_v4());

        // Build the authorization URL from the OAuth requirements.
        // In a production system the runtime would construct a proper OAuth
        // URL with PKCE, state, etc.  For now we use the declared endpoint
        // or a sensible default.
        let authorization_url = oauth
            .authorization_endpoint
            .clone()
            .unwrap_or_else(|| format!("https://auth.example.com/authorize/{}", oauth.provider));

        let redirect_uri = format!("iron://auth/callback/{}", plugin_id);

        Ok(crate::plugin::auth::AuthInteractionRequest {
            request_id,
            plugin_id: plugin_id.to_string(),
            provider: oauth.provider.clone(),
            scopes: oauth.scopes.clone(),
            authorization_url,
            redirect_uri,
            code_verifier: None,
        })
    }

    /// Complete an auth flow by processing the client's response.
    ///
    /// On success, stores the provided credentials and transitions to
    /// `Authenticated`.  On denial, failure, or cancellation, transitions
    /// back to `Unauthenticated`.
    ///
    /// Returns the resulting [`AuthStatusTransition`] so callers can
    /// observe the state change.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin is not found or is not in the
    /// `Authenticating` state.
    pub fn complete_auth_flow(
        &self,
        plugin_id: &str,
        response: crate::plugin::auth::AuthInteractionResponse,
    ) -> Result<crate::plugin::auth::AuthStatusTransition, String> {
        use crate::plugin::auth::AuthInteractionResult;

        let (previous_state, provider, scopes) = {
            let plugins = self.plugins.read().unwrap();
            let state = plugins
                .get(plugin_id)
                .ok_or_else(|| format!("Plugin '{}' not found", plugin_id))?;

            if state.auth_state != AuthState::Authenticating {
                return Err(format!(
                    "Plugin '{}' is not in authenticating state (current: {:?})",
                    plugin_id, state.auth_state
                ));
            }

            let oauth = state
                .manifest
                .as_ref()
                .and_then(|manifest| manifest.auth.as_ref())
                .ok_or_else(|| {
                    format!(
                        "Plugin '{}' does not require authentication or has no manifest",
                        plugin_id
                    )
                })?;

            (
                state.auth_state,
                oauth.provider.clone(),
                oauth.scopes.clone(),
            )
        };

        match response.result {
            AuthInteractionResult::Success { code, .. } => {
                // Until token exchange is runtime-owned, preserve the successful
                // completion signal by storing the returned code as the credential
                // secret while using the plugin-declared provider/scopes.
                let credentials = CredentialBinding {
                    plugin_id: plugin_id.to_string(),
                    provider,
                    access_token: code,
                    refresh_token: None,
                    expires_at: None,
                    scopes,
                };
                self.set_credentials(plugin_id, credentials);
            }
            AuthInteractionResult::Denied { .. }
            | AuthInteractionResult::Failed { .. }
            | AuthInteractionResult::Cancelled => {
                // Reset to unauthenticated on failure/denial/cancel.
                {
                    let mut plugins = self.plugins.write().unwrap();
                    if let Some(state) = plugins.get_mut(plugin_id) {
                        state.auth_state = AuthState::Unauthenticated;
                    }
                }
            }
        }

        let new_state = {
            let plugins = self.plugins.read().unwrap();
            plugins
                .get(plugin_id)
                .map(|s| s.auth_state)
                .unwrap_or(AuthState::Unauthenticated)
        };

        Ok(crate::plugin::auth::AuthStatusTransition {
            auth_id: plugin_id.to_string(),
            previous_state,
            new_state,
        })
    }

    /// Clear runtime-loaded state (manifest, artifact path, credentials)
    /// while preserving the plugin configuration.
    ///
    /// Used during install rollback so a failed reinstall does not leave
    /// stale manifest or artifact references from a prior successful install.
    pub fn clear_runtime_state(&self, plugin_id: &str) {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(state) = plugins.get_mut(plugin_id) {
            state.manifest = None;
            state.artifact_path = None;
            state.credentials = None;
            state.auth_state = AuthState::Unauthenticated;
            state.last_error = None;
            state.install_metadata = None;
        }
    }

    /// Get plugin status for client consumption
    pub fn get_status(&self, plugin_id: &str) -> Option<PluginStatus> {
        let plugins = self.plugins.read().unwrap();
        let state = plugins.get(plugin_id)?;

        let auth = state.auth_availability();

        // Compute per-tool availability canonically.
        let (total_tools, available_tool_count) = self.count_tools(state);

        let runtime_status = PluginStatus::compute_runtime_status(
            state.health,
            &auth,
            total_tools,
            available_tool_count,
        );
        let status_message = PluginStatus::generate_status_message(
            runtime_status,
            state
                .manifest
                .as_ref()
                .map(|m| &m.identity.name)
                .unwrap_or(&state.config.id),
        );

        Some(PluginStatus {
            plugin_id: plugin_id.to_string(),
            health: state.health,
            auth,
            runtime_status,
            status_message,
            ready: available_tool_count > 0,
            available_tool_count,
        })
    }

    /// Get all plugin statuses
    pub fn get_all_statuses(&self) -> Vec<PluginStatus> {
        let plugins = self.plugins.read().unwrap();
        plugins
            .keys()
            .filter_map(|id| self.get_status(id))
            .collect()
    }

    /// Count total and available tools for a plugin using the canonical
    /// [`compute_tool_availability`] function.
    ///
    /// Returns `(total_tools, available_tools)`.
    fn count_tools(&self, state: &PluginState) -> (usize, usize) {
        match &state.manifest {
            None => (0, 0),
            Some(manifest) => {
                let total = manifest.tools.len();
                let available = manifest
                    .tools
                    .iter()
                    .filter(|t| compute_tool_availability(state, t).available)
                    .count();
                (total, available)
            }
        }
    }

    /// Recompute per-tool availability for a plugin and return a summary.
    ///
    /// Intended to be called after auth state transitions (credentials set,
    /// cleared, expired, revoked) so that callers and logs can observe the
    /// effect on tool availability.
    pub fn recompute_availability(&self, plugin_id: &str) -> Option<PluginAvailabilitySummary> {
        let plugins = self.plugins.read().unwrap();
        let state = plugins.get(plugin_id)?;

        let manifest = match &state.manifest {
            Some(m) => m,
            None => {
                return Some(PluginAvailabilitySummary {
                    plugin_id: plugin_id.to_string(),
                    healthy: state.health.is_healthy(),
                    authenticated: state.auth_state.is_authenticated(),
                    total_tools: 0,
                    available_tools: 0,
                    per_tool: vec![],
                });
            }
        };

        let total_tools = manifest.tools.len();
        let mut per_tool = Vec::with_capacity(total_tools);
        let mut available_tools = 0;

        for tool in &manifest.tools {
            let result = compute_tool_availability(state, tool);
            if result.available {
                available_tools += 1;
            }

            // Determine auth satisfaction independently of health.
            let auth_satisfied = match &tool.auth_requirements {
                None => true,
                Some(reqs) => {
                    if reqs.available_unauthenticated {
                        true
                    } else if !state.auth_state.is_authenticated() {
                        false
                    } else {
                        // Authenticated — check scopes
                        let granted: Vec<&str> = state
                            .credentials
                            .as_ref()
                            .map(|c| c.scopes.iter().map(|s| s.as_str()).collect())
                            .unwrap_or_default();
                        reqs.scopes.iter().all(|s| granted.contains(&s.as_str()))
                    }
                }
            };

            per_tool.push(PerToolAvailability {
                name: tool.name.clone(),
                metadata: tool.clone(),
                available: result.available,
                unavailable_reason: result.reason,
                requires_auth: tool.auth_requirements.is_some(),
                auth_satisfied,
            });
        }

        Some(PluginAvailabilitySummary {
            plugin_id: plugin_id.to_string(),
            healthy: state.health.is_healthy(),
            authenticated: state.auth_state.is_authenticated(),
            total_tools,
            available_tools,
            per_tool,
        })
    }

    /// Get per-tool availability for a plugin
    pub fn get_tool_availability(&self, plugin_id: &str) -> Vec<(String, bool, Option<String>)> {
        let plugins = self.plugins.read().unwrap();
        let state = match plugins.get(plugin_id) {
            Some(s) => s,
            None => return Vec::new(),
        };

        let manifest = match &state.manifest {
            Some(m) => m,
            None => return Vec::new(),
        };

        manifest
            .tools
            .iter()
            .map(|tool| {
                let result = compute_tool_availability(state, tool);
                let reason_str = result.reason.as_ref().map(|r| format!("{:?}", r));
                (tool.name.clone(), result.available, reason_str)
            })
            .collect()
    }

    /// Get comprehensive plugin info for client consumption.
    ///
    /// Returns a single serialization-friendly snapshot that combines trusted
    /// runtime metadata with validated (but plugin-declared) manifest metadata.
    pub fn get_plugin_info(&self, plugin_id: &str) -> Option<PluginInfo> {
        let plugins = self.plugins.read().unwrap();
        let state = plugins.get(plugin_id)?;

        let auth = state.auth_availability();

        // Compute per-tool availability canonically.
        let (total_tools, available_tool_count) = self.count_tools(state);

        let runtime_status = PluginStatus::compute_runtime_status(
            state.health,
            &auth,
            total_tools,
            available_tool_count,
        );

        // Extract manifest data (untrusted but validated by the install pipeline).
        let (
            identity_id,
            name,
            version,
            publisher_name,
            publisher_url,
            description,
            category,
            tool_names,
            declared_tool_count,
        ) = state.manifest.as_ref().map_or(
            (None, None, None, None, None, None, None, Vec::new(), 0),
            |m| {
                (
                    Some(m.identity.id.clone()),
                    Some(m.identity.name.clone()),
                    Some(m.identity.version.clone()),
                    Some(m.publisher.name.clone()),
                    m.publisher.url.clone(),
                    Some(m.presentation.description.clone()),
                    m.presentation.category.clone(),
                    m.tools.iter().map(|t| t.name.clone()).collect(),
                    m.tools.len(),
                )
            },
        );

        // Extract trusted install metadata.
        let (source, checksum_verified, installed_at) = state
            .install_metadata
            .as_ref()
            .map(|md| {
                (
                    Some(md.source_description.clone()),
                    md.checksum_verified,
                    Some(md.installed_at),
                )
            })
            .unwrap_or((None, false, None));

        Some(PluginInfo {
            identity_id,
            name,
            version,
            publisher_name,
            publisher_url,
            description,
            category,
            health: state.health,
            runtime_status,
            ready: available_tool_count > 0,
            last_error: state.last_error.clone(),
            declared_tool_count,
            available_tool_count,
            tool_names,
            source,
            checksum_verified,
            installed_at,
            auth_required: auth.auth_required,
            auth_state: state.auth_state,
        })
    }

    /// Get auth prompts for all plugins that require authentication.
    ///
    /// Returns a list of [`AuthPrompt`](crate::plugin::auth::AuthPrompt)
    /// values for every registered plugin that declares OAuth requirements.
    /// Clients can use this to render auth UX without polling individual
    /// plugin statuses.
    pub fn get_auth_prompts(&self) -> Vec<crate::plugin::auth::AuthPrompt> {
        let plugins = self.plugins.read().unwrap();
        plugins
            .values()
            .filter_map(|state| state.auth_prompt())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::auth::CredentialBinding;
    use crate::plugin::config::PluginSource;
    use crate::plugin::effective_tools::UnavailableReason;
    use crate::plugin::manifest::{
        ExportedTool, PluginIdentity, PluginManifest, PluginPublisher, PresentationMetadata,
        ToolAuthRequirements,
    };
    use crate::plugin::network::NetworkPolicy;

    fn create_test_config(id: &str) -> PluginConfig {
        PluginConfig {
            id: id.to_string(),
            source: PluginSource::LocalPath {
                path: PathBuf::from("/dev/null"),
            },
            enabled_by_default: true,
        }
    }

    fn create_test_manifest(tools: Vec<ExportedTool>) -> PluginManifest {
        PluginManifest {
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

    #[test]
    fn test_register_and_get() {
        let registry = PluginRegistry::new();
        let config = create_test_config("test-plugin");

        registry.register(config.clone());

        let state = registry.get("test-plugin");
        assert!(state.is_some());
        assert_eq!(state.unwrap().id(), "test-plugin");
    }

    #[test]
    fn test_unregister() {
        let registry = PluginRegistry::new();
        let config = create_test_config("test-plugin");

        registry.register(config);
        let removed = registry.unregister("test-plugin");

        assert!(removed.is_some());
        assert!(registry.get("test-plugin").is_none());
    }

    #[test]
    fn test_update_health() {
        let registry = PluginRegistry::new();
        let config = create_test_config("test-plugin");

        registry.register(config);
        registry.update_health("test-plugin", PluginHealth::Healthy);

        let state = registry.get("test-plugin").unwrap();
        assert!(state.health.is_healthy());
    }

    #[test]
    fn test_new_plugin_state_health_is_configured_regardless_of_enablement_default() {
        // PluginState::new() must always start as Configured, even when
        // the plugin is disabled-by-default for sessions.  Session
        // enablement is a separate concern managed by
        // SessionPluginEnablement.
        let enabled_config = PluginConfig {
            id: "enabled-plugin".to_string(),
            source: PluginSource::LocalPath {
                path: PathBuf::from("/dev/null"),
            },
            enabled_by_default: true,
        };
        let disabled_config = PluginConfig {
            id: "disabled-plugin".to_string(),
            source: PluginSource::LocalPath {
                path: PathBuf::from("/dev/null"),
            },
            enabled_by_default: false,
        };

        let enabled_state = PluginState::new(enabled_config);
        let disabled_state = PluginState::new(disabled_config);

        assert_eq!(enabled_state.health, PluginHealth::Configured);
        assert_eq!(
            disabled_state.health,
            PluginHealth::Configured,
            "runtime health must not be derived from enabled_by_default"
        );
    }

    // ---- recompute_availability tests ----

    #[test]
    fn test_recompute_availability_no_manifest() {
        let registry = PluginRegistry::new();
        registry.register(create_test_config("p1"));

        let summary = registry.recompute_availability("p1").unwrap();
        assert_eq!(summary.total_tools, 0);
        assert_eq!(summary.available_tools, 0);
        assert!(summary.per_tool.is_empty());
    }

    #[test]
    fn test_recompute_availability_all_available() {
        let registry = PluginRegistry::new();
        registry.register(create_test_config("p1"));
        registry.update_health("p1", PluginHealth::Healthy);
        registry.set_manifest(
            "p1",
            create_test_manifest(vec![make_tool("t1", None), make_tool("t2", None)]),
        );

        let summary = registry.recompute_availability("p1").unwrap();
        assert_eq!(summary.total_tools, 2);
        assert_eq!(summary.available_tools, 2);
        assert!(summary.per_tool.iter().all(|t| t.available));
        assert!(summary
            .per_tool
            .iter()
            .all(|t| t.unavailable_reason.is_none()));
    }

    #[test]
    fn test_recompute_availability_auth_gated() {
        let registry = PluginRegistry::new();
        registry.register(create_test_config("p1"));
        registry.update_health("p1", PluginHealth::Healthy);
        registry.set_manifest(
            "p1",
            create_test_manifest(vec![
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
            ]),
        );

        // Not authenticated
        let summary = registry.recompute_availability("p1").unwrap();
        assert_eq!(summary.total_tools, 2);
        assert_eq!(summary.available_tools, 1);

        let free_tool = summary.per_tool.iter().find(|t| t.name == "free").unwrap();
        assert!(free_tool.available);
        assert!(free_tool.auth_satisfied);

        let gated_tool = summary.per_tool.iter().find(|t| t.name == "gated").unwrap();
        assert!(!gated_tool.available);
        assert!(!gated_tool.auth_satisfied);
        assert!(matches!(
            gated_tool.unavailable_reason,
            Some(UnavailableReason::AuthRequired)
        ));
    }

    #[test]
    fn test_recompute_availability_scope_check() {
        let registry = PluginRegistry::new();
        registry.register(create_test_config("p1"));
        registry.update_health("p1", PluginHealth::Healthy);
        registry.set_manifest(
            "p1",
            create_test_manifest(vec![make_tool(
                "scoped",
                Some(ToolAuthRequirements {
                    scopes: vec!["read".to_string(), "write".to_string()],
                    available_unauthenticated: false,
                }),
            )]),
        );

        // Authenticate with partial scopes
        registry.set_credentials(
            "p1",
            CredentialBinding {
                plugin_id: "p1".to_string(),
                provider: "test".to_string(),
                access_token: "tok".to_string(),
                refresh_token: None,
                expires_at: None,
                scopes: vec!["read".to_string()],
            },
        );

        let summary = registry.recompute_availability("p1").unwrap();
        assert_eq!(summary.total_tools, 1);
        assert_eq!(summary.available_tools, 0);

        let tool = &summary.per_tool[0];
        assert!(!tool.available);
        assert!(matches!(
            &tool.unavailable_reason,
            Some(UnavailableReason::ScopeMissing { missing, .. }) if missing.len() == 1 && missing[0] == "write"
        ));
    }

    #[test]
    fn test_recompute_availability_after_clear_credentials() {
        let registry = PluginRegistry::new();
        registry.register(create_test_config("p1"));
        registry.update_health("p1", PluginHealth::Healthy);
        registry.set_manifest(
            "p1",
            create_test_manifest(vec![make_tool(
                "gated",
                Some(ToolAuthRequirements {
                    scopes: vec![],
                    available_unauthenticated: false,
                }),
            )]),
        );

        // Authenticate
        registry.set_credentials(
            "p1",
            CredentialBinding {
                plugin_id: "p1".to_string(),
                provider: "test".to_string(),
                access_token: "tok".to_string(),
                refresh_token: None,
                expires_at: None,
                scopes: vec![],
            },
        );
        let summary = registry.recompute_availability("p1").unwrap();
        assert_eq!(summary.available_tools, 1);

        // Clear credentials
        registry.clear_credentials("p1");
        let summary = registry.recompute_availability("p1").unwrap();
        assert_eq!(summary.available_tools, 0);
        assert!(matches!(
            summary.per_tool[0].unavailable_reason,
            Some(UnavailableReason::AuthRequired)
        ));
    }

    #[test]
    fn test_recompute_availability_unknown_plugin() {
        let registry = PluginRegistry::new();
        assert!(registry.recompute_availability("nonexistent").is_none());
    }

    #[test]
    fn test_get_status_uses_canonical_availability() {
        let registry = PluginRegistry::new();
        registry.register(create_test_config("p1"));
        registry.update_health("p1", PluginHealth::Healthy);
        registry.set_manifest(
            "p1",
            create_test_manifest(vec![
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
            ]),
        );

        let status = registry.get_status("p1").unwrap();
        assert_eq!(status.available_tool_count, 1); // only "free" available
        assert!(status.ready); // at least one tool
        assert_eq!(
            status.runtime_status,
            crate::plugin::status::PluginRuntimeStatus::Partial
        );
    }

    #[test]
    fn test_get_plugin_info_uses_canonical_availability() {
        let registry = PluginRegistry::new();
        registry.register(create_test_config("p1"));
        registry.update_health("p1", PluginHealth::Healthy);
        registry.set_manifest(
            "p1",
            create_test_manifest(vec![make_tool("t1", None), make_tool("t2", None)]),
        );

        let info = registry.get_plugin_info("p1").unwrap();
        assert_eq!(info.available_tool_count, 2);
        assert_eq!(info.declared_tool_count, 2);
        assert!(info.ready);
    }

    // ---- Auth interaction tests ----

    /// Helper: create a manifest with OAuth requirements and a gated tool.
    fn create_auth_manifest() -> PluginManifest {
        use crate::plugin::auth::OAuthRequirements;
        PluginManifest {
            identity: PluginIdentity {
                id: "com.test.auth-plugin".to_string(),
                name: "Auth Plugin".to_string(),
                version: "1.0.0".to_string(),
            },
            publisher: PluginPublisher {
                name: "Test".to_string(),
                url: None,
                contact: None,
            },
            presentation: PresentationMetadata {
                description: "Auth test plugin".to_string(),
                long_description: None,
                icon: None,
                category: None,
                keywords: vec![],
            },
            network_policy: NetworkPolicy::Wildcard,
            auth: Some(OAuthRequirements {
                provider: "github".to_string(),
                provider_name: "GitHub".to_string(),
                scopes: vec!["repo".to_string()],
                authorization_endpoint: Some(
                    "https://github.com/login/oauth/authorize".to_string(),
                ),
                token_endpoint: None,
            }),
            tools: vec![make_tool(
                "gated_tool",
                Some(ToolAuthRequirements {
                    scopes: vec![],
                    available_unauthenticated: false,
                }),
            )],
            api_version: "1.0".to_string(),
        }
    }

    /// Helper: set up a healthy plugin with auth requirements.
    fn setup_auth_plugin(registry: &PluginRegistry, id: &str) {
        registry.register(create_test_config(id));
        registry.update_health(id, PluginHealth::Healthy);
        registry.set_manifest(id, create_auth_manifest());
    }

    // ---- Task 3.1: Tests for unauthenticated, pending, and authenticated
    //      client-visible auth states ----

    #[test]
    fn test_auth_prompt_unauthenticated_state() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        // Verify the auth prompt is exposed in unauthenticated state.
        let prompts = registry.get_auth_prompts();
        assert_eq!(prompts.len(), 1);
        let prompt = &prompts[0];
        assert_eq!(prompt.auth_id, "auth-plugin");
        assert_eq!(prompt.state, AuthState::Unauthenticated);
        assert_eq!(prompt.title, "Connect GitHub");
        assert!(prompt.description.contains("GitHub"));
    }

    #[test]
    fn test_auth_prompt_pending_state() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        // Start authentication → transitions to Authenticating.
        registry.start_authentication("auth-plugin").unwrap();

        let state = registry.get("auth-plugin").unwrap();
        assert_eq!(state.auth_state, AuthState::Authenticating);

        // Auth prompt should reflect the pending state.
        let prompts = registry.get_auth_prompts();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].state, AuthState::Authenticating);
    }

    #[test]
    fn test_auth_prompt_authenticated_state() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        // Set credentials → transitions to Authenticated.
        registry.set_credentials(
            "auth-plugin",
            CredentialBinding {
                plugin_id: "auth-plugin".to_string(),
                provider: "github".to_string(),
                access_token: "ghp_test123".to_string(),
                refresh_token: None,
                expires_at: None,
                scopes: vec!["repo".to_string()],
            },
        );

        let state = registry.get("auth-plugin").unwrap();
        assert_eq!(state.auth_state, AuthState::Authenticated);

        // Auth prompt should reflect the authenticated state.
        let prompts = registry.get_auth_prompts();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].state, AuthState::Authenticated);
    }

    #[test]
    fn test_auth_prompt_not_returned_for_non_auth_plugin() {
        let registry = PluginRegistry::new();
        registry.register(create_test_config("no-auth-plugin"));
        registry.update_health("no-auth-plugin", PluginHealth::Healthy);
        registry.set_manifest(
            "no-auth-plugin",
            create_test_manifest(vec![make_tool("t1", None)]),
        );

        // Plugin without OAuth requirements should not produce an auth prompt.
        let prompts = registry.get_auth_prompts();
        assert!(prompts.is_empty());
    }

    #[test]
    fn test_auth_availability_action_hints_per_state() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        // Unauthenticated → StartOAuth
        let avail = registry.get("auth-plugin").unwrap().auth_availability();
        assert_eq!(avail.action_hint, AuthActionHint::StartOAuth);
        assert!(avail.can_authenticate);

        // Authenticating → None
        registry.start_authentication("auth-plugin").unwrap();
        let avail = registry.get("auth-plugin").unwrap().auth_availability();
        assert_eq!(avail.action_hint, AuthActionHint::None);
        assert!(!avail.can_authenticate);

        // Reset and authenticate
        registry.update_auth_state("auth-plugin", AuthState::Unauthenticated);
        registry.set_credentials(
            "auth-plugin",
            CredentialBinding {
                plugin_id: "auth-plugin".to_string(),
                provider: "github".to_string(),
                access_token: "tok".to_string(),
                refresh_token: None,
                expires_at: None,
                scopes: vec![],
            },
        );
        let avail = registry.get("auth-plugin").unwrap().auth_availability();
        assert_eq!(avail.action_hint, AuthActionHint::None);
        assert!(!avail.can_authenticate);

        // Expired → RefreshToken
        registry.mark_auth_expired("auth-plugin");
        let avail = registry.get("auth-plugin").unwrap().auth_availability();
        assert_eq!(avail.action_hint, AuthActionHint::RefreshToken);
        assert!(avail.can_authenticate);

        // Revoked → Reauthenticate
        registry.mark_auth_revoked("auth-plugin");
        let avail = registry.get("auth-plugin").unwrap().auth_availability();
        assert_eq!(avail.action_hint, AuthActionHint::Reauthenticate);
        assert!(avail.can_authenticate);
    }

    // ---- Task 3.2: Tests for direct client-started auth flow behavior ----

    #[test]
    fn test_begin_auth_flow_success() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        let request = registry.begin_auth_flow("auth-plugin").unwrap();
        assert_eq!(request.plugin_id, "auth-plugin");
        assert_eq!(request.provider, "github");
        assert_eq!(request.scopes, vec!["repo"]);
        assert!(request.authorization_url.contains("github"));
        assert!(request.request_id.starts_with("auth-auth-plugin-"));

        // State should be Authenticating.
        let state = registry.get("auth-plugin").unwrap();
        assert_eq!(state.auth_state, AuthState::Authenticating);
    }

    #[test]
    fn test_begin_auth_flow_already_authenticating() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        registry.start_authentication("auth-plugin").unwrap();
        let result = registry.begin_auth_flow("auth-plugin");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already in progress"));
    }

    #[test]
    fn test_begin_auth_flow_already_authenticated() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        registry.set_credentials(
            "auth-plugin",
            CredentialBinding {
                plugin_id: "auth-plugin".to_string(),
                provider: "github".to_string(),
                access_token: "tok".to_string(),
                refresh_token: None,
                expires_at: None,
                scopes: vec![],
            },
        );

        let result = registry.begin_auth_flow("auth-plugin");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Already authenticated"));
    }

    #[test]
    fn test_begin_auth_flow_plugin_not_found() {
        let registry = PluginRegistry::new();
        let result = registry.begin_auth_flow("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_begin_auth_flow_no_auth_required() {
        let registry = PluginRegistry::new();
        registry.register(create_test_config("no-auth"));
        registry.update_health("no-auth", PluginHealth::Healthy);
        registry.set_manifest("no-auth", create_test_manifest(vec![make_tool("t1", None)]));

        let result = registry.begin_auth_flow("no-auth");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("does not require authentication"));
    }

    #[test]
    fn test_complete_auth_flow_success() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        let request = registry.begin_auth_flow("auth-plugin").unwrap();

        let response = crate::plugin::auth::AuthInteractionResponse {
            request_id: request.request_id,
            result: crate::plugin::auth::AuthInteractionResult::Success {
                code: "auth-code-123".to_string(),
                state: None,
            },
        };

        let transition = registry
            .complete_auth_flow("auth-plugin", response)
            .unwrap();
        assert_eq!(transition.auth_id, "auth-plugin");
        assert_eq!(transition.previous_state, AuthState::Authenticating);
        assert_eq!(transition.new_state, AuthState::Authenticated);

        // Verify credentials were stored.
        let state = registry.get("auth-plugin").unwrap();
        assert_eq!(state.auth_state, AuthState::Authenticated);
        assert!(state.credentials.is_some());
    }

    #[test]
    fn test_complete_auth_flow_denied() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        registry.begin_auth_flow("auth-plugin").unwrap();

        let response = crate::plugin::auth::AuthInteractionResponse {
            request_id: "test".to_string(),
            result: crate::plugin::auth::AuthInteractionResult::Denied {
                reason: "User refused".to_string(),
            },
        };

        let transition = registry
            .complete_auth_flow("auth-plugin", response)
            .unwrap();
        assert_eq!(transition.previous_state, AuthState::Authenticating);
        assert_eq!(transition.new_state, AuthState::Unauthenticated);
    }

    #[test]
    fn test_complete_auth_flow_cancelled() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        registry.begin_auth_flow("auth-plugin").unwrap();

        let response = crate::plugin::auth::AuthInteractionResponse {
            request_id: "test".to_string(),
            result: crate::plugin::auth::AuthInteractionResult::Cancelled,
        };

        let transition = registry
            .complete_auth_flow("auth-plugin", response)
            .unwrap();
        assert_eq!(transition.previous_state, AuthState::Authenticating);
        assert_eq!(transition.new_state, AuthState::Unauthenticated);
    }

    #[test]
    fn test_complete_auth_flow_not_in_authenticating_state() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        // Try to complete without starting.
        let response = crate::plugin::auth::AuthInteractionResponse {
            request_id: "test".to_string(),
            result: crate::plugin::auth::AuthInteractionResult::Success {
                code: "code".to_string(),
                state: None,
            },
        };

        let result = registry.complete_auth_flow("auth-plugin", response);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not in authenticating state"));
    }

    // ---- Task 3.3: Tests confirming plugin availability updates after
    //      auth transitions ----

    #[test]
    fn test_availability_updates_after_auth_success() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        // Before auth: gated tool is unavailable.
        let summary = registry.recompute_availability("auth-plugin").unwrap();
        assert_eq!(summary.available_tools, 0);
        assert!(!summary.authenticated);

        // Complete auth flow successfully.
        registry.begin_auth_flow("auth-plugin").unwrap();
        let response = crate::plugin::auth::AuthInteractionResponse {
            request_id: "test".to_string(),
            result: crate::plugin::auth::AuthInteractionResult::Success {
                code: "code".to_string(),
                state: None,
            },
        };
        registry
            .complete_auth_flow("auth-plugin", response)
            .unwrap();

        // After auth: gated tool should now be available.
        let summary = registry.recompute_availability("auth-plugin").unwrap();
        assert_eq!(summary.available_tools, 1);
        assert!(summary.authenticated);
        assert!(summary.per_tool[0].available);
    }

    #[test]
    fn test_availability_updates_after_auth_denied() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        // Before auth: gated tool is unavailable.
        let summary = registry.recompute_availability("auth-plugin").unwrap();
        assert_eq!(summary.available_tools, 0);

        // Complete auth flow with denial.
        registry.begin_auth_flow("auth-plugin").unwrap();
        let response = crate::plugin::auth::AuthInteractionResponse {
            request_id: "test".to_string(),
            result: crate::plugin::auth::AuthInteractionResult::Denied {
                reason: "refused".to_string(),
            },
        };
        registry
            .complete_auth_flow("auth-plugin", response)
            .unwrap();

        // After denial: tool still unavailable.
        let summary = registry.recompute_availability("auth-plugin").unwrap();
        assert_eq!(summary.available_tools, 0);
        assert!(!summary.authenticated);
    }

    #[test]
    fn test_availability_updates_after_credential_clear() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        // Authenticate first.
        registry.set_credentials(
            "auth-plugin",
            CredentialBinding {
                plugin_id: "auth-plugin".to_string(),
                provider: "github".to_string(),
                access_token: "tok".to_string(),
                refresh_token: None,
                expires_at: None,
                scopes: vec![],
            },
        );
        let summary = registry.recompute_availability("auth-plugin").unwrap();
        assert_eq!(summary.available_tools, 1);

        // Clear credentials → tool becomes unavailable again.
        registry.clear_credentials("auth-plugin");
        let summary = registry.recompute_availability("auth-plugin").unwrap();
        assert_eq!(summary.available_tools, 0);
        assert!(!summary.authenticated);
    }

    #[test]
    fn test_availability_updates_after_auth_expiry() {
        let registry = PluginRegistry::new();
        setup_auth_plugin(&registry, "auth-plugin");

        // Authenticate first.
        registry.set_credentials(
            "auth-plugin",
            CredentialBinding {
                plugin_id: "auth-plugin".to_string(),
                provider: "github".to_string(),
                access_token: "tok".to_string(),
                refresh_token: None,
                expires_at: None,
                scopes: vec![],
            },
        );
        assert_eq!(
            registry
                .recompute_availability("auth-plugin")
                .unwrap()
                .available_tools,
            1
        );

        // Expire auth → tool becomes unavailable.
        registry.mark_auth_expired("auth-plugin");
        let summary = registry.recompute_availability("auth-plugin").unwrap();
        assert_eq!(summary.available_tools, 0);
        assert!(!summary.authenticated);
    }
}
