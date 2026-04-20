use crate::context::models::CompactedContext;
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
    CompactedContext,
    RecentTail,
    ToolDefinitions,
    CurrentPrompt,
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
}

impl ActiveContextSnapshot {
    pub fn fullness(&self) -> Option<f64> {
        self.context_window_limit
            .filter(|limit| *limit > 0)
            .map(|limit| self.total_tokens as f64 / limit as f64)
    }
}

pub struct ActiveContextAccountant;

impl ActiveContextAccountant {
    pub fn estimate_snapshot(
        instructions: Option<&str>,
        compacted: Option<&CompactedContext>,
        tail_messages: &[Message],
        tool_registry: &ToolRegistry,
        current_prompt: Option<&str>,
        context_window_hint: Option<usize>,
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

        if let Some(ctx) = compacted {
            let rendered = ctx.render_to_text();
            if !rendered.is_empty() {
                let tokens = estimate_tokens(&rendered);
                total += tokens;
                categories.push(ContextCategoryUsage {
                    category: ContextCategory::CompactedContext,
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
        compacted: Option<&CompactedContext>,
        tail_messages: &[Message],
        tool_registry: &ToolRegistry,
        current_prompt: Option<&str>,
        context_window_hint: Option<usize>,
    ) -> ActiveContextSnapshot {
        ActiveContextAccountant::estimate_snapshot(
            instructions,
            compacted,
            tail_messages,
            tool_registry,
            current_prompt,
            context_window_hint,
        )
    }
}
