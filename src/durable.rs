//! Durable, serializable session state and its lifecycle transitions.
//!
//! A [`DurableSession`] owns conversation messages, tool and script records,
//! timeline metadata, compacted summaries, model-switch history, and
//! session-scoped configuration. Transient work for an active turn belongs in
//! [`crate::ephemeral::EphemeralTurn`]. Provider usage establishes a token
//! baseline; newly appended visible content adds local deltas, while context
//! rewrites invalidate that baseline.

use agent_client_protocol::schema::v1 as acp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{btree_map::Entry, BTreeMap, BTreeSet, HashMap};

/// Process-local numeric identity for a durable session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionId(
    /// Numeric session identifier.
    pub u64,
);

impl SessionId {
    /// Allocates the next process-local session identifier.
    ///
    /// Identifiers begin at one and are unique until the underlying counter
    /// wraps. Deserializing an identifier does not advance the allocator.
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

/// Structured user or agent content retained by a durable session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum StructuredMessage {
    /// Content supplied by the user.
    User {
        /// Ordered content blocks in the message.
        content: Vec<ContentBlock>,
    },
    /// Content produced by the agent.
    Agent {
        /// Ordered content blocks in the message.
        content: Vec<ContentBlock>,
    },
}

/// Serializable content retained in a structured message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text content.
    Text {
        /// Text payload.
        text: String,
    },
    /// Encoded image content.
    Image {
        /// Encoded image data.
        data: String,
        /// Media type describing `data`.
        mime_type: String,
    },
    /// Link to an external resource.
    Resource {
        /// Resource URI.
        uri: String,
        /// Optional display name.
        name: Option<String>,
    },
}

impl ContentBlock {
    /// Creates a text content block.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Returns the text payload, or `None` for non-text content.
    pub fn to_text(&self) -> Option<&str> {
        match self {
            ContentBlock::Text { text } => Some(text),
            _ => None,
        }
    }

    /// Converts an ACP content block into durable content.
    ///
    /// ACP variants without a durable representation become the literal text
    /// `[unsupported content]`.
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
    /// Creates a user message containing one text block.
    pub fn user_text(text: impl Into<String>) -> Self {
        Self::User {
            content: vec![ContentBlock::text(text)],
        }
    }

    /// Creates an agent message containing one text block.
    pub fn agent_text(text: impl Into<String>) -> Self {
        Self::Agent {
            content: vec![ContentBlock::text(text)],
        }
    }

    /// Concatenates all text blocks, omitting images and resources.
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

    /// Returns whether this is a user message.
    pub fn is_user(&self) -> bool {
        matches!(self, Self::User { .. })
    }

    /// Returns whether this is an agent message.
    pub fn is_agent(&self) -> bool {
        matches!(self, Self::Agent { .. })
    }

    /// Borrows the ordered content blocks regardless of message role.
    pub fn content_blocks(&self) -> &[ContentBlock] {
        match self {
            Self::User { content } => content,
            Self::Agent { content } => content,
        }
    }

    /// Estimates text tokens at one token per four UTF-8 bytes, rounded up.
    ///
    /// Non-text blocks do not contribute to this estimate.
    pub fn estimated_tokens(&self) -> usize {
        estimate_text_tokens(&self.text_content())
    }
}

fn estimate_text_tokens(text: &str) -> usize {
    (text.len() as f64 * 0.25).ceil() as usize
}

/// Estimates tokens for a serialized tool name and argument payload.
pub fn estimate_tool_call_tokens(tool_name: &str, arguments: &Value) -> usize {
    estimate_text_tokens(&format!("{}: {}", tool_name, arguments))
}

fn estimate_tool_result_tokens(tool_name: &str, result: &Value) -> usize {
    estimate_text_tokens(&format!("{}: {}", tool_name, result))
}

/// Ordered metadata linking durable messages, tools, and model switches.
///
/// `index` values are positions in the current timeline and may be rewritten
/// by compaction. `visible_id` values are stable user-facing range selectors.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineEntry {
    /// A user message retained in the session.
    UserMessage {
        /// Current zero-based timeline position.
        index: u64,
        /// Index into [`DurableSession::messages`].
        message_index: usize,
        /// Stable user-facing identifier used by compaction ranges.
        #[serde(skip_serializing_if = "Option::is_none")]
        visible_id: Option<String>,
        /// Model active when the message was recorded.
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    /// An agent message retained in the session.
    AgentMessage {
        /// Current zero-based timeline position.
        index: u64,
        /// Index into [`DurableSession::messages`].
        message_index: usize,
        /// Stable user-facing identifier used by compaction ranges.
        #[serde(skip_serializing_if = "Option::is_none")]
        visible_id: Option<String>,
        /// Model active when the message was recorded.
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    /// Start boundary of a tool-call lifecycle.
    ToolCallStarted {
        /// Current zero-based timeline position.
        index: u64,
        /// Provider-assigned tool-call identifier.
        call_id: String,
        /// Requested tool name.
        tool_name: String,
        /// Index into [`DurableSession::tool_records`].
        tool_record_index: usize,
        /// Stable user-facing identifier used by compaction ranges.
        #[serde(skip_serializing_if = "Option::is_none")]
        visible_id: Option<String>,
    },
    /// Terminal boundary of a tool-call lifecycle.
    ToolCallTerminal {
        /// Current zero-based timeline position.
        index: u64,
        /// Provider-assigned tool-call identifier.
        call_id: String,
        /// Requested tool name.
        tool_name: String,
        /// Terminal outcome represented by this boundary.
        outcome: ToolTerminalOutcome,
        /// Index into [`DurableSession::tool_records`].
        tool_record_index: usize,
        /// Stable user-facing identifier used by compaction ranges.
        #[serde(skip_serializing_if = "Option::is_none")]
        visible_id: Option<String>,
    },
    /// Metadata boundary recording an applied model switch.
    ///
    /// This entry is excluded from provider transcripts.
    ModelSwitched {
        /// Current zero-based timeline position.
        index: u64,
        /// Model active before the switch.
        from_model: String,
        /// Model active after the switch.
        to_model: String,
        /// Provider active before the switch, if known.
        #[serde(skip_serializing_if = "Option::is_none")]
        from_provider: Option<String>,
        /// Provider active after the switch, if known.
        #[serde(skip_serializing_if = "Option::is_none")]
        to_provider: Option<String>,
        /// Whether context or capabilities were adapted for the target.
        adapted: bool,
        /// Optional stable user-facing identifier.
        #[serde(skip_serializing_if = "Option::is_none")]
        visible_id: Option<String>,
    },
}

impl TimelineEntry {
    /// Returns the entry's current timeline position.
    pub fn index(&self) -> u64 {
        match self {
            Self::UserMessage { index, .. }
            | Self::AgentMessage { index, .. }
            | Self::ToolCallStarted { index, .. }
            | Self::ToolCallTerminal { index, .. }
            | Self::ModelSwitched { index, .. } => *index,
        }
    }

    /// Borrows the stable user-facing identifier, when assigned.
    pub fn visible_id(&self) -> Option<&str> {
        match self {
            Self::UserMessage { visible_id, .. }
            | Self::AgentMessage { visible_id, .. }
            | Self::ToolCallStarted { visible_id, .. }
            | Self::ToolCallTerminal { visible_id, .. }
            | Self::ModelSwitched { visible_id, .. } => visible_id.as_deref(),
        }
    }

    /// Assigns or replaces the stable user-facing identifier.
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

    /// Returns the linked tool-record index for tool lifecycle entries.
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

    /// Returns whether this entry records a model switch.
    pub fn is_model_switched(&self) -> bool {
        matches!(self, Self::ModelSwitched { .. })
    }
}

/// Terminal state represented by a tool timeline boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ToolTerminalOutcome {
    /// The tool returned successfully.
    Completed,
    /// Tool execution failed.
    Failed,
    /// Permission to run the tool was denied.
    Denied,
    /// The tool call was cancelled.
    Cancelled,
}

/// Durable state and timeline links for one tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableToolRecord {
    /// Provider-assigned call identifier.
    pub call_id: String,
    /// Requested tool name.
    pub tool_name: String,
    /// Arguments supplied to the tool.
    pub arguments: Value,
    /// Current lifecycle state.
    pub status: ToolRecordStatus,
    /// Terminal result or error payload, when available.
    pub result: Option<Value>,
    /// Timeline index at which the call was proposed or started.
    pub timeline_started_index: Option<u64>,
    /// Timeline index at which the call became terminal.
    pub timeline_terminal_index: Option<u64>,
    /// Script record that owns this child call, when linked.
    pub parent_script_id: Option<String>,
}

/// Durable state for a script that may own multiple child tool calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableScriptRecord {
    /// Stable script execution identifier.
    pub script_id: String,
    /// Tool call that launched the script.
    pub parent_call_id: String,
    /// Source text executed by the script tool.
    pub script_source: String,
    /// Optional script input payload.
    pub input: Option<Value>,
    /// Current script lifecycle state.
    pub status: ScriptRecordStatus,
    /// Successful or partial result payload.
    pub result: Option<Value>,
    /// Failure payload.
    pub error: Option<Value>,
    /// Tool-call identifiers launched by the script.
    pub child_call_ids: Vec<String>,
}

/// Lifecycle state of a durable script execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ScriptRecordStatus {
    /// Script execution is active.
    Running,
    /// Script and all relevant children completed successfully.
    Completed,
    /// Script produced a result while one or more children failed.
    CompletedWithFailures,
    /// Script execution failed.
    Failed,
    /// Script execution was cancelled.
    Cancelled,
}

impl DurableScriptRecord {
    /// Creates a running script record with no result, error, or children.
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

    /// Marks the script completed and replaces its result and child list.
    pub fn complete(&mut self, result: Value, child_call_ids: Vec<String>) {
        self.status = ScriptRecordStatus::Completed;
        self.result = Some(result);
        self.child_call_ids = child_call_ids;
    }

    /// Marks the script completed with child failures and stores its result.
    pub fn complete_with_failures(&mut self, result: Value, child_call_ids: Vec<String>) {
        self.status = ScriptRecordStatus::CompletedWithFailures;
        self.result = Some(result);
        self.child_call_ids = child_call_ids;
    }

    /// Marks the script failed and stores its error and child list.
    pub fn fail(&mut self, error: Value, child_call_ids: Vec<String>) {
        self.status = ScriptRecordStatus::Failed;
        self.error = Some(error);
        self.child_call_ids = child_call_ids;
    }

    /// Marks the script cancelled without clearing prior result fields.
    pub fn cancel(&mut self) {
        self.status = ScriptRecordStatus::Cancelled;
    }

    /// Returns whether the script has reached any terminal state.
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

/// Lifecycle state of a durable tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolRecordStatus {
    /// The call is recorded but awaits a permission decision.
    PendingApproval,
    /// Tool execution is active.
    Running,
    /// Tool execution returned successfully.
    Completed,
    /// Tool execution failed.
    Failed,
    /// Permission to execute was denied.
    Denied,
    /// The call was cancelled before normal completion.
    Cancelled,
}

impl ToolRecordStatus {
    /// Returns whether no further lifecycle transition is expected.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Denied | Self::Cancelled
        )
    }

    /// Maps a terminal status to its timeline outcome.
    ///
    /// Returns `None` for pending and running calls.
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

/// Serializable owner of durable state for one agent session.
///
/// Messages and tool records are stored separately from their ordered
/// [`TimelineEntry`] links. Compaction may remove and reindex those stores while
/// preserving stable visible IDs for retained entries. [`Self::token_tracker`]
/// is runtime-only and is skipped during serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableSession {
    /// Stable identity of this session.
    pub id: SessionId,
    /// Structured user and agent message storage referenced by the timeline.
    pub messages: Vec<StructuredMessage>,
    /// Tool lifecycle records referenced by tool timeline entries.
    pub tool_records: Vec<DurableToolRecord>,
    /// Ordered provider-facing history and model-switch metadata.
    pub timeline: Vec<TimelineEntry>,
    /// Script lifecycle records and their child tool links.
    pub script_records: Vec<DurableScriptRecord>,
    /// Explicit session instructions, excluding rendered profile identity.
    pub instructions: Option<String>,
    /// Optional serialized description of the session workspace scope.
    pub workspace_scope: Option<String>,
    /// Durable summaries that replace compacted context ranges.
    #[serde(default)]
    pub compressed_blocks: Vec<crate::context::models::CompressedBlock>,
    /// Heuristic token count added since the last compaction reset.
    #[serde(default)]
    pub uncompacted_tokens: usize,
    /// Resolved repository instructions included in prompt construction.
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
    /// Current model identifier for this session.
    #[serde(default)]
    pub current_model: Option<String>,
    /// Provider slug when the session is using a managed provider.
    #[serde(default)]
    pub current_provider_slug: Option<String>,
    /// Optional API key for the current managed provider.
    ///
    /// Stored in a [`crate::secret::SecretString`] so the value is redacted from
    /// debug output while still serializing for durable persistence and handoff.
    #[serde(default)]
    pub current_provider_api_key: Option<crate::secret::SecretString>,
    /// History of applied model switches for this session.
    #[serde(default)]
    pub model_switch_history: Vec<crate::context::model_switch::ModelSwitchRecord>,
    /// Tools hidden due to current-model capability differences.
    #[serde(default)]
    pub hidden_tools: Vec<String>,
    /// Session-scoped active workspace roots.
    #[serde(default)]
    pub workspace_roots: Vec<std::path::PathBuf>,
    /// Pending workspace roots to be applied at the next turn boundary.
    #[serde(default)]
    pub pending_workspace_roots: Option<Vec<std::path::PathBuf>>,
    /// The profile id last used for this session, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<crate::profile::AgentProfileId>,
    /// Profile identity prompt selected for this session, if any.
    /// Rendered in `## 1. Identity` instead of client/session injection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_identity: Option<String>,
    /// Session-effective snapshot of the profile's tool filter at setup time.
    /// `None` means the profile used `Inherit` (or no profile was selected).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_tool_filter: Option<crate::profile::ToolFilter>,
    /// Session-effective snapshot of the profile's approval posture at setup time.
    /// `None` means the profile used `PerTool` (or no profile was selected).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_approval: Option<crate::profile::AgentApproval>,
    /// Session-effective snapshot of the resolved provider context at setup time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_provider_context:
        Option<crate::provider_credential::domain::ProviderPromptContext>,
    /// Session-effective snapshot of the resolved model at setup time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_model: Option<String>,
    /// Whether the session was created with a profile that is no longer available.
    /// Stored as a diagnostic; the session continues with its snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_unavailable: Option<String>,
    /// Runtime token baseline and accumulated provider usage.
    ///
    /// This field is not serialized; a restored session starts without a
    /// provider baseline and must use heuristic accounting until resynchronized.
    #[serde(skip, default)]
    pub token_tracker: crate::context::SessionTokenTracker,
}

impl DurableSession {
    /// Creates an empty durable session with the supplied identity.
    ///
    /// Visible timeline IDs begin at `m0001`, and token accounting starts with
    /// no provider baseline.
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
            profile_id: None,
            profile_identity: None,
            effective_tool_filter: None,
            effective_approval: None,
            effective_provider_context: None,
            effective_model: None,
            profile_unavailable: None,
            token_tracker: crate::context::SessionTokenTracker::default(),
        }
    }

    /// Allocates the next stable visible timeline ID.
    pub fn next_visible_id(&mut self) -> String {
        let id = format!("m{:04}", self.next_visible_id);
        self.next_visible_id += 1;
        id
    }

    /// Appends a one-block user message and records its estimated token delta.
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

    /// Appends a structured user message and records its text token delta.
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

    /// Appends a one-block agent message and records its estimated token delta.
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

    /// Appends a structured agent message and records its text token delta.
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
    /// [`Self::propose_tool_call`]; stream processing defers delta until after the
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

    /// Records a tool call pending approval and accounts for its arguments.
    ///
    /// Returns the new record's index. Use
    /// [`Self::propose_tool_call_without_delta`] when provider usage ordering
    /// requires the local token delta to be applied later.
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

    /// Marks an existing call running or creates a new running call.
    ///
    /// An existing record is selected by `call_id`; only its status changes and
    /// no timeline entry or token delta is added. A new call receives a start
    /// entry and contributes its estimated arguments to token accounting.
    /// Returns the record index in either case.
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

    /// Completes the first tool record matching `call_id`.
    ///
    /// The result is stored, a terminal timeline entry is appended, and its
    /// estimated token delta is recorded. An unknown ID is a no-op.
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

    /// Fails the first tool record matching `call_id`.
    ///
    /// The error is stored as the record result, a terminal timeline entry is
    /// appended, and its estimated token delta is recorded. An unknown ID is a
    /// no-op.
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

    /// Denies the first tool record matching `call_id`.
    ///
    /// A synthetic denial result and terminal timeline entry are added to the
    /// provider-visible context. An unknown ID is a no-op.
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

    /// Cancels the first tool record matching `call_id` if it is non-terminal.
    ///
    /// Cancellation adds a synthetic result and token delta. Unknown or
    /// already terminal records are unchanged.
    pub fn cancel_tool_call(&mut self, call_id: &str) {
        let idx = self.tool_records.iter().position(|r| r.call_id == call_id);
        if let Some(i) = idx {
            self.cancel_record_at(i, "cancelled");
        }
    }

    /// Transitions every non-terminal tool record to `Cancelled`.
    ///
    /// Each transition appends a terminal timeline result and token delta.
    /// The returned IDs preserve tool-record order. Callers holding the shared
    /// durable mutex can perform the whole operation without an await point.
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

    /// Appends an externally created compressed block and resets compaction age.
    ///
    /// This helper does not remove source timeline entries. It resets
    /// `uncompacted_tokens` and invalidates provider-baseline accounting because
    /// adding the rendered block changes provider-visible context.
    pub fn apply_compression(&mut self, block: crate::context::models::CompressedBlock) {
        self.compressed_blocks.push(block);
        self.uncompacted_tokens = 0;
        self.token_tracker.invalidate_baseline();
    }

    /// Removes selected timeline positions and rebuilds referenced storage.
    ///
    /// Retained messages and tool records are compacted into new vectors;
    /// timeline indexes and tool boundary indexes are rewritten accordingly.
    /// Script records are unaffected. Invalid positions are ignored, and a
    /// nonempty request invalidates the provider token baseline.
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
        self.token_tracker.invalidate_baseline();
    }

    /// Resets the heuristic count of tokens added since compaction.
    ///
    /// This does not alter provider baseline/delta accounting.
    pub fn reset_uncompacted_tokens(&mut self) {
        self.uncompacted_tokens = 0;
    }

    /// Returns whether no tool call is pending approval or running.
    pub fn is_idle(&self) -> bool {
        !self.tool_records.iter().any(|r| {
            matches!(
                r.status,
                ToolRecordStatus::PendingApproval | ToolRecordStatus::Running
            )
        })
    }

    // -- Skill activation helpers --

    /// Activates or replaces a session skill and invalidates token baseline.
    ///
    /// Active skill instructions are provider-visible prompt content.
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

    /// Deactivates a named skill and invalidates token baseline.
    pub fn deactivate_skill(&mut self, name: &str) {
        self.skill_state.deactivate(name);
        self.token_tracker.invalidate_baseline();
    }

    /// Returns active skill names in session-defined order.
    pub fn list_active_skills(&self) -> Vec<&str> {
        self.skill_state.active_names()
    }

    /// Renders all active skill instructions for prompt inclusion.
    pub fn active_skill_instructions(&self) -> String {
        self.skill_state.active_skill_instructions()
    }

    /// Returns whether a skill with `name` is active.
    pub fn is_skill_active(&self, name: &str) -> bool {
        self.skill_state.is_active(name)
    }

    /// Replaces the session snapshot of skills available for activation.
    pub fn set_available_skills(&mut self, skills: Vec<crate::skill::LoadedSkill>) {
        self.available_skills = skills;
    }

    /// Borrows the session snapshot of available skills.
    pub fn list_available_skills(&self) -> &[crate::skill::LoadedSkill] {
        &self.available_skills
    }

    /// Clones an available skill whose metadata ID matches `name`.
    pub fn load_available_skill(&self, name: &str) -> Option<crate::skill::LoadedSkill> {
        self.available_skills
            .iter()
            .find(|skill| skill.metadata.id == name)
            .cloned()
    }

    // -- Workspace root helpers --

    /// Borrows workspace roots active for the current turn.
    pub fn active_workspace_roots(&self) -> &[std::path::PathBuf] {
        &self.workspace_roots
    }

    /// Stages replacement workspace roots for the next turn boundary.
    pub fn set_pending_workspace_roots(&mut self, roots: Vec<std::path::PathBuf>) {
        self.pending_workspace_roots = Some(roots);
    }

    /// Discards workspace roots staged for the next turn boundary.
    pub fn clear_pending_workspace_roots(&mut self) {
        self.pending_workspace_roots = None;
    }

    /// Applies staged workspace roots at a turn boundary.
    ///
    /// Returns whether roots were staged. Applying them invalidates provider
    /// token accounting because workspace-derived prompt context may change.
    pub fn apply_pending_workspace_roots(&mut self) -> bool {
        if let Some(roots) = self.pending_workspace_roots.take() {
            self.workspace_roots = roots;
            self.token_tracker.invalidate_baseline();
            true
        } else {
            false
        }
    }

    /// Builds a provider transcript without stable visible-ID prefixes.
    ///
    /// Model-switch entries are metadata and are omitted.
    pub fn to_transcript(&self) -> iron_providers::Transcript {
        self.to_transcript_with_visible_ids(false)
    }

    /// Builds a provider transcript from the ordered durable timeline.
    ///
    /// When `include_visible_ids` is true, text messages are prefixed with
    /// their stable IDs. Non-text message blocks are omitted, terminal tool
    /// records become tool messages, and model-switch metadata is excluded.
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

    /// Returns whether the session has no messages or tool records.
    ///
    /// Compressed blocks, scripts, and model-switch metadata do not affect this
    /// predicate.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.tool_records.is_empty()
    }

    /// Replaces explicit session instructions and invalidates token baseline.
    pub fn set_instructions(&mut self, instructions: impl Into<String>) {
        self.instructions = Some(instructions.into());
        self.token_tracker.invalidate_baseline();
    }

    /// Sets the profile identity, treating blank text as absent.
    ///
    /// The rendered identity is provider-visible, so this invalidates token
    /// baseline accounting.
    pub fn set_profile_identity(&mut self, identity: impl Into<String>) {
        let value = identity.into();
        if value.trim().is_empty() {
            self.profile_identity = None;
        } else {
            self.profile_identity = Some(value);
        }
        self.token_tracker.invalidate_baseline();
    }

    /// Combine rendered identity and explicit session instructions for token
    /// accounting. This mirrors system prompt rendering: a missing or blank
    /// profile identity still renders the core fallback identity in Section 1.
    pub fn instruction_text_for_estimate(&self) -> Option<String> {
        let identity = self
            .profile_identity
            .as_deref()
            .filter(|identity| !identity.trim().is_empty())
            .unwrap_or(crate::prompt::system::DEFAULT_RENDERED_IDENTITY);

        match self.instructions.as_deref() {
            None => Some(identity.to_string()),
            Some(instructions) => Some(format!("{}\n\n{}", identity, instructions)),
        }
    }

    /// Starts a durable script record linked to its launching tool call.
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

    /// Completes the first script record matching `script_id`.
    ///
    /// An unknown ID is a no-op.
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

    /// Completes a matching script while recording child-call failures.
    ///
    /// An unknown ID is a no-op.
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

    /// Fails the first script record matching `script_id`.
    ///
    /// This convenience path records an empty child-call list. An unknown ID is
    /// a no-op.
    pub fn record_script_failed(&mut self, script_id: &str, error: Value) {
        if let Some(rec) = self
            .script_records
            .iter_mut()
            .find(|r| r.script_id == script_id)
        {
            rec.fail(error, Vec::new());
        }
    }

    /// Cancels the first script record matching `script_id`.
    ///
    /// An unknown ID is a no-op.
    pub fn record_script_cancelled(&mut self, script_id: &str) {
        if let Some(rec) = self
            .script_records
            .iter_mut()
            .find(|r| r.script_id == script_id)
        {
            rec.cancel();
        }
    }

    /// Links a child tool call to a script in both durable records.
    ///
    /// Either side is updated independently when present. Existing child links
    /// are not deduplicated.
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

    /// Enables or disables an MCP server for this session.
    ///
    /// MCP tool definitions affect provider-visible context, so changing this
    /// state invalidates token baseline accounting.
    pub fn set_mcp_server_enabled(&mut self, server_id: impl Into<String>, enabled: bool) {
        self.mcp_server_enablement.insert(server_id.into(), enabled);
        self.token_tracker.invalidate_baseline();
    }

    /// Returns explicit MCP server enablement, or `None` when unset.
    pub fn is_mcp_server_enabled(&self, server_id: &str) -> Option<bool> {
        self.mcp_server_enablement.get(server_id).copied()
    }

    /// Returns IDs of MCP servers explicitly enabled for this session.
    pub fn list_enabled_mcp_servers(&self) -> Vec<String> {
        self.mcp_server_enablement
            .iter()
            .filter(|&(_, enabled)| *enabled)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Enables or disables a plugin and invalidates token baseline accounting.
    pub fn set_plugin_enabled(&mut self, plugin_id: impl Into<String>, enabled: bool) {
        self.plugin_enablement.set_enabled(plugin_id, enabled);
        self.token_tracker.invalidate_baseline();
    }

    /// Returns explicit plugin enablement, or `None` when unset.
    pub fn is_plugin_enabled(&self, plugin_id: &str) -> Option<bool> {
        self.plugin_enablement.is_enabled(plugin_id)
    }

    /// Returns IDs of plugins explicitly enabled for this session.
    pub fn list_enabled_plugins(&self) -> Vec<String> {
        self.plugin_enablement.list_enabled()
    }
}

/// Shared, synchronously locked ownership of a [`DurableSession`].
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
