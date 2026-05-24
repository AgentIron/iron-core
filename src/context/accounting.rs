use crate::context::models::CompressedBlock;
use crate::tool::ToolRegistry;
use iron_providers::Message;

fn estimate_tokens(text: &str) -> usize {
    (text.len() as f64 * 0.25).ceil() as usize
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextQuality {
    Exact,
    Estimated,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextCategory {
    Instructions,
    CompressedBlocks,
    RecentTail,
    ToolDefinitions,
    CurrentPrompt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextPressure {
    None,
    Soft,
    Medium,
    Strong,
    Critical,
}

impl ContextPressure {
    pub fn from_fullness(fullness: f64) -> Self {
        Self::from_fullness_with_thresholds(fullness, 0.50, 0.70, 0.85, 0.95)
    }

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

    pub fn as_str(&self) -> &'static str {
        match self {
            ContextPressure::None => "none",
            ContextPressure::Soft => "soft",
            ContextPressure::Medium => "medium",
            ContextPressure::Strong => "strong",
            ContextPressure::Critical => "critical",
        }
    }

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

#[derive(Debug, Clone)]
pub struct ContextCategoryUsage {
    pub category: ContextCategory,
    pub tokens: usize,
    pub quality: ContextQuality,
}

#[derive(Debug, Clone)]
pub struct ActiveContextSnapshot {
    pub total_tokens: usize,
    pub context_window_limit: Option<usize>,
    pub quality: ContextQuality,
    pub categories: Vec<ContextCategoryUsage>,
    /// Current model identifier for this session
    pub current_model: Option<String>,
    /// Number of model switches that have occurred
    pub model_switch_count: usize,
}

impl ActiveContextSnapshot {
    pub fn fullness(&self) -> Option<f64> {
        self.context_window_limit
            .filter(|limit| *limit > 0)
            .map(|limit| self.total_tokens as f64 / limit as f64)
    }

    pub fn pressure(&self) -> ContextPressure {
        self.pressure_with_thresholds(0.50, 0.70, 0.85, 0.95)
    }

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

pub struct SessionModelInfo<'a> {
    pub current_model: Option<&'a str>,
    pub model_switch_count: usize,
}

pub struct ActiveContextAccountant;

impl ActiveContextAccountant {
    pub fn estimate_snapshot(
        instructions: Option<&str>,
        compressed_blocks: &[CompressedBlock],
        tail_messages: &[Message],
        tool_registry: &ToolRegistry,
        current_prompt: Option<&str>,
        context_window_hint: Option<usize>,
        model_info: SessionModelInfo<'_>,
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

        ActiveContextSnapshot {
            total_tokens: total,
            context_window_limit: context_window_hint,
            quality: overall_quality,
            categories,
            current_model: model_info.current_model.map(|m| m.to_string()),
            model_switch_count: model_info.model_switch_count,
        }
    }

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

pub struct ContextTelemetry;

impl ContextTelemetry {
    pub fn for_session(
        instructions: Option<&str>,
        compressed_blocks: &[CompressedBlock],
        tail_messages: &[Message],
        tool_registry: &ToolRegistry,
        current_prompt: Option<&str>,
        context_window_hint: Option<usize>,
        model_info: SessionModelInfo<'_>,
    ) -> ActiveContextSnapshot {
        ActiveContextAccountant::estimate_snapshot(
            instructions,
            compressed_blocks,
            tail_messages,
            tool_registry,
            current_prompt,
            context_window_hint,
            model_info,
        )
    }
}
