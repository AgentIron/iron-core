#![warn(
    rustdoc::broken_intra_doc_links,
    rustdoc::private_intra_doc_links,
    rustdoc::redundant_explicit_links
)]
//! iron-core: Core AgentIron runtime, ACP-native session management, and tool registry
//!
//! This crate provides the ACP-native runtime, session management, tool registration,
//! and configuration types as described in AGENTS.md.
//!
//! # Quick start
//!
//! Use [`IronAgent`] as the primary entry point. It wraps an `IronRuntime` and provides
//! an ergonomic facade over the ACP-native connection/session/prompt model.
//!
//! The **canonical interaction model is stream-first**. Call `session.prompt_stream(text)`
//! to get a split `(PromptHandle, PromptEvents)` pair:
//!
//! ```ignore
//! let agent = IronAgent::new(config, provider);
//! agent.register_tool(my_tool);
//! let connection = agent.connect();
//! let session = connection.create_session()?;
//!
//! let (handle, mut events) = session.prompt_stream("hello");
//!
//! while let Some(event) = events.next().await {
//!     match event {
//!         PromptEvent::Output { text } => { /* render text */ }
//!         PromptEvent::ToolCall { call_id, tool_name, .. } => { /* show tool */ }
//!         PromptEvent::ApprovalRequest { call_id, .. } => {
//!             handle.approve(&call_id).unwrap();
//!         }
//!         PromptEvent::ToolResult { call_id, status, .. } => { /* show outcome */ }
//!         PromptEvent::Complete { outcome } => break,
//!         PromptEvent::Status { message } => { /* advisory */ }
//!     }
//! }
//! ```
//!
//! Legacy `session.prompt().await` + `session.drain_events()` helpers remain as
//! compatibility wrappers but are not recommended for new code.
//!
//! # Architecture
//!
//! The canonical interaction model follows ACP (Agent Client Protocol):
//! - [`IronAgent`] → top-level entry point wrapping [`IronRuntime`]
//! - [`AgentConnection`] → one ACP client association (wraps [`IronConnection`])
//! - [`AgentSession`] → session with prompt/cancel/drain_events (wraps durable state)
//!
//! The runtime supports in-process (primary), stdio, and TCP transports via the
//! `transport` module.
//!
//! # Session Ownership
//!
//! Each session is owned by the connection that created it. Non-owning connections
//! cannot prompt, cancel, or close another connection's session. Violations surface
//! as `RuntimeError` (facade) or `invalid_params` (ACP transport).
//!
//! # Context Window Policy
//!
//! `Config.context_window_policy` is applied consistently in both ACP-native and
//! legacy request paths via a shared request builder (`request_builder` module).
//! `SummarizeAfter` is still rejected in direct request pruning; provider-backed
//! summarization currently lives under `context_management`.
//!
//! # Context Management
//!
//! iron-core distinguishes three context concepts:
//!
//! - **`active_context`**: the provider-visible footprint of the next request,
//!   including instructions, compacted semantic state, retained recent tail, and
//!   tool definitions/schema overhead. Query via [`AgentSession::active_context`].
//!
//! - **`compacted_context`**: a structured semantic summary maintained by
//!   compaction. Future prompts are assembled from this summary plus the retained
//!   recent tail, rather than replaying the full pre-compaction transcript. See
//!   [`CompactedContext`].
//!
//! - **`handoff_bundle`**: a portable continuity payload for cross-session transfer
//!   that excludes source tool capabilities and runtime state. Export via
//!   [`HandoffExporter`] and import via [`HandoffImporter`].
//!
//! Enable context management via [`Config::with_context_management`] with a
//! [`ContextManagementConfig`] that sets thresholds, tail retention rules, and
//! handoff export sizing. Compaction triggers at idle boundaries when
//! uncompacted tokens exceed the configured maintenance threshold.
//!
//! # Durable Tool-Call Lifecycle
//!
//! Tool-call records begin at proposal time (before approval or execution). Records
//! transition: `PendingApproval` → `Running` → terminal (`Completed`, `Failed`,
//! `Denied`, `Cancelled`). Denied and cancelled calls are durable terminal records
//! visible in subsequent prompt transcripts.
//!
//! # Tool Schema Validation
//!
//! Before a tool handler executes, arguments are validated against the tool's declared
//! `input_schema` using JSON Schema validation. Invalid arguments skip the handler and
//! produce a failed durable record. Invalid schemas also fail deterministically.
//!
//! # ACP Version Support Policy
//!
//! Each AgentIron release line pins to a specific `agent-client-protocol` SDK version
//! and declares which ACP protocol methods and features it supports. The policy is:
//!
//! - **Protocol version:** iron-core tracks ACP protocol version 1 (`ProtocolVersion::V1`).
//!   Breaking protocol changes require a new iron-core major version.
//!
//! - **Supported ACP methods (stable):**
//!   `initialize`, `newSession`, `prompt`, `cancel`, `session/update`,
//!   `requestPermission`.
//!
//! - **Supported ACP methods (unstable, opt-in):**
//!   `closeSession` (requires `unstable_session_close` feature on the ACP SDK).
//!
//! - **Deferred methods (not yet supported):**
//!   `loadSession`, `listSessions`, `forkSession`, `resumeSession`,
//!   `setSessionConfigOption`, `authenticate`, `logout`.
//!   These will be added in future releases as the ACP spec stabilizes them.
//!
//! - **Client capabilities (backend overrides):**
//!   `fs.writeTextFile`, `fs.readTextFile`, `terminal/create`,
//!   `terminal/output`, `terminal/release`, `terminal/waitForExit`,
//!   `terminal/kill`. These are optional global backend overrides; built-in
//!   iron-core implementations are used by default. If an override is meant to
//!   be callable as a tool (including from embedded Python), it must materialize
//!   as the corresponding runtime tool registration or substitution.
//!
//! - **Transport support:**
//!   In-process (primary, for embeddings), stdio (subprocess), TCP (cross-process).
//!   All transports enforce identical session ownership, durable history, permission
//!   flow, and cancellation semantics.
//!
//! - **Conformance testing:**
//!   Each supported method has at least one transport-independent unit test
//!   (`acp_runtime_tests`) and at least one interop smoke test through the real ACP
//!   SDK in-process transport (`interop_tests`).
//!
//! The supported ACP surface is also available programmatically via
//! [`transport::AcpSupport`].
//!
//! # Legacy API
//!
//! The older [`SessionHandle`]-centric API (`SessionHandle`, `SessionRuntime`,
//! `TurnHandle`, `TurnEvents`) is still available for backward compatibility but
//! is **deprecated** for new code. It will not receive new features. Migrate to
//! the facade types above.
//!
//! # Tools
//!
//! Register tools via [`IronAgent::register_tool`] or on the [`ToolRegistry`].
//! Sync handlers are automatically routed through `spawn_blocking`. Custom async
//! [`Tool`] implementations must not block the orchestration runtime.

pub mod builtin;
pub mod capability;
pub mod config;
pub mod connection;
pub mod context;
pub mod durable;
pub mod embedded_python;
pub mod ephemeral;
pub mod error;
#[deprecated(
    since = "0.2.0",
    note = "Use `AgentSession::drain_events()` with `AgentEvent` instead. This module will be removed in a future release."
)]
pub mod events;
pub mod facade;
#[deprecated(
    since = "0.2.0",
    note = "Use `AgentLoop` from the new facade instead. This module will be removed in a future release."
)]
pub mod loop_state;
pub mod prompt;
pub mod prompt_lifecycle;
pub mod prompt_runner;
pub mod prompt_turn;
pub mod request_builder;
pub mod runtime;
pub mod schema;
#[deprecated(
    since = "0.2.0",
    note = "Use `AgentSession` from the facade instead. This module will be removed in a future release."
)]
pub mod session;
#[deprecated(
    since = "0.2.0",
    note = "Use `IronAgent` + `AgentConnection` + `AgentSession` from the facade instead. This module will be removed in a future release."
)]
pub mod session_handle;
#[deprecated(
    since = "0.2.0",
    note = "Use `IronRuntime` from the runtime module instead. This module will be removed in a future release."
)]
pub mod session_runtime;
pub mod tool;
pub mod transport;
#[deprecated(
    since = "0.2.0",
    note = "Use `AgentSession::prompt()` + `drain_events()` instead. This module will be removed in a future release."
)]
pub mod turn;

pub use crate::prompt::{
    AdditionalInstructionFile, PromptAssembler, RepoInstructionLoader, RuntimeContextRenderer,
};
pub use config::{
    ApprovalStrategy, Config, ConfigSource, ContextManagementConfig, ContextWindowPolicy,
    EmbeddedPythonConfig, PromptCompositionConfig,
};
pub use connection::IronConnection;
pub use context::{
    ActiveContextAccountant, ActiveContextSnapshot, CompactedContext, CompactionCheckpoint,
    CompactionEngine, CompactionInput, CompactionReason, ContextCategory, ContextCategoryUsage,
    ContextQuality, ContextTelemetry, Decision, HandoffBundle, HandoffBundleMetadata,
    HandoffExportConfig, HandoffExporter, HandoffImporter, PortabilityNote, TailRetentionPolicy,
    TailRetentionRule, UnresolvedQuestion, HANDOFF_DEFAULT_TARGET_TOKENS,
};
pub use durable::{
    ContentBlock, DurableScriptRecord, DurableSession, DurableToolRecord, ScriptRecordStatus,
    SessionId, StructuredMessage, TimelineEntry, ToolRecordStatus, ToolTerminalOutcome,
};
pub use ephemeral::{EphemeralTurn, TurnPhase};
pub use error::{LoopError, LoopResult, RuntimeError, RuntimeResult};
#[allow(deprecated)]
pub use events::StreamEvent;
pub use facade::{
    AgentConnection, AgentEvent, AgentSession, FacadeToolStatus, IronAgent, PermissionRequest,
    PermissionVerdict, PromptEvent, PromptEvents, PromptHandle, PromptOutcome, PromptStatus,
    ToolResultStatus,
};
#[allow(deprecated)]
pub use loop_state::AgentLoop;
pub use prompt_turn::PromptTurn;
pub use runtime::{ConnectionId, IronRuntime};
#[allow(deprecated)]
pub use session::{Message, Session};
#[allow(deprecated)]
pub use session_handle::SessionHandle;
#[allow(deprecated)]
pub use session_runtime::SessionRuntime;
pub use tool::{FunctionTool, Tool, ToolDefinition, ToolRegistry};

pub use builtin::{
    register_builtin_tools, BuiltinErrorCode, BuiltinToolConfig, BuiltinToolError,
    BuiltinToolPolicy, NetworkPolicy, ShellAvailability,
};
pub use transport::{
    connect_tcp_client, create_in_process_transport, create_stdio_agent, serve_tcp_agent,
    AcpSupport, InProcessTransport, TransportKind, TransportMetadata,
};

pub use iron_providers::{
    GenerationConfig, InferenceRequest, OpenAiConfig, OpenAiConfigSource, OpenAiProvider, Provider,
    ProviderError, ProviderEvent, RuntimeConfigSource, ToolCall, ToolPolicy, Transcript,
};

#[allow(deprecated)]
pub use events::{PendingCallInfo, TurnEvent, TurnId, TurnOutcome, TurnStatus};
#[allow(deprecated)]
pub use turn::{TurnEvents, TurnHandle};

#[allow(deprecated)]
pub mod prelude {
    #[allow(deprecated)]
    pub use crate::{
        AgentConnection, AgentEvent, AgentLoop, AgentSession, ApprovalStrategy, Config,
        ConfigSource, ContentBlock, ContextWindowPolicy, FacadeToolStatus, GenerationConfig,
        IronAgent, LoopError, LoopResult, Message, OpenAiConfig, OpenAiConfigSource,
        OpenAiProvider, PendingCallInfo, PermissionVerdict, PromptOutcome, Provider,
        RuntimeConfigSource, Session, SessionHandle, SessionId, SessionRuntime, StreamEvent, Tool,
        ToolDefinition, ToolPolicy, ToolRegistry, Transcript, TurnEvent, TurnEvents, TurnHandle,
        TurnId, TurnOutcome, TurnStatus,
    };
}
