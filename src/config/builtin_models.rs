//! Compiled-in model metadata for known iron-providers models.
//!
//! This catalog is maintained alongside the iron-providers version used by
//! iron-core. When iron-providers is upgraded, this catalog should be reviewed
//! and updated to reflect new or changed models.

/// Metadata for a single built-in model entry.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltinModelEntry {
    /// Provider slug (e.g., "openai", "anthropic").
    pub provider_slug: String,
    /// Model identifier passed to the provider API.
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
    /// Modalities supported by this model (e.g., "text", "image").
    pub supported_modalities: Vec<String>,
    /// Tools known to be unsupported by this model.
    pub unsupported_tools: Vec<String>,
}

impl BuiltinModelEntry {
    /// Create a new built-in model entry with sensible defaults.
    pub fn new(provider_slug: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            provider_slug: provider_slug.into(),
            model_id: model_id.into(),
            display_name: String::new(),
            context_window: None,
            output_limit: None,
            supports_tool_calls: false,
            supports_reasoning: false,
            reasoning_effort_values: Vec::new(),
            supports_vision: false,
            supports_streaming: true,
            supported_modalities: vec!["text".to_string()],
            unsupported_tools: Vec::new(),
        }
    }

    /// Set the display name.
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = name.into();
        self
    }

    /// Set the context window.
    pub fn with_context_window(mut self, window: usize) -> Self {
        self.context_window = Some(window);
        self
    }

    /// Set the output limit.
    pub fn with_output_limit(mut self, limit: u32) -> Self {
        self.output_limit = Some(limit);
        self
    }

    /// Enable tool call support.
    pub fn with_tools(mut self) -> Self {
        self.supports_tool_calls = true;
        self
    }

    /// Enable reasoning support with optional accepted values.
    pub fn with_reasoning(mut self, values: Vec<impl Into<String>>) -> Self {
        self.supports_reasoning = !values.is_empty();
        self.reasoning_effort_values = values.into_iter().map(Into::into).collect();
        self
    }

    /// Enable vision / image input support.
    pub fn with_vision(mut self) -> Self {
        self.supports_vision = true;
        if !self.supported_modalities.contains(&"image".to_string()) {
            self.supported_modalities.push("image".to_string());
        }
        self
    }

    /// Disable streaming support.
    pub fn without_streaming(mut self) -> Self {
        self.supports_streaming = false;
        self
    }

    /// Set supported modalities.
    pub fn with_modalities(mut self, modalities: Vec<impl Into<String>>) -> Self {
        self.supported_modalities = modalities.into_iter().map(Into::into).collect();
        self
    }

    /// Set unsupported tools.
    pub fn with_unsupported_tools(mut self, tools: Vec<impl Into<String>>) -> Self {
        self.unsupported_tools = tools.into_iter().map(Into::into).collect();
        self
    }
}

/// Return the compiled-in catalog of known iron-providers models.
///
/// This catalog should be kept in sync with the iron-providers version used
/// by iron-core. When adding a new provider or model, update this list and
/// add a corresponding entry to the test suite.
pub fn builtin_model_catalog() -> Vec<BuiltinModelEntry> {
    vec![
        // OpenAI
        BuiltinModelEntry::new("openai", "gpt-4o")
            .with_display_name("GPT-4o")
            .with_context_window(128_000)
            .with_output_limit(16_384)
            .with_tools()
            .with_vision(),
        BuiltinModelEntry::new("openai", "gpt-4o-mini")
            .with_display_name("GPT-4o Mini")
            .with_context_window(128_000)
            .with_output_limit(16_384)
            .with_tools()
            .with_vision(),
        BuiltinModelEntry::new("openai", "o3-mini")
            .with_display_name("o3-mini")
            .with_context_window(200_000)
            .with_output_limit(100_000)
            .with_tools()
            .with_reasoning(vec!["low", "medium", "high"]),
        BuiltinModelEntry::new("openai", "o1")
            .with_display_name("o1")
            .with_context_window(200_000)
            .with_output_limit(100_000)
            .with_tools()
            .with_reasoning(vec!["low", "medium", "high"]),
        BuiltinModelEntry::new("openai", "o1-mini")
            .with_display_name("o1-mini")
            .with_context_window(128_000)
            .with_output_limit(65_536)
            .with_tools()
            .with_reasoning(vec!["low", "medium", "high"]),
        // Anthropic
        BuiltinModelEntry::new("anthropic", "claude-3-7-sonnet-20250219")
            .with_display_name("Claude 3.7 Sonnet")
            .with_context_window(200_000)
            .with_output_limit(8_192)
            .with_tools()
            .with_vision(),
        BuiltinModelEntry::new("anthropic", "claude-3-5-sonnet-20241022")
            .with_display_name("Claude 3.5 Sonnet")
            .with_context_window(200_000)
            .with_output_limit(8_192)
            .with_tools()
            .with_vision(),
        BuiltinModelEntry::new("anthropic", "claude-3-5-haiku-20241022")
            .with_display_name("Claude 3.5 Haiku")
            .with_context_window(200_000)
            .with_output_limit(8_192)
            .with_tools(),
        BuiltinModelEntry::new("anthropic", "claude-3-opus-20240229")
            .with_display_name("Claude 3 Opus")
            .with_context_window(200_000)
            .with_output_limit(4_096)
            .with_tools()
            .with_vision(),
        // Google
        BuiltinModelEntry::new("google", "gemini-2.0-flash-001")
            .with_display_name("Gemini 2.0 Flash")
            .with_context_window(1_048_576)
            .with_output_limit(8_192)
            .with_tools()
            .with_vision(),
        BuiltinModelEntry::new("google", "gemini-1.5-pro-002")
            .with_display_name("Gemini 1.5 Pro")
            .with_context_window(2_097_152)
            .with_output_limit(8_192)
            .with_tools()
            .with_vision(),
        BuiltinModelEntry::new("google", "gemini-1.5-flash-002")
            .with_display_name("Gemini 1.5 Flash")
            .with_context_window(1_048_576)
            .with_output_limit(8_192)
            .with_tools()
            .with_vision(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_catalog_is_non_empty() {
        let catalog = builtin_model_catalog();
        assert!(
            !catalog.is_empty(),
            "Built-in model catalog must not be empty"
        );
    }

    #[test]
    fn test_builtin_entries_have_non_empty_ids() {
        for entry in builtin_model_catalog() {
            assert!(
                !entry.provider_slug.trim().is_empty(),
                "Built-in entry must have a non-empty provider slug"
            );
            assert!(
                !entry.model_id.trim().is_empty(),
                "Built-in entry must have a non-empty model ID"
            );
            assert!(
                !entry.display_name.trim().is_empty(),
                "Built-in entry must have a non-empty display name"
            );
        }
    }

    #[test]
    fn test_builtin_entries_have_positive_context_window_when_set() {
        for entry in builtin_model_catalog() {
            if let Some(window) = entry.context_window {
                assert!(
                    window > 0,
                    "Built-in entry ({}, {}) must have a positive context window",
                    entry.provider_slug,
                    entry.model_id
                );
            }
        }
    }

    #[test]
    fn test_builtin_entries_have_text_modality() {
        for entry in builtin_model_catalog() {
            assert!(
                entry.supported_modalities.contains(&"text".to_string()),
                "Built-in entry ({}, {}) must support text modality",
                entry.provider_slug,
                entry.model_id
            );
        }
    }
}
