use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub u64);

impl SessionId {
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::SeqCst))
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session-{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum StructuredMessage {
    User { content: Vec<ContentBlock> },
    Agent { content: Vec<ContentBlock> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Image { data: String, mime_type: String },
    Resource { uri: String, name: Option<String> },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn to_text(&self) -> Option<&str> {
        match self {
            ContentBlock::Text { text } => Some(text),
            _ => None,
        }
    }

    pub fn from_acp_content(block: &agent_client_protocol::ContentBlock) -> Self {
        match block {
            agent_client_protocol::ContentBlock::Text(tc) => ContentBlock::Text {
                text: tc.text.clone(),
            },
            agent_client_protocol::ContentBlock::Image(ic) => ContentBlock::Image {
                data: ic.data.clone(),
                mime_type: ic.mime_type.clone(),
            },
            agent_client_protocol::ContentBlock::ResourceLink(rl) => ContentBlock::Resource {
                uri: rl.uri.clone(),
                name: Some(rl.name.clone()),
            },
            _ => ContentBlock::Text {
                text: "[unsupported content]".into(),
            },
        }
    }
}

impl StructuredMessage {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self::User {
            content: vec![ContentBlock::text(text)],
        }
    }

    pub fn agent_text(text: impl Into<String>) -> Self {
        Self::Agent {
            content: vec![ContentBlock::text(text)],
        }
    }

    pub fn text_content(&self) -> String {
        let blocks = match self {
            Self::User { content } => content,
            Self::Agent { content } => content,
        };
        blocks
            .iter()
            .filter_map(|b| b.to_text())
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn is_user(&self) -> bool {
        matches!(self, Self::User { .. })
    }

    pub fn is_agent(&self) -> bool {
        matches!(self, Self::Agent { .. })
    }

    pub fn content_blocks(&self) -> &[ContentBlock] {
        match self {
            Self::User { content } => content,
            Self::Agent { content } => content,
        }
    }

    pub fn estimated_tokens(&self) -> usize {
        (self.text_content().len() as f64 * 0.25).ceil() as usize
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineEntry {
    UserMessage {
        index: u64,
        message_index: usize,
    },
    AgentMessage {
        index: u64,
        message_index: usize,
    },
    ToolCallStarted {
        index: u64,
        call_id: String,
        tool_name: String,
        tool_record_index: usize,
    },
    ToolCallTerminal {
        index: u64,
        call_id: String,
        tool_name: String,
        outcome: ToolTerminalOutcome,
        tool_record_index: usize,
    },
}

impl TimelineEntry {
    pub fn index(&self) -> u64 {
        match self {
            Self::UserMessage { index, .. }
            | Self::AgentMessage { index, .. }
            | Self::ToolCallStarted { index, .. }
            | Self::ToolCallTerminal { index, .. } => *index,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ToolTerminalOutcome {
    Completed,
    Failed,
    Denied,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableToolRecord {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub status: ToolRecordStatus,
    pub result: Option<Value>,
    pub timeline_started_index: Option<u64>,
    pub timeline_terminal_index: Option<u64>,
    pub parent_script_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableScriptRecord {
    pub script_id: String,
    pub parent_call_id: String,
    pub script_source: String,
    pub input: Option<Value>,
    pub status: ScriptRecordStatus,
    pub result: Option<Value>,
    pub error: Option<Value>,
    pub child_call_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ScriptRecordStatus {
    Running,
    Completed,
    CompletedWithFailures,
    Failed,
    Cancelled,
}

impl DurableScriptRecord {
    pub fn new(
        script_id: impl Into<String>,
        parent_call_id: impl Into<String>,
        script_source: impl Into<String>,
        input: Option<Value>,
    ) -> Self {
        Self {
            script_id: script_id.into(),
            parent_call_id: parent_call_id.into(),
            script_source: script_source.into(),
            input,
            status: ScriptRecordStatus::Running,
            result: None,
            error: None,
            child_call_ids: Vec::new(),
        }
    }

    pub fn complete(&mut self, result: Value, child_call_ids: Vec<String>) {
        self.status = ScriptRecordStatus::Completed;
        self.result = Some(result);
        self.child_call_ids = child_call_ids;
    }

    pub fn complete_with_failures(&mut self, result: Value, child_call_ids: Vec<String>) {
        self.status = ScriptRecordStatus::CompletedWithFailures;
        self.result = Some(result);
        self.child_call_ids = child_call_ids;
    }

    pub fn fail(&mut self, error: Value, child_call_ids: Vec<String>) {
        self.status = ScriptRecordStatus::Failed;
        self.error = Some(error);
        self.child_call_ids = child_call_ids;
    }

    pub fn cancel(&mut self) {
        self.status = ScriptRecordStatus::Cancelled;
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            ScriptRecordStatus::Completed
                | ScriptRecordStatus::CompletedWithFailures
                | ScriptRecordStatus::Failed
                | ScriptRecordStatus::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolRecordStatus {
    PendingApproval,
    Running,
    Completed,
    Failed,
    Denied,
    Cancelled,
}

impl ToolRecordStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Denied | Self::Cancelled
        )
    }

    pub fn terminal_outcome(&self) -> Option<ToolTerminalOutcome> {
        match self {
            Self::Completed => Some(ToolTerminalOutcome::Completed),
            Self::Failed => Some(ToolTerminalOutcome::Failed),
            Self::Denied => Some(ToolTerminalOutcome::Denied),
            Self::Cancelled => Some(ToolTerminalOutcome::Cancelled),
            Self::PendingApproval | Self::Running => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableSession {
    pub id: SessionId,
    pub messages: Vec<StructuredMessage>,
    pub tool_records: Vec<DurableToolRecord>,
    pub timeline: Vec<TimelineEntry>,
    pub script_records: Vec<DurableScriptRecord>,
    pub instructions: Option<String>,
    pub workspace_scope: Option<String>,
    #[serde(default)]
    pub compacted_context: Option<crate::context::models::CompactedContext>,
    #[serde(default)]
    pub uncompacted_tokens: usize,
    #[serde(default)]
    pub repo_instruction_payload: Option<crate::prompt::config::RepoInstructionPayload>,
}

impl DurableSession {
    pub fn new(id: SessionId) -> Self {
        Self {
            id,
            messages: Vec::new(),
            tool_records: Vec::new(),
            timeline: Vec::new(),
            script_records: Vec::new(),
            instructions: None,
            workspace_scope: None,
            compacted_context: None,
            uncompacted_tokens: 0,
            repo_instruction_payload: None,
        }
    }

    pub fn add_user_text(&mut self, text: impl Into<String>) {
        let msg = StructuredMessage::User {
            content: vec![ContentBlock::text(text)],
        };
        let tokens = msg.estimated_tokens();
        let message_index = self.messages.len();
        self.messages.push(msg);
        let timeline_index = self.timeline.len() as u64;
        self.timeline.push(TimelineEntry::UserMessage {
            index: timeline_index,
            message_index,
        });
        self.uncompacted_tokens += tokens;
    }

    pub fn add_user_message(&mut self, content: Vec<ContentBlock>) {
        let msg = StructuredMessage::User { content };
        let tokens = msg.estimated_tokens();
        let message_index = self.messages.len();
        self.messages.push(msg);
        let timeline_index = self.timeline.len() as u64;
        self.timeline.push(TimelineEntry::UserMessage {
            index: timeline_index,
            message_index,
        });
        self.uncompacted_tokens += tokens;
    }

    pub fn add_agent_text(&mut self, text: impl Into<String>) {
        let msg = StructuredMessage::Agent {
            content: vec![ContentBlock::text(text)],
        };
        let tokens = msg.estimated_tokens();
        let message_index = self.messages.len();
        self.messages.push(msg);
        let timeline_index = self.timeline.len() as u64;
        self.timeline.push(TimelineEntry::AgentMessage {
            index: timeline_index,
            message_index,
        });
        self.uncompacted_tokens += tokens;
    }

    pub fn add_agent_message(&mut self, content: Vec<ContentBlock>) {
        let msg = StructuredMessage::Agent { content };
        let tokens = msg.estimated_tokens();
        let message_index = self.messages.len();
        self.messages.push(msg);
        let timeline_index = self.timeline.len() as u64;
        self.timeline.push(TimelineEntry::AgentMessage {
            index: timeline_index,
            message_index,
        });
        self.uncompacted_tokens += tokens;
    }

    pub fn propose_tool_call(
        &mut self,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: Value,
    ) -> usize {
        let call_id = call_id.into();
        let tool_name = tool_name.into();
        let record_index = self.tool_records.len();
        let timeline_index = self.timeline.len() as u64;

        self.tool_records.push(DurableToolRecord {
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
            arguments,
            status: ToolRecordStatus::PendingApproval,
            result: None,
            timeline_started_index: Some(timeline_index),
            timeline_terminal_index: None,
            parent_script_id: None,
        });

        self.timeline.push(TimelineEntry::ToolCallStarted {
            index: timeline_index,
            call_id,
            tool_name,
            tool_record_index: record_index,
        });

        record_index
    }

    pub fn start_tool_call(
        &mut self,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: Value,
    ) -> usize {
        let call_id = call_id.into();
        let tool_name = tool_name.into();

        let existing = self.tool_records.iter().position(|r| r.call_id == call_id);
        if let Some(i) = existing {
            let record = &mut self.tool_records[i];
            record.status = ToolRecordStatus::Running;
            return i;
        }

        let record_index = self.tool_records.len();
        let timeline_index = self.timeline.len() as u64;

        self.tool_records.push(DurableToolRecord {
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
            arguments,
            status: ToolRecordStatus::Running,
            result: None,
            timeline_started_index: Some(timeline_index),
            timeline_terminal_index: None,
            parent_script_id: None,
        });

        self.timeline.push(TimelineEntry::ToolCallStarted {
            index: timeline_index,
            call_id,
            tool_name,
            tool_record_index: record_index,
        });

        record_index
    }

    pub fn complete_tool_call(&mut self, call_id: &str, result: Value) {
        let idx = self.tool_records.iter().position(|r| r.call_id == call_id);
        if let Some(i) = idx {
            let record = &mut self.tool_records[i];
            record.status = ToolRecordStatus::Completed;
            record.result = Some(result);
            let timeline_index = self.timeline.len() as u64;
            record.timeline_terminal_index = Some(timeline_index);

            let call_id_owned = record.call_id.clone();
            let tool_name_owned = record.tool_name.clone();
            self.timeline.push(TimelineEntry::ToolCallTerminal {
                index: timeline_index,
                call_id: call_id_owned,
                tool_name: tool_name_owned,
                outcome: ToolTerminalOutcome::Completed,
                tool_record_index: i,
            });
        }
    }

    pub fn fail_tool_call(&mut self, call_id: &str, error: Value) {
        let idx = self.tool_records.iter().position(|r| r.call_id == call_id);
        if let Some(i) = idx {
            let record = &mut self.tool_records[i];
            record.status = ToolRecordStatus::Failed;
            record.result = Some(error);
            let timeline_index = self.timeline.len() as u64;
            record.timeline_terminal_index = Some(timeline_index);

            let call_id_owned = record.call_id.clone();
            let tool_name_owned = record.tool_name.clone();
            self.timeline.push(TimelineEntry::ToolCallTerminal {
                index: timeline_index,
                call_id: call_id_owned,
                tool_name: tool_name_owned,
                outcome: ToolTerminalOutcome::Failed,
                tool_record_index: i,
            });
        }
    }

    pub fn deny_tool_call(&mut self, call_id: &str) {
        let idx = self.tool_records.iter().position(|r| r.call_id == call_id);
        if let Some(i) = idx {
            let record = &mut self.tool_records[i];
            record.status = ToolRecordStatus::Denied;
            record.result = Some(serde_json::json!({"error": "denied by user"}));
            let timeline_index = self.timeline.len() as u64;
            record.timeline_terminal_index = Some(timeline_index);

            let call_id_owned = record.call_id.clone();
            let tool_name_owned = record.tool_name.clone();
            self.timeline.push(TimelineEntry::ToolCallTerminal {
                index: timeline_index,
                call_id: call_id_owned,
                tool_name: tool_name_owned,
                outcome: ToolTerminalOutcome::Denied,
                tool_record_index: i,
            });
        }
    }

    pub fn cancel_tool_call(&mut self, call_id: &str) {
        let idx = self.tool_records.iter().position(|r| r.call_id == call_id);
        if let Some(i) = idx {
            let record = &mut self.tool_records[i];
            record.status = ToolRecordStatus::Cancelled;
            record.result = Some(serde_json::json!({"error": "cancelled"}));
            let timeline_index = self.timeline.len() as u64;
            record.timeline_terminal_index = Some(timeline_index);

            let call_id_owned = record.call_id.clone();
            let tool_name_owned = record.tool_name.clone();
            self.timeline.push(TimelineEntry::ToolCallTerminal {
                index: timeline_index,
                call_id: call_id_owned,
                tool_name: tool_name_owned,
                outcome: ToolTerminalOutcome::Cancelled,
                tool_record_index: i,
            });
        }
    }

    pub fn apply_compaction(
        &mut self,
        compacted: crate::context::models::CompactedContext,
        retained_tail: Vec<StructuredMessage>,
    ) {
        self.compacted_context = Some(compacted);
        self.messages = retained_tail;
        self.uncompacted_tokens = 0;

        self.timeline.clear();
        for (i, msg) in self.messages.iter().enumerate() {
            let timeline_index = self.timeline.len() as u64;
            let entry = match msg {
                StructuredMessage::User { .. } => TimelineEntry::UserMessage {
                    index: timeline_index,
                    message_index: i,
                },
                StructuredMessage::Agent { .. } => TimelineEntry::AgentMessage {
                    index: timeline_index,
                    message_index: i,
                },
            };
            self.timeline.push(entry);
        }

        for (i, record) in self.tool_records.iter_mut().enumerate() {
            if record.status.is_terminal() {
                if let Some(_start_idx) = record.timeline_started_index {
                    let timeline_index = self.timeline.len() as u64;
                    self.timeline.push(TimelineEntry::ToolCallStarted {
                        index: timeline_index,
                        call_id: record.call_id.clone(),
                        tool_name: record.tool_name.clone(),
                        tool_record_index: i,
                    });
                    record.timeline_started_index = Some(timeline_index);
                }
                let outcome = record
                    .status
                    .terminal_outcome()
                    .unwrap_or(ToolTerminalOutcome::Completed);
                let timeline_index = self.timeline.len() as u64;
                self.timeline.push(TimelineEntry::ToolCallTerminal {
                    index: timeline_index,
                    call_id: record.call_id.clone(),
                    tool_name: record.tool_name.clone(),
                    outcome,
                    tool_record_index: i,
                });
                record.timeline_terminal_index = Some(timeline_index);
            }
        }
    }

    pub fn reset_uncompacted_tokens(&mut self) {
        self.uncompacted_tokens = 0;
    }

    pub fn is_idle(&self) -> bool {
        !self.tool_records.iter().any(|r| {
            matches!(
                r.status,
                ToolRecordStatus::PendingApproval | ToolRecordStatus::Running
            )
        })
    }

    pub fn to_transcript(&self) -> iron_providers::Transcript {
        let mut provider_messages = Vec::new();

        for entry in &self.timeline {
            match entry {
                TimelineEntry::UserMessage { message_index, .. } => {
                    if let Some(StructuredMessage::User { content }) =
                        self.messages.get(*message_index)
                    {
                        let text = content
                            .iter()
                            .filter_map(|b| b.to_text())
                            .collect::<Vec<_>>()
                            .join("");
                        provider_messages.push(iron_providers::Message::User { content: text });
                    }
                }
                TimelineEntry::AgentMessage { message_index, .. } => {
                    if let Some(StructuredMessage::Agent { content }) =
                        self.messages.get(*message_index)
                    {
                        let text = content
                            .iter()
                            .filter_map(|b| b.to_text())
                            .collect::<Vec<_>>()
                            .join("");
                        provider_messages
                            .push(iron_providers::Message::Assistant { content: text });
                    }
                }
                TimelineEntry::ToolCallStarted {
                    tool_record_index, ..
                } => {
                    if let Some(record) = self.tool_records.get(*tool_record_index) {
                        provider_messages.push(iron_providers::Message::AssistantToolCall {
                            call_id: record.call_id.clone(),
                            tool_name: record.tool_name.clone(),
                            arguments: record.arguments.clone(),
                        });
                    }
                }
                TimelineEntry::ToolCallTerminal {
                    tool_record_index, ..
                } => {
                    if let Some(record) = self.tool_records.get(*tool_record_index) {
                        if record.status.is_terminal() {
                            let result = record
                                .result
                                .clone()
                                .unwrap_or(serde_json::json!({"error": "no result"}));
                            provider_messages.push(iron_providers::Message::Tool {
                                call_id: record.call_id.clone(),
                                tool_name: record.tool_name.clone(),
                                result,
                            });
                        }
                    }
                }
            }
        }

        iron_providers::Transcript::with_messages(provider_messages)
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.tool_records.is_empty()
    }

    pub fn set_instructions(&mut self, instructions: impl Into<String>) {
        self.instructions = Some(instructions.into());
    }

    pub fn record_script_start(
        &mut self,
        script_id: impl Into<String>,
        call_id: impl Into<String>,
        source: impl Into<String>,
        input: Option<Value>,
    ) {
        self.script_records
            .push(DurableScriptRecord::new(script_id, call_id, source, input));
    }

    pub fn record_script_complete(
        &mut self,
        script_id: &str,
        result: Value,
        child_call_ids: Vec<String>,
    ) {
        if let Some(rec) = self
            .script_records
            .iter_mut()
            .find(|r| r.script_id == script_id)
        {
            rec.complete(result, child_call_ids);
        }
    }

    pub fn record_script_complete_with_failures(
        &mut self,
        script_id: &str,
        result: Value,
        child_call_ids: Vec<String>,
    ) {
        if let Some(rec) = self
            .script_records
            .iter_mut()
            .find(|r| r.script_id == script_id)
        {
            rec.complete_with_failures(result, child_call_ids);
        }
    }

    pub fn record_script_failed(&mut self, script_id: &str, error: Value) {
        if let Some(rec) = self
            .script_records
            .iter_mut()
            .find(|r| r.script_id == script_id)
        {
            rec.fail(error, Vec::new());
        }
    }

    pub fn record_script_cancelled(&mut self, script_id: &str) {
        if let Some(rec) = self
            .script_records
            .iter_mut()
            .find(|r| r.script_id == script_id)
        {
            rec.cancel();
        }
    }

    pub fn link_child_to_script(&mut self, script_id: &str, child_call_id: &str) {
        if let Some(rec) = self
            .script_records
            .iter_mut()
            .find(|r| r.script_id == script_id)
        {
            rec.child_call_ids.push(child_call_id.to_string());
        }
        if let Some(tool_rec) = self
            .tool_records
            .iter_mut()
            .find(|r| r.call_id == child_call_id)
        {
            tool_rec.parent_script_id = Some(script_id.to_string());
        }
    }
}

pub type SharedDurableSession = std::sync::Arc<std::sync::Mutex<DurableSession>>;
