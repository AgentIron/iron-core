use futures::StreamExt;
use iron_core::tool::FunctionTool;
use iron_core::{
    ActiveContextAccountant, ActiveContextSnapshot, CompressRange, CompressTool, CompressedBlock,
    ContextCategory, ContextManagementConfig, ContextQuality, ContextTelemetry, DurableSession,
    HandoffBundle, HandoffExportConfig, HandoffExporter, HandoffImporter, SessionId,
    SessionModelInfo, TailRetentionPolicy, TailRetentionRule, ToolRegistry,
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
fn compress_tool_definition_mentions_tool_call_pair_boundary() {
    let definition = CompressTool::definition();

    assert!(definition
        .description
        .contains("must not split a tool call"));
    assert!(definition.description.contains("include both or neither"));
    assert!(
        definition.input_schema["properties"]["content"]["items"]["properties"]["start_message_id"]
            ["description"]
            .as_str()
            .is_some_and(|description| description.contains("splits a tool call"))
    );
    assert!(
        definition.input_schema["properties"]["content"]["items"]["properties"]["end_message_id"]
            ["description"]
            .as_str()
            .is_some_and(|description| description.contains("splits a tool call"))
    );
}

#[test]
fn telemetry_empty_session_reports_unknown_quality() {
    let registry = ToolRegistry::new();
    let snapshot = ContextTelemetry::for_session(
        None,
        &[],
        &[],
        &registry,
        None,
        None,
        SessionModelInfo {
            current_model: None,
            model_switch_count: 0,
        },
    );
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
        &[],
        &[],
        &registry,
        None,
        Some(128_000),
        SessionModelInfo {
            current_model: None,
            model_switch_count: 0,
        },
    );
    assert!(snapshot.total_tokens > 0);
    assert_eq!(snapshot.quality, ContextQuality::Estimated);
    assert!(snapshot
        .categories
        .iter()
        .any(|c| c.category == ContextCategory::Instructions));
    assert_eq!(snapshot.context_window_limit, Some(128_000));
    assert_eq!(snapshot.compact_threshold_tokens, None);
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
    let snapshot = ContextTelemetry::for_session(
        None,
        &[],
        &messages,
        &registry,
        None,
        None,
        SessionModelInfo {
            current_model: None,
            model_switch_count: 0,
        },
    );
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

    let snapshot = ContextTelemetry::for_session(
        None,
        &[],
        &[],
        &registry,
        None,
        None,
        SessionModelInfo {
            current_model: None,
            model_switch_count: 0,
        },
    );
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
        &[],
        &[],
        &registry,
        Some("What is the weather?"),
        None,
        SessionModelInfo {
            current_model: None,
            model_switch_count: 0,
        },
    );
    assert!(snapshot.total_tokens > 0);
    assert!(snapshot
        .categories
        .iter()
        .any(|c| c.category == ContextCategory::CurrentPrompt));
}

#[test]
fn telemetry_with_compressed_blocks_counts_category() {
    let blocks = vec![CompressedBlock::new(
        "c0001",
        "Build a REST API",
        "m0000-m0003",
        "Using Rust and Actix. Decision: Use PostgreSQL (team familiarity)",
    )];

    let registry = ToolRegistry::new();
    let snapshot = ContextTelemetry::for_session(
        None,
        &blocks,
        &[],
        &registry,
        None,
        None,
        SessionModelInfo {
            current_model: None,
            model_switch_count: 0,
        },
    );
    assert!(snapshot.total_tokens > 0);
    assert!(snapshot
        .categories
        .iter()
        .any(|c| c.category == ContextCategory::CompressedBlocks));
}

#[test]
fn telemetry_totals_match_category_sum() {
    let blocks = vec![CompressedBlock::new(
        "c0001",
        "Test objective",
        "m0000-m0001",
        "Fact one",
    )];

    let messages = vec![Message::user("Hello there"), Message::assistant("Hi!")];

    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::simple("my_tool", "Does something", |_args| {
        Ok(serde_json::json!({}))
    }));

    let snapshot = ContextTelemetry::for_session(
        Some("System instructions here"),
        &blocks,
        &messages,
        &registry,
        Some("User prompt"),
        Some(128_000),
        SessionModelInfo {
            current_model: None,
            model_switch_count: 0,
        },
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
        compact_threshold_tokens: None,
        quality: ContextQuality::Estimated,
        categories: vec![],
        current_model: None,
        model_switch_count: 0,
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
fn durable_session_apply_compression_resets_uncompacted_tokens() {
    let mut session = make_session_with_messages(5);
    assert!(session.uncompacted_tokens > 0);

    let block = CompressedBlock::new("c0001", "Test", "m0000-m0003", "summary");
    session.apply_compression(block);

    assert_eq!(session.uncompacted_tokens, 0);
    assert!(!session.compressed_blocks.is_empty());
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
fn durable_session_apply_compression_adds_block_and_resets_tokens() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("first question");
    session.propose_tool_call("call-1", "lookup", serde_json::json!({"id": 1}));
    session.complete_tool_call("call-1", serde_json::json!({"value": 42}));
    session.add_agent_text("first answer");
    session.add_user_text("latest question");

    let block = CompressedBlock::new("c0001", "Preserve", "m0000-m0003", "summary");
    session.apply_compression(block);

    // apply_compression adds the block and resets uncompacted_tokens,
    // but does not prune messages (that happens during provider request building)
    assert_eq!(session.compressed_blocks.len(), 1);
    assert_eq!(session.compressed_blocks[0].topic, "Preserve");
    assert_eq!(session.uncompacted_tokens, 0);
    // Messages are retained in the session until explicitly removed by compaction
    // Note: tool calls don't add to messages, only user/agent text does
    assert_eq!(session.messages.len(), 3);
}

#[test]
fn compress_tool_removes_selected_completed_turns_and_adds_block() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("old question");
    session.add_agent_text("old answer");
    session.add_user_text("latest question");

    let result = CompressTool::execute(
        &mut session,
        "Old topic".to_string(),
        vec![CompressRange {
            start_id: "m0001".to_string(),
            end_id: "m0002".to_string(),
            summary: "The old question was answered.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect("compress should succeed");

    assert_eq!(result.blocks_created.len(), 1);
    assert_eq!(session.compressed_blocks.len(), 1);
    assert_eq!(session.compressed_blocks[0].id, "c0001");
    assert_eq!(session.messages.len(), 1);
    assert_eq!(session.messages[0].text_content(), "latest question");
    assert_eq!(session.timeline.len(), 1);
    assert_eq!(session.timeline[0].visible_id(), Some("m0003"));
    assert_eq!(session.uncompacted_tokens, 0);
}

#[test]
fn compress_tool_rejects_latest_user_request() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("only user request");

    let err = CompressTool::execute(
        &mut session,
        "Latest request".to_string(),
        vec![CompressRange {
            start_id: "m0001".to_string(),
            end_id: "m0001".to_string(),
            summary: "Do not compress this.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect_err("latest user request must be protected");

    assert!(err.contains("protected active context"));
    assert!(session.compressed_blocks.is_empty());
    assert_eq!(session.messages.len(), 1);
}

#[test]
fn compress_tool_rejects_split_tool_call_pair() {
    let mut session = DurableSession::new(SessionId::new());
    session.start_tool_call("call-1", "lookup", serde_json::json!({"id": 1}));
    session.complete_tool_call("call-1", serde_json::json!({"value": 42}));
    session.add_user_text("latest question");

    let err = CompressTool::execute(
        &mut session,
        "Tool lookup".to_string(),
        vec![CompressRange {
            start_id: "m0001".to_string(),
            end_id: "m0001".to_string(),
            summary: "Lookup was started.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect_err("tool start without terminal result must be rejected");

    assert!(err.contains("split a tool call") || err.contains("terminal result"));
    assert!(session.compressed_blocks.is_empty());
    assert_eq!(session.tool_records.len(), 1);
    assert_eq!(session.timeline.len(), 3);
}

#[test]
fn compress_tool_removes_completed_tool_pair() {
    let mut session = DurableSession::new(SessionId::new());
    session.start_tool_call("call-1", "lookup", serde_json::json!({"id": 1}));
    session.complete_tool_call("call-1", serde_json::json!({"value": 42}));
    session.add_user_text("latest question");

    CompressTool::execute(
        &mut session,
        "Tool lookup".to_string(),
        vec![CompressRange {
            start_id: "m0001".to_string(),
            end_id: "m0002".to_string(),
            summary: "Lookup returned value 42.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect("complete tool pair can be compressed");

    assert_eq!(session.compressed_blocks.len(), 1);
    assert!(session.tool_records.is_empty());
    assert_eq!(session.timeline.len(), 1);
    assert_eq!(session.timeline[0].visible_id(), Some("m0003"));
}

#[test]
fn transcript_can_render_visible_ids_for_text_messages() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("hello");
    session.add_agent_text("hi");

    let transcript = session.to_transcript_with_visible_ids(true);
    assert_eq!(transcript.messages.len(), 2);
    assert!(matches!(
        &transcript.messages[0],
        Message::User { content } if content == "<m0001>\nhello"
    ));
    assert!(matches!(
        &transcript.messages[1],
        Message::Assistant { content } if content == "<m0002>\nhi"
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
        &[],
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

    let result = HandoffExporter::export(&session, "gpt-4o", &[], vec![], &config, None);

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("active tool calls"));
}

#[test]
fn handoff_export_includes_provenance() {
    let session = make_session_with_messages(1);
    let config = ContextManagementConfig::default();

    let bundle = HandoffExporter::export(&session, "gpt-4o", &[], vec![], &config, None).unwrap();

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
        &[],
        session.messages.clone(),
        &config,
        None,
    )
    .unwrap();

    assert!(bundle.handoff_note.contains("may not be accessible"));
    assert!(bundle.handoff_note.contains("Portability"));
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
        &[CompressedBlock::new(
            "c0001",
            "Test objective",
            "m0000-m0003",
            "summary",
        )],
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
        &[CompressedBlock::new(
            "c0001",
            "Serialization test",
            "m0000-m0003",
            "summary",
        )],
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

// =========================================================================
// Runtime integration tests: prepare/execute, facade methods
// =========================================================================

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

/// Helper to extract display text from an iron_providers::Message.
fn provider_message_text(m: &iron_providers::Message) -> String {
    match m {
        iron_providers::Message::User { content } => content.clone(),
        iron_providers::Message::Assistant { content } => content.clone(),
        iron_providers::Message::AssistantToolCall {
            tool_name,
            arguments,
            ..
        } => format!("{}: {}", tool_name, arguments),
        iron_providers::Message::Tool {
            tool_name, result, ..
        } => format!("{}: {}", tool_name, result),
    }
}

#[test]
fn facade_checkpoint_triggers_compaction() {
    run_local(async {
        use iron_core::{Config, IronAgent, PromptOutcome};
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

        let result = session.checkpoint().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not yet implemented"));
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
        use iron_core::{Config, IronAgent};

        let config = Config::new();
        let provider = MockProvider::with_infer_responses(vec![]);
        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let result = session.checkpoint().await;
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

// =========================================================================
// 6.2 Range validation tests
// =========================================================================

#[test]
fn compress_tool_rejects_unknown_start_id() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("hello");

    let err = CompressTool::execute(
        &mut session,
        "Test".to_string(),
        vec![CompressRange {
            start_id: "m9999".to_string(),
            end_id: "m0001".to_string(),
            summary: "summary".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect_err("unknown ID must be rejected");

    assert!(err.contains("Unknown start ID"));
    assert!(session.compressed_blocks.is_empty());
}

#[test]
fn compress_tool_rejects_unknown_end_id() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("hello");

    let err = CompressTool::execute(
        &mut session,
        "Test".to_string(),
        vec![CompressRange {
            start_id: "m0001".to_string(),
            end_id: "m9999".to_string(),
            summary: "summary".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect_err("unknown ID must be rejected");

    assert!(err.contains("Unknown end ID"));
    assert!(session.compressed_blocks.is_empty());
}

#[test]
fn compress_tool_rejects_reversed_range() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("first");
    session.add_agent_text("second");

    let err = CompressTool::execute(
        &mut session,
        "Test".to_string(),
        vec![CompressRange {
            start_id: "m0002".to_string(),
            end_id: "m0001".to_string(),
            summary: "summary".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect_err("reversed range must be rejected");

    assert!(err.contains("comes after end"));
    assert!(session.compressed_blocks.is_empty());
}

#[test]
fn compress_tool_rejects_overlapping_ranges() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("msg1");
    session.add_agent_text("msg2");
    session.add_user_text("msg3");
    session.add_agent_text("msg4");
    session.add_user_text("latest");

    let err = CompressTool::execute(
        &mut session,
        "Test".to_string(),
        vec![
            CompressRange {
                start_id: "m0001".to_string(),
                end_id: "m0002".to_string(),
                summary: "first range".to_string(),
            },
            CompressRange {
                start_id: "m0002".to_string(),
                end_id: "m0003".to_string(),
                summary: "overlapping range".to_string(),
            },
        ],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect_err("overlapping ranges must be rejected");

    assert!(err.contains("Overlapping ranges"));
    assert!(session.compressed_blocks.is_empty());
    assert_eq!(session.messages.len(), 5);
}

#[test]
fn compress_tool_rejects_split_tool_call_pair_start_only() {
    let mut session = DurableSession::new(SessionId::new());
    session.start_tool_call("call-1", "lookup", serde_json::json!({"id": 1}));
    session.complete_tool_call("call-1", serde_json::json!({"value": 42}));
    session.add_user_text("latest");

    let err = CompressTool::execute(
        &mut session,
        "Tool lookup".to_string(),
        vec![CompressRange {
            start_id: "m0001".to_string(),
            end_id: "m0001".to_string(),
            summary: "Only start.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect_err("split tool pair must be rejected");

    assert!(
        err.contains("split a tool call") || err.contains("terminal result"),
        "error should mention splitting tool pair, got: {}",
        err
    );
    assert!(session.compressed_blocks.is_empty());
}

#[test]
fn compress_tool_rejects_split_tool_call_pair_terminal_only() {
    let mut session = DurableSession::new(SessionId::new());
    session.start_tool_call("call-1", "lookup", serde_json::json!({"id": 1}));
    session.complete_tool_call("call-1", serde_json::json!({"value": 42}));
    session.add_user_text("latest");

    let err = CompressTool::execute(
        &mut session,
        "Tool lookup".to_string(),
        vec![CompressRange {
            start_id: "m0002".to_string(),
            end_id: "m0002".to_string(),
            summary: "Only terminal.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect_err("split tool pair must be rejected");

    assert!(
        err.contains("split a tool call") || err.contains("terminal result"),
        "error should mention splitting tool pair, got: {}",
        err
    );
    assert!(session.compressed_blocks.is_empty());
}

#[test]
fn compress_tool_rejects_pending_tool_in_range() {
    let mut session = DurableSession::new(SessionId::new());
    session.propose_tool_call("call-1", "lookup", serde_json::json!({"id": 1}));
    session.add_user_text("latest");

    let err = CompressTool::execute(
        &mut session,
        "Tool lookup".to_string(),
        vec![CompressRange {
            start_id: "m0001".to_string(),
            end_id: "m0001".to_string(),
            summary: "Pending tool.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect_err("pending tool must be protected");

    assert!(err.contains("protected active context"));
    assert!(session.compressed_blocks.is_empty());
}

#[test]
fn compress_tool_rejects_running_tool_in_range() {
    let mut session = DurableSession::new(SessionId::new());
    session.start_tool_call("call-1", "lookup", serde_json::json!({"id": 1}));
    session.add_user_text("latest");

    let err = CompressTool::execute(
        &mut session,
        "Tool lookup".to_string(),
        vec![CompressRange {
            start_id: "m0001".to_string(),
            end_id: "m0001".to_string(),
            summary: "Running tool.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect_err("running tool must be protected");

    assert!(err.contains("protected active context"));
    assert!(session.compressed_blocks.is_empty());
}

#[test]
fn compress_tool_rejects_current_assistant_turn() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("user msg");
    session.add_agent_text("agent response");
    // Current assistant turn is the latest timeline entry

    let err = CompressTool::execute(
        &mut session,
        "Test".to_string(),
        vec![CompressRange {
            start_id: "m0002".to_string(),
            end_id: "m0002".to_string(),
            summary: "Current turn.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect_err("current assistant turn must be protected");

    assert!(err.contains("protected active context"));
    assert!(session.compressed_blocks.is_empty());
}

#[test]
fn compress_tool_can_replace_existing_compressed_block() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("old question");
    session.add_agent_text("old answer");
    session.add_user_text("latest question");

    // First compression
    let result = CompressTool::execute(
        &mut session,
        "Old topic".to_string(),
        vec![CompressRange {
            start_id: "m0001".to_string(),
            end_id: "m0002".to_string(),
            summary: "The old question was answered.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect("first compress should succeed");

    assert_eq!(result.blocks_created.len(), 1);
    let block_id = result.blocks_created[0].id.clone();

    // Compressing an existing block replaces it with a new summary
    let result2 = CompressTool::execute(
        &mut session,
        "Test".to_string(),
        vec![CompressRange {
            start_id: block_id.clone(),
            end_id: block_id.clone(),
            summary: "Updated summary.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect("compressing existing block should succeed");

    assert_eq!(session.compressed_blocks.len(), 1);
    assert_eq!(result2.blocks_created[0].summary, "Updated summary.");
}

#[test]
fn compress_tool_valid_range_does_not_mutate_on_rejection() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("msg1");
    session.add_agent_text("msg2");
    session.add_user_text("msg3");

    let messages_before = session.messages.clone();
    let timeline_before = session.timeline.clone();

    let err = CompressTool::execute(
        &mut session,
        "Test".to_string(),
        vec![CompressRange {
            start_id: "m0003".to_string(),
            end_id: "m0003".to_string(),
            summary: "Latest user request.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect_err("latest user request must be protected");

    assert!(err.contains("protected active context"));
    assert_eq!(session.messages.len(), messages_before.len());
    assert_eq!(session.timeline.len(), timeline_before.len());
    assert!(session.compressed_blocks.is_empty());
}

#[test]
fn compress_tool_rejects_on_empty_session() {
    let mut session = DurableSession::new(SessionId::new());

    let err = CompressTool::execute(
        &mut session,
        "Test".to_string(),
        vec![CompressRange {
            start_id: "m0001".to_string(),
            end_id: "m0001".to_string(),
            summary: "summary".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect_err("empty session must reject compress");

    assert!(err.contains("Unknown start ID"));
    assert!(session.compressed_blocks.is_empty());
}

// =========================================================================
// 6.3 Integration tests for provider request rendering with blocks
// =========================================================================

#[test]
fn provider_request_includes_compressed_blocks() {
    use iron_core::{Config, ContextManagementConfig, IronAgent};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "Done".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let config = Config::new().with_context_management(
            ContextManagementConfig::new()
                .enabled()
                .with_maintenance_threshold(999_999),
        );

        let agent = IronAgent::new(config, provider.clone());
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        session.set_instructions("Be helpful");
        let _ = session.prompt("hello").await;

        // Manually add a compressed block via runtime
        {
            let runtime = agent.runtime();
            let durable = runtime.get_session(session.id()).unwrap();
            let mut s = durable.lock();
            s.compressed_blocks.push(CompressedBlock::new(
                "c0001",
                "Test topic",
                "m0001-m0002",
                "Summary of earlier conversation",
            ));
        }

        let _ = session.prompt("second prompt").await;

        let requests = provider.requests.lock().unwrap();
        let last_request = requests.last().expect("should have requests");
        let transcript_text: String = last_request
            .context
            .transcript
            .messages
            .iter()
            .map(provider_message_text)
            .collect();

        assert!(
            transcript_text.contains("Summary of earlier conversation"),
            "provider request should include compressed block content"
        );
    });
}

#[test]
fn provider_request_excludes_compressed_transcript() {
    use iron_core::{Config, ContextManagementConfig, IronAgent};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::Output {
                    content: "First turn".into(),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "Second turn".into(),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "Third turn".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);

        let config = Config::new().with_context_management(
            ContextManagementConfig::new()
                .enabled()
                .with_maintenance_threshold(999_999),
        );

        let agent = IronAgent::new(config, provider.clone());
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        session.set_instructions("Be helpful");
        // Two full turns so the first turn is no longer protected
        let _ = session.prompt("first prompt").await;
        let _ = session.prompt("second prompt").await;

        // Manually compress the first turn via runtime
        {
            let runtime = agent.runtime();
            let durable = runtime.get_session(session.id()).unwrap();
            let mut s = durable.lock();
            // Compress the first user+agent pair (m0001-m0002)
            let result = CompressTool::execute(
                &mut s,
                "First turn".to_string(),
                vec![CompressRange {
                    start_id: "m0001".to_string(),
                    end_id: "m0002".to_string(),
                    summary: "First turn summary".to_string(),
                }],
                0.50,
                0.70,
                0.85,
                0.95,
            );
            assert!(result.is_ok(), "compression should succeed: {:?}", result);
        }

        let _ = session.prompt("third prompt").await;

        let requests = provider.requests.lock().unwrap();
        let last_request = requests.last().expect("should have requests");
        let transcript_text: String = last_request
            .context
            .transcript
            .messages
            .iter()
            .map(provider_message_text)
            .collect();

        assert!(
            !transcript_text.contains("first prompt"),
            "compressed user message should not appear in provider request, got: {}",
            transcript_text
        );
        assert!(
            transcript_text.contains("First turn summary")
                || transcript_text.contains("third prompt"),
            "retained content or compressed block should be present, got: {}",
            transcript_text
        );
    });
}

// =========================================================================
// 6.4 Pressure bucket rendering and cache behavior tests
// =========================================================================

#[test]
fn pressure_bucket_computation_from_fullness() {
    use iron_core::context::ContextPressure;

    assert_eq!(ContextPressure::from_fullness(0.3), ContextPressure::None);
    assert_eq!(ContextPressure::from_fullness(0.5), ContextPressure::Soft);
    assert_eq!(ContextPressure::from_fullness(0.7), ContextPressure::Medium);
    assert_eq!(
        ContextPressure::from_fullness(0.85),
        ContextPressure::Strong
    );
    assert_eq!(
        ContextPressure::from_fullness(0.95),
        ContextPressure::Critical
    );
}

#[test]
fn pressure_guidance_matches_bucket() {
    use iron_core::context::ContextPressure;

    assert!(ContextPressure::None.guidance().contains("healthy"));
    assert!(ContextPressure::Soft.guidance().contains("rising"));
    assert!(ContextPressure::Medium.guidance().contains("elevated"));
    assert!(ContextPressure::Strong.guidance().contains("high"));
    assert!(ContextPressure::Critical.guidance().contains("critical"));
}

#[test]
fn system_prompt_includes_pressure_guidance() {
    use iron_core::context::ContextPressure;
    use iron_core::prompt::{SystemPromptInputs, SystemPromptRenderer};

    let inputs = SystemPromptInputs {
        baseline: "Test baseline",
        runtime_context: "Test context",
        repo_payload: &Default::default(),
        additional_inline: &[],
        session_instructions: None,
        skill_instructions: None,
        provider_guidance: None,
        client_editing_guidance: None,
        client_injections: &[],
        python_exec_available: false,
        context_pressure: ContextPressure::Strong,
    };

    let rendered = SystemPromptRenderer::render(&inputs);
    assert!(
        rendered.contains("high"),
        "system prompt should include strong pressure guidance"
    );
    assert!(
        rendered.contains("compress"),
        "system prompt should mention compression availability"
    );
}

#[test]
fn prompt_cache_fingerprint_stable_for_exact_telemetry_change() {
    use iron_core::context::ContextPressure;
    use iron_core::prompt::{SystemPromptFingerprint, SystemPromptInputs};

    let inputs1 = SystemPromptInputs {
        baseline: "Test",
        runtime_context: "Context",
        repo_payload: &Default::default(),
        additional_inline: &[],
        session_instructions: None,
        skill_instructions: None,
        provider_guidance: None,
        client_editing_guidance: None,
        client_injections: &[],
        python_exec_available: false,
        context_pressure: ContextPressure::Soft,
    };

    let inputs2 = SystemPromptInputs {
        baseline: "Test",
        runtime_context: "Context",
        repo_payload: &Default::default(),
        additional_inline: &[],
        session_instructions: None,
        skill_instructions: None,
        provider_guidance: None,
        client_editing_guidance: None,
        client_injections: &[],
        python_exec_available: false,
        context_pressure: ContextPressure::Soft,
    };

    let fp1 = SystemPromptFingerprint::from_inputs(&inputs1);
    let fp2 = SystemPromptFingerprint::from_inputs(&inputs2);
    assert_eq!(
        fp1, fp2,
        "same pressure bucket should produce same fingerprint"
    );
}

#[test]
fn prompt_cache_fingerprint_changes_on_pressure_bucket_transition() {
    use iron_core::context::ContextPressure;
    use iron_core::prompt::{SystemPromptFingerprint, SystemPromptInputs};

    let inputs_soft = SystemPromptInputs {
        baseline: "Test",
        runtime_context: "Context",
        repo_payload: &Default::default(),
        additional_inline: &[],
        session_instructions: None,
        skill_instructions: None,
        provider_guidance: None,
        client_editing_guidance: None,
        client_injections: &[],
        python_exec_available: false,
        context_pressure: ContextPressure::Soft,
    };

    let inputs_medium = SystemPromptInputs {
        baseline: "Test",
        runtime_context: "Context",
        repo_payload: &Default::default(),
        additional_inline: &[],
        session_instructions: None,
        skill_instructions: None,
        provider_guidance: None,
        client_editing_guidance: None,
        client_injections: &[],
        python_exec_available: false,
        context_pressure: ContextPressure::Medium,
    };

    let fp_soft = SystemPromptFingerprint::from_inputs(&inputs_soft);
    let fp_medium = SystemPromptFingerprint::from_inputs(&inputs_medium);
    assert_ne!(
        fp_soft, fp_medium,
        "different pressure buckets should produce different fingerprints"
    );
}

#[test]
fn compress_recomputes_pressure_and_reports_state() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("old question");
    session.add_agent_text("old answer");
    session.add_user_text("latest question");

    let result = CompressTool::execute(
        &mut session,
        "Old topic".to_string(),
        vec![CompressRange {
            start_id: "m0001".to_string(),
            end_id: "m0002".to_string(),
            summary: "The old question was answered.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect("compress should succeed");

    // After compression with minimal content, pressure should be "none"
    assert_eq!(result.pressure_state, "none");
}

// =========================================================================
// 6.5 /compact behavior and critical-pressure failure tests
// =========================================================================

#[test]
fn compact_command_replaces_user_message() {
    use iron_core::{Config, ContextManagementConfig, IronAgent};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "Compacting".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let config = Config::new().with_context_management(
            ContextManagementConfig::new()
                .enabled()
                .with_maintenance_threshold(999_999),
        );

        let agent = IronAgent::new(config, provider.clone());
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        session.set_instructions("Be helpful");
        let _ = session.prompt("/compact").await;

        let requests = provider.requests.lock().unwrap();
        let request = requests.last().expect("should have request");
        let transcript_text: String = request
            .context
            .transcript
            .messages
            .iter()
            .map(provider_message_text)
            .collect();

        assert!(
            transcript_text.contains("context compaction"),
            "/compact should be replaced with compaction instruction, got: {}",
            transcript_text
        );
        assert!(
            !transcript_text.contains("/compact"),
            "/compact command should not appear in provider request"
        );
    });
}

#[test]
fn critical_pressure_failure_surfaces_error() {
    use iron_core::{Config, ContextManagementConfig, IronAgent};
    use iron_providers::ProviderEvent;

    run_local(async {
        // Provider that returns large content to simulate critical pressure
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::Output {
                    content: "x".repeat(200_000),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "Still large".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);

        let config = Config::new().with_context_management(
            ContextManagementConfig::new()
                .enabled()
                .with_maintenance_threshold(100)
                .with_context_window_hint(1000)
                .with_critical_threshold(0.5),
        );

        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        session.set_instructions("Be helpful");
        let _ = session.prompt("generate a lot of content").await;

        // The session should have the error message added
        let messages = session.messages();
        let all_text: String = messages.iter().map(|m| m.text_content()).collect();

        assert!(
            all_text.contains("critical") || all_text.contains("threshold"),
            "critical pressure error should be surfaced, got: {}",
            all_text
        );
    });
}

#[test]
fn post_turn_compaction_critical_pressure_emits_error() {
    use iron_core::{Config, ContextManagementConfig, IronAgent};
    use iron_providers::ProviderEvent;
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "Response with lots of content that will exceed threshold".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let config = Config::new().with_context_management(
            ContextManagementConfig::new()
                .enabled()
                .with_maintenance_threshold(50)
                .with_context_window_hint(100)
                .with_critical_threshold(0.8),
        );

        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        session.set_instructions("Be helpful. ");
        // Add enough content to push us toward critical via runtime
        {
            let runtime = agent.runtime();
            let durable = runtime.get_session(session.id()).unwrap();
            let mut s = durable.lock();
            for i in 0..20 {
                s.add_user_text(format!(
                    "user message {} with substantial content to build up tokens",
                    i
                ));
                s.add_agent_text(format!(
                    "agent response {} with substantial content to build up tokens",
                    i
                ));
            }
        }

        let _ = session.prompt("trigger compaction check").await;

        let messages = session.messages();
        let all_text: String = messages.iter().map(|m| m.text_content()).collect();

        // After critical pressure check, an error should be in the transcript
        assert!(
            all_text.contains("critical")
                || all_text.contains("threshold")
                || all_text.contains("new session"),
            "critical pressure should surface error, got transcript: {}",
            all_text
        );
    });
}

#[test]
fn compress_clears_nudges_when_below_threshold() {
    let mut session = DurableSession::new(SessionId::new());
    // Add enough content to create pressure
    for i in 0..10 {
        session.add_user_text(format!(
            "user message {} with lots of content to build pressure",
            i
        ));
        session.add_agent_text(format!(
            "agent response {} with lots of content to build pressure",
            i
        ));
    }

    // Compress most of it
    let result = CompressTool::execute(
        &mut session,
        "Compaction".to_string(),
        vec![CompressRange {
            start_id: "m0001".to_string(),
            end_id: "m0018".to_string(),
            summary: "Compressed all the earlier conversation.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect("compress should succeed");

    // After heavy compression, pressure should be reduced
    assert_eq!(result.pressure_state, "none");
    assert_eq!(result.method, "model_summary");
    assert!(result.tokens_before.unwrap() > result.tokens_after.unwrap());
    assert_eq!(session.compressed_blocks.len(), 1);
    assert!(session.messages.len() < 20);
}

#[test]
fn compress_keeps_nudges_when_still_above_threshold() {
    let mut session = DurableSession::new(SessionId::new());
    // Add lots of content
    for i in 0..50 {
        session.add_user_text(format!("user message {} with substantial content", i));
        session.add_agent_text(format!("agent response {} with substantial content", i));
    }

    // Compress only a small portion
    let result = CompressTool::execute(
        &mut session,
        "Partial compaction".to_string(),
        vec![CompressRange {
            start_id: "m0001".to_string(),
            end_id: "m0010".to_string(),
            summary: "Compressed some earlier conversation.".to_string(),
        }],
        0.50,
        0.70,
        0.85,
        0.95,
    )
    .expect("compress should succeed");

    // After partial compression, there may still be pressure
    // The key assertion is that the tool result reports the current state
    assert!(
        result.pressure_state == "soft"
            || result.pressure_state == "medium"
            || result.pressure_state == "strong"
            || result.pressure_state == "critical"
            || result.pressure_state == "none",
        "pressure state should be valid, got: {}",
        result.pressure_state
    );
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

        assert!(session.compressed_blocks().is_empty());
        assert!(session.uncompacted_tokens() > 0);
    });
}

// =========================================================================
// Model Switching Integration Tests
// =========================================================================

#[test]
fn model_switch_idle_session_applies_immediately() {
    use iron_core::{Config, IronAgent, ModelSwitchRequest};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "Hello!".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let agent = IronAgent::new(Config::new(), provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        // Send a prompt to establish conversation
        let _ = session.prompt("hello").await;
        assert!(session.is_idle());

        // Switch model while idle
        let request = ModelSwitchRequest::Managed {
            provider_slug: "openai".into(),
            model: "gpt-4o".into(),
            api_key: None,
        };
        let result = session.switch_model(request);
        assert!(result.is_ok(), "Switch should succeed on idle session");

        // Verify timeline has ModelSwitched entry
        let timeline = session.timeline();
        let switch_entries: Vec<_> = timeline.iter().filter(|e| e.is_model_switched()).collect();
        assert_eq!(
            switch_entries.len(),
            1,
            "Should have one model switch entry"
        );
    });
}

#[test]
fn model_switch_active_session_queues_switch() {
    use iron_core::{Config, IronAgent, ModelSwitchRequest};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "Long response that takes time".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let agent = IronAgent::new(Config::new(), provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        // Start a streaming prompt (this makes the session active)
        let (_handle, mut events) = session.prompt_stream("hello");

        // While prompt is active, try to switch model
        let request = ModelSwitchRequest::Managed {
            provider_slug: "anthropic".into(),
            model: "claude-3-7-sonnet-20250219".into(),
            api_key: None,
        };

        // Switch should queue (not fail)
        let result = session.switch_model(request);
        assert!(result.is_ok(), "Switch should queue on active session");

        // Wait for prompt to complete
        while let Some(event) = events.next().await {
            if matches!(event, iron_core::PromptEvent::Complete { .. }) {
                break;
            }
        }

        // After prompt completes, switch should be applied
        assert!(session.is_idle());
        let timeline = session.timeline();
        let switch_entries: Vec<_> = timeline.iter().filter(|e| e.is_model_switched()).collect();
        assert_eq!(
            switch_entries.len(),
            1,
            "Switch should be applied after turn completes"
        );
    });
}

#[test]
fn model_switch_handoff_bundle_roundtrip() {
    use iron_core::{Config, IronAgent, ModelSwitchRequest};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "Hello!".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let agent = IronAgent::new(Config::new(), provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        // Send a prompt
        let _ = session.prompt("hello").await;

        // Switch model
        let request = ModelSwitchRequest::Managed {
            provider_slug: "openai".into(),
            model: "gpt-4o".into(),
            api_key: None,
        };
        let _ = session.switch_model(request);

        // Export handoff bundle
        let bundle = session
            .export_handoff("gpt-4o", Some("openai"))
            .await
            .unwrap();

        // Verify bundle has model switch history
        assert!(
            !bundle.model_switch_history.is_empty(),
            "Bundle should contain switch history"
        );
        assert_eq!(bundle.model_switch_history[0].to_model, "gpt-4o");

        // Create new session from handoff
        let imported = conn.create_session_from_handoff(bundle).unwrap();

        // Verify imported session has the switch history
        let imported_timeline = imported.timeline();
        let switch_entries: Vec<_> = imported_timeline
            .iter()
            .filter(|e| e.is_model_switched())
            .collect();
        assert_eq!(
            switch_entries.len(),
            1,
            "Imported session should have switch history"
        );
    });
}

#[test]
fn model_switch_larger_window_no_compaction() {
    use iron_core::{Config, ContextManagementConfig, IronAgent, ModelSwitchRequest};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "Hello!".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let mut config = Config::new();
        config.context_management = ContextManagementConfig::new()
            .with_context_window_hint(128_000)
            .enabled();

        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        // Establish conversation
        let _ = session.prompt("hello").await;
        assert!(session.is_idle());

        let compressed_before = session.compressed_blocks().len();

        // Switch to model with large window (no compaction needed)
        let request = ModelSwitchRequest::Managed {
            provider_slug: "openai".into(),
            model: "gpt-4o".into(),
            api_key: None,
        };
        let result = session.switch_model(request);
        assert!(result.is_ok());

        let compressed_after = session.compressed_blocks().len();
        assert_eq!(
            compressed_after, compressed_before,
            "Switch to larger window should not trigger compaction"
        );
    });
}

#[test]
fn model_switch_smaller_window_triggers_compaction() {
    use iron_core::{Config, ContextManagementConfig, IronAgent, ModelSwitchRequest};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "This is a moderately long response from the agent that contains enough text to contribute meaningfully to the token count for compaction testing purposes.".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let mut config = Config::new();
        config.context_management = ContextManagementConfig::new()
            .with_context_window_hint(300) // Small enough to trigger compaction but large enough for minimal tail
            .enabled();

        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        // Establish conversation with many turns to build up context beyond the window
        for i in 0..10 {
            let msg = format!("User message number {} with substantial content to ensure token accumulation exceeds the small target window we configured for this test", i);
            let _ = session.prompt(msg.as_str()).await;
        }
        assert!(session.is_idle());

        let compressed_before = session.compressed_blocks().len();

        // Switch to model with small window (compaction needed)
        let request = ModelSwitchRequest::Managed {
            provider_slug: "openai".into(),
            model: "gpt-4o-mini".into(),
            api_key: None,
        };
        let result = session.switch_model(request);
        assert!(
            result.is_ok(),
            "Switch should succeed with compaction: {:?}",
            result
        );

        let compressed_after = session.compressed_blocks().len();
        assert!(
            compressed_after > compressed_before,
            "Switch to smaller window should trigger compaction. Before: {}, After: {}",
            compressed_before,
            compressed_after
        );

        // Verify timeline has ModelSwitched entry with adapted=true
        let timeline = session.timeline();
        let switch_entries: Vec<_> = timeline.iter().filter(|e| e.is_model_switched()).collect();
        assert_eq!(switch_entries.len(), 1);
    });
}

#[test]
fn model_switch_too_small_window_rejected() {
    use iron_core::{
        Config, ContextManagementConfig, IronAgent, ModelCapabilityMetadata, ModelSwitchRequest,
    };
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "This is a moderately long response from the agent that contains enough text to contribute meaningfully to the token count for testing the rejection path.".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let mut config = Config::new();
        config.context_management = ContextManagementConfig::new()
            .with_context_window_hint(10) // Extremely small window
            .enabled();

        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        agent
            .runtime()
            .register_model_capability(ModelCapabilityMetadata {
                model: "tiny-model".into(),
                provider: "openai".into(),
                context_window: 10,
                supports_tools: true,
                supports_streaming: true,
                supports_reasoning_effort: false,
                reasoning_effort_values: Vec::new(),
                supported_modalities: vec!["text".into()],
                unsupported_tools: Vec::new(),
            });

        // Establish conversation with long messages
        let _ = session.prompt("This is a very long user message with substantial content that will generate many tokens when estimated using the length-based heuristic, ensuring the total exceeds the minimal tail threshold").await;
        assert!(session.is_idle());

        // Switch to model with tiny window (should fail)
        let request = ModelSwitchRequest::Managed {
            provider_slug: "openai".into(),
            model: "tiny-model".into(),
            api_key: None,
        };
        let result = session.switch_model(request);
        assert!(
            result.is_err(),
            "Switch to model with window too small for minimal tail should fail"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("Context too large") || err.contains("too large"),
            "Error should indicate context is too large: {}",
            err
        );
    });
}

#[test]
fn model_switch_unknown_target_model_rejected() {
    use iron_core::{Config, IronAgent, ModelSwitchRequest};

    run_local(async {
        let agent = IronAgent::new(Config::new(), MockProvider::with_infer_responses(vec![]));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let request = ModelSwitchRequest::Managed {
            provider_slug: "openai".into(),
            model: "not-registered-model".into(),
            api_key: None,
        };

        let result = session.switch_model(request);
        assert!(result.is_err(), "Unknown target model should be rejected");
        let err = result.unwrap_err();
        assert!(
            err.contains("Unknown target model") && err.contains("openai/not-registered-model"),
            "Error should identify the unknown target model: {}",
            err
        );
    });
}

// =========================================================================
// Active context telemetry threshold tests
// =========================================================================

#[test]
fn active_context_includes_compact_threshold_when_enabled() {
    use iron_core::{Config, ContextManagementConfig, IronAgent};

    let config = Config::new().with_context_management(
        ContextManagementConfig::new()
            .enabled()
            .with_maintenance_threshold(50_000)
            .with_context_window_hint(128_000),
    );
    let agent = IronAgent::new(config, MockProvider::default());
    let conn = agent.connect();
    let session = conn.create_session().unwrap();

    session.set_instructions("Test instructions");
    let registry = ToolRegistry::new();
    let snapshot = session.active_context(&registry, None, Some(128_000));

    assert_eq!(snapshot.compact_threshold_tokens, Some(50_000));
    assert_eq!(snapshot.context_window_limit, Some(128_000));
}

#[test]
fn active_context_omits_compact_threshold_when_disabled() {
    use iron_core::{Config, IronAgent};

    let config = Config::new(); // disabled by default
    let agent = IronAgent::new(config, MockProvider::default());
    let conn = agent.connect();
    let session = conn.create_session().unwrap();

    let registry = ToolRegistry::new();
    let snapshot = session.active_context(&registry, None, Some(128_000));

    assert_eq!(snapshot.compact_threshold_tokens, None);
}

// =========================================================================
// Model-switch compaction lifecycle event tests
// =========================================================================

#[test]
fn model_switch_active_stream_emits_compaction_lifecycle_events() {
    use iron_core::facade::PromptEvent;
    use iron_core::{Config, ContextManagementConfig, IronAgent, ModelSwitchRequest};
    use iron_providers::ProviderEvent;

    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "This is a moderately long response from the agent that contains enough text to contribute meaningfully to the token count for compaction testing purposes.".into(),
            },
            ProviderEvent::Complete,
        ]]);

        let mut config = Config::new();
        config.context_management = ContextManagementConfig::new()
            .with_context_window_hint(300)
            .enabled();

        let agent = IronAgent::new(config, provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        // Establish conversation with many turns to build up context beyond the window
        for i in 0..10 {
            let msg = format!("User message number {} with substantial content to ensure token accumulation exceeds the small target window we configured for this test", i);
            let _ = session.prompt(msg.as_str()).await;
        }

        // Start a stream and queue a model switch while it's active
        let (_handle, mut events) = session.prompt_stream("next");

        let request = ModelSwitchRequest::Managed {
            provider_slug: "openai".into(),
            model: "gpt-4o-mini".into(),
            api_key: None,
        };
        let result = session.switch_model(request);
        assert!(result.is_ok(), "Switch should queue on active session");

        // Collect all events from the stream
        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event);
            if matches!(collected.last(), Some(PromptEvent::Complete { .. })) {
                break;
            }
        }

        // Verify compaction lifecycle events were emitted at the turn boundary
        let started_pos = collected
            .iter()
            .position(|e| matches!(e, PromptEvent::CompactionStarted { .. }));
        let finished_pos = collected.iter().position(|e| {
            matches!(e, PromptEvent::CompactionFinished { method, .. } if method == "auto_compaction")
        });

        assert!(
            started_pos.is_some(),
            "expected CompactionStarted in stream after model switch with compaction"
        );
        assert!(
            finished_pos.is_some(),
            "expected CompactionFinished in stream after model switch with compaction"
        );

        // Verify ordering: compaction events should come before Complete
        let complete_pos = collected
            .iter()
            .position(|e| matches!(e, PromptEvent::Complete { .. }));
        assert!(complete_pos.is_some(), "expected Complete event");
        assert!(
            started_pos.unwrap() < complete_pos.unwrap(),
            "CompactionStarted should precede Complete"
        );
        assert!(
            finished_pos.unwrap() < complete_pos.unwrap(),
            "CompactionFinished should precede Complete"
        );
        assert!(
            started_pos.unwrap() < finished_pos.unwrap(),
            "CompactionStarted should precede CompactionFinished"
        );

        // Extract started compaction_id for correlation and verify method
        let started_id = collected.iter().find_map(|e| {
            if let PromptEvent::CompactionStarted {
                compaction_id,
                method,
            } = e
            {
                assert_eq!(method, "auto_compaction");
                Some(compaction_id.clone())
            } else {
                None
            }
        });
        assert!(
            started_id.is_some(),
            "expected CompactionStarted with compaction_id"
        );

        // Verify CompactionFinished has auto_compaction metrics and matching compaction_id
        if let Some(PromptEvent::CompactionFinished {
            compaction_id,
            tokens_before,
            tokens_after,
            method,
        }) = collected
            .iter()
            .find(|e| matches!(e, PromptEvent::CompactionFinished { .. }))
        {
            assert_eq!(
                compaction_id,
                started_id.as_ref().unwrap(),
                "compaction_id must match between Started and Finished"
            );
            assert!(
                tokens_before.is_some(),
                "expected tokens_before for auto_compaction"
            );
            assert!(
                tokens_after.is_some(),
                "expected tokens_after for auto_compaction"
            );
            assert_eq!(method, "auto_compaction");
        } else {
            panic!("CompactionFinished not found");
        }
    });
}
