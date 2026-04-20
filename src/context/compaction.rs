use crate::context::config::TailRetentionRule;
use crate::context::models::CompactedContext;
use crate::durable::{DurableSession, StructuredMessage};
use iron_providers::{InferenceRequest, Message, Provider, ProviderEvent, ToolPolicy, Transcript};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionReason {
    Maintenance,
    HardFit,
    Handoff,
    Checkpoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionCheckpoint {
    TaskComplete,
    HandoffReadiness,
}

pub struct CompactionInput {
    pub prompt_text: String,
    pub tail: Vec<StructuredMessage>,
}

pub struct CompactionEngine;

impl CompactionEngine {
    pub fn should_compact(uncompacted_tokens: usize, threshold: usize, enabled: bool) -> bool {
        enabled && uncompacted_tokens >= threshold
    }

    pub fn split_session(
        session: &DurableSession,
        retention: &TailRetentionRule,
    ) -> (Vec<StructuredMessage>, Vec<StructuredMessage>) {
        let messages = &session.messages;
        let tail_count = match retention {
            TailRetentionRule::Messages(n) => *n,
            TailRetentionRule::Tokens(_max) => {
                let mut count = 0usize;
                let mut total_est = 0usize;
                for msg in messages.iter().rev() {
                    let est = estimate_structured_tokens(msg);
                    if total_est + est > *_max && count > 0 {
                        break;
                    }
                    total_est += est;
                    count += 1;
                }
                count.max(1)
            }
            TailRetentionRule::Policy(policy) => {
                let mut count = 0usize;
                let mut total_est = 0usize;
                for msg in messages.iter().rev() {
                    let est = estimate_structured_tokens(msg);
                    if let Some(max_tokens) = policy.max_tokens {
                        if total_est + est > max_tokens && count >= policy.min_messages {
                            break;
                        }
                    }
                    total_est += est;
                    count += 1;
                }
                count.max(policy.min_messages)
            }
        };

        let tail_count = tail_count.min(messages.len());
        let split_point = messages.len().saturating_sub(tail_count);

        let older: Vec<StructuredMessage> = messages[..split_point].to_vec();
        let tail: Vec<StructuredMessage> = messages[split_point..].to_vec();

        (older, tail)
    }

    pub fn build_compaction_input(
        previous: Option<&CompactedContext>,
        older_messages: &[Message],
        reason: CompactionReason,
    ) -> String {
        let mut parts = Vec::new();

        parts.push(format!(
            "You are a context compaction engine. Produce a structured summary of the session state.\nReason: {:?}",
            reason
        ));

        if let Some(prev) = previous {
            let rendered = prev.render_to_text();
            if !rendered.is_empty() {
                parts.push(format!("Previous compacted context:\n{}", rendered));
            }
        }

        if !older_messages.is_empty() {
            let text: Vec<String> = older_messages
                .iter()
                .enumerate()
                .map(|(i, msg)| format!("[{}] {}", i, render_provider_message(msg)))
                .collect();
            parts.push(format!("Raw material to summarize:\n{}", text.join("\n")));
        }

        parts.push(
            "Update the compacted context by replacing, rewriting, or removing superseded items. \
             Do not accumulate contradictions. \
             Output a JSON object matching the CompactedContext schema."
                .into(),
        );

        parts.join("\n\n")
    }

    pub fn parse_compacted_context(raw: &str) -> Result<CompactedContext, String> {
        let json_str = extract_json_object(raw);
        serde_json::from_str::<CompactedContext>(&json_str)
            .map_err(|e| format!("Failed to parse compacted context: {}", e))
    }

    pub fn prepare(
        session: &DurableSession,
        retention: &TailRetentionRule,
        reason: CompactionReason,
    ) -> CompactionInput {
        let (_older, tail) = Self::split_session(session, retention);
        let split_point = session.messages.len().saturating_sub(tail.len());
        let older_transcript = older_provider_messages(session, split_point);
        let prompt_text = Self::build_compaction_input(
            session.compacted_context.as_ref(),
            &older_transcript,
            reason,
        );
        CompactionInput { prompt_text, tail }
    }

    pub async fn execute(
        input: CompactionInput,
        provider: &dyn Provider,
        model: &str,
    ) -> Result<(CompactedContext, Vec<StructuredMessage>), String> {
        let transcript = Transcript::with_messages(vec![Message::user(input.prompt_text)]);

        let request = InferenceRequest::new(model, transcript).with_tool_policy(ToolPolicy::None);

        let events = provider
            .infer(request)
            .await
            .map_err(|e| format!("Compaction inference failed: {}", e))?;

        let mut output = String::new();
        for event in events {
            match event {
                ProviderEvent::Output { content } => output.push_str(&content),
                ProviderEvent::Error { source } => {
                    return Err(format!("Compaction provider error: {}", source));
                }
                _ => {}
            }
        }

        if output.trim().is_empty() {
            return Err("Compaction produced empty output".into());
        }

        let compacted = Self::parse_compacted_context(&output)?;
        Ok((compacted, input.tail))
    }

    pub fn reconstruct_messages(
        tail: &[StructuredMessage],
        compacted: &CompactedContext,
    ) -> Vec<StructuredMessage> {
        let mut result = Vec::new();

        let rendered = compacted.render_to_text();
        if !rendered.is_empty() {
            result.push(StructuredMessage::agent_text(format!(
                "[Compacted session context]\n{}",
                rendered
            )));
        }

        result.extend(tail.iter().cloned());
        result
    }
}

fn estimate_structured_tokens(msg: &StructuredMessage) -> usize {
    (msg.text_content().len() as f64 * 0.25).ceil() as usize
}

fn older_provider_messages(session: &DurableSession, split_point: usize) -> Vec<Message> {
    let first_retained_timeline_index = session.timeline.iter().find_map(|entry| match entry {
        crate::durable::TimelineEntry::UserMessage {
            index,
            message_index,
        }
        | crate::durable::TimelineEntry::AgentMessage {
            index,
            message_index,
        } if *message_index >= split_point => Some(*index),
        _ => None,
    });

    let mut messages = Vec::new();
    for entry in &session.timeline {
        if let Some(cutoff) = first_retained_timeline_index {
            if entry.index() >= cutoff {
                break;
            }
        }

        match entry {
            crate::durable::TimelineEntry::UserMessage { message_index, .. } => {
                if let Some(StructuredMessage::User { content }) =
                    session.messages.get(*message_index)
                {
                    let text = content
                        .iter()
                        .filter_map(|b| b.to_text())
                        .collect::<Vec<_>>()
                        .join("");
                    messages.push(Message::User { content: text });
                }
            }
            crate::durable::TimelineEntry::AgentMessage { message_index, .. } => {
                if let Some(StructuredMessage::Agent { content }) =
                    session.messages.get(*message_index)
                {
                    let text = content
                        .iter()
                        .filter_map(|b| b.to_text())
                        .collect::<Vec<_>>()
                        .join("");
                    messages.push(Message::Assistant { content: text });
                }
            }
            crate::durable::TimelineEntry::ToolCallStarted {
                tool_record_index, ..
            } => {
                if let Some(record) = session.tool_records.get(*tool_record_index) {
                    messages.push(Message::AssistantToolCall {
                        call_id: record.call_id.clone(),
                        tool_name: record.tool_name.clone(),
                        arguments: record.arguments.clone(),
                    });
                }
            }
            crate::durable::TimelineEntry::ToolCallTerminal {
                tool_record_index, ..
            } => {
                if let Some(record) = session.tool_records.get(*tool_record_index) {
                    if record.status.is_terminal() {
                        let result = record
                            .result
                            .clone()
                            .unwrap_or(serde_json::json!({"error": "no result"}));
                        messages.push(Message::Tool {
                            call_id: record.call_id.clone(),
                            tool_name: record.tool_name.clone(),
                            result,
                        });
                    }
                }
            }
        }
    }

    messages
}

fn render_provider_message(msg: &Message) -> String {
    match msg {
        Message::User { content } => format!("user: {}", content),
        Message::Assistant { content } => format!("assistant: {}", content),
        Message::AssistantToolCall {
            tool_name,
            arguments,
            ..
        } => format!("assistant_tool_call {}: {}", tool_name, arguments),
        Message::Tool {
            tool_name, result, ..
        } => format!("tool_result {}: {}", tool_name, result),
    }
}

fn extract_json_object(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(json) = try_extract_balanced_braces(trimmed) {
        return json;
    }
    if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    trimmed.to_string()
}

fn try_extract_balanced_braces(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let mut depth = 0i32;
    for (i, ch) in s[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..=start + i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}
