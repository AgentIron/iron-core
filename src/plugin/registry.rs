use crate::plugin::auth::{AuthActionHint, AuthAvailability, AuthState, CredentialBinding};
use crate::plugin::config::{Checksum, PluginConfig, PluginSource};
use crate::plugin::manifest::PluginManifest;
use crate::plugin::status::{PluginHealth, PluginStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::{error, info, warn};

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
}

impl PluginState {
    pub fn new(config: PluginConfig) -> Self {
        let enabled_by_default = config.enabled_by_default;
        Self {
            config,
            manifest: None,
            health: if enabled_by_default {
                PluginHealth::Configured
            } else {
                PluginHealth::Disabled
            },
            last_error: None,
            artifact_path: None,
            auth_state: AuthState::Unauthenticated,
            credentials: None,
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

    /// Update plugin auth state
    pub fn update_auth_state(&self, plugin_id: &str, auth_state: AuthState) {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(state) = plugins.get_mut(plugin_id) {
            state.auth_state = auth_state;
        }
    }

    /// Set credentials for a plugin
    pub fn set_credentials(&self, plugin_id: &str, credentials: CredentialBinding) {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(state) = plugins.get_mut(plugin_id) {
            state.credentials = Some(credentials);
            state.auth_state = AuthState::Authenticated;
        }
    }

    /// Clear credentials for a plugin
    pub fn clear_credentials(&self, plugin_id: &str) {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(state) = plugins.get_mut(plugin_id) {
            state.credentials = None;
            state.auth_state = AuthState::Unauthenticated;
        }
    }

    /// Get plugin status for client consumption
    pub fn get_status(&self, plugin_id: &str) -> Option<PluginStatus> {
        let plugins = self.plugins.read().unwrap();
        let state = plugins.get(plugin_id)?;

        let auth = state.auth_availability();
        let runtime_status = PluginStatus::compute_runtime_status(state.health, &auth);
        let status_message = PluginStatus::generate_status_message(
            runtime_status,
            state
                .manifest
                .as_ref()
                .map(|m| &m.identity.name)
                .unwrap_or(&state.config.id),
        );

        let available_tool_count = if state.is_usable() {
            state.manifest.as_ref().map(|m| m.tools.len()).unwrap_or(0)
        } else {
            0
        };

        Some(PluginStatus {
            plugin_id: plugin_id.to_string(),
            health: state.health,
            auth,
            runtime_status,
            status_message,
            ready: state.is_usable(),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::config::PluginSource;

    fn create_test_config(id: &str) -> PluginConfig {
        PluginConfig {
            id: id.to_string(),
            source: PluginSource::LocalPath {
                path: PathBuf::from("/dev/null"),
            },
            enabled_by_default: true,
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
}
