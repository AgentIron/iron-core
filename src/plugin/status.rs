use crate::plugin::auth::{AuthAvailability, AuthState};
use crate::plugin::manifest::ExportedTool;
use serde::{Deserialize, Serialize};

/// Runtime status of a plugin exposed to clients
///
/// This separates health, auth, and presentation concerns so clients
/// can render appropriate UX for different plugin states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginStatus {
    /// Plugin identifier
    pub plugin_id: String,
    /// Runtime health state
    pub health: PluginHealth,
    /// Authentication availability
    pub auth: AuthAvailability,
    /// Overall runtime status
    pub runtime_status: PluginRuntimeStatus,
    /// User-facing status message
    pub status_message: String,
    /// Whether the plugin is ready to execute tools
    pub ready: bool,
    /// Number of tools available in current state
    pub available_tool_count: usize,
}

/// Plugin health state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginHealth {
    /// Initial state before loading
    Configured,
    /// Currently loading/initializing
    Loading,
    /// Loaded and operational
    Healthy,
    /// Plugin failed to load
    Error,
    /// Plugin is disabled
    Disabled,
}

impl PluginHealth {
    /// Check if the plugin is healthy enough to execute
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy)
    }

    /// Check if the plugin is in an error state
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error)
    }
}

/// Overall plugin runtime status
///
/// This aggregates health and auth into a single status for simple clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeStatus {
    /// Plugin is configured but not loaded
    Configured,
    /// Plugin is loading
    Loading,
    /// Plugin is ready for use (healthy + authenticated)
    Ready,
    /// Plugin is healthy but needs authentication
    AwaitingAuth,
    /// Plugin has partial functionality (some tools available)
    Partial,
    /// Plugin encountered an error
    Error,
    /// Plugin is disabled
    Disabled,
}

/// Per-tool availability information
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerToolAvailability {
    /// Tool name
    pub name: String,
    /// Tool metadata
    pub metadata: ExportedTool,
    /// Whether this tool is available in current state
    pub available: bool,
    /// Reason if not available
    #[serde(default)]
    pub unavailable_reason: Option<String>,
    /// Whether authentication is required for this tool
    pub requires_auth: bool,
    /// Whether required authentication is satisfied
    pub auth_satisfied: bool,
}

impl PluginStatus {
    /// Compute runtime status from health and auth
    pub fn compute_runtime_status(
        health: PluginHealth,
        auth: &AuthAvailability,
    ) -> PluginRuntimeStatus {
        match health {
            PluginHealth::Disabled => PluginRuntimeStatus::Disabled,
            PluginHealth::Error => PluginRuntimeStatus::Error,
            PluginHealth::Configured => PluginRuntimeStatus::Configured,
            PluginHealth::Loading => PluginRuntimeStatus::Loading,
            PluginHealth::Healthy => {
                if auth.auth_required && !auth.state.is_authenticated() {
                    if auth.state.requires_reauth() {
                        PluginRuntimeStatus::AwaitingAuth
                    } else {
                        PluginRuntimeStatus::Partial
                    }
                } else {
                    PluginRuntimeStatus::Ready
                }
            }
        }
    }

    /// Generate a user-facing status message
    pub fn generate_status_message(
        runtime_status: PluginRuntimeStatus,
        plugin_name: &str,
    ) -> String {
        match runtime_status {
            PluginRuntimeStatus::Ready => format!("{} is ready to use", plugin_name),
            PluginRuntimeStatus::AwaitingAuth => format!("{} needs authentication", plugin_name),
            PluginRuntimeStatus::Partial => format!("{} has limited functionality", plugin_name),
            PluginRuntimeStatus::Error => format!("{} encountered an error", plugin_name),
            PluginRuntimeStatus::Loading => format!("{} is loading...", plugin_name),
            PluginRuntimeStatus::Configured => format!("{} is configured", plugin_name),
            PluginRuntimeStatus::Disabled => format!("{} is disabled", plugin_name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_runtime_status_ready() {
        let health = PluginHealth::Healthy;
        let auth = AuthAvailability {
            state: AuthState::Authenticated,
            auth_required: true,
            can_authenticate: false,
            message: "Authenticated".to_string(),
            action_hint: crate::plugin::auth::AuthActionHint::None,
        };

        assert_eq!(
            PluginStatus::compute_runtime_status(health, &auth),
            PluginRuntimeStatus::Ready
        );
    }

    #[test]
    fn test_compute_runtime_status_awaiting_auth() {
        let health = PluginHealth::Healthy;
        let auth = AuthAvailability {
            state: AuthState::Unauthenticated,
            auth_required: true,
            can_authenticate: true,
            message: "Needs auth".to_string(),
            action_hint: crate::plugin::auth::AuthActionHint::StartOAuth,
        };

        assert_eq!(
            PluginStatus::compute_runtime_status(health, &auth),
            PluginRuntimeStatus::AwaitingAuth
        );
    }

    #[test]
    fn test_compute_runtime_status_partial() {
        let health = PluginHealth::Healthy;
        let auth = AuthAvailability {
            state: AuthState::Authenticating,
            auth_required: true,
            can_authenticate: false,
            message: "Authenticating".to_string(),
            action_hint: crate::plugin::auth::AuthActionHint::None,
        };

        assert_eq!(
            PluginStatus::compute_runtime_status(health, &auth),
            PluginRuntimeStatus::Partial
        );
    }
}
