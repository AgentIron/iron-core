use futures::StreamExt;
use iron_core::tool::FunctionTool;
use iron_core::{
    ActiveContextAccountant, ActiveContextSnapshot, CompactedContext, CompactionEngine,
    CompactionReason, ContextCategory, ContextManagementConfig, ContextQuality, ContextTelemetry,
    Decision, DurableSession, HandoffBundle, HandoffExportConfig, HandoffExporter, HandoffImporter,
    PortabilityNote, SessionId, StructuredMessage, TailRetentionPolicy, TailRetentionRule,
    ToolRegistry, UnresolvedQuestion,
};
use iron_providers::Message;

fn make_session_with_messages(n: usize) -> DurableSession {
    let mut session = DurableSession::new(SessionId::new());
    for i in 0..n {
        session.add_user_text(format!("User message {} with some content", i));
        session.add_agent_text(format!("Agent response {} with some content", i));
    }
    session
}

#[test]
fn telemetry_empty_session_reports_unknown_quality() {
    let registry = ToolRegistry::new();
    let snapshot = ContextTelemetry::for_session(None, None, &[], &registry, None, None);
    assert_eq!(snapshot.total_tokens, 0);
    assert_eq!(snapshot.quality, ContextQuality::Unknown);
    assert!(snapshot.categories.is_empty());
    assert!(snapshot.fullness().is_none());
}

#[test]
fn telemetry_with_instructions_counts_category() {
    let registry = ToolRegistry::new();
    let snapshot = ContextTelemetry::for_session(
        Some("You are a helpful assistant. Be concise and accurate."),
        None,
        &[],
        &registry,
        None,
        Some(128_000),
    );
    assert!(snapshot.total_tokens > 0);
    assert_eq!(snapshot.quality, ContextQuality::Estimated);
    assert!(snapshot
        .categories
        .iter()
        .any(|c| c.category == ContextCategory::Instructions));
    assert_eq!(snapshot.context_window_limit, Some(128_000));
    let fullness = snapshot.fullness().unwrap();
    assert!(fullness > 0.0 && fullness < 1.0);
}

#[test]
fn telemetry_with_messages_counts_tail_category() {
    let messages = vec![
        Message::user("Hello"),
        Message::assistant("Hi there"),
        Message::user("How are you?"),
    ];
    let registry = ToolRegistry::new();
    let snapshot = ContextTelemetry::for_session(None, None, &messages, &registry, None, None);
    assert!(snapshot.total_tokens > 0);
    assert!(snapshot
        .categories
        .iter()
        .any(|c| c.category == ContextCategory::RecentTail));
}

#[test]
fn telemetry_with_tools_counts_tool_definitions_category() {
    let mut registry = ToolRegistry::new();
    let _def = iron_core::ToolDefinition::new(
        "test_tool",
        "A test tool",
        serde_json::json!({"type": "object", "properties": {"arg": {"type": "string"}}}),
    );
    registry.register(FunctionTool::simple("test_tool", "A test tool", |_args| {
        Ok(serde_json::json!({}))
    }));

    let snapshot = ContextTelemetry::for_session(None, None, &[], &registry, None, None);
    assert!(snapshot.total_tokens > 0);
    assert!(snapshot
        .categories
        .iter()
        .any(|c| c.category == ContextCategory::ToolDefinitions));
}

#[test]
fn telemetry_with_current_prompt_counts_prompt_category() {
    let registry = ToolRegistry::new();
    let snapshot = ContextTelemetry::for_session(
        None,
        None,
        &[],
        &registry,
        Some("What is the weather?"),
        None,
    );
    assert!(snapshot.total_tokens > 0);
    assert!(snapshot
        .categories
        .iter()
        .any(|c| c.category == ContextCategory::CurrentPrompt));
}

#[test]
fn telemetry_with_compacted_context_counts_category() {
    let ctx = CompactedContext::new()
        .with_objective("Build a REST API")
        .add_fact("Using Rust and Actix")
        .add_decision(Decision::new("Use PostgreSQL").with_rationale("team familiarity"));

    let registry = ToolRegistry::new();
    let snapshot = ContextTelemetry::for_session(None, Some(&ctx), &[], &registry, None, None);
    assert!(snapshot.total_tokens > 0);
    assert!(snapshot
        .categories
        .iter()
        .any(|c| c.category == ContextCategory::CompactedContext));
}

#[test]
fn telemetry_totals_match_category_sum() {
    let ctx = CompactedContext::new()
        .with_objective("Test objective")
        .add_fact("Fact one");

    let messages = vec![Message::user("Hello there"), Message::assistant("Hi!")];

    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::simple("my_tool", "Does something", |_args| {
        Ok(serde_json::json!({}))
    }));

    let snapshot = ContextTelemetry::for_session(
        Some("System instructions here"),
        Some(&ctx),
        &messages,
        &registry,
        Some("User prompt"),
        Some(128_000),
    );

    let category_sum: usize = snapshot.categories.iter().map(|c| c.tokens).sum();
    assert_eq!(snapshot.total_tokens, category_sum);
    assert!(snapshot.fullness().unwrap() > 0.0);
}

#[test]
fn telemetry_estimate_messages_tokens() {
    let messages = vec![
        Message::user("Hello, this is a test message"),
        Message::assistant("I understand, let me help"),
    ];
    let tokens = ActiveContextAccountant::estimate_messages_tokens(&messages);
    assert!(tokens > 0);
}

#[test]
fn telemetry_without_context_window_has_no_fullness() {
    let snapshot = ActiveContextSnapshot {
        total_tokens: 1000,
        context_window_limit: None,
        quality: ContextQuality::Estimated,
        categories: vec![],
    };
    assert!(snapshot.fullness().is_none());
}

#[test]
fn config_default_is_disabled() {
    let config = ContextManagementConfig::default();
    assert!(!config.enabled);
}

#[test]
fn config_builder_enables() {
    let config = ContextManagementConfig::new().enabled();
    assert!(config.enabled);
}

#[test]
fn config_validate_rejects_zero_threshold() {
    let mut config = ContextManagementConfig::new().enabled();
    config.maintenance_threshold = 0;
    assert!(config.validate().is_err());
}

#[test]
fn config_validate_rejects_zero_window_hint() {
    let config = ContextManagementConfig::new()
        .enabled()
        .with_context_window_hint(0);
    assert!(config.validate().is_err());
}

#[test]
fn config_validate_accepts_valid() {
    let config = ContextManagementConfig::new()
        .enabled()
        .with_maintenance_threshold(10_000)
        .with_context_window_hint(128_000);
    assert!(config.validate().is_ok());
}

#[test]
fn config_validate_skips_when_disabled() {
    let config = ContextManagementConfig {
        maintenance_threshold: 0,
        ..Default::default()
    };
    assert!(config.validate().is_ok());
}

#[test]
fn handoff_export_config_validate_rejects_zero() {
    let config = HandoffExportConfig::default();
    let mut bad = config.clone();
    bad.default_target_tokens = 0;
    assert!(bad.validate().is_err());
}

#[test]
fn tail_retention_messages_validate_rejects_zero() {
    let rule = TailRetentionRule::Messages(0);
    assert!(rule.validate().is_err());
}

#[test]
fn tail_retention_tokens_validate_rejects_zero() {
    let rule = TailRetentionRule::Tokens(0);
    assert!(rule.validate().is_err());
}

#[test]
fn tail_retention_policy_validate_rejects_zero_min() {
    let policy = TailRetentionPolicy {
        min_messages: 0,
        max_tokens: None,
    };
    assert!(policy.validate().is_err());
}

#[test]
fn compaction_should_compact_false_when_disabled() {
    assert!(!CompactionEngine::should_compact(100_000, 50_000, false));
}

#[test]
fn compaction_should_compact_true_when_over_threshold() {
    assert!(CompactionEngine::should_compact(60_000, 50_000, true));
}

#[test]
fn compaction_should_compact_false_when_under_threshold() {
    assert!(!CompactionEngine::should_compact(30_000, 50_000, true));
}

#[test]
fn compaction_split_session_messages_rule() {
    let session = make_session_with_messages(10);
    let rule = TailRetentionRule::Messages(4);

    let (older, tail) = CompactionEngine::split_session(&session, &rule);

    assert_eq!(tail.len(), 4);
    assert_eq!(older.len(), 16);
    assert_eq!(session.messages.len(), 20);
}

#[test]
fn compaction_split_session_tokens_rule() {
    let session = make_session_with_messages(10);
    let rule = TailRetentionRule::Tokens(50);

    let (older, tail) = CompactionEngine::split_session(&session, &rule);

    assert!(!tail.is_empty());
    assert_eq!(older.len() + tail.len(), session.messages.len());
}

#[test]
fn compaction_split_session_policy_rule() {
    let session = make_session_with_messages(10);
    let policy = TailRetentionPolicy {
        min_messages: 2,
        max_tokens: Some(100),
    };
    let rule = TailRetentionRule::Policy(policy);

    let (older, tail) = CompactionEngine::split_session(&session, &rule);

    assert!(tail.len() >= 2);
    assert_eq!(older.len() + tail.len(), session.messages.len());
}

#[test]
fn compaction_split_session_small_session() {
    let session = make_session_with_messages(2);
    let rule = TailRetentionRule::Messages(10);

    let (older, tail) = CompactionEngine::split_session(&session, &rule);

    assert!(older.is_empty());
    assert_eq!(tail.len(), 4);
}

#[test]
fn compaction_build_input_includes_previous_context() {
    let previous = CompactedContext::new()
        .with_objective("Test objective")
        .add_fact("Established fact");

    let input = CompactionEngine::build_compaction_input(
        Some(&previous),
        &[Message::user("New message")],
        CompactionReason::Maintenance,
    );

    assert!(input.contains("Previous compacted context"));
    assert!(input.contains("Test objective"));
    assert!(input.contains("Maintenance"));
    assert!(input.contains("New message"));
}

#[test]
fn compaction_build_input_without_previous() {
    let input = CompactionEngine::build_compaction_input(None, &[], CompactionReason::Checkpoint);

    assert!(!input.contains("Previous compacted context"));
    assert!(input.contains("Checkpoint"));
}

#[test]
fn compaction_parse_compacted_context_valid_json() {
    let json = r#"{"objective": "Build something", "established_facts": ["fact1"]}"#;
    let result = CompactionEngine::parse_compacted_context(json);
    assert!(result.is_ok());
    let ctx = result.unwrap();
    assert_eq!(ctx.objective.as_deref(), Some("Build something"));
    assert_eq!(ctx.established_facts.as_ref().unwrap().len(), 1);
}

#[test]
fn compaction_parse_compacted_context_wrapped_in_code_block() {
    let raw = "Here is the summary:\n```json\n{\"objective\": \"Test\"}\n```\nDone.";
    let result = CompactionEngine::parse_compacted_context(raw);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().objective.as_deref(), Some("Test"));
}

#[test]
fn compaction_parse_compacted_context_invalid_json() {
    let result = CompactionEngine::parse_compacted_context("not json at all");
    assert!(result.is_err());
}

#[test]
fn compaction_reconstruct_messages_includes_summary_and_tail() {
    let ctx = CompactedContext::new()
        .with_objective("Build API")
        .add_fact("Using Rust");

    let tail = vec![
        StructuredMessage::user_text("Latest question"),
        StructuredMessage::agent_text("Latest answer"),
    ];

    let result = CompactionEngine::reconstruct_messages(&tail, &ctx);

    assert_eq!(result.len(), 3);
    assert!(result[0].is_agent());
    let first_text = result[0].text_content();
    assert!(first_text.contains("[Compacted session context]"));
    assert!(first_text.contains("Build API"));
    assert!(result[1].is_user());
    assert_eq!(result[1].text_content(), "Latest question");
}

#[test]
fn durable_session_tracks_uncompacted_tokens() {
    let mut session = DurableSession::new(SessionId::new());
    assert_eq!(session.uncompacted_tokens, 0);

    session.add_user_text("Hello, this is a test");
    assert!(session.uncompacted_tokens > 0);

    let tokens_before = session.uncompacted_tokens;
    session.add_agent_text("Response here");
    assert!(session.uncompacted_tokens > tokens_before);
}

#[test]
fn durable_session_apply_compaction_resets_uncompacted_tokens() {
    let mut session = make_session_with_messages(5);
    assert!(session.uncompacted_tokens > 0);

    let compacted = CompactedContext::new().with_objective("Test");
    let tail = vec![session.messages.pop().unwrap()];
    session.apply_compaction(compacted, tail);

    assert_eq!(session.uncompacted_tokens, 0);
    assert!(session.compacted_context.is_some());
}

#[test]
fn durable_session_tracks_tool_tokens_for_compaction() {
    let mut session = DurableSession::new(SessionId::new());

    session.propose_tool_call("call-1", "lookup", serde_json::json!({"id": 1}));
    assert!(session.uncompacted_tokens > 0);

    let after_call = session.uncompacted_tokens;
    session.complete_tool_call("call-1", serde_json::json!({"value": 42}));
    assert!(session.uncompacted_tokens > after_call);
}

#[test]
fn durable_session_apply_compaction_prunes_historical_tool_records() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("first question");
    session.propose_tool_call("call-1", "lookup", serde_json::json!({"id": 1}));
    session.complete_tool_call("call-1", serde_json::json!({"value": 42}));
    session.add_agent_text("first answer");
    session.add_user_text("latest question");

    let compacted = CompactedContext::new().with_objective("Preserve the result semantically");
    let tail = vec![session.messages.last().unwrap().clone()];
    session.apply_compaction(compacted, tail);

    assert_eq!(session.messages.len(), 1);
    assert_eq!(session.messages[0].text_content(), "latest question");
    assert!(session.tool_records.is_empty());
    assert_eq!(session.timeline.len(), 1);
    assert!(matches!(
        session.timeline[0],
        iron_core::TimelineEntry::UserMessage { .. }
    ));
}

#[test]
fn durable_session_is_idle_initially() {
    let session = DurableSession::new(SessionId::new());
    assert!(session.is_idle());
}

#[test]
fn durable_session_idle_when_no_active_tools() {
    let session = DurableSession::new(SessionId::new());
    assert!(session.is_idle());
}

#[test]
fn durable_session_not_idle_with_pending_tool() {
    let mut session = DurableSession::new(SessionId::new());
    session.propose_tool_call("call-1", "tool", serde_json::json!({}));
    assert!(!session.is_idle());
}

#[test]
fn handoff_export_idle_session_succeeds() {
    let session = make_session_with_messages(3);
    let config = ContextManagementConfig::default();

    let result = HandoffExporter::export(
        &session,
        "gpt-4o",
        None,
        vec![session.messages.last().unwrap().clone()],
        &config,
        Some("openai"),
    );

    assert!(result.is_ok());
    let bundle = result.unwrap();
    assert_eq!(bundle.version, "1");
    assert_eq!(bundle.metadata.source_model, "gpt-4o");
    assert_eq!(bundle.metadata.source_provider.as_deref(), Some("openai"));
    assert!(bundle.metadata.size_estimate_tokens > 0);
    assert!(bundle.handoff_note.contains("transferred from session"));
}

#[test]
fn handoff_export_rejects_active_session() {
    let mut session = make_session_with_messages(1);
    session.propose_tool_call("call-1", "tool", serde_json::json!({}));
    let config = ContextManagementConfig::default();

    let result = HandoffExporter::export(&session, "gpt-4o", None, vec![], &config, None);

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("active tool calls"));
}

#[test]
fn handoff_export_includes_provenance() {
    let session = make_session_with_messages(1);
    let config = ContextManagementConfig::default();

    let bundle = HandoffExporter::export(&session, "gpt-4o", None, vec![], &config, None).unwrap();

    assert!(!bundle.handoff_note.is_empty());
    assert!(bundle.handoff_note.contains("gpt-4o"));
    assert!(bundle.handoff_note.contains(&session.id.to_string()));
}

#[test]
fn handoff_export_detects_local_resources() {
    use iron_core::ContentBlock;

    let mut session = DurableSession::new(SessionId::new());
    session.add_user_message(vec![
        ContentBlock::text("Check this file"),
        ContentBlock::Resource {
            uri: "file:///tmp/secret.txt".into(),
            name: Some("secret".into()),
        },
    ]);
    let config = ContextManagementConfig::default();

    let bundle = HandoffExporter::export(
        &session,
        "gpt-4o",
        None,
        session.messages.clone(),
        &config,
        None,
    )
    .unwrap();

    assert!(bundle.handoff_note.contains("may not be accessible"));
    assert!(bundle.compacted_context.portability_notes.is_some());
}

#[test]
fn handoff_export_default_15k_target() {
    let config = ContextManagementConfig::default();
    assert_eq!(config.handoff_export.default_target_tokens, 15_000);
}

#[test]
fn handoff_export_custom_target() {
    let config = ContextManagementConfig::default().with_handoff_export(HandoffExportConfig {
        default_target_tokens: 5_000,
        include_portability_notes: true,
    });
    assert_eq!(config.handoff_export.default_target_tokens, 5_000);
}

#[test]
fn handoff_hydrate_into_new_session() {
    let session = make_session_with_messages(2);
    let config = ContextManagementConfig::default();

    let bundle = HandoffExporter::export(
        &session,
        "gpt-4o",
        Some(&CompactedContext::new().with_objective("Test objective")),
        session.messages.clone(),
        &config,
        None,
    )
    .unwrap();

    let new_session = HandoffImporter::hydrate_into_new(bundle);

    assert!(!new_session.messages.is_empty());
    let all_text: String = new_session
        .messages
        .iter()
        .map(|m| m.text_content())
        .collect();
    assert!(all_text.contains("[Handoff]"));
    assert!(all_text.contains("Test objective"));
}

#[test]
fn handoff_bundle_serialization_round_trip() {
    let session = make_session_with_messages(1);
    let config = ContextManagementConfig::default();

    let bundle = HandoffExporter::export(
        &session,
        "gpt-4o",
        Some(&CompactedContext::new().with_objective("Serialization test")),
        session.messages.clone(),
        &config,
        None,
    )
    .unwrap();

    let json = serde_json::to_string(&bundle).unwrap();
    let deserialized: HandoffBundle = serde_json::from_str(&json).unwrap();

    assert_eq!(bundle, deserialized);
}

#[test]
fn compacted_context_is_empty_when_default() {
    let ctx = CompactedContext::new();
    assert!(ctx.is_empty());
    assert!(ctx.render_to_text().is_empty());
}

#[test]
fn compacted_context_render_includes_all_fields() {
    let ctx = CompactedContext::new()
        .with_objective("Build a web app")
        .with_next_step("Set up database")
        .add_fact("Using PostgreSQL")
        .add_user_constraint("Must be fast")
        .add_decision(Decision::new("Use Actix").with_rationale("team preference"))
        .add_unresolved_question(UnresolvedQuestion::new("Which cache?").blocking())
        .add_recent_result("Server starts on port 8080")
        .with_notes("Freeform notes here");

    let text = ctx.render_to_text();
    assert!(text.contains("Build a web app"));
    assert!(text.contains("Set up database"));
    assert!(text.contains("PostgreSQL"));
    assert!(text.contains("fast"));
    assert!(text.contains("Actix"));
    assert!(text.contains("team preference"));
    assert!(text.contains("Which cache?"));
    assert!(text.contains("[BLOCKING]"));
    assert!(text.contains("port 8080"));
    assert!(text.contains("Freeform notes"));
}

#[test]
fn compacted_context_sparse_fields_not_rendered() {
    let ctx = CompactedContext::new().with_objective("Just an objective");
    let text = ctx.render_to_text();
    assert!(text.contains("Just an objective"));
    assert!(!text.contains("Next step"));
    assert!(!text.contains("Established facts"));
}

#[test]
fn portability_note_portable() {
    let note = PortabilityNote::portable("This is portable");
    assert!(!note.non_portable);
    assert!(note.reason.is_none());
}

#[test]
fn portability_note_non_portable() {
    let note = PortabilityNote::non_portable("Local file ref", "file:///tmp/data");
    assert!(note.non_portable);
    assert_eq!(note.reason.as_deref(), Some("file:///tmp/data"));
}

#[test]
fn unresolved_question_blocking_flag() {
    let q = UnresolvedQuestion::new("Which DB?").blocking();
    assert!(q.blocking);

    let q2 = UnresolvedQuestion::new("Color scheme?");
    assert!(!q2.blocking);
}

#[test]
fn decision_with_rationale() {
    let d = Decision::new("Use Rust").with_rationale("Performance");
    assert_eq!(d.decision, "Use Rust");
    assert_eq!(d.rationale.as_deref(), Some("Performance"));
}

#[test]
fn config_integration_with_main_config() {
    let config =
        iron_core::Config::new().with_context_management(ContextManagementConfig::new().enabled());
    assert!(config.context_management.enabled);
    assert!(config.validate().is_ok());
}

#[test]
fn config_integration_validates_context_management() {
    let mut ctx_config = ContextManagementConfig::new().enabled();
    ctx_config.maintenance_threshold = 0;
    let config = iron_core::Config::new().with_context_management(ctx_config);
    assert!(config.validate().is_err());
}

#[test]
fn request_builder_keeps_compacted_context_when_recent_tail_is_pruned() {
    let config = iron_core::Config::new()
        .with_context_window_policy(iron_core::ContextWindowPolicy::KeepRecent(1));
    let compacted = CompactedContext::new().with_objective("Carry this forward");
    let registry = ToolRegistry::new();
    let messages = vec![Message::user("older"), Message::user("latest")];

    let request = iron_core::request_builder::build_inference_request_with_context(
        &config,
        &messages,
        Some(&compacted),
        None,
        &registry,
    )
    .unwrap();

    assert_eq!(request.context.transcript.messages.len(), 2);
    assert!(matches!(
        &request.context.transcript.messages[0],
        Message::Assistant { content }
            if content.contains("[Compacted session context]")
                && content.contains("Carry this forward")
    ));
    assert!(matches!(
        &request.context.transcript.messages[1],
        Message::User { content } if content == "latest"
    ));
}

// =========================================================================
// Runtime integration tests: prepare/execute, facade methods
// =========================================================================

#[test]
fn compaction_prepare_extracts_tail_and_builds_prompt() {
    let mut session = make_session_with_messages(10);
    session.set_instructions("You are a helper");
    let config = ContextManagementConfig::new()
        .enabled()
        .with_tail_retention(TailRetentionRule::Messages(4));

    let input = CompactionEngine::prepare(
        &session,
        &config.tail_retention,
        CompactionReason::Maintenance,
    );

    assert!(!input.prompt_text.is_empty());
    assert!(input.prompt_text.contains("Maintenance"));
    assert_eq!(input.tail.len(), 4);
}

#[test]
fn compaction_prepare_with_existing_context_includes_previous() {
    let mut session = make_session_with_messages(5);
    session.compacted_context = Some(CompactedContext::new().with_objective("Build something"));
    let config = ContextManagementConfig::new();

    let input = CompactionEngine::prepare(
        &session,
        &config.tail_retention,
        CompactionReason::Checkpoint,
    );

    assert!(input.prompt_text.contains("Previous compacted context"));
    assert!(input.prompt_text.contains("Build something"));
}

#[test]
fn compaction_prepare_includes_historical_tool_transcript() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("Need a lookup");
    session.propose_tool_call("call-1", "lookup", serde_json::json!({"id": 7}));
    session.complete_tool_call("call-1", serde_json::json!({"value": "seven"}));
    session.add_agent_text("Lookup complete");
    session.add_user_text("What next?");

    let config = ContextManagementConfig::new().with_tail_retention(TailRetentionRule::Messages(1));

    let input = CompactionEngine::prepare(
        &session,
        &config.tail_retention,
        CompactionReason::Maintenance,
    );

    assert!(input.prompt_text.contains("assistant_tool_call lookup"));
    assert!(input.prompt_text.contains("tool_result lookup"));
    assert!(input.prompt_text.contains("seven"));
}

// ---------------------------------------------------------------------------
// Mock provider for async integration tests
// ---------------------------------------------------------------------------

use std::collections::VecDeque;
use std::sync::Arc as StdArc;
use std::sync::Mutex as StdMutex;

#[derive(Clone, Default)]
struct MockProvider {
    infer_responses: StdArc<StdMutex<VecDeque<Vec<iron_providers::ProviderEvent>>>>,
    requests: StdArc<StdMutex<Vec<iron_providers::InferenceRequest>>>,
}

impl MockProvider {
    fn with_infer_responses(responses: Vec<Vec<iron_providers::ProviderEvent>>) -> Self {
        Self {
            infer_responses: StdArc::new(StdMutex::new(responses.into())),
            ..Self::default()
        }
    }

    fn requests(&self) -> Vec<iron_providers::InferenceRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl iron_providers::Provider for MockProvider {
    fn infer(
        &self,
        request: iron_providers::InferenceRequest,
    ) -> iron_providers::ProviderFuture<'_, Vec<iron_providers::ProviderEvent>> {
        self.requests.lock().unwrap().push(request);
        let response = self
            .infer_responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![iron_providers::ProviderEvent::Complete]);
        Box::pin(async move { Ok(response) })
    }

    fn infer_stream(
        &self,
        request: iron_providers::InferenceRequest,
    ) -> iron_providers::ProviderFuture<
        '_,
        futures::stream::BoxStream<
            'static,
            iron_providers::ProviderResult<iron_providers::ProviderEvent>,
        >,
    > {
        self.requests.lock().unwrap().push(request);
        let response = self
            .infer_responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![iron_providers::ProviderEvent::Complete]);
        let stream = futures::stream::iter(response.into_iter().map(Ok));
        Box::pin(async move { Ok(stream.boxed()) })
    }
}

fn run_local<F>(future: F) -> F::Output
where
    F: std::future::Future,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build current_thread runtime");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, future)
}

#[test]
fn compaction_execute_calls_provider_and_parses_result() {
    run_local(async {
        let mut session = make_session_with_messages(10);
        session.compacted_context = None;
        let config = ContextManagementConfig::new();

        let input = CompactionEngine::prepare(
            &session,
            &config.tail_retention,
            CompactionReason::Maintenance,
        );

        let provider = MockProvider::with_infer_responses(vec![vec![
            iron_providers::ProviderEvent::Output {
                content: r#"{"objective": "Testing compaction", "established_facts": ["fact1"]}"#
                    .into(),
            },
            iron_providers::ProviderEvent::Complete,
        ]]);

        let result = CompactionEngine::execute(input, &provider, "gpt-4o").await;
        assert!(result.is_ok());

        let (compacted, tail) = result.unwrap();
        assert_eq!(compacted.objective.as_deref(), Some("Testing compaction"));
        assert!(!tail.is_empty());
    });
}

#[test]
fn compaction_execute_handles_provider_error() {
    run_local(async {
        let session = make_session_with_messages(5);
        let config = ContextManagementConfig::new();

        let input =
            CompactionEngine::prepare(&session, &config.tail_retention, CompactionReason::HardFit);

        let provider =
            MockProvider::with_infer_responses(vec![vec![iron_providers::ProviderEvent::Error {
                source: iron_providers::ProviderError::general("Provider unavailable"),
            }]]);

        let result = CompactionEngine::execute(input, &provider, "gpt-4o").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Provider unavailable"));
    });
}

#[test]
fn compaction_execute_handles_empty_output() {
    run_local(async {
        let session = make_session_with_messages(5);
        let config = ContextManagementConfig::new();

        let input = CompactionEngine::prepare(
            &session,
            &config.tail_retention,
            CompactionReason::Maintenance,
        );

        let provider =
            MockProvider::with_infer_responses(vec![vec![iron_providers::ProviderEvent::Complete]]);

        let result = CompactionEngine::execute(input, &provider, "gpt-4o").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty output"));
    });
}

#[test]
fn compaction_execute_handles_malformed_json() {
    run_local(async {
        let session = make_session_with_messages(5);
        let config = ContextManagementConfig::new();

        let input = CompactionEngine::prepare(
            &session,
            &config.tail_retention,
            CompactionReason::Maintenance,
        );

        let provider = MockProvider::with_infer_responses(vec![vec![
            iron_providers::ProviderEvent::Output {
                content: "This is not JSON at all".into(),
            },
            iron_providers::ProviderEvent::Complete,
        ]]);

        let result = CompactionEngine::execute(input, &provider, "gpt-4o").await;
        assert!(result.is_err());
    });
}

#[test]
fn facade_checkpoint_triggers_compaction() {
    run_local(async {
        use iron_core::{CompactionCheckpoint, Config, IronAgent, PromptOutcome};
        use iron_providers::ProviderEvent;

        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::Output {
                    content: "Hello!".into(),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: r#"{"objective": "Post-checkpoint", "next_step": "Continue"}"#.into(),
                },
                ProviderEvent::Complete,
            ],
        ]);

        let config = Config::new().with_context_management(
            ContextManagementConfig::new()
                .enabled()
                .with_maintenance_threshold(999_999),
        );

        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        session.set_instructions("Test instructions");

        let outcome = session.prompt("hello").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);

        assert!(session.is_idle());
        assert!(session.uncompacted_tokens() > 0);

        let result = session.checkpoint(CompactionCheckpoint::TaskComplete).await;
        assert!(result.is_ok());

        let ctx = session.compacted_context();
        assert!(ctx.is_some());
        assert_eq!(ctx.unwrap().objective.as_deref(), Some("Post-checkpoint"));
        assert_eq!(session.uncompacted_tokens(), 0);
    });
}

#[test]
fn facade_checkpoint_rejects_non_idle_session() {
    use iron_core::{Config, ContextManagementConfig, IronAgent};

    let config = Config::new().with_context_management(ContextManagementConfig::new().enabled());

    let provider = MockProvider::with_infer_responses(vec![]);
    let agent = IronAgent::new(config, provider);
    let conn = agent.connect();
    let session = conn.create_session().unwrap();

    assert!(session.is_idle());
}

#[test]
fn facade_checkpoint_rejects_disabled_context_management() {
    run_local(async {
        use iron_core::{CompactionCheckpoint, Config, IronAgent};

        let config = Config::new();
        let provider = MockProvider::with_infer_responses(vec![]);
        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let result = session.checkpoint(CompactionCheckpoint::TaskComplete).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not enabled"));
    });
}

#[test]
fn facade_export_handoff_returns_bundle() {
    use iron_core::{Config, IronAgent};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "Hi!".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let config =
            Config::new().with_context_management(ContextManagementConfig::new().enabled());

        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        session.set_instructions("Be helpful");
        let outcome = session.prompt("hello").await;
        assert_eq!(outcome, iron_core::PromptOutcome::EndTurn);

        let bundle = session.export_handoff("gpt-4o", Some("openai")).await;
        assert!(bundle.is_ok());

        let bundle = bundle.unwrap();
        assert_eq!(bundle.metadata.source_model, "gpt-4o");
        assert_eq!(bundle.metadata.source_provider.as_deref(), Some("openai"));
        assert!(bundle.metadata.size_estimate_tokens > 0);
    });
}

#[test]
fn facade_create_session_from_handoff() {
    use iron_core::{Config, ContextManagementConfig, IronAgent};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "Hello!".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let config = Config::new().with_context_management(
            ContextManagementConfig::new()
                .enabled()
                .with_maintenance_threshold(999_999),
        );

        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();

        let original = conn.create_session().unwrap();
        original.set_instructions("Be helpful");
        let _ = original.prompt("hello").await;

        let bundle = original.export_handoff("gpt-4o", None).await.unwrap();

        let imported = conn.create_session_from_handoff(bundle).unwrap();

        let messages = imported.messages();
        let all_text: String = messages.iter().map(|m| m.text_content()).collect();
        assert!(all_text.contains("[Handoff]"));
    });
}

#[test]
fn prompt_flow_sets_turn_active_during_prompt() {
    use iron_core::{Config, ContextManagementConfig, IronAgent};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "Response".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let config = Config::new().with_context_management(
            ContextManagementConfig::new()
                .enabled()
                .with_maintenance_threshold(999_999),
        );

        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let outcome = session.prompt("test").await;
        assert_eq!(outcome, iron_core::PromptOutcome::EndTurn);
        assert!(session.is_idle());
    });
}

#[test]
fn post_turn_compaction_triggers_when_threshold_exceeded() {
    use iron_core::{Config, ContextManagementConfig, IronAgent, TailRetentionRule};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::Output {
                    content: "First response".into(),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: r#"{"objective": "Auto-compacted", "established_facts": ["test"]}"#
                        .into(),
                },
                ProviderEvent::Complete,
            ],
        ]);

        let config = Config::new().with_context_management(
            ContextManagementConfig::new()
                .enabled()
                .with_maintenance_threshold(1)
                .with_tail_retention(TailRetentionRule::Messages(2)),
        );

        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        session.set_instructions("Test instructions");

        let _ = session.prompt("first message").await;
        assert!(session.is_idle());

        let ctx = session.compacted_context();
        assert!(ctx.is_some());
        assert_eq!(ctx.unwrap().objective.as_deref(), Some("Auto-compacted"));
        assert_eq!(session.uncompacted_tokens(), 0);
    });
}

#[test]
fn post_turn_compaction_skipped_when_under_threshold() {
    use iron_core::{Config, ContextManagementConfig, IronAgent};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "Short".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let config = Config::new().with_context_management(
            ContextManagementConfig::new()
                .enabled()
                .with_maintenance_threshold(999_999),
        );

        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let _ = session.prompt("hello").await;
        assert!(session.is_idle());

        assert!(session.compacted_context().is_none());
        assert!(session.uncompacted_tokens() > 0);
    });
}

#[test]
fn tool_heavy_post_turn_compaction_shapes_future_requests() {
    use iron_core::{Config, ContextManagementConfig, IronAgent, PromptOutcome, TailRetentionRule};
    use iron_providers::{Message as ProviderMessage, ProviderEvent, ToolCall};

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("tc1", "lookup", serde_json::json!({"id": 1})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "done".into(),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content:
                        r#"{"objective":"Lookup complete","recent_results":["lookup returned 42"]}"#
                            .into(),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "next answer".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);

        let config = Config::new().with_context_management(
            ContextManagementConfig::new()
                .enabled()
                .with_maintenance_threshold(5)
                .with_tail_retention(TailRetentionRule::Messages(1)),
        );

        let agent = IronAgent::new(config, provider.clone());
        agent.register_tool(FunctionTool::simple("lookup", "lookup", |_| {
            Ok(serde_json::json!({"value": 42}))
        }));

        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let first = session.prompt("go").await;
        assert_eq!(first, PromptOutcome::EndTurn);
        assert!(session.compacted_context().is_some());
        assert_eq!(session.uncompacted_tokens(), 0);

        let second = session.prompt("what now?").await;
        assert_eq!(second, PromptOutcome::EndTurn);

        let requests = provider.requests();
        let second_prompt_request = &requests[3];
        let transcript = &second_prompt_request.context.transcript.messages;

        assert!(matches!(
            &transcript[0],
            ProviderMessage::Assistant { content }
                if content.contains("[Compacted session context]")
                    && content.contains("Lookup complete")
        ));
        assert!(transcript.iter().any(|msg| {
            matches!(msg, ProviderMessage::Assistant { content } if content == "done")
        }));
        assert!(transcript.iter().any(|msg| {
            matches!(msg, ProviderMessage::User { content } if content == "what now?")
        }));
        assert!(!transcript.iter().any(|msg| {
            matches!(
                msg,
                ProviderMessage::AssistantToolCall { .. } | ProviderMessage::Tool { .. }
            )
        }));
    });
}

#[test]
fn tool_heavy_hard_fit_compaction_shapes_future_request() {
    use iron_core::{Config, ContextManagementConfig, IronAgent, PromptOutcome, TailRetentionRule};
    use iron_providers::{Message as ProviderMessage, ProviderEvent, ToolCall};

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("tc1", "lookup", serde_json::json!({"id": 1})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "done".into(),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: r#"{"objective":"Hard fit summary","recent_results":["lookup returned 42"]}"#.into(),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "after compaction".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);

        let config = Config::new().with_context_management(
            ContextManagementConfig::new()
                .enabled()
                .with_maintenance_threshold(999_999)
                .with_context_window_hint(5)
                .with_tail_retention(TailRetentionRule::Messages(1)),
        );

        let agent = IronAgent::new(config, provider.clone());
        agent.register_tool(FunctionTool::simple("lookup", "lookup", |_| {
            Ok(serde_json::json!({"value": 42}))
        }));

        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let first = session.prompt("go").await;
        assert_eq!(first, PromptOutcome::EndTurn);
        assert!(session.compacted_context().is_none());

        let second = session.prompt("what now?").await;
        assert_eq!(second, PromptOutcome::EndTurn);
        assert!(session.compacted_context().is_some());

        let requests = provider.requests();
        let second_prompt_request = &requests[3];
        let transcript = &second_prompt_request.context.transcript.messages;

        assert!(matches!(
            &transcript[0],
            ProviderMessage::Assistant { content }
                if content.contains("[Compacted session context]")
                    && content.contains("Hard fit summary")
        ));
        assert!(!transcript.iter().any(|msg| {
            matches!(
                msg,
                ProviderMessage::AssistantToolCall { .. } | ProviderMessage::Tool { .. }
            )
        }));
        assert!(transcript.iter().any(|msg| {
            matches!(msg, ProviderMessage::User { content } if content == "what now?")
        }));
    });
}
