//! Configuration for context pressure, retention, handoff, and model switches.

use serde::{Deserialize, Serialize};

const DEFAULT_MAINTENANCE_THRESHOLD: usize = 50_000;
const DEFAULT_TAIL_MESSAGES: usize = 20;
const DEFAULT_SOFT_THRESHOLD: f64 = 0.50;
const DEFAULT_MEDIUM_THRESHOLD: f64 = 0.70;
const DEFAULT_STRONG_THRESHOLD: f64 = 0.85;
const DEFAULT_CRITICAL_THRESHOLD: f64 = 0.95;

/// Context-management policy applied to a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextManagementConfig {
    /// Whether automatic context management is enabled.
    pub enabled: bool,
    /// Uncompacted token count that requests maintenance.
    pub maintenance_threshold: usize,
    /// Policy for preserving recent uncompacted conversation context.
    pub tail_retention: TailRetentionRule,
    /// Policy for exporting portable handoff bundles.
    pub handoff_export: HandoffExportConfig,
    /// Fallback model context-window size when provider metadata is unavailable.
    pub context_window_hint: Option<usize>,
    /// Fullness fraction at which soft pressure begins.
    pub soft_threshold: f64,
    /// Fullness fraction at which medium pressure begins.
    pub medium_threshold: f64,
    /// Fullness fraction at which strong pressure begins.
    pub strong_threshold: f64,
    /// Fullness fraction at which critical pressure begins.
    pub critical_threshold: f64,
    /// Configuration for model-switch context adaptation.
    #[serde(default)]
    pub model_switch: ModelSwitchConfig,
}

impl Default for ContextManagementConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            maintenance_threshold: DEFAULT_MAINTENANCE_THRESHOLD,
            tail_retention: TailRetentionRule::default(),
            handoff_export: HandoffExportConfig::default(),
            context_window_hint: None,
            soft_threshold: DEFAULT_SOFT_THRESHOLD,
            medium_threshold: DEFAULT_MEDIUM_THRESHOLD,
            strong_threshold: DEFAULT_STRONG_THRESHOLD,
            critical_threshold: DEFAULT_CRITICAL_THRESHOLD,
            model_switch: ModelSwitchConfig::default(),
        }
    }
}

impl ContextManagementConfig {
    /// Creates the disabled default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables automatic context management.
    pub fn enabled(mut self) -> Self {
        self.enabled = true;
        self
    }

    /// Sets the uncompacted token count that requests maintenance.
    pub fn with_maintenance_threshold(mut self, threshold: usize) -> Self {
        self.maintenance_threshold = threshold;
        self
    }

    /// Sets the recent-tail retention policy.
    pub fn with_tail_retention(mut self, rule: TailRetentionRule) -> Self {
        self.tail_retention = rule;
        self
    }

    /// Sets handoff export policy.
    pub fn with_handoff_export(mut self, config: HandoffExportConfig) -> Self {
        self.handoff_export = config;
        self
    }

    /// Sets a fallback context-window size in tokens.
    pub fn with_context_window_hint(mut self, hint: usize) -> Self {
        self.context_window_hint = Some(hint);
        self
    }

    /// Sets the soft pressure fraction.
    pub fn with_soft_threshold(mut self, threshold: f64) -> Self {
        self.soft_threshold = threshold;
        self
    }

    /// Sets the medium pressure fraction.
    pub fn with_medium_threshold(mut self, threshold: f64) -> Self {
        self.medium_threshold = threshold;
        self
    }

    /// Sets the strong pressure fraction.
    pub fn with_strong_threshold(mut self, threshold: f64) -> Self {
        self.strong_threshold = threshold;
        self
    }

    /// Sets the critical pressure fraction.
    pub fn with_critical_threshold(mut self, threshold: f64) -> Self {
        self.critical_threshold = threshold;
        self
    }

    /// Validates enabled context-management configuration.
    ///
    /// Disabled configuration is accepted without validating dormant values.
    ///
    /// # Errors
    ///
    /// Returns an error for zero token/count limits, fractions outside
    /// `0.0..=1.0`, unordered pressure thresholds, or invalid nested policy.
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.maintenance_threshold == 0 {
            return Err("maintenance_threshold must be greater than 0".into());
        }
        if let Some(hint) = self.context_window_hint {
            if hint == 0 {
                return Err("context_window_hint must be greater than 0".into());
            }
        }
        for (name, threshold) in [
            ("soft_threshold", self.soft_threshold),
            ("medium_threshold", self.medium_threshold),
            ("strong_threshold", self.strong_threshold),
            ("critical_threshold", self.critical_threshold),
        ] {
            if !(0.0..=1.0).contains(&threshold) {
                return Err(format!(
                    "{} must be between 0.0 and 1.0, got {}",
                    name, threshold
                ));
            }
        }
        if !(self.soft_threshold <= self.medium_threshold
            && self.medium_threshold <= self.strong_threshold
            && self.strong_threshold <= self.critical_threshold)
        {
            return Err(
                "pressure thresholds must be ordered: soft <= medium <= strong <= critical".into(),
            );
        }
        self.tail_retention.validate()?;
        self.handoff_export.validate()?;
        self.model_switch.validate()
    }

    /// Sets model-switch adaptation policy.
    pub fn with_model_switch_config(mut self, config: ModelSwitchConfig) -> Self {
        self.model_switch = config;
        self
    }
}

/// Size and portability policy for handoff exports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandoffExportConfig {
    /// Approximate target size of an exported bundle in tokens.
    pub default_target_tokens: usize,
    /// Whether local-resource portability warnings are appended to the note.
    pub include_portability_notes: bool,
}

impl Default for HandoffExportConfig {
    fn default() -> Self {
        Self {
            default_target_tokens: crate::context::models::HANDOFF_DEFAULT_TARGET_TOKENS,
            include_portability_notes: true,
        }
    }
}

impl HandoffExportConfig {
    /// Sets the approximate exported-bundle target in tokens.
    pub fn with_target_tokens(mut self, tokens: usize) -> Self {
        self.default_target_tokens = tokens;
        self
    }

    /// Validates that the target token count is nonzero.
    ///
    /// # Errors
    ///
    /// Returns an error when `default_target_tokens` is zero.
    pub fn validate(&self) -> Result<(), String> {
        if self.default_target_tokens == 0 {
            return Err("handoff default_target_tokens must be greater than 0".into());
        }
        Ok(())
    }
}

/// Rule for choosing the recent context tail retained by maintenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TailRetentionRule {
    /// Retain a fixed number of recent messages.
    Messages(usize),
    /// Retain recent messages up to an approximate token budget.
    Tokens(usize),
    /// Apply combined minimum-message and optional token constraints.
    Policy(TailRetentionPolicy),
}

impl Default for TailRetentionRule {
    fn default() -> Self {
        Self::Messages(DEFAULT_TAIL_MESSAGES)
    }
}

impl TailRetentionRule {
    /// Validates that configured retention counts are nonzero.
    ///
    /// # Errors
    ///
    /// Returns an error for a zero count or invalid nested policy.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Messages(n) | Self::Tokens(n) => {
                if *n == 0 {
                    return Err("tail retention count must be greater than 0".into());
                }
                Ok(())
            }
            Self::Policy(p) => p.validate(),
        }
    }
}

/// Combined constraints for retaining an uncompacted recent tail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TailRetentionPolicy {
    /// Minimum number of recent messages to retain.
    pub min_messages: usize,
    /// Optional approximate token ceiling for retained messages.
    pub max_tokens: Option<usize>,
}

impl Default for TailRetentionPolicy {
    fn default() -> Self {
        Self {
            min_messages: 4,
            max_tokens: None,
        }
    }
}

impl TailRetentionPolicy {
    /// Sets the minimum retained message count.
    pub fn with_min_messages(mut self, n: usize) -> Self {
        self.min_messages = n;
        self
    }

    /// Sets the approximate retained-tail token ceiling.
    pub fn with_max_tokens(mut self, tokens: usize) -> Self {
        self.max_tokens = Some(tokens);
        self
    }

    /// Validates that configured message and token counts are nonzero.
    ///
    /// # Errors
    ///
    /// Returns an error when `min_messages` or a present `max_tokens` is zero.
    pub fn validate(&self) -> Result<(), String> {
        if self.min_messages == 0 {
            return Err("min_messages must be greater than 0".into());
        }
        if let Some(t) = self.max_tokens {
            if t == 0 {
                return Err("max_tokens must be greater than 0".into());
            }
        }
        Ok(())
    }
}

/// Configuration for context adaptation at a model-switch boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSwitchConfig {
    /// Whether to compact context before switching to a smaller window.
    pub compact_on_window_shrink: bool,
    /// Default number of messages to retain in the tail after compaction.
    pub default_tail_messages: usize,
    /// Smallest target context window accepted, in tokens.
    pub minimum_window_tokens: usize,
    /// Whether to generate a session briefing when the provider changes.
    pub briefing_on_provider_change: bool,
}

impl Default for ModelSwitchConfig {
    fn default() -> Self {
        Self {
            compact_on_window_shrink: true,
            default_tail_messages: 20,
            minimum_window_tokens: 4000,
            briefing_on_provider_change: true,
        }
    }
}

impl ModelSwitchConfig {
    /// Validates model-switch count and token limits.
    ///
    /// # Errors
    ///
    /// Returns an error when `default_tail_messages` or
    /// `minimum_window_tokens` is zero.
    pub fn validate(&self) -> Result<(), String> {
        if self.default_tail_messages == 0 {
            return Err("default_tail_messages must be greater than 0".into());
        }
        if self.minimum_window_tokens == 0 {
            return Err("minimum_window_tokens must be greater than 0".into());
        }
        Ok(())
    }

    /// Sets whether window shrinkage may trigger compaction.
    pub fn with_compact_on_window_shrink(mut self, enabled: bool) -> Self {
        self.compact_on_window_shrink = enabled;
        self
    }

    /// Sets the default number of recent messages retained after adaptation.
    pub fn with_default_tail_messages(mut self, messages: usize) -> Self {
        self.default_tail_messages = messages;
        self
    }

    /// Sets the smallest accepted target context window.
    pub fn with_minimum_window_tokens(mut self, tokens: usize) -> Self {
        self.minimum_window_tokens = tokens;
        self
    }

    /// Sets whether provider changes generate a session briefing.
    pub fn with_briefing_on_provider_change(mut self, enabled: bool) -> Self {
        self.briefing_on_provider_change = enabled;
        self
    }
}
