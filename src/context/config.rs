use serde::{Deserialize, Serialize};

const DEFAULT_MAINTENANCE_THRESHOLD: usize = 50_000;
const DEFAULT_TAIL_MESSAGES: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextManagementConfig {
    pub enabled: bool,
    pub maintenance_threshold: usize,
    pub tail_retention: TailRetentionRule,
    pub handoff_export: HandoffExportConfig,
    pub context_window_hint: Option<usize>,
}

impl Default for ContextManagementConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            maintenance_threshold: DEFAULT_MAINTENANCE_THRESHOLD,
            tail_retention: TailRetentionRule::default(),
            handoff_export: HandoffExportConfig::default(),
            context_window_hint: None,
        }
    }
}

impl ContextManagementConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enabled(mut self) -> Self {
        self.enabled = true;
        self
    }

    pub fn with_maintenance_threshold(mut self, threshold: usize) -> Self {
        self.maintenance_threshold = threshold;
        self
    }

    pub fn with_tail_retention(mut self, rule: TailRetentionRule) -> Self {
        self.tail_retention = rule;
        self
    }

    pub fn with_handoff_export(mut self, config: HandoffExportConfig) -> Self {
        self.handoff_export = config;
        self
    }

    pub fn with_context_window_hint(mut self, hint: usize) -> Self {
        self.context_window_hint = Some(hint);
        self
    }

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
        self.tail_retention.validate()?;
        self.handoff_export.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandoffExportConfig {
    pub default_target_tokens: usize,
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
    pub fn with_target_tokens(mut self, tokens: usize) -> Self {
        self.default_target_tokens = tokens;
        self
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.default_target_tokens == 0 {
            return Err("handoff default_target_tokens must be greater than 0".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TailRetentionRule {
    Messages(usize),
    Tokens(usize),
    Policy(TailRetentionPolicy),
}

impl Default for TailRetentionRule {
    fn default() -> Self {
        Self::Messages(DEFAULT_TAIL_MESSAGES)
    }
}

impl TailRetentionRule {
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TailRetentionPolicy {
    pub min_messages: usize,
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
    pub fn with_min_messages(mut self, n: usize) -> Self {
        self.min_messages = n;
        self
    }

    pub fn with_max_tokens(mut self, tokens: usize) -> Self {
        self.max_tokens = Some(tokens);
        self
    }

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
