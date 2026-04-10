//! Session-scoped plugin enablement
//!
//! This module manages per-session plugin enablement state, similar to
//! how MCP server enablement is handled in the durable session.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Plugin enablement state for a session
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionPluginEnablement {
    /// Maps plugin IDs to their enabled state for this session
    enabled: HashMap<String, bool>,
}

impl SessionPluginEnablement {
    /// Create a new empty enablement state
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize from runtime defaults
    pub fn from_defaults(plugin_ids: &[String], default_enabled: bool) -> Self {
        let enabled = plugin_ids
            .iter()
            .map(|id| (id.clone(), default_enabled))
            .collect();
        Self { enabled }
    }

    /// Enable or disable a plugin for this session
    pub fn set_enabled(&mut self, plugin_id: impl Into<String>, enabled: bool) {
        self.enabled.insert(plugin_id.into(), enabled);
    }

    /// Check if a plugin is enabled for this session
    /// Returns None if not explicitly set
    pub fn is_enabled(&self, plugin_id: &str) -> Option<bool> {
        self.enabled.get(plugin_id).copied()
    }

    /// Get all enabled plugins
    pub fn list_enabled(&self) -> Vec<String> {
        self.enabled
            .iter()
            .filter(|(_, &enabled)| enabled)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get all plugin IDs with their enablement state
    pub fn list_all(&self) -> &HashMap<String, bool> {
        &self.enabled
    }

    /// Remove a plugin from enablement tracking
    pub fn remove(&mut self, plugin_id: &str) {
        self.enabled.remove(plugin_id);
    }
}

/// Plugin enablement configuration at the runtime level
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginEnablementDefaults {
    /// Whether plugins are enabled by default for new sessions
    pub enabled_by_default: bool,
    /// Per-plugin overrides of the default
    pub plugin_overrides: HashMap<String, bool>,
}

impl Default for PluginEnablementDefaults {
    fn default() -> Self {
        Self {
            enabled_by_default: true,
            plugin_overrides: HashMap::new(),
        }
    }
}

impl PluginEnablementDefaults {
    /// Create new defaults with the given setting
    pub fn new(enabled_by_default: bool) -> Self {
        Self {
            enabled_by_default,
            plugin_overrides: HashMap::new(),
        }
    }

    /// Set an override for a specific plugin
    pub fn set_override(&mut self, plugin_id: impl Into<String>, enabled: bool) {
        self.plugin_overrides.insert(plugin_id.into(), enabled);
    }

    /// Get the effective default for a plugin
    pub fn effective_default(&self, plugin_id: &str) -> bool {
        self.plugin_overrides
            .get(plugin_id)
            .copied()
            .unwrap_or(self.enabled_by_default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_enablement() {
        let mut enablement = SessionPluginEnablement::new();

        enablement.set_enabled("plugin1", true);
        enablement.set_enabled("plugin2", false);

        assert_eq!(enablement.is_enabled("plugin1"), Some(true));
        assert_eq!(enablement.is_enabled("plugin2"), Some(false));
        assert_eq!(enablement.is_enabled("plugin3"), None);

        let enabled = enablement.list_enabled();
        assert_eq!(enabled, vec!["plugin1"]);
    }

    #[test]
    fn test_from_defaults() {
        let plugin_ids = vec!["plugin1".to_string(), "plugin2".to_string()];
        let enablement = SessionPluginEnablement::from_defaults(&plugin_ids, true);

        assert_eq!(enablement.is_enabled("plugin1"), Some(true));
        assert_eq!(enablement.is_enabled("plugin2"), Some(true));
        assert_eq!(enablement.is_enabled("plugin3"), None);
    }

    #[test]
    fn test_enablement_defaults() {
        let mut defaults = PluginEnablementDefaults::new(true);

        assert!(defaults.effective_default("any_plugin"));

        defaults.set_override("special_plugin", false);
        assert!(defaults.effective_default("any_plugin"));
        assert!(!defaults.effective_default("special_plugin"));
    }
}
