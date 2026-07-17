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
//! ```no_run
//! use iron_core::{Config, DebugEvent, DebugSink, IronAgent};
//! use iron_providers::{ApiFamily, ProviderConnection, ProviderProfile, RuntimeConfig};
//! use std::sync::Arc;
//!
//! struct MyDebugSink;
//! impl DebugSink for MyDebugSink {
//!     fn emit(&self, event: DebugEvent) {
//!         eprintln!("debug event {}: {:?}", event.sequence, event.payload);
//!     }
//! }
//!
//! let provider = ProviderConnection::from_profile(
//!     ProviderProfile::new("openai", ApiFamily::Responses, "https://api.openai.com/v1"),
//!     RuntimeConfig::new("sk-example"),
//! )?;
//! let agent = IronAgent::new(Config::default(), provider);
//! agent.set_debug_sink(Some(Arc::new(MyDebugSink)));
//! # Ok::<(), Box<dyn std::error::Error>>(())
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
/// Emission is synchronous and is not isolated from the caller. Implementations
/// must return quickly, avoid blocking, and avoid panicking so observation does
/// not disrupt prompt execution, tool execution, model switching, or session
/// state.
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
    /// UTC time at which the event envelope was created.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Runtime-local monotonically increasing event sequence number.
    pub sequence: u64,
    /// Operational importance assigned by the emitter.
    pub severity: DebugSeverity,
    /// Correlation identifiers available at the emission site.
    pub scope: DebugScope,
    /// Domain-specific observation carried by the event.
    pub payload: DebugPayload,
}

/// Severity level of a debug event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DebugSeverity {
    /// Informational observation of normal runtime behavior.
    Info,
    /// Unexpected or degraded behavior that did not necessarily stop execution.
    Warning,
    /// A semantic operation failed.
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
    /// Identifier of the runtime instance, when assigned.
    pub runtime_id: Option<String>,
    /// Identifier of the client connection, when the event is connection-scoped.
    pub connection_id: Option<String>,
    /// Session associated with the event.
    pub session_id: Option<crate::SessionId>,
    /// Prompt-turn identifier associated with the event.
    pub turn_id: Option<String>,
    /// Tool-call identifier associated with the event.
    pub tool_call_id: Option<String>,
    /// Provider selected for the relevant model request.
    pub provider_name: Option<String>,
    /// Model selected for the relevant model request.
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
    Prompt(
        /// Prompt observation details.
        PromptDebugEvent,
    ),
    /// Context-related debug events.
    Context(
        /// Context observation details.
        ContextDebugEvent,
    ),
    /// Compaction-related debug events.
    Compaction(
        /// Compaction observation details.
        CompactionDebugEvent,
    ),
    /// Tool-related debug events.
    Tool(
        /// Tool observation details.
        ToolDebugEvent,
    ),
    /// Provider/model-switch-related debug events.
    Provider(
        /// Provider or model-switch observation details.
        ProviderDebugEvent,
    ),
    /// Configuration-related debug events.
    Config(
        /// Redacted configuration observation details.
        ConfigDebugEvent,
    ),
    /// Skill-related debug events.
    Skill(
        /// Skill observation details.
        SkillDebugEvent,
    ),
}

// ── Prompt ──────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
/// Observations about prompt construction and model-input influences.
pub enum PromptDebugEvent {
    /// A system prompt was rendered for an inference request.
    SystemPromptRendered {
        /// Stable digest of the rendered prompt, without exposing its text.
        fingerprint: String,
        /// Character count of the complete rendered prompt.
        total_chars: usize,
        /// Safe metadata for each composed prompt section.
        sections: Vec<SectionSummary>,
        /// Whether the fingerprint changed from the prior render, if known.
        changed: Option<bool>,
    },
    /// A model input influence was added, removed, changed, or suppressed.
    ModelInputInfluence {
        /// Runtime condition or instruction that caused the influence.
        source: InfluenceSource,
        /// Model-input component affected by the influence.
        destination: InfluenceDestination,
        /// How the destination was altered.
        effect: InfluenceEffect,
        /// Safe explanation of why the influence was applied.
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// Sources that can influence model input composition.
pub enum InfluenceSource {
    /// Current context-window pressure.
    ContextPressure,
    /// Whether context compaction is available.
    CompactionAvailability,
    /// Tools currently visible to the model.
    ToolAvailability,
    /// Activation of a reusable skill.
    SkillActivation,
    /// Provider-specific request guidance.
    ProviderGuidance,
    /// Explicit instruction from the connected client.
    ClientInstruction,
    /// Instruction discovered in the active repository.
    RepoInstruction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// Model-input components that can be influenced by runtime decisions.
pub enum InfluenceDestination {
    /// Named section of the rendered system prompt.
    SystemPromptSection(
        /// Human-readable name of the affected section.
        String,
    ),
    /// Transformation of the user-supplied prompt.
    UserPromptRewrite,
    /// Definition of a model-visible tool.
    ToolDefinition,
    /// Context supplied to continue a prior interaction.
    ContinuationContext,
    /// Provider request metadata outside prompt text.
    RequestMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// Kind of change made to a model-input component.
pub enum InfluenceEffect {
    /// New content or metadata was introduced.
    Added,
    /// Existing content or metadata was removed.
    Removed,
    /// Existing content or metadata was modified.
    Changed,
    /// A candidate influence was intentionally not applied.
    Suppressed,
}

/// Safe metadata describing one rendered system-prompt section.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionSummary {
    /// Human-readable section title.
    pub name: String,
    /// Component responsible for the section, when available.
    pub owner: Option<String>,
    /// Section stability classification, when available.
    pub temperature: Option<String>,
    /// Character count of the section, or zero when unavailable.
    pub chars: usize,
}

// ── Context ─────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
/// Observations about context size, quality, and pressure transitions.
pub enum ContextDebugEvent {
    /// Active context snapshot was estimated.
    SnapshotEstimated {
        /// Estimated tokens in active model context.
        total_tokens: usize,
        /// Maximum model context size, when known.
        context_window_limit: Option<usize>,
        /// Token threshold at which compaction is considered, when configured.
        compact_threshold_tokens: Option<usize>,
        /// Confidence or completeness of the context estimate.
        quality: crate::ContextQuality,
        /// Current context-pressure classification.
        pressure: String,
        /// Per-category token estimates and their quality classifications.
        categories: Vec<(String, usize, crate::ContextQuality)>,
        /// Provider-reported cumulative usage, when available.
        accumulated_usage: Option<crate::context::TokenUsageTotals>,
    },
    /// Context pressure classification changed.
    PressureChanged {
        /// Pressure classification before the transition.
        old_pressure: String,
        /// Pressure classification after the transition.
        new_pressure: String,
        /// Safe explanation of the transition.
        reason: String,
    },
}

// ── Compaction ──────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
/// Observations from the context-compaction lifecycle.
pub enum CompactionDebugEvent {
    /// The model requested compaction.
    Requested {
        /// Whether the request specified a compaction topic.
        topic_present: bool,
        /// Number of transcript ranges requested for compaction.
        range_count: usize,
        /// Safe rendering of applicable thresholds, when available.
        thresholds: Option<String>,
    },
    /// Compaction was rejected by validation.
    Rejected {
        /// Validation reason for rejecting the request.
        reason: String,
    },
    /// Compaction was applied successfully.
    Applied {
        /// Number of transcript blocks compacted.
        block_count: usize,
        /// Estimated token count before compaction, when available.
        old_size_tokens: Option<usize>,
        /// Estimated token count after compaction, when available.
        new_size_tokens: Option<usize>,
        /// Compaction strategy or implementation, when reported.
        method: Option<String>,
        /// Context-pressure classification after compaction.
        pressure_state: String,
        /// Percentage token reduction, when both estimates are available.
        reduction_pct: Option<f64>,
    },
}

// ── Tool ────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
/// Observations from tool approval and execution.
pub enum ToolDebugEvent {
    /// Tool approval was evaluated.
    ApprovalEvaluated {
        /// Registered name of the proposed tool.
        tool_name: String,
        /// Final approval decision.
        approved: bool,
        /// Policy, user, or other source of the decision.
        decision_source: String,
        /// Whether evaluation required a user-facing approval request.
        user_approval_requested: bool,
        /// Safe explanation of the decision.
        reason: String,
    },
    /// Tool execution started.
    ExecutionStarted {
        /// Registered name of the executing tool.
        tool_name: String,
        /// Tool provider category, such as builtin or plugin.
        tool_source: String,
        /// Identifier correlating this start with its completion event.
        call_id: String,
    },
    /// Tool execution finished.
    ExecutionFinished {
        /// Registered name of the executed tool.
        tool_name: String,
        /// Identifier correlating this completion with its start event.
        call_id: String,
        /// Terminal execution status.
        status: String,
        /// Elapsed execution time in milliseconds, when measured.
        duration_ms: Option<u64>,
        /// Whether returned tool content was truncated.
        truncated: bool,
        /// Safe failure or truncation explanation, when available.
        reason: Option<String>,
    },
}

// ── Provider / Model Switch ─────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
/// Observations from provider and model-switch decisions.
pub enum ProviderDebugEvent {
    /// A model switch was queued for the next turn boundary.
    ModelSwitchQueued {
        /// Model requested for the next turn.
        target_model: String,
        /// Provider that serves the target model.
        target_provider: String,
    },
    /// A model switch plan was created.
    ModelSwitchPlanCreated {
        /// Estimated tokens in the current context.
        current_tokens: usize,
        /// Context-window size of the target model, when known.
        target_window: Option<usize>,
        /// Whether context must be adapted before switching.
        adaptation_needed: bool,
        /// Quality classification of the token estimate.
        estimate_quality: String,
    },
    /// A model switch was applied.
    ModelSwitchApplied {
        /// Model active before the switch.
        from_model: String,
        /// Provider active before the switch.
        from_provider: String,
        /// Model active after the switch.
        to_model: String,
        /// Provider active after the switch.
        to_provider: String,
        /// Safe summary of capability changes, when available.
        capability_diff: Option<String>,
    },
    /// A model switch failed.
    ModelSwitchFailed {
        /// Model that could not be activated.
        target_model: String,
        /// Provider that could not serve the switch.
        target_provider: String,
        /// Safe explanation of the failure.
        reason: String,
    },
}

// ── Config ──────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
/// Redacted observations about effective runtime configuration.
pub enum ConfigDebugEvent {
    /// Runtime was initialized with a configuration summary.
    RuntimeConfigured {
        /// Selected provider name.
        provider_name: String,
        /// Selected model identifier.
        model_id: String,
        /// Safe rendering of the default approval strategy.
        approval_strategy: String,
        /// Whether automatic context management is enabled.
        context_management_enabled: bool,
        /// Whether repository prompt composition is enabled.
        prompt_composition_enabled: bool,
        /// Safe rendering of the default tool policy.
        tool_policy: String,
        /// Whether plugin loading is enabled.
        plugin_enabled: bool,
        /// Whether MCP integration is enabled.
        mcp_enabled: bool,
        /// Whether skill discovery and activation are enabled.
        skill_enabled: bool,
        /// Number of configured workspace roots.
        workspace_roots: usize,
    },
}

// ── Skill ───────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
/// Observations from skill discovery, availability, and activation.
pub enum SkillDebugEvent {
    /// Skill catalog was refreshed.
    CatalogRefreshed {
        /// Safe names of catalog sources that were scanned.
        sources: Vec<String>,
        /// Number of skills discovered before trust filtering.
        discovered_count: usize,
        /// Number of discovered skills accepted as trusted.
        trusted_count: usize,
        /// Number of discovered skills classified as untrusted.
        untrusted_count: usize,
        /// Number of diagnostics produced during refresh.
        diagnostic_count: usize,
    },
    /// Skills were made available to a session.
    AvailableToSession {
        /// Number of skills exposed to the session.
        count: usize,
        /// Categories of sources contributing available skills.
        source_categories: Vec<String>,
    },
    /// Skill activation succeeded.
    ActivationSuccess {
        /// Name of the activated skill.
        skill_name: String,
        /// Kind of source from which the skill was loaded.
        source_kind: String,
        /// Runtime action or request that triggered activation.
        activation_source: String,
    },
    /// Skill activation was rejected.
    ActivationRejected {
        /// Name of the skill that was not activated.
        skill_name: String,
        /// Safe explanation of the rejection.
        reason: String,
    },
}

// ── Internal helpers ────────────────────────────────────────────

/// Thread-safe sequence generator for debug events.
pub(crate) struct SequenceGenerator {
    counter: AtomicU64,
}

impl SequenceGenerator {
    /// Creates a generator whose first returned sequence is one.
    pub(crate) fn new() -> Self {
        Self {
            counter: AtomicU64::new(1),
        }
    }

    /// Atomically returns the next runtime-local sequence number.
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
    /// Correlation scope copied into events built from this context.
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

    /// Sets the provider name in this context and returns it for chaining.
    pub fn with_provider(&mut self, provider: impl Into<String>) -> &mut Self {
        self.scope.provider_name = Some(provider.into());
        self
    }

    /// Sets the model identifier in this context and returns it for chaining.
    pub fn with_model(&mut self, model: impl Into<String>) -> &mut Self {
        self.scope.model_id = Some(model.into());
        self
    }

    /// Builds a timestamped [`DebugEvent`] by cloning this context's scope.
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
/// Debug sinks used by this module's tests and crate-internal test suites.
pub(crate) mod test_helpers {
    use super::*;
    use std::sync::Mutex;

    /// A debug sink that records all emitted events for test inspection.
    #[derive(Debug, Default)]
    pub struct RecordingDebugSink {
        events: Mutex<Vec<DebugEvent>>,
    }

    impl RecordingDebugSink {
        /// Creates an empty recording sink.
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
