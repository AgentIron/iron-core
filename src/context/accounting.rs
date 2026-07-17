//! Estimates the context that will be visible to an inference provider.
//!
//! Category totals use a four-characters-per-token heuristic. When a session
//! tracker has a provider-reported input baseline, that baseline plus local
//! post-response deltas replaces the heuristic total while the heuristic
//! category breakdown remains available.

use crate::context::models::CompressedBlock;
use crate::tool::ToolRegistry;
use iron_providers::Message;

fn estimate_tokens(text: &str) -> usize {
    (text.len() as f64 * 0.25).ceil() as usize
}

/// Confidence level attached to a context token count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextQuality {
    /// The count is an unchanged provider-reported baseline.
    Exact,
    /// At least part of the count was estimated locally.
    Estimated,
    /// No countable context was available.
    Unknown,
}

/// Provider-visible source of context tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextCategory {
    /// System, identity, and session instructions.
    Instructions,
    /// Durable summaries replacing compacted history.
    CompressedBlocks,
    /// Uncompacted recent transcript messages.
    RecentTail,
    /// Tool schemas supplied to the provider.
    ToolDefinitions,
    /// User prompt about to be sent.
    CurrentPrompt,
}

/// Severity derived from the fraction of a model context window in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextPressure {
    /// Usage is below the soft threshold.
    None,
    /// Usage has reached the soft threshold.
    Soft,
    /// Usage has reached the medium threshold.
    Medium,
    /// Usage has reached the strong threshold.
    Strong,
    /// Usage has reached the critical threshold.
    Critical,
}

impl ContextPressure {
    /// Classifies `fullness` using the default 50%, 70%, 85%, and 95% thresholds.
    pub fn from_fullness(fullness: f64) -> Self {
        Self::from_fullness_with_thresholds(fullness, 0.50, 0.70, 0.85, 0.95)
    }

    /// Classifies a context-window fraction using caller-supplied thresholds.
    ///
    /// Thresholds are tested from `critical` down to `soft`; this method does
    /// not validate their ordering.
    pub fn from_fullness_with_thresholds(
        fullness: f64,
        soft: f64,
        medium: f64,
        strong: f64,
        critical: f64,
    ) -> Self {
        match fullness {
            f if f >= critical => ContextPressure::Critical,
            f if f >= strong => ContextPressure::Strong,
            f if f >= medium => ContextPressure::Medium,
            f if f >= soft => ContextPressure::Soft,
            _ => ContextPressure::None,
        }
    }

    /// Returns the stable lowercase label for this pressure level.
    pub fn as_str(&self) -> &'static str {
        match self {
            ContextPressure::None => "none",
            ContextPressure::Soft => "soft",
            ContextPressure::Medium => "medium",
            ContextPressure::Strong => "strong",
            ContextPressure::Critical => "critical",
        }
    }

    /// Returns operational guidance associated with this pressure level.
    pub fn guidance(&self) -> &'static str {
        match self {
            ContextPressure::None => "Context usage is healthy. No action needed.",
            ContextPressure::Soft => "Context usage is rising. You may compress older resolved context if you wish.",
            ContextPressure::Medium => "Context usage is elevated. Consider compressing older resolved context to preserve working room.",
            ContextPressure::Strong => "Context usage is high. Compressing older resolved context is recommended to avoid approaching the limit.",
            ContextPressure::Critical => "Context usage is critical. You must compress resolved context before continuing. Summaries permanently replace selected ranges, so preserve all durable facts, decisions, constraints, file paths, errors, tool results, and user intent.",
        }
    }
}

/// Token usage attributed to one provider-visible context category.
#[derive(Debug, Clone)]
pub struct ContextCategoryUsage {
    /// Source category represented by this entry.
    pub category: ContextCategory,
    /// Token count for the category.
    pub tokens: usize,
    /// Confidence in the category count.
    pub quality: ContextQuality,
}

/// Point-in-time estimate of the next provider request's context footprint.
#[derive(Debug, Clone)]
pub struct ActiveContextSnapshot {
    /// Best available total token count.
    pub total_tokens: usize,
    /// Active model's context-window limit, when known.
    pub context_window_limit: Option<usize>,
    /// Token threshold at which automatic context compaction is requested.
    pub compact_threshold_tokens: Option<usize>,
    /// Confidence in [`Self::total_tokens`].
    pub quality: ContextQuality,
    /// Heuristic breakdown by context source.
    pub categories: Vec<ContextCategoryUsage>,
    /// Current model identifier for this session
    pub current_model: Option<String>,
    /// Number of model switches that have occurred
    pub model_switch_count: usize,
    /// Accumulated provider-reported usage totals, if a tracker is present.
    pub accumulated_usage: Option<crate::context::TokenUsageTotals>,
}

impl ActiveContextSnapshot {
    /// Returns the fraction of the known context window currently occupied.
    ///
    /// Returns `None` when the limit is absent or zero. Values may exceed `1.0`.
    pub fn fullness(&self) -> Option<f64> {
        self.context_window_limit
            .filter(|limit| *limit > 0)
            .map(|limit| self.total_tokens as f64 / limit as f64)
    }

    /// Classifies fullness using the default pressure thresholds.
    pub fn pressure(&self) -> ContextPressure {
        self.pressure_with_thresholds(0.50, 0.70, 0.85, 0.95)
    }

    /// Classifies fullness using caller-supplied pressure thresholds.
    ///
    /// An unknown context-window limit produces [`ContextPressure::None`].
    pub fn pressure_with_thresholds(
        &self,
        soft: f64,
        medium: f64,
        strong: f64,
        critical: f64,
    ) -> ContextPressure {
        self.fullness()
            .map(|f| {
                ContextPressure::from_fullness_with_thresholds(f, soft, medium, strong, critical)
            })
            .unwrap_or(ContextPressure::None)
    }
}

/// Model identity and switch history supplied to an accounting snapshot.
pub struct SessionModelInfo<'a> {
    /// Current model identifier, when one has been selected.
    pub current_model: Option<&'a str>,
    /// Number of completed model switches in the session.
    pub model_switch_count: usize,
}

/// Stateless calculator for provider-visible context estimates.
pub struct ActiveContextAccountant;

impl ActiveContextAccountant {
    /// Estimates a context snapshot from rendered request components.
    ///
    /// A valid `tracker` baseline supplies the total, with `current_prompt`
    /// added because it has not yet appeared in provider usage. Without a
    /// baseline, all components use the local text-length heuristic.
    #[allow(clippy::too_many_arguments)]
    pub fn estimate_snapshot(
        instructions: Option<&str>,
        compressed_blocks: &[CompressedBlock],
        tail_messages: &[Message],
        tool_registry: &ToolRegistry,
        current_prompt: Option<&str>,
        context_window_hint: Option<usize>,
        model_info: SessionModelInfo<'_>,
        tracker: Option<&crate::context::SessionTokenTracker>,
    ) -> ActiveContextSnapshot {
        let mut categories = Vec::new();
        let mut total = 0usize;
        let mut overall_quality = ContextQuality::Unknown;

        if let Some(instr) = instructions {
            let tokens = estimate_tokens(instr);
            total += tokens;
            categories.push(ContextCategoryUsage {
                category: ContextCategory::Instructions,
                tokens,
                quality: ContextQuality::Estimated,
            });
            overall_quality = ContextQuality::Estimated;
        }

        for block in compressed_blocks {
            let rendered = block.render_to_text();
            if !rendered.is_empty() {
                let tokens = estimate_tokens(&rendered);
                total += tokens;
                categories.push(ContextCategoryUsage {
                    category: ContextCategory::CompressedBlocks,
                    tokens,
                    quality: ContextQuality::Estimated,
                });
                overall_quality = ContextQuality::Estimated;
            }
        }

        if !tail_messages.is_empty() {
            let text: String = tail_messages
                .iter()
                .map(|m| match m {
                    Message::User { content } => content.clone(),
                    Message::Assistant { content } => content.clone(),
                    Message::AssistantToolCall {
                        tool_name,
                        arguments,
                        ..
                    } => format!("{}: {}", tool_name, arguments),
                    Message::Tool {
                        tool_name, result, ..
                    } => format!("{}: {}", tool_name, result),
                })
                .collect::<Vec<_>>()
                .join("\n");
            let tokens = estimate_tokens(&text);
            total += tokens;
            categories.push(ContextCategoryUsage {
                category: ContextCategory::RecentTail,
                tokens,
                quality: ContextQuality::Estimated,
            });
            overall_quality = ContextQuality::Estimated;
        }

        if !tool_registry.is_empty() {
            let defs = tool_registry.provider_definitions();
            let json = serde_json::to_string(&defs).unwrap_or_default();
            let tokens = estimate_tokens(&json);
            total += tokens;
            categories.push(ContextCategoryUsage {
                category: ContextCategory::ToolDefinitions,
                tokens,
                quality: ContextQuality::Estimated,
            });
            overall_quality = ContextQuality::Estimated;
        }

        if let Some(prompt) = current_prompt {
            let tokens = estimate_tokens(prompt);
            total += tokens;
            categories.push(ContextCategoryUsage {
                category: ContextCategory::CurrentPrompt,
                tokens,
                quality: ContextQuality::Estimated,
            });
        }

        // If a tracker with a valid baseline is available, use its estimate for
        // the total while keeping the heuristic category breakdown.
        let (total, overall_quality) = if let Some(tracker) = tracker {
            if let Some(tracker_estimate) = tracker.estimate_current_context() {
                // Add current_prompt tokens to the tracker estimate; the prompt
                // is not yet provider-visible so it is not covered by the baseline.
                let current_prompt_tokens = current_prompt.map(estimate_tokens).unwrap_or(0);
                let adjusted_estimate = tracker_estimate.saturating_add(current_prompt_tokens);
                let quality = if current_prompt_tokens > 0 {
                    ContextQuality::Estimated
                } else {
                    tracker.quality()
                };
                (adjusted_estimate, quality)
            } else {
                (total, overall_quality)
            }
        } else {
            (total, overall_quality)
        };

        let accumulated_usage = tracker.map(|t| t.accumulated_totals());

        ActiveContextSnapshot {
            total_tokens: total,
            context_window_limit: context_window_hint,
            compact_threshold_tokens: None,
            quality: overall_quality,
            categories,
            current_model: model_info.current_model.map(|m| m.to_string()),
            model_switch_count: model_info.model_switch_count,
            accumulated_usage,
        }
    }

    /// Estimates tokens for transcript messages using text length.
    ///
    /// Tool names and their JSON arguments or results are included.
    pub fn estimate_messages_tokens(messages: &[Message]) -> usize {
        let text: String = messages
            .iter()
            .map(|m| match m {
                Message::User { content } => content.clone(),
                Message::Assistant { content } => content.clone(),
                Message::AssistantToolCall {
                    tool_name,
                    arguments,
                    ..
                } => format!("{}: {}", tool_name, arguments),
                Message::Tool {
                    tool_name, result, ..
                } => format!("{}: {}", tool_name, result),
            })
            .collect::<Vec<_>>()
            .join("\n");
        estimate_tokens(&text)
    }
}

/// Stateless facade for producing session context telemetry.
pub struct ContextTelemetry;

impl ContextTelemetry {
    /// Produces the same snapshot as [`ActiveContextAccountant::estimate_snapshot`].
    #[allow(clippy::too_many_arguments)]
    pub fn for_session(
        instructions: Option<&str>,
        compressed_blocks: &[CompressedBlock],
        tail_messages: &[Message],
        tool_registry: &ToolRegistry,
        current_prompt: Option<&str>,
        context_window_hint: Option<usize>,
        model_info: SessionModelInfo<'_>,
        tracker: Option<&crate::context::SessionTokenTracker>,
    ) -> ActiveContextSnapshot {
        ActiveContextAccountant::estimate_snapshot(
            instructions,
            compressed_blocks,
            tail_messages,
            tool_registry,
            current_prompt,
            context_window_hint,
            model_info,
            tracker,
        )
    }
}
