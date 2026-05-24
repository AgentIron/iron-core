use crate::context::models::CompressedBlock;
use crate::durable::{DurableSession, TimelineEntry};
use crate::tool::ToolDefinition;
use serde_json::Value;
use std::collections::BTreeSet;

pub const COMPRESS_TOOL_NAME: &str = "compress";

/// Runtime-owned compress tool: validates ranges, applies compression, and
/// produces new compressed blocks.
pub struct CompressTool;

impl CompressTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition::new(
            COMPRESS_TOOL_NAME,
            "Compress resolved older conversation context. Your summaries permanently replace the selected ranges, so preserve all durable facts, decisions, constraints, file paths, errors, tool results, and user intent needed for future work.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "Short topic label for the compressed context."
                    },
                    "content": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "start_message_id": { "type": "string" },
                                "end_message_id": { "type": "string" },
                                "summary": {
                                    "type": "string",
                                    "description": "Durable replacement summary for this range."
                                }
                            },
                            "required": ["start_message_id", "end_message_id", "summary"]
                        }
                    }
                },
                "required": ["topic", "content"]
            }),
        )
    }

    pub fn parse_arguments(arguments: &Value) -> Result<(String, Vec<CompressRange>), String> {
        let topic = arguments
            .get("topic")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "Missing non-empty 'topic'".to_string())?
            .to_string();

        let content = arguments
            .get("content")
            .and_then(Value::as_array)
            .filter(|items| !items.is_empty())
            .ok_or_else(|| "Missing non-empty 'content' array".to_string())?;

        let mut ranges = Vec::with_capacity(content.len());
        for item in content {
            let start_id = item
                .get("start_message_id")
                .or_else(|| item.get("start_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Each content item requires 'start_message_id'".to_string())?;
            let end_id = item
                .get("end_message_id")
                .or_else(|| item.get("end_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Each content item requires 'end_message_id'".to_string())?;
            let summary = item
                .get("summary")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Each content item requires non-empty 'summary'".to_string())?;

            ranges.push(CompressRange {
                start_id: start_id.to_string(),
                end_id: end_id.to_string(),
                summary: summary.to_string(),
            });
        }

        Ok((topic, ranges))
    }

    /// Execute a compress request from the model.
    ///
    /// The model provides a topic and one or more source ranges with summaries.
    /// The runtime validates all ranges before mutating state.
    pub fn execute(
        session: &mut DurableSession,
        topic: String,
        ranges: Vec<CompressRange>,
        soft_threshold: f64,
        medium_threshold: f64,
        strong_threshold: f64,
        critical_threshold: f64,
    ) -> Result<CompressResult, String> {
        let resolved = Self::validate_ranges(session, &ranges)?;

        // Apply compression: remove selected entries, add compressed blocks
        let mut blocks_created = Vec::new();
        let mut positions_to_remove = BTreeSet::new();
        let mut block_ids_to_remove = BTreeSet::new();
        for resolved_range in &resolved {
            positions_to_remove.extend(resolved_range.timeline_positions.iter().copied());
            block_ids_to_remove.extend(resolved_range.block_ids.iter().cloned());
        }

        session.remove_timeline_positions(&positions_to_remove);
        if !block_ids_to_remove.is_empty() {
            session
                .compressed_blocks
                .retain(|block| !block_ids_to_remove.contains(&block.id));
        }

        for (range, resolved_range) in ranges.into_iter().zip(resolved.iter()) {
            let block = Self::create_block(session, &range, resolved_range, &topic);
            session.compressed_blocks.push(block.clone());
            blocks_created.push(block);
        }
        session.uncompacted_tokens = 0;

        Ok(CompressResult {
            blocks_created,
            pressure_state: Self::compute_pressure_state(
                session,
                soft_threshold,
                medium_threshold,
                strong_threshold,
                critical_threshold,
            ),
        })
    }

    fn validate_ranges(
        session: &DurableSession,
        ranges: &[CompressRange],
    ) -> Result<Vec<ResolvedRange>, String> {
        let mut resolved = Vec::with_capacity(ranges.len());
        let mut occupied = BTreeSet::new();
        for range in ranges {
            let resolved_range = Self::resolve_range(session, range)?;
            for key in resolved_range.logical_indexes() {
                if !occupied.insert(key) {
                    return Err(format!(
                        "Overlapping ranges include {}-{}",
                        range.start_id, range.end_id
                    ));
                }
            }
            resolved.push(resolved_range);
        }
        Ok(resolved)
    }

    fn resolve_range(
        session: &DurableSession,
        range: &CompressRange,
    ) -> Result<ResolvedRange, String> {
        let start = Self::logical_index(session, &range.start_id)
            .ok_or_else(|| format!("Unknown start ID: {}", range.start_id))?;
        let end = Self::logical_index(session, &range.end_id)
            .ok_or_else(|| format!("Unknown end ID: {}", range.end_id))?;
        if start > end {
            return Err(format!(
                "Invalid range: start {} comes after end {}",
                range.start_id, range.end_id
            ));
        }

        let mut resolved = ResolvedRange::default();
        for (block_index, block) in session.compressed_blocks.iter().enumerate() {
            let logical = block_index;
            if start <= logical && logical <= end {
                resolved.block_ids.insert(block.id.clone());
            }
        }

        let timeline_offset = session.compressed_blocks.len();
        for (timeline_index, entry) in session.timeline.iter().enumerate() {
            let logical = timeline_offset + timeline_index;
            if start <= logical && logical <= end {
                if Self::is_protected_timeline_entry(session, timeline_index, entry) {
                    return Err("Range includes protected active context (latest user request, current turn, pending tool, running tool, or pending approval)".into());
                }
                resolved.timeline_positions.insert(timeline_index);
            }
        }

        Self::validate_tool_pairs(session, &resolved.timeline_positions)?;
        Ok(resolved)
    }

    fn logical_index(session: &DurableSession, id: &str) -> Option<usize> {
        for (i, block) in session.compressed_blocks.iter().enumerate() {
            if block.id == id {
                return Some(i);
            }
        }
        let timeline_offset = session.compressed_blocks.len();
        for (i, entry) in session.timeline.iter().enumerate() {
            if entry.visible_id() == Some(id) {
                return Some(timeline_offset + i);
            }
        }
        None
    }

    fn is_protected_timeline_entry(
        session: &DurableSession,
        timeline_index: usize,
        entry: &TimelineEntry,
    ) -> bool {
        if timeline_index + 1 == session.timeline.len() {
            return true;
        }
        if matches!(entry, TimelineEntry::UserMessage { .. })
            && session.timeline[timeline_index + 1..]
                .iter()
                .all(|later| !matches!(later, TimelineEntry::UserMessage { .. }))
        {
            return true;
        }
        if let Some(record_index) = entry.tool_record_index() {
            return session
                .tool_records
                .get(record_index)
                .map(|record| !record.status.is_terminal())
                .unwrap_or(true);
        }
        false
    }

    fn validate_tool_pairs(
        session: &DurableSession,
        selected_positions: &BTreeSet<usize>,
    ) -> Result<(), String> {
        for (idx, entry) in session.timeline.iter().enumerate() {
            let TimelineEntry::ToolCallStarted { call_id, .. } = entry else {
                continue;
            };
            let terminal_index = session.timeline.iter().position(|candidate| {
                matches!(candidate, TimelineEntry::ToolCallTerminal { call_id: terminal_call_id, .. } if terminal_call_id == call_id)
            });
            let Some(terminal_index) = terminal_index else {
                if selected_positions.contains(&idx) {
                    return Err("Range includes a tool call without a terminal result".into());
                }
                continue;
            };
            let start_selected = selected_positions.contains(&idx);
            let terminal_selected = selected_positions.contains(&terminal_index);
            if start_selected != terminal_selected {
                return Err(
                    "Range would split a tool call from its result; include both or neither".into(),
                );
            }
        }
        Ok(())
    }

    fn create_block(
        session: &DurableSession,
        range: &CompressRange,
        resolved: &ResolvedRange,
        topic: &str,
    ) -> CompressedBlock {
        let block_id = format!("c{:04}", session.compressed_blocks.len() + 1);

        let mut block = CompressedBlock::new(
            block_id,
            topic,
            format!("{}-{}", range.start_id, range.end_id),
            range.summary.clone(),
        );
        let rough_source_items = resolved.timeline_positions.len() + resolved.block_ids.len();
        block.token_estimate_before = Some((rough_source_items as u32).saturating_mul(32));
        block.token_estimate_after = Some((range.summary.len() as f64 * 0.25).ceil() as u32);
        block
    }

    fn compute_pressure_state(
        session: &DurableSession,
        soft_threshold: f64,
        medium_threshold: f64,
        strong_threshold: f64,
        critical_threshold: f64,
    ) -> String {
        use crate::context::ActiveContextAccountant;
        use crate::tool::ToolRegistry;
        let messages = session.to_transcript().messages;
        let snapshot = ActiveContextAccountant::estimate_snapshot(
            session.instructions.as_deref(),
            &session.compressed_blocks,
            &messages,
            &ToolRegistry::new(),
            None,
            None,
            session.current_model.as_deref(),
            session.model_switch_history.len(),
        );
        let pressure = snapshot.pressure_with_thresholds(
            soft_threshold,
            medium_threshold,
            strong_threshold,
            critical_threshold,
        );
        pressure.as_str().to_string()
    }
}

#[derive(Debug, Default)]
struct ResolvedRange {
    timeline_positions: BTreeSet<usize>,
    block_ids: BTreeSet<String>,
}

impl ResolvedRange {
    fn logical_indexes(&self) -> BTreeSet<String> {
        self.timeline_positions
            .iter()
            .map(|idx| format!("m:{idx}"))
            .chain(self.block_ids.iter().map(|id| format!("c:{id}")))
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct CompressRange {
    pub start_id: String,
    pub end_id: String,
    pub summary: String,
}

#[derive(Debug, Clone)]
pub struct CompressResult {
    pub blocks_created: Vec<CompressedBlock>,
    pub pressure_state: String,
}
