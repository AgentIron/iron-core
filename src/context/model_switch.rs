//! Model switching: turn-boundary model changes with context adaptation
//!
//! This module implements model switching as a first-class operation that
//! preserves session identity while adapting context and reconciling capabilities.

use serde::{Deserialize, Serialize};

/// Plan for adapting a session to a new model
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSwitchPlan {
    /// Source model identifier
    pub source_model: String,
    /// Target model identifier
    pub target_model: String,
    /// Source provider slug (if managed)
    pub source_provider: Option<String>,
    /// Target provider slug (if managed)
    pub target_provider: Option<String>,
    /// Context adaptation required
    pub context_adaptation: ContextAdaptationPlan,
    /// Capability differences between source and target
    pub capability_diff: CapabilityDiff,
    /// Whether compaction was triggered
    pub compaction_triggered: bool,
    /// Estimated tokens after adaptation
    pub estimated_tokens_after: usize,
    /// Target context window (if known)
    pub target_window: Option<usize>,
}

/// Plan for adapting context to fit target constraints
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextAdaptationPlan {
    /// Whether context needs to be compacted
    pub needs_compaction: bool,
    /// Number of messages to retain in tail
    pub tail_messages: usize,
    /// Whether the tail fits within target window
    pub tail_fits: bool,
    /// Estimated tokens of retained context
    pub retained_tokens: usize,
}

/// Differences in capabilities between source and target models
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDiff {
    /// Tools that are unavailable in the target model
    pub hidden_tools: Vec<String>,
    /// Modalities unsupported by target (e.g., "image", "pdf")
    pub unsupported_modalities: Vec<String>,
    /// Context window shrink in tokens (if smaller)
    pub window_shrink: Option<usize>,
    /// Features unsupported by target
    pub unsupported_features: Vec<String>,
    /// Whether the target supports tool calling
    pub tools_supported: bool,
    /// Whether the target supports streaming
    pub streaming_supported: bool,
}

impl Default for CapabilityDiff {
    fn default() -> Self {
        Self {
            hidden_tools: Vec::new(),
            unsupported_modalities: Vec::new(),
            window_shrink: None,
            unsupported_features: Vec::new(),
            tools_supported: true,
            streaming_supported: true,
        }
    }
}

/// Request to switch models
#[derive(Debug, Clone, PartialEq)]
pub enum ModelSwitchRequest {
    /// Switch to a managed provider (resolved via credential store)
    Managed {
        provider_slug: String,
        model: String,
        api_key: Option<String>,
    },
    /// Switch to an unmanaged provider (direct Provider instance)
    Unmanaged {
        model: String,
        provider_name: String,
    },
}

/// Record of a model switch for timeline/history
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSwitchRecord {
    /// Source model
    pub from_model: String,
    /// Target model
    pub to_model: String,
    /// Source provider
    pub from_provider: Option<String>,
    /// Target provider
    pub to_provider: Option<String>,
    /// Whether context was adapted
    pub adapted: bool,
    /// Capability differences
    pub capability_diff: CapabilityDiff,
    /// Timestamp of the switch
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Metrics from an automatic compaction performed during a model switch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionInfo {
    /// Estimated tokens before compaction.
    pub tokens_before: u32,
    /// Estimated tokens after compaction.
    pub tokens_after: u32,
    /// Method used (e.g. "auto_compaction").
    pub method: String,
}

/// Pending model switch queued for turn boundary
#[derive(Debug, Clone)]
pub struct PendingModelSwitch {
    /// The switch request
    pub request: ModelSwitchRequest,
    /// When the switch was requested
    pub requested_at: chrono::DateTime<chrono::Utc>,
}

impl ModelSwitchPlan {
    /// Create a new plan with default adaptation
    pub fn new(source_model: impl Into<String>, target_model: impl Into<String>) -> Self {
        Self {
            source_model: source_model.into(),
            target_model: target_model.into(),
            source_provider: None,
            target_provider: None,
            context_adaptation: ContextAdaptationPlan {
                needs_compaction: false,
                tail_messages: 20,
                tail_fits: true,
                retained_tokens: 0,
            },
            capability_diff: CapabilityDiff::default(),
            compaction_triggered: false,
            estimated_tokens_after: 0,
            target_window: None,
        }
    }

    /// Whether the switch requires any adaptation
    pub fn requires_adaptation(&self) -> bool {
        self.context_adaptation.needs_compaction
            || !self.capability_diff.hidden_tools.is_empty()
            || !self.capability_diff.unsupported_modalities.is_empty()
            || !self.capability_diff.unsupported_features.is_empty()
    }
}

impl ContextAdaptationPlan {
    /// Whether the context fits within the target window
    pub fn fits(&self) -> bool {
        self.tail_fits && !self.needs_compaction
    }
}

/// Planner for model switches that estimates context and creates adaptation plans
pub struct ModelSwitchPlanner;

impl ModelSwitchPlanner {
    /// Create a plan for switching from source to target model
    ///
    /// Estimates current context size and determines if compaction is needed
    /// based on the target model's context window.
    pub fn create_plan(
        source_model: &str,
        target_model: &str,
        target_window: Option<usize>,
        current_tokens: usize,
    ) -> ModelSwitchPlan {
        Self::create_plan_with_capabilities(
            source_model,
            target_model,
            target_window,
            current_tokens,
            None,
            None,
        )
    }

    /// Create a plan with capability comparison
    pub fn create_plan_with_capabilities(
        source_model: &str,
        target_model: &str,
        target_window: Option<usize>,
        current_tokens: usize,
        source_capabilities: Option<&ModelCapabilityMetadata>,
        target_capabilities: Option<&ModelCapabilityMetadata>,
    ) -> ModelSwitchPlan {
        let mut plan = ModelSwitchPlan::new(source_model, target_model);
        plan.target_window = target_window;

        // Compute capability diff if both metadata are available
        if let (Some(source), Some(target)) = (source_capabilities, target_capabilities) {
            plan.capability_diff = compare_capabilities(source, target);
        }

        if let Some(window) = target_window {
            let _estimated_tail_tokens = Self::estimate_tail_tokens(current_tokens);

            if current_tokens > window {
                // Context exceeds target window - need compaction
                plan.context_adaptation.needs_compaction = true;
                plan.compaction_triggered = true;

                // Estimate how many messages we can retain
                let avg_tokens_per_message = if current_tokens > 0 {
                    current_tokens / 20 // rough estimate based on default tail
                } else {
                    100
                };

                let max_tail_messages = (window / avg_tokens_per_message.max(1)).max(5);
                plan.context_adaptation.tail_messages = max_tail_messages.min(20);
                plan.context_adaptation.retained_tokens =
                    plan.context_adaptation.tail_messages * avg_tokens_per_message;
                plan.context_adaptation.tail_fits =
                    plan.context_adaptation.retained_tokens <= window;
                plan.estimated_tokens_after = plan.context_adaptation.retained_tokens;
            } else {
                // Context fits within target window
                plan.context_adaptation.needs_compaction = false;
                plan.context_adaptation.tail_messages = 20;
                plan.context_adaptation.tail_fits = true;
                plan.context_adaptation.retained_tokens = current_tokens;
                plan.estimated_tokens_after = current_tokens;
            }
        } else {
            // Unknown target window - assume it fits
            plan.context_adaptation.needs_compaction = false;
            plan.context_adaptation.tail_fits = true;
            plan.context_adaptation.retained_tokens = current_tokens;
            plan.estimated_tokens_after = current_tokens;
        }

        plan
    }

    /// Estimate the token count of the tail (recent messages)
    ///
    /// Uses a simple heuristic: recent messages typically contain
    /// most of the tokens in an active conversation.
    fn estimate_tail_tokens(total_tokens: usize) -> usize {
        // Tail is typically ~60% of total context
        (total_tokens as f64 * 0.6) as usize
    }

    /// Estimate total context size from a session
    ///
    /// Sums uncompacted tokens plus an estimate for compressed blocks.
    pub fn estimate_session_tokens(
        uncompacted_tokens: usize,
        compressed_blocks: &[crate::context::models::CompressedBlock],
    ) -> usize {
        let compressed_estimate: usize = compressed_blocks
            .iter()
            .map(|block| block.summary.len() / 4) // rough token estimate
            .sum();

        uncompacted_tokens + compressed_estimate
    }
}

/// Simple model capability metadata
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCapabilityMetadata {
    pub model: String,
    pub provider: String,
    pub context_window: usize,
    pub supports_tools: bool,
    pub supports_streaming: bool,
    #[serde(default)]
    pub supports_reasoning_effort: bool,
    #[serde(default)]
    pub reasoning_effort_values: Vec<String>,
    pub supported_modalities: Vec<String>,
    pub unsupported_tools: Vec<String>,
}

/// Compare capabilities between source and target models
pub fn compare_capabilities(
    source: &ModelCapabilityMetadata,
    target: &ModelCapabilityMetadata,
) -> CapabilityDiff {
    let mut diff = CapabilityDiff::default();

    // Check context window shrink
    if target.context_window < source.context_window {
        diff.window_shrink = Some(source.context_window - target.context_window);
    }

    // Check tool support
    if !target.supports_tools {
        diff.tools_supported = false;
    }

    // Check streaming support
    if !target.supports_streaming {
        diff.streaming_supported = false;
    }

    // Check reasoning effort support
    if source.supports_reasoning_effort && !target.supports_reasoning_effort {
        diff.unsupported_features.push("reasoning_effort".into());
    }

    // Check modalities
    for modality in &source.supported_modalities {
        if !target.supported_modalities.contains(modality) {
            diff.unsupported_modalities.push(modality.clone());
        }
    }

    // Check tools
    for tool in &target.unsupported_tools {
        diff.hidden_tools.push(tool.clone());
    }

    diff
}

/// Registry for model capability metadata
#[derive(Debug, Clone, Default)]
pub struct ModelCapabilityRegistry {
    models: std::collections::HashMap<(String, String), ModelCapabilityMetadata>,
}

impl ModelCapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, metadata: ModelCapabilityMetadata) {
        let key = (metadata.provider.clone(), metadata.model.clone());
        self.models.insert(key, metadata);
    }

    pub fn get(&self, provider: &str, model: &str) -> Option<&ModelCapabilityMetadata> {
        self.models.get(&(provider.to_string(), model.to_string()))
    }

    pub fn compare(
        &self,
        source_provider: &str,
        source_model: &str,
        target_provider: &str,
        target_model: &str,
    ) -> Option<CapabilityDiff> {
        let source = self.get(source_provider, source_model)?;
        let target = self.get(target_provider, target_model)?;
        Some(compare_capabilities(source, target))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_switch_plan_requires_adaptation() {
        let plan = ModelSwitchPlan {
            source_model: "model-a".into(),
            target_model: "model-b".into(),
            source_provider: None,
            target_provider: None,
            context_adaptation: ContextAdaptationPlan {
                needs_compaction: true,
                tail_messages: 10,
                tail_fits: true,
                retained_tokens: 5000,
            },
            capability_diff: CapabilityDiff::default(),
            compaction_triggered: true,
            estimated_tokens_after: 5000,
            target_window: Some(10000),
        };

        assert!(plan.requires_adaptation());
    }

    #[test]
    fn test_model_switch_plan_no_adaptation_needed() {
        let plan = ModelSwitchPlan::new("model-a", "model-b");
        assert!(!plan.requires_adaptation());
        assert!(plan.context_adaptation.fits());
    }

    #[test]
    fn test_model_switch_planner_context_fits() {
        let plan = ModelSwitchPlanner::create_plan("model-a", "model-b", Some(100000), 50000);
        assert!(!plan.context_adaptation.needs_compaction);
        assert!(plan.context_adaptation.fits());
        assert_eq!(plan.context_adaptation.tail_messages, 20);
    }

    #[test]
    fn test_model_switch_planner_context_exceeds_window() {
        let plan = ModelSwitchPlanner::create_plan("model-a", "model-b", Some(10000), 50000);
        assert!(plan.context_adaptation.needs_compaction);
        assert!(plan.compaction_triggered);
        assert!(!plan.context_adaptation.fits());
        assert!(plan.context_adaptation.tail_messages < 20);
    }

    #[test]
    fn test_model_switch_planner_unknown_window() {
        let plan = ModelSwitchPlanner::create_plan("model-a", "model-b", None, 50000);
        assert!(!plan.context_adaptation.needs_compaction);
        assert!(plan.context_adaptation.fits());
    }

    #[test]
    fn test_estimate_session_tokens() {
        let blocks = vec![crate::context::models::CompressedBlock::new(
            "c0001",
            "Test topic",
            "m0001-m0010",
            "a".repeat(400),
        )];
        let tokens = ModelSwitchPlanner::estimate_session_tokens(1000, &blocks);
        assert_eq!(tokens, 1100); // 1000 + 400/4
    }

    #[test]
    fn test_capability_diff_default() {
        let diff = CapabilityDiff::default();
        assert!(diff.hidden_tools.is_empty());
        assert!(diff.unsupported_modalities.is_empty());
        assert!(diff.unsupported_features.is_empty());
        assert!(diff.tools_supported);
        assert!(diff.streaming_supported);
        assert_eq!(diff.window_shrink, None);
    }

    #[test]
    fn test_compare_capabilities_window_shrink() {
        let source = ModelCapabilityMetadata {
            model: "model-a".into(),
            provider: "provider-a".into(),
            context_window: 100000,
            supports_tools: true,
            supports_streaming: true,
            supports_reasoning_effort: true,
            reasoning_effort_values: vec!["low".into(), "medium".into(), "high".into()],
            supported_modalities: vec!["text".into()],
            unsupported_tools: vec![],
        };

        let target = ModelCapabilityMetadata {
            model: "model-b".into(),
            provider: "provider-b".into(),
            context_window: 50000,
            supports_tools: true,
            supports_streaming: true,
            supports_reasoning_effort: true,
            reasoning_effort_values: vec!["low".into(), "medium".into(), "high".into()],
            supported_modalities: vec!["text".into()],
            unsupported_tools: vec![],
        };

        let diff = compare_capabilities(&source, &target);
        assert_eq!(diff.window_shrink, Some(50000));
        assert!(diff.tools_supported);
    }

    #[test]
    fn test_compare_capabilities_tool_loss() {
        let source = ModelCapabilityMetadata {
            model: "model-a".into(),
            provider: "provider-a".into(),
            context_window: 100000,
            supports_tools: true,
            supports_streaming: true,
            supports_reasoning_effort: true,
            reasoning_effort_values: vec!["low".into(), "medium".into(), "high".into()],
            supported_modalities: vec!["text".into()],
            unsupported_tools: vec![],
        };

        let target = ModelCapabilityMetadata {
            model: "model-b".into(),
            provider: "provider-b".into(),
            context_window: 100000,
            supports_tools: false,
            supports_streaming: true,
            supports_reasoning_effort: false,
            reasoning_effort_values: vec![],
            supported_modalities: vec!["text".into()],
            unsupported_tools: vec!["tool1".into()],
        };

        let diff = compare_capabilities(&source, &target);
        assert!(!diff.tools_supported);
        assert_eq!(diff.hidden_tools, vec!["tool1"]);
        assert_eq!(diff.unsupported_features, vec!["reasoning_effort"]);
    }

    #[test]
    fn test_model_capability_metadata_exposes_reasoning_effort() {
        let meta = ModelCapabilityMetadata {
            model: "o3".into(),
            provider: "openai".into(),
            context_window: 200000,
            supports_tools: true,
            supports_streaming: true,
            supports_reasoning_effort: true,
            reasoning_effort_values: vec!["low".into(), "medium".into(), "high".into()],
            supported_modalities: vec!["text".into()],
            unsupported_tools: vec![],
        };

        assert!(meta.supports_reasoning_effort);
        assert_eq!(meta.reasoning_effort_values, vec!["low", "medium", "high"]);
    }

    #[test]
    fn test_model_capability_registry() {
        let mut registry = ModelCapabilityRegistry::new();
        let meta = ModelCapabilityMetadata {
            model: "gpt-4".into(),
            provider: "openai".into(),
            context_window: 8192,
            supports_tools: true,
            supports_streaming: true,
            supports_reasoning_effort: false,
            reasoning_effort_values: vec![],
            supported_modalities: vec!["text".into()],
            unsupported_tools: vec![],
        };
        registry.register(meta);

        assert!(registry.get("openai", "gpt-4").is_some());
        assert!(registry.get("openai", "gpt-3").is_none());

        let diff = registry.compare("openai", "gpt-4", "openai", "gpt-4");
        assert!(diff.is_some());
        let diff = diff.unwrap();
        assert_eq!(diff.window_shrink, None);
        assert!(diff.tools_supported);
    }
}
