use crate::plugin::auth::{AuthAvailability, AuthState};
use crate::plugin::effective_tools::UnavailableReason;
use crate::plugin::manifest::ExportedTool;
use chrono::{DateTime, Utc};
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

/// Plugin runtime health state.
///
/// Health is a **runtime-level** concern: it reflects whether the plugin
/// artifact could be loaded and is operational.  Whether a plugin is
/// *enabled for a particular session* is tracked separately in
/// `SessionPluginEnablement`, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginHealth {
    /// Registered but artifact not yet loaded
    Configured,
    /// Currently loading/initializing
    Loading,
    /// Loaded and operational
    Healthy,
    /// Plugin failed to load or entered an error state
    Error,
    /// Explicitly disabled at the runtime level (e.g. by admin action).
    /// This is *not* set from per-session enablement defaults.
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
    pub unavailable_reason: Option<UnavailableReason>,
    /// Whether authentication is required for this tool
    pub requires_auth: bool,
    /// Whether required authentication is satisfied
    pub auth_satisfied: bool,
}

/// Comprehensive plugin inspection shape for clients and tests.
///
/// Combines trusted runtime metadata with validated (but plugin-declared)
/// manifest metadata so that callers get a single consistent view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    // -- Identity (from manifest, validated) --
    /// Plugin identity ID (e.g. "com.example.my-plugin").
    pub identity_id: Option<String>,
    /// Human-readable plugin name.
    pub name: Option<String>,
    /// Plugin semantic version.
    pub version: Option<String>,

    // -- Publisher (from manifest, untrusted) --
    /// Publisher name.
    pub publisher_name: Option<String>,
    /// Publisher URL.
    pub publisher_url: Option<String>,

    // -- Presentation (from manifest, untrusted) --
    /// Short description.
    pub description: Option<String>,
    /// Plugin category.
    pub category: Option<String>,

    // -- Runtime status (trusted) --
    /// Current plugin health.
    pub health: PluginHealth,
    /// Overall runtime status.
    pub runtime_status: PluginRuntimeStatus,
    /// Whether the plugin is ready to execute tools.
    pub ready: bool,
    /// Last error message, if any.
    pub last_error: Option<String>,

    // -- Tooling --
    /// Total number of tools declared in the manifest.
    pub declared_tool_count: usize,
    /// Number of tools currently available (health + auth satisfied).
    pub available_tool_count: usize,
    /// List of declared tool names.
    pub tool_names: Vec<String>,

    // -- Install provenance (trusted) --
    /// Artifact source description (e.g. "local:/path" or "remote:https://...").
    pub source: Option<String>,
    /// Whether the artifact was verified with a checksum.
    pub checksum_verified: bool,
    /// When the plugin was installed.
    pub installed_at: Option<DateTime<Utc>>,

    // -- Auth --
    /// Whether authentication is required.
    pub auth_required: bool,
    /// Current auth state.
    pub auth_state: AuthState,
}

impl PluginStatus {
    /// Compute runtime status from health, auth, and tool availability.
    ///
    /// `total_tools` and `available_tools` are the declared and currently
    /// available tool counts, respectively, computed via the canonical
    /// [`compute_tool_availability`](crate::plugin::effective_tools::compute_tool_availability).
    pub fn compute_runtime_status(
        health: PluginHealth,
        _auth: &AuthAvailability,
        total_tools: usize,
        available_tools: usize,
    ) -> PluginRuntimeStatus {
        match health {
            PluginHealth::Disabled => PluginRuntimeStatus::Disabled,
            PluginHealth::Error => PluginRuntimeStatus::Error,
            PluginHealth::Configured => PluginRuntimeStatus::Configured,
            PluginHealth::Loading => PluginRuntimeStatus::Loading,
            PluginHealth::Healthy => {
                if total_tools == 0 {
                    // No tools declared (or no manifest) — still just configured.
                    PluginRuntimeStatus::Configured
                } else if available_tools == total_tools {
                    // All declared tools are available.
                    PluginRuntimeStatus::Ready
                } else if available_tools > 0 {
                    // Some but not all tools available.
                    PluginRuntimeStatus::Partial
                } else {
                    // No tools available but some declared — must be auth-gated.
                    PluginRuntimeStatus::AwaitingAuth
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
            PluginStatus::compute_runtime_status(health, &auth, 3, 3),
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

        // 2 declared tools, 0 available → AwaitingAuth
        assert_eq!(
            PluginStatus::compute_runtime_status(health, &auth, 2, 0),
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

        // 3 declared tools, 1 available → Partial
        assert_eq!(
            PluginStatus::compute_runtime_status(health, &auth, 3, 1),
            PluginRuntimeStatus::Partial
        );
    }

    #[test]
    fn test_compute_runtime_status_no_tools_configured() {
        let health = PluginHealth::Healthy;
        let auth = AuthAvailability {
            state: AuthState::Unauthenticated,
            auth_required: false,
            can_authenticate: false,
            message: "No auth".to_string(),
            action_hint: crate::plugin::auth::AuthActionHint::None,
        };

        // 0 declared tools → Configured (even if healthy)
        assert_eq!(
            PluginStatus::compute_runtime_status(health, &auth, 0, 0),
            PluginRuntimeStatus::Configured
        );
    }

    #[test]
    fn test_compute_runtime_status_error() {
        let auth = AuthAvailability {
            state: AuthState::Unauthenticated,
            auth_required: false,
            can_authenticate: false,
            message: "No auth".to_string(),
            action_hint: crate::plugin::auth::AuthActionHint::None,
        };
        assert_eq!(
            PluginStatus::compute_runtime_status(PluginHealth::Error, &auth, 1, 0),
            PluginRuntimeStatus::Error
        );
    }

    #[test]
    fn test_compute_runtime_status_disabled() {
        let auth = AuthAvailability {
            state: AuthState::Unauthenticated,
            auth_required: false,
            can_authenticate: false,
            message: "No auth".to_string(),
            action_hint: crate::plugin::auth::AuthActionHint::None,
        };
        assert_eq!(
            PluginStatus::compute_runtime_status(PluginHealth::Disabled, &auth, 1, 0),
            PluginRuntimeStatus::Disabled
        );
    }

    #[test]
    fn test_compute_runtime_status_loading() {
        let auth = AuthAvailability {
            state: AuthState::Unauthenticated,
            auth_required: false,
            can_authenticate: false,
            message: "No auth".to_string(),
            action_hint: crate::plugin::auth::AuthActionHint::None,
        };
        assert_eq!(
            PluginStatus::compute_runtime_status(PluginHealth::Loading, &auth, 1, 0),
            PluginRuntimeStatus::Loading
        );
    }
}
