use agent_client_protocol::schema as acp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{btree_map::Entry, BTreeMap, BTreeSet, HashMap};

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

    pub fn from_acp_content(block: &acp::ContentBlock) -> Self {
        match block {
            acp::ContentBlock::Text(tc) => ContentBlock::Text {
                text: tc.text.clone(),
            },
            acp::ContentBlock::Image(ic) => ContentBlock::Image {
                data: ic.data.clone(),
                mime_type: ic.mime_type.clone(),
            },
            acp::ContentBlock::ResourceLink(rl) => ContentBlock::Resource {
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
        estimate_text_tokens(&self.text_content())
    }
}

fn estimate_text_tokens(text: &str) -> usize {
    (text.len() as f64 * 0.25).ceil() as usize
}

pub fn estimate_tool_call_tokens(tool_name: &str, arguments: &Value) -> usize {
    estimate_text_tokens(&format!("{}: {}", tool_name, arguments))
}

fn estimate_tool_result_tokens(tool_name: &str, result: &Value) -> usize {
    estimate_text_tokens(&format!("{}: {}", tool_name, result))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineEntry {
    UserMessage {
        index: u64,
        message_index: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        visible_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    AgentMessage {
        index: u64,
        message_index: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        visible_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    ToolCallStarted {
        index: u64,
        call_id: String,
        tool_name: String,
        tool_record_index: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        visible_id: Option<String>,
    },
    ToolCallTerminal {
        index: u64,
        call_id: String,
        tool_name: String,
        outcome: ToolTerminalOutcome,
        tool_record_index: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        visible_id: Option<String>,
    },
    ModelSwitched {
        index: u64,
        from_model: String,
        to_model: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        from_provider: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to_provider: Option<String>,
        adapted: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        visible_id: Option<String>,
    },
}

impl TimelineEntry {
    pub fn index(&self) -> u64 {
        match self {
            Self::UserMessage { index, .. }
            | Self::AgentMessage { index, .. }
            | Self::ToolCallStarted { index, .. }
            | Self::ToolCallTerminal { index, .. }
            | Self::ModelSwitched { index, .. } => *index,
        }
    }

    pub fn visible_id(&self) -> Option<&str> {
        match self {
            Self::UserMessage { visible_id, .. }
            | Self::AgentMessage { visible_id, .. }
            | Self::ToolCallStarted { visible_id, .. }
            | Self::ToolCallTerminal { visible_id, .. }
            | Self::ModelSwitched { visible_id, .. } => visible_id.as_deref(),
        }
    }

    pub fn set_visible_id(&mut self, id: String) {
        match self {
            Self::UserMessage { visible_id, .. }
            | Self::AgentMessage { visible_id, .. }
            | Self::ToolCallStarted { visible_id, .. }
            | Self::ToolCallTerminal { visible_id, .. }
            | Self::ModelSwitched { visible_id, .. } => {
                *visible_id = Some(id);
            }
        }
    }

    pub fn tool_record_index(&self) -> Option<usize> {
        match self {
            Self::ToolCallStarted {
                tool_record_index, ..
            }
            | Self::ToolCallTerminal {
                tool_record_index, ..
            } => Some(*tool_record_index),
            _ => None,
        }
    }

    pub fn is_model_switched(&self) -> bool {
        matches!(self, Self::ModelSwitched { .. })
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
    pub compressed_blocks: Vec<crate::context::models::CompressedBlock>,
    #[serde(default)]
    pub uncompacted_tokens: usize,
    #[serde(default)]
    pub repo_instruction_payload: Option<crate::prompt::config::RepoInstructionPayload>,
    /// Session-scoped MCP server enablement state.
    /// Maps MCP server IDs to whether they are enabled for this session.
    #[serde(default)]
    pub mcp_server_enablement: HashMap<String, bool>,
    /// Session-scoped plugin enablement state.
    /// Maps plugin IDs to whether they are enabled for this session.
    /// NOTE: This is excluded from handoff bundles (see handoff.rs).
    #[serde(default)]
    pub plugin_enablement: crate::plugin::session::SessionPluginEnablement,
    /// Session-scoped skill activation state.
    #[serde(default)]
    pub skill_state: crate::skill::SessionSkillState,
    /// Session-scoped snapshot of skills available for activation.
    #[serde(default)]
    pub available_skills: Vec<crate::skill::LoadedSkill>,
    /// Counter for generating stable visible timeline IDs.
    #[serde(default)]
    pub next_visible_id: u64,
    /// Current model identifier for this session
    #[serde(default)]
    pub current_model: Option<String>,
    /// Provider slug when the session is using a managed provider
    /// (set by a managed model switch)
    #[serde(default)]
    pub current_provider_slug: Option<String>,
    /// Optional API key for the current managed provider
    #[serde(default)]
    pub current_provider_api_key: Option<String>,
    /// History of model switches for this session
    #[serde(default)]
    pub model_switch_history: Vec<crate::context::model_switch::ModelSwitchRecord>,
    /// Tools hidden due to model capability differences
    #[serde(default)]
    pub hidden_tools: Vec<String>,
    /// Session-scoped active workspace roots.
    #[serde(default)]
    pub workspace_roots: Vec<std::path::PathBuf>,
    /// Pending workspace roots to be applied at the next turn boundary.
    #[serde(default)]
    pub pending_workspace_roots: Option<Vec<std::path::PathBuf>>,
    #[serde(skip, default)]
    pub token_tracker: crate::context::SessionTokenTracker,
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
            compressed_blocks: Vec::new(),
            uncompacted_tokens: 0,
            repo_instruction_payload: None,
            mcp_server_enablement: HashMap::new(),
            plugin_enablement: crate::plugin::session::SessionPluginEnablement::new(),
            skill_state: crate::skill::SessionSkillState::default(),
            available_skills: Vec::new(),
            next_visible_id: 1,
            current_model: None,
            current_provider_slug: None,
            current_provider_api_key: None,
            model_switch_history: Vec::new(),
            hidden_tools: Vec::new(),
            workspace_roots: Vec::new(),
            pending_workspace_roots: None,
            token_tracker: crate::context::SessionTokenTracker::default(),
        }
    }

    pub fn next_visible_id(&mut self) -> String {
        let id = format!("m{:04}", self.next_visible_id);
        self.next_visible_id += 1;
        id
    }

    pub fn add_user_text(&mut self, text: impl Into<String>) {
        let msg = StructuredMessage::User {
            content: vec![ContentBlock::text(text)],
        };
        let tokens = msg.estimated_tokens();
        let message_index = self.messages.len();
        self.messages.push(msg);
        let timeline_index = self.timeline.len() as u64;
        let visible_id = self.next_visible_id();
        self.timeline.push(TimelineEntry::UserMessage {
            index: timeline_index,
            message_index,
            visible_id: Some(visible_id),
            model: self.current_model.clone(),
        });
        self.uncompacted_tokens += tokens;
        self.token_tracker.add_delta(tokens);
    }

    pub fn add_user_message(&mut self, content: Vec<ContentBlock>) {
        let msg = StructuredMessage::User { content };
        let tokens = msg.estimated_tokens();
        let message_index = self.messages.len();
        self.messages.push(msg);
        let timeline_index = self.timeline.len() as u64;
        let visible_id = self.next_visible_id();
        self.timeline.push(TimelineEntry::UserMessage {
            index: timeline_index,
            message_index,
            visible_id: Some(visible_id),
            model: self.current_model.clone(),
        });
        self.uncompacted_tokens += tokens;
        self.token_tracker.add_delta(tokens);
    }

    pub fn add_agent_text(&mut self, text: impl Into<String>) {
        let msg = StructuredMessage::Agent {
            content: vec![ContentBlock::text(text)],
        };
        let tokens = msg.estimated_tokens();
        let message_index = self.messages.len();
        self.messages.push(msg);
        let timeline_index = self.timeline.len() as u64;
        let visible_id = self.next_visible_id();
        self.timeline.push(TimelineEntry::AgentMessage {
            index: timeline_index,
            message_index,
            visible_id: Some(visible_id),
            model: self.current_model.clone(),
        });
        self.uncompacted_tokens += tokens;
        self.token_tracker.add_delta(tokens);
    }

    pub fn add_agent_message(&mut self, content: Vec<ContentBlock>) {
        let msg = StructuredMessage::Agent { content };
        let tokens = msg.estimated_tokens();
        let message_index = self.messages.len();
        self.messages.push(msg);
        let timeline_index = self.timeline.len() as u64;
        let visible_id = self.next_visible_id();
        self.timeline.push(TimelineEntry::AgentMessage {
            index: timeline_index,
            message_index,
            visible_id: Some(visible_id),
            model: self.current_model.clone(),
        });
        self.uncompacted_tokens += tokens;
        self.token_tracker.add_delta(tokens);
    }

    /// Create the durable record for a tool call without updating token
    /// tracking.  Callers that need the delta recorded immediately should use
    /// [`propose_tool_call`]; stream processing defers delta until after the
    /// usage event to avoid losing it when `ProviderEvent::Usage` resets the
    /// baseline.
    pub fn propose_tool_call_without_delta(
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

        let visible_id = self.next_visible_id();
        self.timeline.push(TimelineEntry::ToolCallStarted {
            index: timeline_index,
            call_id,
            tool_name,
            tool_record_index: record_index,
            visible_id: Some(visible_id),
        });

        record_index
    }

    pub fn propose_tool_call(
        &mut self,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: Value,
    ) -> usize {
        let record_index = self.propose_tool_call_without_delta(call_id, tool_name, arguments);
        let tool_tokens = estimate_tool_call_tokens(
            &self.tool_records[record_index].tool_name,
            &self.tool_records[record_index].arguments,
        );
        self.uncompacted_tokens += tool_tokens;
        self.token_tracker.add_delta(tool_tokens);
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

        let visible_id = self.next_visible_id();
        self.timeline.push(TimelineEntry::ToolCallStarted {
            index: timeline_index,
            call_id,
            tool_name,
            tool_record_index: record_index,
            visible_id: Some(visible_id),
        });

        let tool_tokens = estimate_tool_call_tokens(
            &self.tool_records[record_index].tool_name,
            &self.tool_records[record_index].arguments,
        );
        self.uncompacted_tokens += tool_tokens;
        self.token_tracker.add_delta(tool_tokens);

        record_index
    }

    pub fn complete_tool_call(&mut self, call_id: &str, result: Value) {
        let idx = self.tool_records.iter().position(|r| r.call_id == call_id);
        if let Some(i) = idx {
            let (call_id_owned, tool_name_owned) = {
                let record = &self.tool_records[i];
                (record.call_id.clone(), record.tool_name.clone())
            };

            let record = &mut self.tool_records[i];
            record.status = ToolRecordStatus::Completed;
            record.result = Some(result);
            let timeline_index = self.timeline.len() as u64;
            record.timeline_terminal_index = Some(timeline_index);
            let tool_name = record.tool_name.clone();
            let result_ref = record.result.as_ref().unwrap().clone();

            let visible_id = self.next_visible_id();
            self.timeline.push(TimelineEntry::ToolCallTerminal {
                index: timeline_index,
                call_id: call_id_owned,
                tool_name: tool_name_owned,
                outcome: ToolTerminalOutcome::Completed,
                tool_record_index: i,
                visible_id: Some(visible_id),
            });

            let result_tokens = estimate_tool_result_tokens(&tool_name, &result_ref);
            self.uncompacted_tokens += result_tokens;
            self.token_tracker.add_delta(result_tokens);
        }
    }

    pub fn fail_tool_call(&mut self, call_id: &str, error: Value) {
        let idx = self.tool_records.iter().position(|r| r.call_id == call_id);
        if let Some(i) = idx {
            let (call_id_owned, tool_name_owned) = {
                let record = &self.tool_records[i];
                (record.call_id.clone(), record.tool_name.clone())
            };

            let record = &mut self.tool_records[i];
            record.status = ToolRecordStatus::Failed;
            record.result = Some(error);
            let timeline_index = self.timeline.len() as u64;
            record.timeline_terminal_index = Some(timeline_index);
            let tool_name = record.tool_name.clone();
            let result_ref = record.result.as_ref().unwrap().clone();

            let visible_id = self.next_visible_id();
            self.timeline.push(TimelineEntry::ToolCallTerminal {
                index: timeline_index,
                call_id: call_id_owned,
                tool_name: tool_name_owned,
                outcome: ToolTerminalOutcome::Failed,
                tool_record_index: i,
                visible_id: Some(visible_id),
            });

            let result_tokens = estimate_tool_result_tokens(&tool_name, &result_ref);
            self.uncompacted_tokens += result_tokens;
            self.token_tracker.add_delta(result_tokens);
        }
    }

    pub fn deny_tool_call(&mut self, call_id: &str) {
        let idx = self.tool_records.iter().position(|r| r.call_id == call_id);
        if let Some(i) = idx {
            let (call_id_owned, tool_name_owned) = {
                let record = &self.tool_records[i];
                (record.call_id.clone(), record.tool_name.clone())
            };

            let record = &mut self.tool_records[i];
            record.status = ToolRecordStatus::Denied;
            record.result = Some(serde_json::json!({"error": "denied by user"}));
            let timeline_index = self.timeline.len() as u64;
            record.timeline_terminal_index = Some(timeline_index);
            let tool_name = record.tool_name.clone();
            let result_ref = record.result.as_ref().unwrap().clone();

            let visible_id = self.next_visible_id();
            self.timeline.push(TimelineEntry::ToolCallTerminal {
                index: timeline_index,
                call_id: call_id_owned,
                tool_name: tool_name_owned,
                outcome: ToolTerminalOutcome::Denied,
                tool_record_index: i,
                visible_id: Some(visible_id),
            });

            let result_tokens = estimate_tool_result_tokens(&tool_name, &result_ref);
            self.uncompacted_tokens += result_tokens;
            self.token_tracker.add_delta(result_tokens);
        }
    }

    pub fn cancel_tool_call(&mut self, call_id: &str) {
        let idx = self.tool_records.iter().position(|r| r.call_id == call_id);
        if let Some(i) = idx {
            self.cancel_record_at(i, "cancelled");
        }
    }

    /// Transition every non-terminal tool record (`Running` or
    /// `PendingApproval`) to `Cancelled` atomically under the durable mutex.
    ///
    /// Why: the cancel path previously exited without tying off records whose
    /// tool futures were still in flight, so a subsequent resume or status
    /// query would observe a permanently-`Running` record. Because this method
    /// does not await, holding the durable mutex for its duration is safe and
    /// makes the transition atomic with respect to other session writes.
    ///
    /// Returns the list of `call_id`s that were transitioned, for logging.
    pub fn cancel_running_tool_calls(&mut self, reason: &str) -> Vec<String> {
        let indices: Vec<usize> = self
            .tool_records
            .iter()
            .enumerate()
            .filter_map(|(i, r)| {
                if matches!(
                    r.status,
                    ToolRecordStatus::Running | ToolRecordStatus::PendingApproval
                ) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();

        let mut cancelled = Vec::with_capacity(indices.len());
        for i in indices {
            let call_id = self.tool_records[i].call_id.clone();
            self.cancel_record_at(i, reason);
            cancelled.push(call_id);
        }
        cancelled
    }

    fn cancel_record_at(&mut self, i: usize, reason: &str) {
        let (call_id_owned, tool_name_owned) = {
            let record = &self.tool_records[i];
            if matches!(
                record.status,
                ToolRecordStatus::Completed
                    | ToolRecordStatus::Failed
                    | ToolRecordStatus::Denied
                    | ToolRecordStatus::Cancelled
            ) {
                return;
            }
            (record.call_id.clone(), record.tool_name.clone())
        };

        let record = &mut self.tool_records[i];
        record.status = ToolRecordStatus::Cancelled;
        record.result = Some(serde_json::json!({"error": reason}));
        let timeline_index = self.timeline.len() as u64;
        record.timeline_terminal_index = Some(timeline_index);
        let tool_name = record.tool_name.clone();
        let result_ref = record.result.as_ref().unwrap().clone();

        let visible_id = self.next_visible_id();
        self.timeline.push(TimelineEntry::ToolCallTerminal {
            index: timeline_index,
            call_id: call_id_owned,
            tool_name: tool_name_owned,
            outcome: ToolTerminalOutcome::Cancelled,
            tool_record_index: i,
            visible_id: Some(visible_id),
        });

        let result_tokens = estimate_tool_result_tokens(&tool_name, &result_ref);
        self.uncompacted_tokens += result_tokens;
        self.token_tracker.add_delta(result_tokens);
    }

    pub fn apply_compression(&mut self, block: crate::context::models::CompressedBlock) {
        self.compressed_blocks.push(block);
        self.uncompacted_tokens = 0;
        self.token_tracker.invalidate_baseline();
    }

    pub fn remove_timeline_positions(&mut self, positions: &BTreeSet<usize>) {
        if positions.is_empty() {
            return;
        }

        let retained_entries = self
            .timeline
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| {
                if positions.contains(&idx) {
                    None
                } else {
                    Some(entry.clone())
                }
            })
            .collect::<Vec<_>>();

        let mut message_map = BTreeMap::new();
        let mut messages = Vec::new();
        for entry in &retained_entries {
            let old_message_index = match entry {
                TimelineEntry::UserMessage { message_index, .. }
                | TimelineEntry::AgentMessage { message_index, .. } => Some(*message_index),
                _ => None,
            };
            if let Some(old_index) = old_message_index {
                if let Entry::Vacant(entry) = message_map.entry(old_index) {
                    if let Some(message) = self.messages.get(old_index).cloned() {
                        let new_index = messages.len();
                        messages.push(message);
                        entry.insert(new_index);
                    }
                }
            }
        }

        let mut tool_record_map = BTreeMap::new();
        let mut tool_records = Vec::new();
        for entry in &retained_entries {
            if let Some(old_index) = entry.tool_record_index() {
                if let Entry::Vacant(entry) = tool_record_map.entry(old_index) {
                    if let Some(record) = self.tool_records.get(old_index).cloned() {
                        let new_index = tool_records.len();
                        tool_records.push(record);
                        entry.insert(new_index);
                    }
                }
            }
        }

        let mut timeline = Vec::new();
        for (new_index, entry) in retained_entries.into_iter().enumerate() {
            let index = new_index as u64;
            match entry {
                TimelineEntry::UserMessage {
                    message_index,
                    visible_id,
                    model,
                    ..
                } => {
                    if let Some(mapped) = message_map.get(&message_index).copied() {
                        timeline.push(TimelineEntry::UserMessage {
                            index,
                            message_index: mapped,
                            visible_id,
                            model,
                        });
                    }
                }
                TimelineEntry::AgentMessage {
                    message_index,
                    visible_id,
                    model,
                    ..
                } => {
                    if let Some(mapped) = message_map.get(&message_index).copied() {
                        timeline.push(TimelineEntry::AgentMessage {
                            index,
                            message_index: mapped,
                            visible_id,
                            model,
                        });
                    }
                }
                TimelineEntry::ToolCallStarted {
                    call_id,
                    tool_name,
                    tool_record_index,
                    visible_id,
                    ..
                } => {
                    if let Some(mapped) = tool_record_map.get(&tool_record_index).copied() {
                        timeline.push(TimelineEntry::ToolCallStarted {
                            index,
                            call_id,
                            tool_name,
                            tool_record_index: mapped,
                            visible_id,
                        });
                    }
                }
                TimelineEntry::ToolCallTerminal {
                    call_id,
                    tool_name,
                    outcome,
                    tool_record_index,
                    visible_id,
                    ..
                } => {
                    if let Some(mapped) = tool_record_map.get(&tool_record_index).copied() {
                        timeline.push(TimelineEntry::ToolCallTerminal {
                            index,
                            call_id,
                            tool_name,
                            outcome,
                            tool_record_index: mapped,
                            visible_id,
                        });
                    }
                }
                TimelineEntry::ModelSwitched {
                    from_model,
                    to_model,
                    from_provider,
                    to_provider,
                    adapted,
                    visible_id,
                    ..
                } => {
                    timeline.push(TimelineEntry::ModelSwitched {
                        index,
                        from_model,
                        to_model,
                        from_provider,
                        to_provider,
                        adapted,
                        visible_id,
                    });
                }
            }
        }

        for record in &mut tool_records {
            record.timeline_started_index = None;
            record.timeline_terminal_index = None;
        }
        for entry in &timeline {
            match entry {
                TimelineEntry::ToolCallStarted {
                    index,
                    tool_record_index,
                    ..
                } => tool_records[*tool_record_index].timeline_started_index = Some(*index),
                TimelineEntry::ToolCallTerminal {
                    index,
                    tool_record_index,
                    ..
                } => tool_records[*tool_record_index].timeline_terminal_index = Some(*index),
                _ => {}
            }
        }

        self.messages = messages;
        self.tool_records = tool_records;
        self.timeline = timeline;
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

    // -- Skill activation helpers --

    pub fn activate_skill(
        &mut self,
        name: impl Into<String>,
        body: impl Into<String>,
        resources: Vec<crate::skill::SkillResourceEntry>,
    ) {
        let record = crate::skill::ActivatedSkillRecord {
            name: name.into(),
            body: body.into(),
            resources,
        };
        self.skill_state.activate(record);
        self.token_tracker.invalidate_baseline();
    }

    pub fn deactivate_skill(&mut self, name: &str) {
        self.skill_state.deactivate(name);
        self.token_tracker.invalidate_baseline();
    }

    pub fn list_active_skills(&self) -> Vec<&str> {
        self.skill_state.active_names()
    }

    pub fn active_skill_instructions(&self) -> String {
        self.skill_state.active_skill_instructions()
    }

    pub fn is_skill_active(&self, name: &str) -> bool {
        self.skill_state.is_active(name)
    }

    pub fn set_available_skills(&mut self, skills: Vec<crate::skill::LoadedSkill>) {
        self.available_skills = skills;
    }

    pub fn list_available_skills(&self) -> &[crate::skill::LoadedSkill] {
        &self.available_skills
    }

    pub fn load_available_skill(&self, name: &str) -> Option<crate::skill::LoadedSkill> {
        self.available_skills
            .iter()
            .find(|skill| skill.metadata.id == name)
            .cloned()
    }

    // -- Workspace root helpers --

    pub fn active_workspace_roots(&self) -> &[std::path::PathBuf] {
        &self.workspace_roots
    }

    pub fn set_pending_workspace_roots(&mut self, roots: Vec<std::path::PathBuf>) {
        self.pending_workspace_roots = Some(roots);
    }

    pub fn clear_pending_workspace_roots(&mut self) {
        self.pending_workspace_roots = None;
    }

    pub fn apply_pending_workspace_roots(&mut self) -> bool {
        if let Some(roots) = self.pending_workspace_roots.take() {
            self.workspace_roots = roots;
            self.token_tracker.invalidate_baseline();
            true
        } else {
            false
        }
    }

    pub fn to_transcript(&self) -> iron_providers::Transcript {
        self.to_transcript_with_visible_ids(false)
    }

    pub fn to_transcript_with_visible_ids(
        &self,
        include_visible_ids: bool,
    ) -> iron_providers::Transcript {
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
                        provider_messages.push(iron_providers::Message::User {
                            content: render_with_visible_id(entry, text, include_visible_ids),
                        });
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
                        provider_messages.push(iron_providers::Message::Assistant {
                            content: render_with_visible_id(entry, text, include_visible_ids),
                        });
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
                TimelineEntry::ModelSwitched { .. } => {
                    // Model switches are metadata, not provider-facing messages.
                    // They are intentionally excluded from the transcript sent to
                    // inference providers to avoid confusing the model with synthetic
                    // boundary markers. Switch history is available via
                    // DurableSession::model_switch_history for client-side rendering.
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
        self.token_tracker.invalidate_baseline();
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

    /// Enable or disable an MCP server for this session.
    pub fn set_mcp_server_enabled(&mut self, server_id: impl Into<String>, enabled: bool) {
        self.mcp_server_enablement.insert(server_id.into(), enabled);
        self.token_tracker.invalidate_baseline();
    }

    /// Check if an MCP server is enabled for this session.
    /// Returns None if not explicitly set.
    pub fn is_mcp_server_enabled(&self, server_id: &str) -> Option<bool> {
        self.mcp_server_enablement.get(server_id).copied()
    }

    /// Get list of MCP server IDs that are enabled for this session.
    pub fn list_enabled_mcp_servers(&self) -> Vec<String> {
        self.mcp_server_enablement
            .iter()
            .filter(|(_, &enabled)| enabled)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Enable or disable a plugin for this session.
    pub fn set_plugin_enabled(&mut self, plugin_id: impl Into<String>, enabled: bool) {
        self.plugin_enablement.set_enabled(plugin_id, enabled);
        self.token_tracker.invalidate_baseline();
    }

    /// Check if a plugin is enabled for this session.
    /// Returns None if not explicitly set.
    pub fn is_plugin_enabled(&self, plugin_id: &str) -> Option<bool> {
        self.plugin_enablement.is_enabled(plugin_id)
    }

    /// Get list of plugin IDs that are enabled for this session.
    pub fn list_enabled_plugins(&self) -> Vec<String> {
        self.plugin_enablement.list_enabled()
    }
}

pub type SharedDurableSession = std::sync::Arc<parking_lot::Mutex<DurableSession>>;

fn render_with_visible_id(
    entry: &TimelineEntry,
    text: String,
    include_visible_ids: bool,
) -> String {
    if include_visible_ids {
        if let Some(id) = entry.visible_id() {
            return format!("<{}>\n{}", id, text);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_session() -> DurableSession {
        DurableSession::new(SessionId(1))
    }

    #[test]
    fn cancel_running_transitions_running_and_pending() {
        let mut s = fresh_session();
        s.start_tool_call("a", "tool_a", serde_json::json!({}));
        s.start_tool_call("b", "tool_b", serde_json::json!({}));
        // Flip b to PendingApproval via request_tool_approval if exposed,
        // otherwise set directly for the test.
        s.tool_records[1].status = ToolRecordStatus::PendingApproval;

        let cancelled = s.cancel_running_tool_calls("cancelled");
        assert_eq!(cancelled.len(), 2);
        assert!(cancelled.contains(&"a".to_string()));
        assert!(cancelled.contains(&"b".to_string()));

        for record in &s.tool_records {
            assert!(matches!(record.status, ToolRecordStatus::Cancelled));
            assert!(record.timeline_terminal_index.is_some());
        }
    }

    #[test]
    fn cancel_running_skips_already_terminal_records() {
        let mut s = fresh_session();
        s.start_tool_call("done", "t", serde_json::json!({}));
        s.complete_tool_call("done", serde_json::json!({"ok": true}));

        s.start_tool_call("running", "t", serde_json::json!({}));

        let cancelled = s.cancel_running_tool_calls("cancelled");
        assert_eq!(cancelled, vec!["running".to_string()]);

        // Completed record unchanged.
        let done = s.tool_records.iter().find(|r| r.call_id == "done").unwrap();
        assert!(matches!(done.status, ToolRecordStatus::Completed));
    }

    #[test]
    fn cancel_running_with_no_running_is_noop() {
        let mut s = fresh_session();
        let cancelled = s.cancel_running_tool_calls("cancelled");
        assert!(cancelled.is_empty());
    }

    #[test]
    fn cancel_running_leaves_no_running_records_after() {
        let mut s = fresh_session();
        for i in 0..5 {
            s.start_tool_call(format!("c{}", i), "t", serde_json::json!({}));
        }
        s.cancel_running_tool_calls("cancelled");
        for record in &s.tool_records {
            assert!(
                !matches!(
                    record.status,
                    ToolRecordStatus::Running | ToolRecordStatus::PendingApproval
                ),
                "record {} left in non-terminal state after cancel",
                record.call_id
            );
        }
    }
}
