//! Debug observation primitive for iron-core.
//!
//! This module provides a typed debug observation surface for engine-level
//! semantic decisions. It is distinct from:
//!
//! - `PromptSink`: client-visible prompt lifecycle events (output deltas,
//!   tool proposals, approval requests)
//! - `tracing`: process-level operational logging
//!
//! Debug events are observational only: they are not persisted in session
//! transcripts, are not model-visible context, and are not required for
//! correct runtime behavior.
//!
//! ## Usage
//!
//! ```rust,ignore
//! let runtime = IronRuntime::new(config, provider);
//! runtime.set_debug_sink(Some(Arc::new(MyDebugSink)));
//! ```
//!
//! ## Redaction
//!
//! Debug payloads prefer safe metadata (names, counts, lengths, fingerprints,
//! decisions, statuses) over raw content (prompt text, tool arguments, skill
//! contents, credentials).

use crate::{ConnectionId, SessionId};
use std::sync::atomic::{AtomicU64, Ordering};

/// A sink that receives structured debug events from the runtime.
///
/// Implementations should return quickly and not block the runtime.
/// Debug observation is best-effort: sink failures or slowness must
/// never affect prompt execution, tool execution, model switching,
/// or session state.
pub trait DebugSink: Send + Sync {
    /// Emit a debug event. This is synchronous and should not block.
    fn emit(&self, event: DebugEvent);
}

/// A no-op debug sink used by default when no sink is registered.
pub struct NullDebugSink;

impl DebugSink for NullDebugSink {
    fn emit(&self, _event: DebugEvent) {}
}

/// A debug event emitted at a semantic runtime transition.
///
/// Events are observational only: they are not persisted in the
/// session transcript, are not model-visible, and are not required
/// for correct behavior.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugEvent {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub sequence: u64,
    pub severity: DebugSeverity,
    pub scope: DebugScope,
    pub payload: DebugPayload,
}

/// Severity level of a debug event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DebugSeverity {
    Info,
    Warning,
    Error,
}

/// Correlation scope for a debug event, capturing available IDs
/// so clients can reconstruct causality across sessions, turns,
/// model requests, and tool calls.
///
/// Not all events have all IDs: runtime-level events may only
/// have runtime scope, while tool events should include session,
/// turn, and tool-call scope where available.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DebugScope {
    pub runtime_id: Option<String>,
    pub connection_id: Option<String>,
    pub session_id: Option<crate::SessionId>,
    pub turn_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub provider_name: Option<String>,
    pub model_id: Option<String>,
}

/// Grouped domain payloads for debug events.
///
/// New variants can be added without breaking existing consumers
/// that do not match on the full enum.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DebugPayload {
    /// Prompt-related debug events.
    Prompt(PromptDebugEvent),
    /// Context-related debug events.
    Context(ContextDebugEvent),
    /// Compaction-related debug events.
    Compaction(CompactionDebugEvent),
    /// Tool-related debug events.
    Tool(ToolDebugEvent),
    /// Provider/model-switch-related debug events.
    Provider(ProviderDebugEvent),
    /// Configuration-related debug events.
    Config(ConfigDebugEvent),
    /// Skill-related debug events.
    Skill(SkillDebugEvent),
}

// ── Prompt ──────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PromptDebugEvent {
    /// A system prompt was rendered for an inference request.
    SystemPromptRendered {
        fingerprint: String,
        total_chars: usize,
        sections: Vec<SectionSummary>,
        changed: Option<bool>,
    },
    /// A model input influence was added, removed, changed, or suppressed.
    ModelInputInfluence {
        source: InfluenceSource,
        destination: InfluenceDestination,
        effect: InfluenceEffect,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InfluenceSource {
    ContextPressure,
    CompactionAvailability,
    ToolAvailability,
    SkillActivation,
    ProviderGuidance,
    ClientInstruction,
    RepoInstruction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InfluenceDestination {
    SystemPromptSection(String),
    UserPromptRewrite,
    ToolDefinition,
    ContinuationContext,
    RequestMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InfluenceEffect {
    Added,
    Removed,
    Changed,
    Suppressed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SectionSummary {
    pub name: String,
    pub owner: Option<String>,
    pub temperature: Option<String>,
    pub chars: usize,
}

// ── Context ─────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ContextDebugEvent {
    /// Active context snapshot was estimated.
    SnapshotEstimated {
        total_tokens: usize,
        context_window_limit: Option<usize>,
        quality: crate::ContextQuality,
        pressure: String,
        categories: Vec<(String, usize, crate::ContextQuality)>,
    },
    /// Context pressure classification changed.
    PressureChanged {
        old_pressure: String,
        new_pressure: String,
        reason: String,
    },
}

// ── Compaction ──────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CompactionDebugEvent {
    /// The model requested compaction.
    Requested {
        topic_present: bool,
        range_count: usize,
        thresholds: Option<String>,
    },
    /// Compaction was rejected by validation.
    Rejected { reason: String },
    /// Compaction was applied successfully.
    Applied {
        block_count: usize,
        old_size_tokens: Option<usize>,
        new_size_tokens: Option<usize>,
        pressure_state: String,
        reduction_pct: Option<f64>,
    },
}

// ── Tool ────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ToolDebugEvent {
    /// Tool approval was evaluated.
    ApprovalEvaluated {
        tool_name: String,
        approved: bool,
        decision_source: String,
        user_approval_requested: bool,
        reason: String,
    },
    /// Tool execution started.
    ExecutionStarted {
        tool_name: String,
        tool_source: String,
        call_id: String,
    },
    /// Tool execution finished.
    ExecutionFinished {
        tool_name: String,
        call_id: String,
        status: String,
        duration_ms: Option<u64>,
        truncated: bool,
        reason: Option<String>,
    },
}

// ── Provider / Model Switch ─────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ProviderDebugEvent {
    /// A model switch was queued for the next turn boundary.
    ModelSwitchQueued {
        target_model: String,
        target_provider: String,
    },
    /// A model switch plan was created.
    ModelSwitchPlanCreated {
        current_tokens: usize,
        target_window: Option<usize>,
        adaptation_needed: bool,
        estimate_quality: String,
    },
    /// A model switch was applied.
    ModelSwitchApplied {
        from_model: String,
        from_provider: String,
        to_model: String,
        to_provider: String,
        capability_diff: Option<String>,
    },
    /// A model switch failed.
    ModelSwitchFailed {
        target_model: String,
        target_provider: String,
        reason: String,
    },
}

// ── Config ──────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ConfigDebugEvent {
    /// Runtime was initialized with a configuration summary.
    RuntimeConfigured {
        provider_name: String,
        model_id: String,
        approval_strategy: String,
        context_management_enabled: bool,
        prompt_composition_enabled: bool,
        tool_policy: String,
        plugin_enabled: bool,
        mcp_enabled: bool,
        skill_enabled: bool,
        workspace_roots: usize,
    },
}

// ── Skill ───────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SkillDebugEvent {
    /// Skill catalog was refreshed.
    CatalogRefreshed {
        sources: Vec<String>,
        discovered_count: usize,
        trusted_count: usize,
        untrusted_count: usize,
        diagnostic_count: usize,
    },
    /// Skills were made available to a session.
    AvailableToSession {
        count: usize,
        source_categories: Vec<String>,
    },
    /// Skill activation succeeded.
    ActivationSuccess {
        skill_name: String,
        source_kind: String,
        activation_source: String,
    },
    /// Skill activation was rejected.
    ActivationRejected { skill_name: String, reason: String },
}

// ── Internal helpers ────────────────────────────────────────────

/// Thread-safe sequence generator for debug events.
pub(crate) struct SequenceGenerator {
    counter: AtomicU64,
}

impl SequenceGenerator {
    pub(crate) fn new() -> Self {
        Self {
            counter: AtomicU64::new(1),
        }
    }

    pub(crate) fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }
}

impl Default for SequenceGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// A lightweight context object that can be passed through execution
/// paths to build up correlation scope for debug events.
///
/// This is similar to a tracing span: it accumulates IDs as it flows
/// through runtime → connection → session → turn → tool call.
#[derive(Clone, Debug, Default)]
pub struct DebugContext {
    pub scope: DebugScope,
}

impl DebugContext {
    /// Create a new debug context with runtime-level scope.
    pub fn new(runtime_id: Option<String>) -> Self {
        Self {
            scope: DebugScope {
                runtime_id,
                ..DebugScope::default()
            },
        }
    }

    /// Fork a new context for a connection.
    pub fn for_connection(&self, connection_id: ConnectionId) -> Self {
        let mut fork = self.clone();
        fork.scope.connection_id = Some(connection_id.0.to_string());
        fork
    }

    /// Fork a new context for a session.
    pub fn for_session(&self, session_id: SessionId) -> Self {
        let mut fork = self.clone();
        fork.scope.session_id = Some(session_id);
        fork
    }

    /// Fork a new context for a prompt turn.
    pub fn for_turn(&self, turn_id: impl Into<String>) -> Self {
        let mut fork = self.clone();
        fork.scope.turn_id = Some(turn_id.into());
        fork
    }

    /// Fork a new context for a tool call.
    pub fn for_tool_call(&self, call_id: impl Into<String>) -> Self {
        let mut fork = self.clone();
        fork.scope.tool_call_id = Some(call_id.into());
        fork
    }

    /// Set provider name in scope.
    pub fn with_provider(&mut self, provider: impl Into<String>) -> &mut Self {
        self.scope.provider_name = Some(provider.into());
        self
    }

    /// Set model id in scope.
    pub fn with_model(&mut self, model: impl Into<String>) -> &mut Self {
        self.scope.model_id = Some(model.into());
        self
    }

    /// Build a DebugEvent from this context.
    pub fn event(
        &self,
        sequence: u64,
        severity: DebugSeverity,
        payload: DebugPayload,
    ) -> DebugEvent {
        DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence,
            severity,
            scope: self.scope.clone(),
            payload,
        }
    }
}

// ── Redaction helpers ───────────────────────────────────────────

/// Redact a system prompt into safe summary metadata.
pub fn redact_system_prompt(
    fingerprint: impl Into<String>,
    total_chars: usize,
    sections: Vec<crate::prompt::system::PromptSectionMetadata>,
    changed: Option<bool>,
) -> PromptDebugEvent {
    let section_summaries = sections
        .into_iter()
        .map(|meta| SectionSummary {
            name: meta.title.to_string(),
            owner: Some(format!("{:?}", meta.owner)),
            temperature: Some(format!("{:?}", meta.temperature)),
            chars: 0, // Content length not available at this abstraction
        })
        .collect();

    PromptDebugEvent::SystemPromptRendered {
        fingerprint: fingerprint.into(),
        total_chars,
        sections: section_summaries,
        changed,
    }
}

/// Create a safe tool execution summary without raw arguments or results.
pub fn redact_tool_execution(
    tool_name: impl Into<String>,
    call_id: impl Into<String>,
    status: impl Into<String>,
    duration_ms: Option<u64>,
    truncated: bool,
    reason: Option<String>,
) -> ToolDebugEvent {
    ToolDebugEvent::ExecutionFinished {
        tool_name: tool_name.into(),
        call_id: call_id.into(),
        status: status.into(),
        duration_ms,
        truncated,
        reason,
    }
}

/// Create a safe config summary without credentials or sensitive values.
pub fn redact_config(config: &crate::config::Config) -> ConfigDebugEvent {
    ConfigDebugEvent::RuntimeConfigured {
        provider_name: config.provider_name.clone().unwrap_or_default(),
        model_id: config.model.clone(),
        approval_strategy: format!("{:?}", config.default_approval_strategy),
        context_management_enabled: config.context_management.enabled,
        prompt_composition_enabled: config.prompt_composition.repo_instructions.enabled,
        tool_policy: format!("{:?}", config.default_tool_policy),
        plugin_enabled: config.plugins.enabled,
        mcp_enabled: config.mcp.enabled,
        skill_enabled: config.skills.enabled,
        workspace_roots: config.workspace_roots.len(),
    }
}

/// Internal helper to emit a debug event if a sink is present.
pub(crate) fn emit_debug(sink: &dyn DebugSink, event: DebugEvent) {
    sink.emit(event);
}

// ── Test utilities ──────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;
    use std::sync::Mutex;

    /// A debug sink that records all emitted events for test inspection.
    #[derive(Debug, Default)]
    pub struct RecordingDebugSink {
        events: Mutex<Vec<DebugEvent>>,
    }

    impl RecordingDebugSink {
        pub fn new() -> Self {
            Self::default()
        }

        /// Take all recorded events, clearing the internal buffer.
        pub fn take_events(&self) -> Vec<DebugEvent> {
            std::mem::take(&mut *self.events.lock().unwrap())
        }

        /// Peek at recorded events without clearing.
        pub fn events(&self) -> Vec<DebugEvent> {
            self.events.lock().unwrap().clone()
        }

        /// Count recorded events.
        pub fn len(&self) -> usize {
            self.events.lock().unwrap().len()
        }

        /// Check if any events were recorded.
        pub fn is_empty(&self) -> bool {
            self.events.lock().unwrap().is_empty()
        }

        /// Check if any event matches a predicate.
        pub fn has_event<F>(&self, predicate: F) -> bool
        where
            F: Fn(&DebugEvent) -> bool,
        {
            self.events().iter().any(predicate)
        }
    }

    impl DebugSink for RecordingDebugSink {
        fn emit(&self, event: DebugEvent) {
            self.events.lock().unwrap().push(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_helpers::RecordingDebugSink;
    use super::*;

    #[test]
    fn null_sink_is_no_op() {
        let sink = NullDebugSink;
        let event = DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: 1,
            severity: DebugSeverity::Info,
            scope: DebugScope::default(),
            payload: DebugPayload::Config(ConfigDebugEvent::RuntimeConfigured {
                provider_name: "test".to_string(),
                model_id: "test".to_string(),
                approval_strategy: "auto".to_string(),
                context_management_enabled: false,
                prompt_composition_enabled: false,
                tool_policy: "auto".to_string(),
                plugin_enabled: false,
                mcp_enabled: false,
                skill_enabled: false,
                workspace_roots: 0,
            }),
        };
        sink.emit(event);
    }

    #[test]
    fn recording_sink_captures_events() {
        let sink = RecordingDebugSink::new();
        assert!(sink.is_empty());

        let event = DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: 1,
            severity: DebugSeverity::Info,
            scope: DebugScope::default(),
            payload: DebugPayload::Config(ConfigDebugEvent::RuntimeConfigured {
                provider_name: "test".to_string(),
                model_id: "test".to_string(),
                approval_strategy: "auto".to_string(),
                context_management_enabled: false,
                prompt_composition_enabled: false,
                tool_policy: "auto".to_string(),
                plugin_enabled: false,
                mcp_enabled: false,
                skill_enabled: false,
                workspace_roots: 0,
            }),
        };
        sink.emit(event.clone());
        assert_eq!(sink.len(), 1);
        assert!(sink.has_event(|e| e.sequence == 1));

        let events = sink.take_events();
        assert_eq!(events.len(), 1);
        assert!(sink.is_empty());
    }

    #[test]
    fn debug_context_builds_scope() {
        let ctx = DebugContext::new(Some("runtime-1".to_string()));
        assert_eq!(ctx.scope.runtime_id, Some("runtime-1".to_string()));

        let conn_ctx = ctx.for_connection(crate::ConnectionId(0));
        assert!(conn_ctx.scope.connection_id.is_some());
        assert_eq!(conn_ctx.scope.runtime_id, Some("runtime-1".to_string()));

        let session_ctx = conn_ctx.for_session(crate::SessionId::new());
        assert!(session_ctx.scope.session_id.is_some());
        assert!(session_ctx.scope.connection_id.is_some());

        let turn_ctx = session_ctx.for_turn("turn-1");
        assert_eq!(turn_ctx.scope.turn_id, Some("turn-1".to_string()));

        let tool_ctx = turn_ctx.for_tool_call("call-1");
        assert_eq!(tool_ctx.scope.tool_call_id, Some("call-1".to_string()));
    }

    #[test]
    fn debug_event_envelope_fields_present() {
        let event = DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: 42,
            severity: DebugSeverity::Warning,
            scope: DebugScope::default(),
            payload: DebugPayload::Tool(ToolDebugEvent::ExecutionStarted {
                tool_name: "test".to_string(),
                tool_source: "builtin".to_string(),
                call_id: "call-1".to_string(),
            }),
        };

        assert_eq!(event.sequence, 42);
        assert!(matches!(event.severity, DebugSeverity::Warning));
        assert!(matches!(event.payload, DebugPayload::Tool(_)));
    }

    #[test]
    fn redact_config_omits_sensitive_values() {
        use crate::config::Config;
        let config = Config::default();
        let event = redact_config(&config);

        match event {
            ConfigDebugEvent::RuntimeConfigured {
                approval_strategy,
                tool_policy,
                workspace_roots,
                ..
            } => {
                assert!(!approval_strategy.is_empty());
                assert!(!tool_policy.is_empty());
                assert_eq!(workspace_roots, config.workspace_roots.len());
            }
        }
    }

    #[test]
    fn sequence_generator_increments() {
        let gen = SequenceGenerator::new();
        assert_eq!(gen.next(), 1);
        assert_eq!(gen.next(), 2);
        assert_eq!(gen.next(), 3);
    }

    #[test]
    fn prompt_system_prompt_rendered_event_has_safe_fields() {
        let event = PromptDebugEvent::SystemPromptRendered {
            fingerprint: "abc123".to_string(),
            total_chars: 1500,
            sections: vec![SectionSummary {
                name: "Identity".to_string(),
                owner: Some("Core".to_string()),
                temperature: Some("Cold".to_string()),
                chars: 0,
            }],
            changed: Some(true),
        };
        match event {
            PromptDebugEvent::SystemPromptRendered {
                fingerprint,
                total_chars,
                sections,
                changed,
            } => {
                assert_eq!(fingerprint, "abc123");
                assert_eq!(total_chars, 1500);
                assert_eq!(sections.len(), 1);
                assert_eq!(changed, Some(true));
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn prompt_model_input_influence_event_represents_hint() {
        let event = PromptDebugEvent::ModelInputInfluence {
            source: InfluenceSource::ContextPressure,
            destination: InfluenceDestination::SystemPromptSection("Tool Philosophy".to_string()),
            effect: InfluenceEffect::Added,
            reason: "context pressure guidance: Soft".to_string(),
        };
        match event {
            PromptDebugEvent::ModelInputInfluence {
                source,
                destination: _destination,
                effect,
                reason,
            } => {
                assert!(matches!(source, InfluenceSource::ContextPressure));
                assert!(matches!(effect, InfluenceEffect::Added));
                assert!(reason.contains("Soft"));
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn provider_model_switch_failed_event_has_reason() {
        let event = ProviderDebugEvent::ModelSwitchFailed {
            target_model: "gpt-4".to_string(),
            target_provider: "openai".to_string(),
            reason: "session not found".to_string(),
        };
        match event {
            ProviderDebugEvent::ModelSwitchFailed {
                target_model,
                target_provider,
                reason,
            } => {
                assert_eq!(target_model, "gpt-4");
                assert_eq!(target_provider, "openai");
                assert_eq!(reason, "session not found");
            }
            _ => panic!("unexpected variant"),
        }
    }
}
