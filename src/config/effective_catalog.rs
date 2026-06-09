//! Effective model catalog that merges built-in and custom model metadata.

use super::builtin_models::BuiltinModelEntry;
use super::records::CustomModelRecord;
use crate::context::model_switch::ModelCapabilityMetadata;
use std::collections::HashMap;

/// Errors that can occur when building or querying the effective model catalog.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum CatalogError {
    #[error("Custom model duplicates built-in entry: ({0}, {1})")]
    DuplicateBuiltIn(String, String),
}

/// A unified model entry in the effective catalog.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveModelEntry {
    /// Provider slug.
    pub provider_slug: String,
    /// Model identifier.
    pub model_id: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Context window in tokens, if known.
    pub context_window: Option<usize>,
    /// Maximum output tokens, if known.
    pub output_limit: Option<u32>,
    /// Whether the model supports tool calls.
    pub supports_tool_calls: bool,
    /// Whether the model supports reasoning / reasoning effort.
    pub supports_reasoning: bool,
    /// Supported reasoning effort values, if applicable.
    pub reasoning_effort_values: Vec<String>,
    /// Whether the model supports vision / image input.
    pub supports_vision: bool,
    /// Whether the model supports streaming responses.
    pub supports_streaming: bool,
    /// Modalities supported by this model.
    pub supported_modalities: Vec<String>,
    /// Tools known to be unsupported by this model.
    pub unsupported_tools: Vec<String>,
    /// Input cost per million tokens, if known.
    pub cost_input_per_million: Option<f64>,
    /// Output cost per million tokens, if known.
    pub cost_output_per_million: Option<f64>,
    /// Whether this entry originated from a custom model record.
    pub is_custom: bool,
}

impl EffectiveModelEntry {
    /// Return the context window as a `usize`, defaulting to 0 when unknown.
    ///
    /// This is convenient for consumers that require a concrete window size.
    pub fn context_window_or_zero(&self) -> usize {
        self.context_window.unwrap_or(0)
    }
}

/// A unified model catalog that merges built-in provider metadata with
/// ConfigStore custom model records.
#[derive(Debug, Clone, Default)]
pub struct EffectiveModelCatalog {
    models: HashMap<(String, String), EffectiveModelEntry>,
}

impl EffectiveModelCatalog {
    /// Create an empty catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a model by provider slug and model ID.
    pub fn get(&self, provider: &str, model: &str) -> Option<&EffectiveModelEntry> {
        self.models.get(&(provider.to_string(), model.to_string()))
    }

    /// List all models for a given provider slug.
    pub fn list_for_provider(&self, provider: &str) -> Vec<&EffectiveModelEntry> {
        self.models
            .values()
            .filter(|entry| entry.provider_slug == provider)
            .collect()
    }

    /// Return all entries in the catalog.
    pub fn all_entries(&self) -> Vec<&EffectiveModelEntry> {
        self.models.values().collect()
    }

    /// Return true if the catalog contains the given provider/model key.
    pub fn contains(&self, provider: &str, model: &str) -> bool {
        self.models
            .contains_key(&(provider.to_string(), model.to_string()))
    }

    /// Register a built-in entry into the catalog.
    fn register_builtin(&mut self, entry: &BuiltinModelEntry) {
        let key = (entry.provider_slug.clone(), entry.model_id.clone());
        self.models.insert(
            key,
            EffectiveModelEntry {
                provider_slug: entry.provider_slug.clone(),
                model_id: entry.model_id.clone(),
                display_name: entry.display_name.clone(),
                context_window: entry.context_window,
                output_limit: entry.output_limit,
                supports_tool_calls: entry.supports_tool_calls,
                supports_reasoning: entry.supports_reasoning,
                reasoning_effort_values: entry.reasoning_effort_values.clone(),
                supports_vision: entry.supports_vision,
                supports_streaming: entry.supports_streaming,
                supported_modalities: entry.supported_modalities.clone(),
                unsupported_tools: entry.unsupported_tools.clone(),
                cost_input_per_million: None,
                cost_output_per_million: None,
                is_custom: false,
            },
        );
    }

    /// Register a custom entry into the catalog.
    fn register_custom(&mut self, entry: &CustomModelRecord) {
        let mut modalities = vec!["text".to_string()];
        if entry.supports_vision {
            modalities.push("image".to_string());
        }

        let key = (entry.provider_slug.clone(), entry.model_id.clone());
        self.models.insert(
            key,
            EffectiveModelEntry {
                provider_slug: entry.provider_slug.clone(),
                model_id: entry.model_id.clone(),
                display_name: entry.display_name.clone(),
                context_window: entry.context_window.map(|v| v as usize),
                output_limit: entry.output_limit,
                supports_tool_calls: entry.supports_tool_calls,
                supports_reasoning: entry.supports_reasoning
                    || !entry.reasoning_effort_values.is_empty(),
                reasoning_effort_values: entry.reasoning_effort_values.clone(),
                supports_vision: entry.supports_vision,
                supports_streaming: entry.supports_streaming,
                supported_modalities: modalities,
                unsupported_tools: Vec::new(),
                cost_input_per_million: entry.cost_input_per_million,
                cost_output_per_million: entry.cost_output_per_million,
                is_custom: true,
            },
        );
    }
}

/// Build an effective model catalog from built-in and custom model records.
///
/// Custom entries extend the catalog. If a custom entry has the same
/// `(provider_slug, model_id)` as a built-in entry, a
/// `CatalogError::DuplicateBuiltIn` error is returned.
pub fn build_effective_catalog(
    builtin: &[BuiltinModelEntry],
    custom: &[CustomModelRecord],
) -> Result<EffectiveModelCatalog, CatalogError> {
    let mut catalog = EffectiveModelCatalog::new();

    for entry in builtin {
        catalog.register_builtin(entry);
    }

    for entry in custom {
        if catalog.contains(&entry.provider_slug, &entry.model_id) {
            return Err(CatalogError::DuplicateBuiltIn(
                entry.provider_slug.clone(),
                entry.model_id.clone(),
            ));
        }
        catalog.register_custom(entry);
    }

    Ok(catalog)
}

impl From<&EffectiveModelEntry> for ModelCapabilityMetadata {
    fn from(entry: &EffectiveModelEntry) -> Self {
        Self {
            model: entry.model_id.clone(),
            provider: entry.provider_slug.clone(),
            context_window: entry.context_window.unwrap_or(0),
            supports_tools: entry.supports_tool_calls,
            supports_streaming: entry.supports_streaming,
            supports_reasoning_effort: entry.supports_reasoning,
            reasoning_effort_values: entry.reasoning_effort_values.clone(),
            supported_modalities: entry.supported_modalities.clone(),
            unsupported_tools: entry.unsupported_tools.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::builtin_models::BuiltinModelEntry;
    use super::*;
    use chrono::Utc;

    fn custom_model_record(provider_slug: &str, model_id: &str) -> CustomModelRecord {
        CustomModelRecord {
            provider_slug: provider_slug.to_string(),
            model_id: model_id.to_string(),
            display_name: format!("Custom {}", model_id),
            context_window: Some(100_000),
            output_limit: Some(4_096),
            supports_tool_calls: true,
            supports_reasoning: false,
            supports_vision: true,
            supports_streaming: true,
            reasoning_effort_values: Vec::new(),
            cost_input_per_million: None,
            cost_output_per_million: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_catalog_merge_built_in_and_custom() {
        let builtin = vec![BuiltinModelEntry::new("openai", "gpt-4o")
            .with_display_name("GPT-4o")
            .with_context_window(128_000)
            .with_tools()
            .with_vision()];
        let custom = vec![custom_model_record("custom-provider", "custom-model")];

        let catalog = build_effective_catalog(&builtin, &custom).unwrap();
        assert!(catalog.get("openai", "gpt-4o").is_some());
        assert!(catalog.get("custom-provider", "custom-model").is_some());
    }

    #[test]
    fn test_catalog_rejects_duplicate_built_in() {
        let builtin = vec![BuiltinModelEntry::new("openai", "gpt-4o")
            .with_display_name("GPT-4o")
            .with_context_window(128_000)
            .with_tools()];
        let custom = vec![custom_model_record("openai", "gpt-4o")];

        let result = build_effective_catalog(&builtin, &custom);
        assert!(matches!(
            result,
            Err(CatalogError::DuplicateBuiltIn(ref p, ref m)) if p == "openai" && m == "gpt-4o"
        ));
    }

    #[test]
    fn test_catalog_lookup_and_list_by_provider() {
        let builtin = vec![
            BuiltinModelEntry::new("openai", "gpt-4o").with_context_window(128_000),
            BuiltinModelEntry::new("openai", "gpt-4o-mini").with_context_window(128_000),
            BuiltinModelEntry::new("anthropic", "claude-3-5-sonnet-20241022")
                .with_context_window(200_000),
        ];
        let custom = vec![custom_model_record("openai", "custom-openai")];

        let catalog = build_effective_catalog(&builtin, &custom).unwrap();

        let openai_models = catalog.list_for_provider("openai");
        assert_eq!(openai_models.len(), 3);

        let anthropic_models = catalog.list_for_provider("anthropic");
        assert_eq!(anthropic_models.len(), 1);

        assert!(catalog.get("openai", "gpt-4o").is_some());
        assert!(catalog.get("openai", "nonexistent").is_none());
    }

    #[test]
    fn test_catalog_converts_to_capability_metadata() {
        let builtin = vec![BuiltinModelEntry::new("openai", "gpt-4o")
            .with_context_window(128_000)
            .with_tools()
            .with_vision()];
        let catalog = build_effective_catalog(&builtin, &[]).unwrap();
        let entry = catalog.get("openai", "gpt-4o").unwrap();
        let meta: ModelCapabilityMetadata = entry.into();

        assert_eq!(meta.model, "gpt-4o");
        assert_eq!(meta.provider, "openai");
        assert_eq!(meta.context_window, 128_000);
        assert!(meta.supports_tools);
        assert!(meta.supports_streaming);
        assert!(meta.supported_modalities.contains(&"image".to_string()));
    }

    #[test]
    fn test_custom_entry_inherits_modalities_from_vision_flag() {
        let custom = vec![CustomModelRecord {
            provider_slug: "custom".to_string(),
            model_id: "vision-model".to_string(),
            display_name: "Vision".to_string(),
            context_window: Some(100_000),
            output_limit: Some(4_096),
            supports_tool_calls: true,
            supports_reasoning: false,
            supports_vision: true,
            supports_streaming: true,
            reasoning_effort_values: Vec::new(),
            cost_input_per_million: None,
            cost_output_per_million: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }];

        let catalog = build_effective_catalog(&[], &custom).unwrap();
        let entry = catalog.get("custom", "vision-model").unwrap();
        assert!(entry.supported_modalities.contains(&"text".to_string()));
        assert!(entry.supported_modalities.contains(&"image".to_string()));
    }
}
