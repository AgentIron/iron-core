//! Transport-independent tests for the ACP-native runtime.
//!
//! These tests exercise runtime/session ownership, prompt/update ordering,
//! durable timeline commits, permission flow, and cancellation behaviour
//! without any network transport.  The facade tests use a `current_thread`
//! tokio runtime with a `LocalSet` because the ACP SDK uses
//! `#[async_trait(?Send)]`.
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use futures::stream::{self, BoxStream};
use futures::StreamExt;
use iron_core::{
    config::ApprovalStrategy,
    facade::{PermissionVerdict, PromptEvent, PromptOutcome, ToolResultStatus},
    skill::{LoadedSkill, SkillMetadata, SkillOrigin},
    tool::{FunctionTool, ToolDefinition},
    AuthInteractionResponse, AuthInteractionResult, AuthState, Config, ConnectionId, ContentBlock,
    DurableSession, EphemeralTurn, IronAgent, IronRuntime, OAuthRequirements, PluginHealth,
    PluginIdentity, PluginManifest, PluginPublisher, PluginSource, PluginSourceConfig,
    PresentationMetadata, Provider, ProviderEvent, SessionId, ToolAuthRequirements,
    ToolRecordStatus, ToolTerminalOutcome, TurnPhase,
};
use iron_providers::{InferenceRequest, ToolCall};
use serde_json::json;

// ---------------------------------------------------------------------------
// Mock provider (same pattern as existing test files)
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct MockProvider {
    infer_responses: Arc<Mutex<VecDeque<Vec<ProviderEvent>>>>,
    requests: Arc<Mutex<Vec<InferenceRequest>>>,
}

impl MockProvider {
    fn with_infer_responses(responses: Vec<Vec<ProviderEvent>>) -> Self {
        Self {
            infer_responses: Arc::new(Mutex::new(responses.into())),
            ..Self::default()
        }
    }

    #[allow(dead_code)]
    fn requests(&self) -> Vec<InferenceRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Provider for MockProvider {
    fn infer(
        &self,
        request: InferenceRequest,
    ) -> iron_providers::ProviderFuture<'_, Vec<ProviderEvent>> {
        self.requests.lock().unwrap().push(request);
        let response = self
            .infer_responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![ProviderEvent::Complete]);
        Box::pin(async move { Ok(response) })
    }

    fn infer_stream(
        &self,
        request: InferenceRequest,
    ) -> iron_providers::ProviderFuture<
        '_,
        BoxStream<'static, iron_providers::ProviderResult<ProviderEvent>>,
    > {
        self.requests.lock().unwrap().push(request);
        let response = self
            .infer_responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![ProviderEvent::Complete]);
        Box::pin(async move { Ok(stream::iter(response.into_iter().map(Ok)).boxed()) })
    }
}

// ---------------------------------------------------------------------------
// Helper: run an async test on a current_thread runtime with LocalSet
// ---------------------------------------------------------------------------

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

fn register_test_auth_plugin(agent: &IronAgent, plugin_id: &str) {
    agent.register_plugin(PluginSourceConfig {
        id: plugin_id.to_string(),
        source: PluginSource::LocalPath {
            path: "/dev/null".into(),
        },
        enabled_by_default: true,
    });

    let registry = agent.plugin_registry();
    registry.update_health(plugin_id, PluginHealth::Healthy);
    registry.set_manifest(
        plugin_id,
        PluginManifest {
            identity: PluginIdentity {
                id: format!("com.test.{plugin_id}"),
                name: "Auth Test Plugin".to_string(),
                version: "1.0.0".to_string(),
            },
            publisher: PluginPublisher {
                name: "Test".to_string(),
                url: None,
                contact: None,
            },
            presentation: PresentationMetadata {
                description: "Auth test plugin".to_string(),
                long_description: None,
                icon: None,
                category: None,
                keywords: vec![],
            },
            network_policy: iron_core::PluginNetworkPolicy::Wildcard,
            auth: Some(OAuthRequirements {
                provider: "github".to_string(),
                provider_name: "GitHub".to_string(),
                scopes: vec!["repo".to_string()],
                authorization_endpoint: Some(
                    "https://github.com/login/oauth/authorize".to_string(),
                ),
                token_endpoint: None,
            }),
            tools: vec![iron_core::ExportedTool {
                name: "gated_tool".to_string(),
                description: "Gated tool".to_string(),
                input_schema: json!({"type":"object"}),
                requires_approval: false,
                auth_requirements: Some(ToolAuthRequirements {
                    scopes: vec![],
                    available_unauthenticated: false,
                }),
            }],
            max_memory_bytes: None,
            api_version: "1.0".to_string(),
        },
    );
}

fn make_skill(name: &str, description: &str) -> LoadedSkill {
    LoadedSkill {
        metadata: SkillMetadata {
            id: name.to_string(),
            display_name: name.to_string(),
            description: description.to_string(),
            origin: SkillOrigin::ClientProvided,
            auto_activate: false,
            tags: vec![],
            requires_tools: vec![],
            requires_capabilities: vec![],
            requires_trust: false,
        },
        location: None,
        body: format!("# {}\nUse this skill carefully.", name),
        resources: vec![],
    }
}

#[test]
fn activate_skill_returns_structured_content_and_resources() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("sk1", "activate_skill", json!({"skill_name": "review"})),
                },
                ProviderEvent::Complete,
            ],
            vec![ProviderEvent::Complete],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.runtime().register_skill(LoadedSkill {
            metadata: SkillMetadata {
                id: "review".to_string(),
                display_name: "review".to_string(),
                description: "Review code changes".to_string(),
                origin: SkillOrigin::ClientProvided,
                auto_activate: false,
                tags: vec![],
                requires_tools: vec![],
                requires_capabilities: vec![],
                requires_trust: false,
            },
            location: None,
            body: "# review\nUse this skill carefully.".to_string(),
            resources: vec![iron_core::skill::SkillResourceEntry {
                path: "checklist.md".to_string(),
                description: String::new(),
            }],
        });
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (_handle, mut events) = session.prompt_stream("go");

        let mut tool_result = None;
        while let Some(event) = events.next().await {
            if let PromptEvent::ToolResult {
                call_id,
                status,
                result,
                ..
            } = event.clone()
            {
                if call_id == "sk1" {
                    tool_result = Some((status, result));
                }
            }
            if matches!(event, PromptEvent::Complete { .. }) {
                break;
            }
        }

        let (status, result) = tool_result.expect("expected activate_skill result");
        assert_eq!(status, ToolResultStatus::Completed);
        let result = result.expect("expected result payload");
        assert_eq!(result["status"], "activated");
        assert!(result["content"]
            .as_str()
            .unwrap()
            .contains("<skill_content name=\"review\">"));
        assert_eq!(result["resources"][0], "checklist.md");
        assert_eq!(session.list_active_skills(), vec!["review".to_string()]);
    });
}

#[test]
fn activate_skill_rejects_names_outside_session_catalog() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("sk2", "activate_skill", json!({"skill_name": "missing"})),
            },
            ProviderEvent::Complete,
        ]]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.runtime().register_skill(make_skill("review", "Review code changes"));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (_handle, mut events) = session.prompt_stream("go");

        let mut tool_result = None;
        while let Some(event) = events.next().await {
            if let PromptEvent::ToolResult {
                call_id,
                status,
                result,
                ..
            } = event.clone()
            {
                if call_id == "sk2" {
                    tool_result = Some((status, result));
                }
            }
            if matches!(event, PromptEvent::Complete { .. }) {
                break;
            }
        }

        let (status, result) = tool_result.expect("expected activate_skill failure");
        assert_eq!(status, ToolResultStatus::Failed);
        let result = result.expect("expected failure payload");
        let error = result["error"].as_str().unwrap();
        assert!(error.contains("missing"));
    });
}

#[test]
fn activate_skill_duplicate_activation_returns_lightweight_confirmation() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("sk3", "activate_skill", json!({"skill_name": "review"})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("sk4", "activate_skill", json!({"skill_name": "review"})),
                },
                ProviderEvent::Complete,
            ],
            vec![ProviderEvent::Complete],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.runtime().register_skill(make_skill("review", "Review code changes"));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (_handle, mut events) = session.prompt_stream("go");

        let mut first_result = None;
        let mut second_result = None;
        while let Some(event) = events.next().await {
            if let PromptEvent::ToolResult {
                call_id,
                status,
                result,
                ..
            } = event.clone()
            {
                if call_id == "sk3" {
                    first_result = Some((status, result.clone()));
                }
                if call_id == "sk4" {
                    second_result = Some((status, result));
                }
            }
            if matches!(event, PromptEvent::Complete { .. }) {
                break;
            }
        }

        let (first_status, first_payload) = first_result.expect("expected first activation result");
        assert_eq!(first_status, ToolResultStatus::Completed);
        assert_eq!(first_payload.unwrap()["status"], "activated");

        let (second_status, second_payload) = second_result.expect("expected second activation result");
        assert_eq!(second_status, ToolResultStatus::Completed);
        let second_payload = second_payload.unwrap();
        assert_eq!(second_payload["status"], "already_active");
        assert!(second_payload.get("content").is_none());
    });
}

#[test]
fn facade_activation_allows_trust_required_skills() {
    let agent = IronAgent::new(Config::default(), MockProvider::default());
    agent.runtime().register_skill(LoadedSkill {
        metadata: SkillMetadata {
            id: "trusted-review".to_string(),
            display_name: "trusted-review".to_string(),
            description: "Review privileged changes".to_string(),
            origin: SkillOrigin::ClientProvided,
            auto_activate: false,
            tags: vec![],
            requires_tools: vec![],
            requires_capabilities: vec![],
            requires_trust: true,
        },
        location: None,
        body: "# trusted-review\nHandle privileged review tasks.".to_string(),
        resources: vec![],
    });

    let conn = agent.connect();
    let session = conn.create_session().unwrap();

    session
        .activate_skill("trusted-review")
        .expect("direct facade activation should remain user-controlled");

    assert_eq!(session.list_active_skills(), vec!["trusted-review".to_string()]);
}

#[test]
fn normalized_plugin_result_reaches_tool_result_fields_and_output_stream() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("vr1", "plugin_demo_render", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![ProviderEvent::Complete],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.register_tool(FunctionTool::simple(
            "plugin_demo_render",
            "plugin_demo_render",
            |_| {
                Ok(json!({
                    "kind": "plugin_tool_result",
                    "transcript": {"text": "Updated todo list."},
                    "view": {
                        "id": "todo:session:1",
                        "mode": "replace",
                        "payload": {
                            "kind": "todo_list",
                            "title": "Current Tasks",
                            "items": [
                                {"id": "task_1", "label": "Review tool efficiency", "done": true},
                                {"id": "task_2", "label": "Add plugin rich output", "done": false}
                            ]
                        }
                    },
                    "metadata": {
                        "plugin_id": "demo-plugin",
                        "tool_name": "render"
                    }
                }))
            },
        ));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();
        let (_handle, mut events) = session.prompt_stream("go");

        let mut saw_output = false;
        let mut tool_result = None;

        while let Some(event) = events.next().await {
            match event {
                PromptEvent::Output { text } if text == "Updated todo list." => {
                    saw_output = true;
                }
                PromptEvent::ToolResult { ref call_id, .. } if call_id == "vr1" => {
                    tool_result = Some(event.clone());
                }
                PromptEvent::Complete { .. } => break,
                _ => {}
            }
        }

        assert!(
            saw_output,
            "normalized transcript text should reach the output stream"
        );

        let Some(PromptEvent::ToolResult {
            status,
            result,
            transcript_text,
            view,
            ..
        }) = tool_result
        else {
            panic!("expected tool result for normalized plugin output");
        };

        assert_eq!(status, ToolResultStatus::Completed);
        assert_eq!(transcript_text.as_deref(), Some("Updated todo list."));
        assert_eq!(
            view,
            Some(json!({
                "id": "todo:session:1",
                "mode": "replace",
                "payload": {
                    "kind": "todo_list",
                    "title": "Current Tasks",
                    "items": [
                        {"id": "task_1", "label": "Review tool efficiency", "done": true},
                        {"id": "task_2", "label": "Add plugin rich output", "done": false}
                    ]
                }
            }))
        );
        assert!(
            result.is_some(),
            "raw result should remain available for compatibility"
        );

        let records = session.tool_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].result, result);
    });
}

// ===================================================================
// 1. Runtime / session ownership
// ===================================================================

#[test]
fn runtime_creates_connection_and_session() {
    let rt = IronRuntime::new(Config::default(), MockProvider::default());

    let conn_id = ConnectionId(1);
    rt.register_connection(conn_id);

    assert_eq!(rt.connection_count(), 1);

    let (sid, _session) = rt.create_session(conn_id).unwrap();
    assert_eq!(rt.session_count(), 1);
    assert_eq!(rt.get_session_connection(sid), Some(conn_id));
}

#[test]
fn session_is_bound_to_its_connection() {
    let rt = IronRuntime::new(Config::default(), MockProvider::default());

    let conn_a = ConnectionId(10);
    let conn_b = ConnectionId(20);
    rt.register_connection(conn_a);
    rt.register_connection(conn_b);

    let (sid_a, _) = rt.create_session(conn_a).unwrap();
    let (sid_b, _) = rt.create_session(conn_b).unwrap();

    assert_eq!(rt.get_session_connection(sid_a), Some(conn_a));
    assert_eq!(rt.get_session_connection(sid_b), Some(conn_b));
    assert_eq!(rt.sessions_for_connection(conn_a), vec![sid_a]);
    assert_eq!(rt.sessions_for_connection(conn_b), vec![sid_b]);
}

#[test]
fn closing_connection_removes_its_sessions() {
    let rt = IronRuntime::new(Config::default(), MockProvider::default());

    let conn_a = ConnectionId(100);
    let conn_b = ConnectionId(200);
    rt.register_connection(conn_a);
    rt.register_connection(conn_b);

    let (sid_a, _) = rt.create_session(conn_a).unwrap();
    let (sid_b, _) = rt.create_session(conn_b).unwrap();

    assert_eq!(rt.session_count(), 2);

    rt.close_connection(conn_a);

    // conn_a's session should be gone, conn_b's should remain.
    assert!(rt.get_session(sid_a).is_none());
    assert!(rt.get_session(sid_b).is_some());
    assert_eq!(rt.session_count(), 1);
    assert_eq!(rt.connection_count(), 1);
}

#[test]
fn shutdown_clears_everything() {
    let rt = IronRuntime::new(Config::default(), MockProvider::default());

    let conn = ConnectionId(1);
    rt.register_connection(conn);
    let _ = rt.create_session(conn).unwrap();

    assert!(!rt.is_shutdown());
    rt.shutdown();

    assert!(rt.is_shutdown());
    assert_eq!(rt.connection_count(), 0);
    assert_eq!(rt.session_count(), 0);
}

#[test]
fn create_session_on_shutdown_errors() {
    let rt = IronRuntime::new(Config::default(), MockProvider::default());
    let conn = ConnectionId(1);
    rt.register_connection(conn);
    rt.shutdown();

    let result = rt.create_session(conn);
    assert!(result.is_err());
}

#[test]
fn close_session_removes_only_that_session() {
    let rt = IronRuntime::new(Config::default(), MockProvider::default());

    let conn = ConnectionId(1);
    rt.register_connection(conn);

    let (sid1, _) = rt.create_session(conn).unwrap();
    let (sid2, _) = rt.create_session(conn).unwrap();

    assert_eq!(rt.session_count(), 2);

    rt.close_session(sid1);

    assert!(rt.get_session(sid1).is_none());
    assert!(rt.get_session(sid2).is_some());
    assert_eq!(rt.session_count(), 1);
}

// ===================================================================
// 2. Prompt / update ordering (facade)
// ===================================================================

#[test]
fn simple_prompt_returns_end_turn() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "Hello!".into(),
            },
            ProviderEvent::Complete,
        ]]);
        let agent = IronAgent::new(Config::default().with_embedded_python_enabled(), provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let outcome = session.prompt("hi").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);
    });
}

// ===================================================================
// 3. Durable timeline commits
// ===================================================================

#[test]
fn durable_session_records_user_and_agent_messages() {
    let mut session = DurableSession::new(SessionId::new());
    assert!(session.is_empty());

    session.add_user_text("hello");
    session.add_agent_text("world");

    assert_eq!(session.messages.len(), 2);
    assert!(session.messages[0].is_user());
    assert!(session.messages[1].is_agent());
    assert_eq!(session.messages[0].text_content(), "hello");
    assert_eq!(session.messages[1].text_content(), "world");
}

#[test]
fn durable_session_timeline_indexes_are_monotonic() {
    let mut session = DurableSession::new(SessionId::new());

    session.add_user_text("q1");
    session.add_agent_text("a1");
    session.add_user_text("q2");

    let indices: Vec<u64> = session.timeline.iter().map(|e| e.index()).collect();
    assert_eq!(indices, vec![0, 1, 2]);
}

#[test]
fn tool_call_lifecycle_records_start_and_terminal_in_timeline() {
    let mut session = DurableSession::new(SessionId::new());

    session.add_user_text("do it");
    session.start_tool_call("c1", "my_tool", json!({"x": 1}));
    session.complete_tool_call("c1", json!(42));
    session.add_agent_text("done");

    // Timeline: UserMessage, ToolCallStarted, ToolCallTerminal(Completed), AgentMessage
    assert_eq!(session.timeline.len(), 4);

    use iron_core::TimelineEntry;
    assert!(matches!(
        &session.timeline[0],
        TimelineEntry::UserMessage { .. }
    ));
    assert!(matches!(
        &session.timeline[1],
        TimelineEntry::ToolCallStarted { call_id, tool_name, .. }
        if call_id == "c1" && tool_name == "my_tool"
    ));
    assert!(matches!(
        &session.timeline[2],
        TimelineEntry::ToolCallTerminal {
            call_id,
            outcome: ToolTerminalOutcome::Completed,
            ..
        } if call_id == "c1"
    ));
    assert!(matches!(
        &session.timeline[3],
        TimelineEntry::AgentMessage { .. }
    ));
}

#[test]
fn tool_record_tracks_status_through_lifecycle() {
    let mut session = DurableSession::new(SessionId::new());

    session.start_tool_call("c1", "tool_a", json!({"a": 1}));
    assert_eq!(session.tool_records.len(), 1);
    assert_eq!(session.tool_records[0].status, ToolRecordStatus::Running);
    assert_eq!(session.tool_records[0].call_id, "c1");
    assert_eq!(session.tool_records[0].tool_name, "tool_a");

    session.complete_tool_call("c1", json!({"ok": true}));
    assert_eq!(session.tool_records[0].status, ToolRecordStatus::Completed);
    assert_eq!(session.tool_records[0].result, Some(json!({"ok": true})));
}

#[test]
fn tool_record_failed_outcome() {
    let mut session = DurableSession::new(SessionId::new());

    session.start_tool_call("c2", "fail_tool", json!({}));
    session.fail_tool_call("c2", json!({"error": "boom"}));

    assert_eq!(session.tool_records[0].status, ToolRecordStatus::Failed);
    assert_eq!(
        session.tool_records[0].result,
        Some(json!({"error": "boom"}))
    );

    // Timeline terminal entry has Failed outcome
    use iron_core::TimelineEntry;
    let terminal = session.timeline.last().unwrap();
    assert!(matches!(
        terminal,
        TimelineEntry::ToolCallTerminal {
            outcome: ToolTerminalOutcome::Failed,
            ..
        }
    ));
}

#[test]
fn tool_record_denied_outcome() {
    let mut session = DurableSession::new(SessionId::new());

    session.start_tool_call("c3", "denied_tool", json!({}));
    session.deny_tool_call("c3");

    assert_eq!(session.tool_records[0].status, ToolRecordStatus::Denied);

    use iron_core::TimelineEntry;
    let terminal = session.timeline.last().unwrap();
    assert!(matches!(
        terminal,
        TimelineEntry::ToolCallTerminal {
            outcome: ToolTerminalOutcome::Denied,
            ..
        }
    ));
}

#[test]
fn tool_record_cancelled_outcome() {
    let mut session = DurableSession::new(SessionId::new());

    session.start_tool_call("c4", "cancel_tool", json!({}));
    session.cancel_tool_call("c4");

    assert_eq!(session.tool_records[0].status, ToolRecordStatus::Cancelled);

    use iron_core::TimelineEntry;
    let terminal = session.timeline.last().unwrap();
    assert!(matches!(
        terminal,
        TimelineEntry::ToolCallTerminal {
            outcome: ToolTerminalOutcome::Cancelled,
            ..
        }
    ));
}

#[test]
fn to_transcript_preserves_order() {
    let mut session = DurableSession::new(SessionId::new());

    session.add_user_text("hello");
    session.add_agent_text("hi there");
    session.add_user_text("do something");

    session.start_tool_call("c1", "tool_x", json!({}));
    session.complete_tool_call("c1", json!("result"));

    session.add_agent_text("all done");

    let transcript = session.to_transcript();
    // User hello, Assistant hi there, User do something, AssistantToolCall c1, Tool c1 result, Assistant all done
    assert_eq!(transcript.messages.len(), 6);

    use iron_providers::Message;
    assert!(matches!(&transcript.messages[0], Message::User { content } if content == "hello"));
    assert!(
        matches!(&transcript.messages[1], Message::Assistant { content } if content == "hi there")
    );
    assert!(
        matches!(&transcript.messages[3], Message::AssistantToolCall { call_id, .. } if call_id == "c1")
    );
    assert!(matches!(&transcript.messages[4], Message::Tool { call_id, .. } if call_id == "c1"));
    assert!(
        matches!(&transcript.messages[5], Message::Assistant { content } if content == "all done")
    );
}

#[test]
fn durable_session_records_tool_call_after_prompt() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("tc1", "calc_tool", json!({"x": 42})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "result: 42".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.register_tool(FunctionTool::simple("calc_tool", "calc_tool", |args| {
            Ok(args)
        }));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let outcome = session.prompt("calculate").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);

        // Verify durable state
        let messages = session.messages();
        // Should have user message and two agent messages
        assert!(messages
            .iter()
            .any(|m| m.is_user() && m.text_content() == "calculate"));
        assert!(messages
            .iter()
            .any(|m| m.is_agent() && m.text_content().contains("result: 42")));

        let tool_records = session.tool_records();
        assert_eq!(tool_records.len(), 1);
        assert_eq!(tool_records[0].call_id, "tc1");
        assert_eq!(tool_records[0].tool_name, "calc_tool");
        assert_eq!(tool_records[0].status, ToolRecordStatus::Completed);

        let timeline = session.timeline();
        // UserMessage, ToolCallStarted, ToolCallTerminal(Completed), AgentMessage
        assert!(timeline.len() >= 4);
    });
}

// ===================================================================
// 4. Permission flow
// ===================================================================

#[test]
fn permission_handler_is_called_for_approval_tool() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("pc1", "risky_op", json!({"danger": true})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "approved result".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        let executions = Arc::new(AtomicUsize::new(0));
        let exec_clone = executions.clone();
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("risky_op", "risky_op", json!({})).with_approval(true),
            move |_| {
                exec_clone.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"ok": true}))
            },
        ));

        let conn = agent.connect();
        let permission_calls = Arc::new(AtomicUsize::new(0));
        let perm_clone = permission_calls.clone();
        conn.on_permission(move |_call_id| {
            perm_clone.fetch_add(1, Ordering::SeqCst);
            PermissionVerdict::AllowOnce
        });

        let session = conn.create_session().unwrap();
        let outcome = session.prompt("do risky thing").await;

        assert_eq!(outcome, PromptOutcome::EndTurn);
        assert_eq!(permission_calls.load(Ordering::SeqCst), 1);
        assert_eq!(executions.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn permission_deny_prevents_tool_execution() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("pd1", "risky_op", json!({})),
            },
            ProviderEvent::Complete,
        ]]);
        let agent = IronAgent::new(
            Config::default().with_approval_strategy(ApprovalStrategy::PerTool),
            provider,
        );
        let executions = Arc::new(AtomicUsize::new(0));
        let exec_clone = executions.clone();
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("risky_op", "risky_op", json!({})).with_approval(true),
            move |_| {
                exec_clone.fetch_add(1, Ordering::SeqCst);
                Ok(json!("should not run"))
            },
        ));

        let conn = agent.connect();
        conn.on_permission(|_call_id| PermissionVerdict::Deny);

        let session = conn.create_session().unwrap();
        let outcome = session.prompt("do risky thing").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);

        // Tool should NOT have been executed
        assert_eq!(executions.load(Ordering::SeqCst), 0);

        // Denied tool calls are now durably recorded
        let tool_records = session.tool_records();
        assert_eq!(tool_records.len(), 1);
        assert_eq!(tool_records[0].call_id, "pd1");
        assert_eq!(tool_records[0].status, ToolRecordStatus::Denied);
    });
}

#[test]
fn approval_strategy_always_triggers_for_all_tools() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("pa1", "safe_op", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "ok".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(
            Config::default().with_approval_strategy(ApprovalStrategy::Always),
            provider,
        );
        // safe_op does NOT require approval on its own, but Always overrides
        agent.register_tool(FunctionTool::simple("safe_op", "safe_op", |_| {
            Ok(json!("safe"))
        }));

        let conn = agent.connect();
        let perm_calls = Arc::new(AtomicUsize::new(0));
        let perm_clone = perm_calls.clone();
        conn.on_permission(move |_| {
            perm_clone.fetch_add(1, Ordering::SeqCst);
            PermissionVerdict::AllowOnce
        });

        let session = conn.create_session().unwrap();
        let outcome = session.prompt("go").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);
        assert_eq!(perm_calls.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn approval_strategy_never_skips_even_for_approval_tools() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("pn1", "risky_op", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "done".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(
            Config::default().with_approval_strategy(ApprovalStrategy::Never),
            provider,
        );
        let executions = Arc::new(AtomicUsize::new(0));
        let exec_clone = executions.clone();
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("risky_op", "risky_op", json!({})).with_approval(true),
            move |_| {
                exec_clone.fetch_add(1, Ordering::SeqCst);
                Ok(json!("auto-approved"))
            },
        ));

        let conn = agent.connect();
        let perm_calls = Arc::new(AtomicUsize::new(0));
        let perm_clone = perm_calls.clone();
        conn.on_permission(move |_| {
            perm_clone.fetch_add(1, Ordering::SeqCst);
            PermissionVerdict::AllowOnce
        });

        let session = conn.create_session().unwrap();
        let outcome = session.prompt("go").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);

        // Never strategy → tool executed without permission handler call
        assert_eq!(perm_calls.load(Ordering::SeqCst), 0);
        assert_eq!(executions.load(Ordering::SeqCst), 1);
    });
}

// ===================================================================
// 5. Cancellation behaviour
// ===================================================================

#[test]
fn cancel_sets_cancel_requested_flag_on_ephemeral_turn() {
    let mut turn = EphemeralTurn::new(SessionId::new());
    turn.start();
    assert_eq!(turn.phase, TurnPhase::Running);
    assert!(!turn.is_cancel_requested());

    turn.cancel();
    assert!(turn.is_cancel_requested());
    assert_eq!(turn.phase, TurnPhase::Cancelled);
}

#[test]
fn cancel_clears_pending_permissions() {
    let mut turn = EphemeralTurn::new(SessionId::new());
    turn.start();

    turn.request_permission("c1".into(), "tool_a".into(), json!({}));
    assert_eq!(turn.phase, TurnPhase::WaitingPermission);
    assert_eq!(turn.pending_permissions.len(), 1);

    turn.cancel();
    assert_eq!(turn.phase, TurnPhase::Cancelled);
    assert!(turn.pending_permissions.is_empty());
}

#[test]
fn resolve_permission_removes_specific_pending_permission() {
    let mut turn = EphemeralTurn::new(SessionId::new());
    turn.start();

    turn.request_permission("c1".into(), "tool_a".into(), json!({}));
    turn.request_permission("c2".into(), "tool_b".into(), json!({}));
    assert_eq!(turn.pending_permissions.len(), 2);

    let resolved = turn.resolve_permission("c1");
    assert!(resolved);
    assert_eq!(turn.pending_permissions.len(), 1);
}

#[test]
fn cancel_token_is_shared() {
    let turn = EphemeralTurn::new(SessionId::new());
    let token = turn.cancel_token();

    assert!(!token.load(Ordering::SeqCst));
    token.store(true, Ordering::SeqCst);
    // The turn's is_cancel_requested reads from the same AtomicBool
    assert!(turn.is_cancel_requested());
}

#[test]
fn agent_session_cancel_signals_cancellation() {
    run_local(async {
        // Provider that returns a tool call — the prompt loop will be in the
        // middle of processing when we cancel.
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("cc1", "slow_tool", json!({})),
                },
                ProviderEvent::Complete,
            ],
            // The loop won't get here because of cancellation, but we need a
            // response in case it does.
            vec![
                ProviderEvent::Output {
                    content: "should not appear".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.register_tool(FunctionTool::simple("slow_tool", "slow_tool", |_| {
            // Simulate a slow tool — sleep a bit so cancel can race
            std::thread::sleep(std::time::Duration::from_millis(50));
            Ok(json!("done"))
        }));

        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let session_clone = session.clone();
        // Spawn the cancel call concurrently
        let cancel_handle = tokio::task::spawn_local(async move {
            // Give the prompt loop time to start
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            session_clone.cancel().await;
        });

        let outcome = session.prompt("go").await;
        let _ = cancel_handle.await;

        // The outcome should be either Cancelled or EndTurn (race condition),
        // but Cancelled is expected when cancel wins the race.
        assert!(
            outcome == PromptOutcome::Cancelled || outcome == PromptOutcome::EndTurn,
            "expected Cancelled or EndTurn, got {:?}",
            outcome
        );
    });
}

// ===================================================================
// EphemeralTurn state machine
// ===================================================================

#[test]
fn ephemeral_turn_starts_in_idle() {
    let turn = EphemeralTurn::new(SessionId::new());
    assert_eq!(turn.phase, TurnPhase::Idle);
    assert!(!turn.is_terminal());
}

#[test]
fn ephemeral_turn_completed_is_terminal() {
    let mut turn = EphemeralTurn::new(SessionId::new());
    turn.start();
    turn.complete();
    assert_eq!(turn.phase, TurnPhase::Completed);
    assert!(turn.is_terminal());
}

#[test]
fn ephemeral_turn_cancelled_is_terminal() {
    let mut turn = EphemeralTurn::new(SessionId::new());
    turn.start();
    turn.cancel();
    assert_eq!(turn.phase, TurnPhase::Cancelled);
    assert!(turn.is_terminal());
}

#[test]
fn add_chunk_records_partial_output() {
    let mut turn = EphemeralTurn::new(SessionId::new());
    turn.start();

    turn.add_chunk("hello ".into());
    turn.add_chunk("world".into());

    assert_eq!(turn.partial_chunks, vec!["hello ", "world"]);
}

#[test]
fn complete_clears_chunks_and_permissions() {
    let mut turn = EphemeralTurn::new(SessionId::new());
    turn.start();
    turn.add_chunk("data".into());
    turn.request_permission("c1".into(), "t".into(), json!({}));

    turn.complete();

    assert!(turn.partial_chunks.is_empty());
    assert!(turn.pending_permissions.is_empty());
}

// ===================================================================
// Additional facade tests: max iterations
// ===================================================================

#[test]
fn max_iterations_returns_max_turn_requests() {
    run_local(async {
        // Provider always returns a tool call, forcing the loop to iterate.
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("mi1", "loop_tool", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("mi2", "loop_tool", json!({})),
                },
                ProviderEvent::Complete,
            ],
            // Third iteration will hit max_iterations = 2
            vec![
                ProviderEvent::Output {
                    content: "should not get here".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let config = Config::default().with_max_iterations(2);
        let agent = IronAgent::new(config, provider);
        agent.register_tool(FunctionTool::simple("loop_tool", "loop_tool", |_| {
            Ok(json!(0))
        }));

        let conn = agent.connect();
        let session = conn.create_session().unwrap();
        let outcome = session.prompt("loop").await;

        assert_eq!(outcome, PromptOutcome::MaxTurnRequests);
    });
}

// ===================================================================
// 5. Async permission flow
// ===================================================================

#[test]
fn async_permission_handler_receives_rich_request() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("ar1", "risky_op", json!({"danger": true})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "approved result".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("risky_op", "risky_op", json!({})).with_approval(true),
            |_| Ok(json!({"ok": true})),
        ));

        let conn = agent.connect();
        let captured_call_id = Arc::new(Mutex::new(String::new()));
        let captured_tool_name = Arc::new(Mutex::new(String::new()));
        let captured_args = Arc::new(Mutex::new(None::<serde_json::Value>));

        let cid = captured_call_id.clone();
        let ctn = captured_tool_name.clone();
        let ca = captured_args.clone();
        conn.on_permission_async(move |req| {
            let mut id = cid.lock().unwrap();
            let mut tn = ctn.lock().unwrap();
            let mut a = ca.lock().unwrap();
            *id = req.call_id.clone();
            *tn = req.tool_name.clone();
            *a = Some(req.arguments.clone());
            Box::pin(async { PermissionVerdict::AllowOnce })
        });

        let session = conn.create_session().unwrap();
        let outcome = session.prompt("do risky thing").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);

        assert_eq!(*captured_call_id.lock().unwrap(), "ar1");
        assert_eq!(*captured_tool_name.lock().unwrap(), "risky_op");
        assert_eq!(
            captured_args.lock().unwrap().clone(),
            Some(json!({"danger": true}))
        );
    });
}

#[test]
fn async_permission_cancel_verdict_cancels_turn() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("ac1", "risky_op", json!({})),
            },
            ProviderEvent::Complete,
        ]]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("risky_op", "risky_op", json!({})).with_approval(true),
            |_| Ok(json!("should not run")),
        ));

        let conn = agent.connect();
        conn.on_permission_async(|_req| Box::pin(async { PermissionVerdict::Cancel }));

        let session = conn.create_session().unwrap();
        let outcome = session.prompt("do risky thing").await;
        assert_eq!(outcome, PromptOutcome::Cancelled);
    });
}

#[cfg(feature = "embedded-python")]
#[test]
fn python_tools_namespace_child_calls_request_permission_and_record_durable_history() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new(
                        "py1",
                        "python_exec",
                        json!({
                            "script": "await tools.secure_tool({'value': input['value']})",
                            "input": {"value": 7}
                        }),
                    ),
                },
                ProviderEvent::Complete,
            ],
            vec![ProviderEvent::Complete],
        ]);
        let agent = IronAgent::new(Config::default().with_embedded_python_enabled(), provider);
        agent.register_python_exec_tool();
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new(
                "secure_tool",
                "secure_tool",
                json!({
                    "type": "object",
                    "properties": {
                        "value": {"type": "integer"}
                    },
                    "required": ["value"]
                }),
            )
            .with_approval(true),
            |args| Ok(json!({"seen": args["value"]})),
        ));

        let conn = agent.connect();
        let seen_tools = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen_tools_handle = seen_tools.clone();
        conn.on_permission_async(move |req| {
            seen_tools_handle
                .lock()
                .unwrap()
                .push(req.tool_name.clone());
            Box::pin(async { PermissionVerdict::AllowOnce })
        });

        let session = conn.create_session().unwrap();
        let outcome = session.prompt("run python").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);

        assert_eq!(
            *seen_tools.lock().unwrap(),
            vec!["python_exec".to_string(), "secure_tool".to_string()]
        );

        let records = session.tool_records();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].tool_name, "python_exec");
        assert_eq!(records[1].tool_name, "secure_tool");
        assert_eq!(records[1].status, ToolRecordStatus::Completed);
        assert_eq!(records[1].result.clone().unwrap(), json!({"seen": 7}));
        assert!(records[1].parent_script_id.is_some());
    });
}

#[cfg(feature = "embedded-python")]
#[test]
fn python_tools_namespace_child_calls_apply_schema_validation() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new(
                        "py1",
                        "python_exec",
                        json!({
                            "script": "await tools.typed_tool({'wrong': 1})",
                            "input": {}
                        }),
                    ),
                },
                ProviderEvent::Complete,
            ],
            vec![ProviderEvent::Complete],
        ]);
        let config = Config::default()
            .with_embedded_python_enabled()
            .with_approval_strategy(ApprovalStrategy::Never);
        let agent = IronAgent::new(config, provider);
        agent.register_python_exec_tool();
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new(
                "typed_tool",
                "typed_tool",
                json!({
                    "type": "object",
                    "properties": {
                        "value": {"type": "integer"}
                    },
                    "required": ["value"]
                }),
            ),
            |_| Ok(json!({"ok": true})),
        ));

        let conn = agent.connect();
        let session = conn.create_session().unwrap();
        let outcome = session.prompt("run python").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);

        let records = session.tool_records();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].tool_name, "typed_tool");
        assert_eq!(records[1].status, ToolRecordStatus::Failed);
        let error = records[1].result.clone().unwrap();
        assert!(error["error"]
            .as_str()
            .unwrap()
            .contains("schema validation failed"));
        assert!(error["validation_errors"].is_array());
    });
}

#[test]
fn sync_permission_handler_still_works_with_cancel_verdict_available() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("sc1", "risky_op", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "sync ok".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("risky_op", "risky_op", json!({})).with_approval(true),
            |_| Ok(json!({"ok": true})),
        ));

        let conn = agent.connect();
        let perm_calls = Arc::new(AtomicUsize::new(0));
        let pc = perm_calls.clone();
        conn.on_permission(move |_call_id| {
            pc.fetch_add(1, Ordering::SeqCst);
            PermissionVerdict::AllowOnce
        });

        let session = conn.create_session().unwrap();
        let outcome = session.prompt("do risky thing").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);
        assert_eq!(perm_calls.load(Ordering::SeqCst), 1);
    });
}

// ===================================================================
// 6. Session ownership enforcement
// ===================================================================

#[test]
fn close_session_on_non_owned_session_returns_error() {
    run_local(async {
        let provider = MockProvider::default();
        let agent = IronAgent::new(Config::default(), provider);
        let conn_a = agent.connect();
        let conn_b = agent.connect();

        let session = conn_a.create_session().unwrap();

        let result = conn_b.close_session(&session);
        assert!(
            result.is_err(),
            "expected error when closing non-owned session"
        );

        let result_own = conn_a.close_session(&session);
        assert!(result_own.is_ok(), "expected ok when closing owned session");
    });
}

// ===================================================================
// 7. Context-window policy enforcement
// ===================================================================

#[test]
fn keep_recent_prunes_acp_native_transcript() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::Output {
                    content: "one".into(),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "two".into(),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "three".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let config = Config::default()
            .with_context_window_policy(iron_core::ContextWindowPolicy::KeepRecent(2));
        let agent = IronAgent::new(config, provider.clone());
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let _ = session.prompt("first").await;
        let _ = session.prompt("second").await;
        let _ = session.prompt("third").await;

        let requests = provider.requests();
        assert!(!requests.is_empty());
        let last_request = requests.last().unwrap();
        let msg_count = last_request.context.transcript.messages.len();
        assert!(
            msg_count <= 2,
            "expected at most 2 messages after KeepRecent(2), got {}",
            msg_count
        );
    });
}

#[test]
fn keep_recent_parity_between_legacy_and_acp() {
    use iron_core::ContextWindowPolicy;

    let config = Config::default()
        .with_context_window_policy(ContextWindowPolicy::KeepRecent(3))
        .with_max_iterations(1);

    let messages = vec![
        iron_providers::Message::User {
            content: "a".into(),
        },
        iron_providers::Message::Assistant {
            content: "b".into(),
        },
        iron_providers::Message::User {
            content: "c".into(),
        },
        iron_providers::Message::Assistant {
            content: "d".into(),
        },
        iron_providers::Message::User {
            content: "e".into(),
        },
    ];

    let mut pruned = messages.clone();
    iron_core::ContextWindowPolicy::KeepRecent(3).apply(&mut pruned, |_| {
        iron_providers::Message::Assistant {
            content: "summary".into(),
        }
    });
    assert_eq!(pruned.len(), 3);

    let tool_registry = iron_core::ToolRegistry::new();
    let result = iron_core::request_builder::build_inference_request(
        &config,
        &messages,
        None,
        &tool_registry,
    );
    assert!(result.is_ok());
    let request = result.unwrap();
    assert_eq!(
        request.context.transcript.messages.len(),
        3,
        "builder should prune to 3 messages"
    );
}

// ===================================================================
// 8. Durable tool-call lifecycle (proposal-time records)
// ===================================================================

#[test]
fn propose_tool_call_creates_pending_approval_record() {
    let mut session = DurableSession::new(SessionId::new());
    session.propose_tool_call("pc1", "my_tool", json!({"x": 1}));

    assert_eq!(session.tool_records.len(), 1);
    assert_eq!(session.tool_records[0].call_id, "pc1");
    assert_eq!(
        session.tool_records[0].status,
        ToolRecordStatus::PendingApproval
    );
    assert!(!session.tool_records[0].status.is_terminal());

    use iron_core::TimelineEntry;
    assert!(matches!(
        &session.timeline[0],
        TimelineEntry::ToolCallStarted { call_id, .. } if call_id == "pc1"
    ));
}

#[test]
fn denied_proposed_tool_call_has_terminal_durable_record() {
    let mut session = DurableSession::new(SessionId::new());
    session.propose_tool_call("dc1", "denied_tool", json!({}));
    session.deny_tool_call("dc1");

    assert_eq!(session.tool_records.len(), 1);
    assert_eq!(session.tool_records[0].status, ToolRecordStatus::Denied);
    assert!(session.tool_records[0].status.is_terminal());

    use iron_core::TimelineEntry;
    let terminal = session.timeline.last().unwrap();
    assert!(matches!(
        terminal,
        TimelineEntry::ToolCallTerminal {
            outcome: ToolTerminalOutcome::Denied,
            ..
        }
    ));
}

#[test]
fn cancelled_proposed_tool_call_has_terminal_durable_record() {
    let mut session = DurableSession::new(SessionId::new());
    session.propose_tool_call("cc1", "cancel_tool", json!({}));
    session.cancel_tool_call("cc1");

    assert_eq!(session.tool_records.len(), 1);
    assert_eq!(session.tool_records[0].status, ToolRecordStatus::Cancelled);
    assert!(session.tool_records[0].status.is_terminal());

    use iron_core::TimelineEntry;
    let terminal = session.timeline.last().unwrap();
    assert!(matches!(
        terminal,
        TimelineEntry::ToolCallTerminal {
            outcome: ToolTerminalOutcome::Cancelled,
            ..
        }
    ));
}

#[test]
fn denied_tool_call_appears_in_replay_transcript() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("start");
    session.propose_tool_call("dt1", "dangerous_tool", json!({"x": 1}));
    session.deny_tool_call("dt1");

    let transcript = session.to_transcript();
    use iron_providers::Message;
    assert!(matches!(
        &transcript.messages[1],
        Message::AssistantToolCall { call_id, tool_name, .. }
        if call_id == "dt1" && tool_name == "dangerous_tool"
    ));
    assert!(matches!(
        &transcript.messages[2],
        Message::Tool { call_id, .. } if call_id == "dt1"
    ));
}

#[test]
fn cancelled_tool_call_appears_in_replay_transcript() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("start");
    session.propose_tool_call("ct1", "slow_tool", json!({}));
    session.cancel_tool_call("ct1");

    let transcript = session.to_transcript();
    use iron_providers::Message;
    assert!(matches!(
        &transcript.messages[1],
        Message::AssistantToolCall { call_id, .. } if call_id == "ct1"
    ));
    assert!(matches!(
        &transcript.messages[2],
        Message::Tool { call_id, .. } if call_id == "ct1"
    ));
}

#[test]
fn proposed_then_started_then_completed_tool_call() {
    let mut session = DurableSession::new(SessionId::new());
    session.propose_tool_call("lc1", "lifecycle_tool", json!({"x": 1}));
    assert_eq!(
        session.tool_records[0].status,
        ToolRecordStatus::PendingApproval
    );

    session.start_tool_call("lc1", "lifecycle_tool", json!({"x": 1}));
    assert_eq!(session.tool_records[0].status, ToolRecordStatus::Running);

    session.complete_tool_call("lc1", json!({"result": "ok"}));
    assert_eq!(session.tool_records[0].status, ToolRecordStatus::Completed);
    assert!(session.tool_records[0].status.is_terminal());
}

// ===================================================================
// 9. Integration: deny/cancel through prompt loop + replay
// ===================================================================

#[test]
fn denied_tool_call_replayed_in_subsequent_prompt_transcript() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("deny1", "risky_op", json!({"x": 1})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "after deny".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(
            Config::default().with_approval_strategy(ApprovalStrategy::PerTool),
            provider.clone(),
        );
        let executions = Arc::new(AtomicUsize::new(0));
        let exec_clone = executions.clone();
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("risky_op", "risky_op", json!({})).with_approval(true),
            move |_| {
                exec_clone.fetch_add(1, Ordering::SeqCst);
                Ok(json!("should not run"))
            },
        ));

        let conn = agent.connect();
        conn.on_permission(|_| PermissionVerdict::Deny);

        let session = conn.create_session().unwrap();
        let outcome = session.prompt("do risky").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);
        assert_eq!(executions.load(Ordering::SeqCst), 0);

        let tool_records = session.tool_records();
        assert_eq!(tool_records.len(), 1);
        assert_eq!(tool_records[0].status, ToolRecordStatus::Denied);

        let second_outcome = session.prompt("continue").await;
        assert_eq!(second_outcome, PromptOutcome::EndTurn);

        let requests = provider.requests();
        assert!(requests.len() >= 2);
        let second_request = &requests[requests.len() - 1];
        let transcript = &second_request.context.transcript;
        let has_denied_call = transcript.messages.iter().any(|m| {
            matches!(m, iron_providers::Message::AssistantToolCall { call_id, tool_name, .. }
                if call_id == "deny1" && tool_name == "risky_op")
        });
        let has_denied_result = transcript.messages.iter().any(
            |m| matches!(m, iron_providers::Message::Tool { call_id, .. } if call_id == "deny1"),
        );
        assert!(
            has_denied_call,
            "denied tool call should appear in replay transcript"
        );
        assert!(
            has_denied_result,
            "denied tool result should appear in replay transcript"
        );
    });
}

#[test]
fn cancelled_tool_call_replayed_in_subsequent_prompt_transcript() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("cancel1", "risky_op", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "after cancel".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider.clone());
        let executions = Arc::new(AtomicUsize::new(0));
        let exec_clone = executions.clone();
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("risky_op", "risky_op", json!({})).with_approval(true),
            move |_| {
                exec_clone.fetch_add(1, Ordering::SeqCst);
                Ok(json!("should not run"))
            },
        ));

        let conn = agent.connect();
        conn.on_permission_async(|_req| Box::pin(async { PermissionVerdict::Cancel }));

        let session = conn.create_session().unwrap();
        let outcome = session.prompt("do risky").await;
        assert_eq!(outcome, PromptOutcome::Cancelled);
        assert_eq!(executions.load(Ordering::SeqCst), 0);

        let tool_records = session.tool_records();
        assert_eq!(tool_records.len(), 1);
        assert_eq!(tool_records[0].status, ToolRecordStatus::Cancelled);

        let second_outcome = session.prompt("continue").await;
        assert_eq!(second_outcome, PromptOutcome::EndTurn);

        let requests = provider.requests();
        assert!(requests.len() >= 2);
        let second_request = &requests[requests.len() - 1];
        let transcript = &second_request.context.transcript;
        let has_cancelled_call = transcript.messages.iter().any(|m| {
            matches!(m, iron_providers::Message::AssistantToolCall { call_id, .. } if call_id == "cancel1")
        });
        let has_cancelled_result = transcript.messages.iter().any(
            |m| matches!(m, iron_providers::Message::Tool { call_id, .. } if call_id == "cancel1"),
        );
        assert!(
            has_cancelled_call,
            "cancelled tool call should appear in replay transcript"
        );
        assert!(
            has_cancelled_result,
            "cancelled tool result should appear in replay transcript"
        );
    });
}

#[test]
fn multiple_non_executed_tool_calls_all_appear_in_replay() {
    let mut session = DurableSession::new(SessionId::new());
    session.add_user_text("start");

    session.propose_tool_call("denied1", "tool_a", json!({"a": 1}));
    session.deny_tool_call("denied1");

    session.propose_tool_call("cancelled1", "tool_b", json!({"b": 2}));
    session.cancel_tool_call("cancelled1");

    session.add_agent_text("after denials");

    let transcript = session.to_transcript();
    use iron_providers::Message;

    assert!(
        transcript.messages.iter().any(
            |m| matches!(m, Message::AssistantToolCall { call_id, .. } if call_id == "denied1")
        ),
        "denied1 should be in transcript"
    );
    assert!(
        transcript
            .messages
            .iter()
            .any(|m| matches!(m, Message::Tool { call_id, .. } if call_id == "denied1")),
        "denied1 result should be in transcript"
    );
    assert!(
        transcript.messages.iter().any(
            |m| matches!(m, Message::AssistantToolCall { call_id, .. } if call_id == "cancelled1")
        ),
        "cancelled1 should be in transcript"
    );
    assert!(
        transcript
            .messages
            .iter()
            .any(|m| matches!(m, Message::Tool { call_id, .. } if call_id == "cancelled1")),
        "cancelled1 result should be in transcript"
    );

    let denied_pos = transcript
        .messages
        .iter()
        .position(
            |m| matches!(m, Message::AssistantToolCall { call_id, .. } if call_id == "denied1"),
        )
        .unwrap();
    let denied_result_pos = transcript
        .messages
        .iter()
        .position(|m| matches!(m, Message::Tool { call_id, .. } if call_id == "denied1"))
        .unwrap();
    assert!(
        denied_pos < denied_result_pos,
        "tool call should precede its result"
    );

    let cancelled_pos = transcript
        .messages
        .iter()
        .position(
            |m| matches!(m, Message::AssistantToolCall { call_id, .. } if call_id == "cancelled1"),
        )
        .unwrap();
    let cancelled_result_pos = transcript
        .messages
        .iter()
        .position(|m| matches!(m, Message::Tool { call_id, .. } if call_id == "cancelled1"))
        .unwrap();
    assert!(
        cancelled_pos < cancelled_result_pos,
        "tool call should precede its result"
    );
}

// ===================================================================
// 10. Tool input-schema validation
// ===================================================================

#[test]
fn valid_arguments_execute_normally() {
    run_local(async {
        let schema = json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer" }
            },
            "required": ["x"]
        });
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("sv1", "validated_tool", json!({"x": 42})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "done".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        let executions = Arc::new(AtomicUsize::new(0));
        let exec_clone = executions.clone();
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("validated_tool", "validated_tool", schema),
            move |args| {
                exec_clone.fetch_add(1, Ordering::SeqCst);
                Ok(args)
            },
        ));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let outcome = session.prompt("go").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);

        assert_eq!(
            executions.load(Ordering::SeqCst),
            1,
            "handler should have been called"
        );

        let records = session.tool_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ToolRecordStatus::Completed);
    });
}

#[test]
fn invalid_arguments_skip_handler_and_fail_durable() {
    run_local(async {
        let schema = json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer" }
            },
            "required": ["x"]
        });
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("sv2", "validated_tool", json!({"x": "not_an_int"})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "after fail".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        let executions = Arc::new(AtomicUsize::new(0));
        let exec_clone = executions.clone();
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("validated_tool", "validated_tool", schema),
            move |_| {
                exec_clone.fetch_add(1, Ordering::SeqCst);
                Ok(json!("should not run"))
            },
        ));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let outcome = session.prompt("go").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);

        assert_eq!(
            executions.load(Ordering::SeqCst),
            0,
            "handler should NOT have been called"
        );

        let records = session.tool_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ToolRecordStatus::Failed);
        let result = records[0].result.as_ref().unwrap();
        let error_msg = result.get("error").unwrap().as_str().unwrap();
        assert!(
            error_msg.contains("schema validation failed"),
            "expected schema validation error, got: {}",
            error_msg
        );
    });
}

#[test]
fn unusable_schema_fails_deterministically() {
    run_local(async {
        let bad_schema = json!("not an object");
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("sv3", "bad_schema_tool", json!({"x": 1})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "after bad schema".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        let executions = Arc::new(AtomicUsize::new(0));
        let exec_clone = executions.clone();
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("bad_schema_tool", "bad_schema_tool", bad_schema),
            move |_| {
                exec_clone.fetch_add(1, Ordering::SeqCst);
                Ok(json!("should not run"))
            },
        ));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let outcome = session.prompt("go").await;
        assert_eq!(outcome, PromptOutcome::EndTurn);

        assert_eq!(
            executions.load(Ordering::SeqCst),
            0,
            "handler should NOT run for bad schema"
        );

        let records = session.tool_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ToolRecordStatus::Failed);
        let result = records[0].result.as_ref().unwrap();
        let error_msg = result.get("error").unwrap().as_str().unwrap();
        assert!(
            error_msg.contains("invalid tool schema"),
            "expected invalid tool schema error, got: {}",
            error_msg
        );
    });
}

#[test]
fn schema_validation_failure_is_replayable() {
    run_local(async {
        let schema = json!({
            "type": "object",
            "properties": {
                "count": { "type": "number" }
            },
            "required": ["count"]
        });
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("sv4", "num_tool", json!({"count": "wrong"})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "replayed".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider.clone());
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("num_tool", "num_tool", schema),
            |_| Ok(json!("should not run")),
        ));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let _ = session.prompt("go").await;
        let _ = session.prompt("continue").await;

        let requests = provider.requests();
        assert!(requests.len() >= 2);
        let last_request = requests.last().unwrap();
        let transcript = &last_request.context.transcript;
        let has_failed_call = transcript.messages.iter().any(|m| {
            matches!(m, iron_providers::Message::AssistantToolCall { call_id, .. } if call_id == "sv4")
        });
        let has_failed_result = transcript.messages.iter().any(
            |m| matches!(m, iron_providers::Message::Tool { call_id, .. } if call_id == "sv4"),
        );
        assert!(
            has_failed_call,
            "failed schema-validated call should appear in replay"
        );
        assert!(
            has_failed_result,
            "failed schema-validated result should appear in replay"
        );
    });
}

#[test]
fn unit_schema_validate_valid() {
    use iron_core::schema::{validate_arguments, SchemaValidationOutcome};
    let schema =
        json!({"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]});
    match validate_arguments(&schema, &json!({"name": "Alice"})) {
        SchemaValidationOutcome::Valid => {}
        other => panic!("expected Valid, got {:?}", other),
    }
}

#[test]
fn unit_schema_validate_invalid() {
    use iron_core::schema::{validate_arguments, SchemaValidationOutcome};
    let schema =
        json!({"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]});
    match validate_arguments(&schema, &json!({"age": 30})) {
        SchemaValidationOutcome::Invalid { errors } => {
            assert!(!errors.is_empty(), "should have validation errors");
        }
        other => panic!("expected Invalid, got {:?}", other),
    }
}

#[test]
fn unit_schema_validate_bad_schema() {
    use iron_core::schema::{validate_arguments, SchemaValidationOutcome};
    let bad_schema = json!("not_a_schema");
    match validate_arguments(&bad_schema, &json!({})) {
        SchemaValidationOutcome::BadSchema { .. } => {}
        other => panic!("expected BadSchema, got {:?}", other),
    }
}

// ===================================================================
// 11. Stream-first facade tests (Task 4.3)
// ===================================================================

use iron_core::PromptStatus;

#[test]
fn stream_prompt_emits_ordered_events() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("sc1", "my_tool", json!({"x": 1})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "done".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.register_tool(FunctionTool::simple("my_tool", "my_tool", |_| {
            Ok(json!(42))
        }));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (handle, mut events) = session.prompt_stream("go");

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event);
            if matches!(collected.last(), Some(PromptEvent::Complete { .. })) {
                break;
            }
        }

        assert!(handle.status() == PromptStatus::Completed);

        let has_tool_call = collected
            .iter()
            .any(|e| matches!(e, PromptEvent::ToolCall { call_id, .. } if call_id == "sc1"));
        let has_tool_result = collected.iter().any(|e| matches!(e, PromptEvent::ToolResult { call_id, status: ToolResultStatus::Completed, .. } if call_id == "sc1"));
        let has_output = collected
            .iter()
            .any(|e| matches!(e, PromptEvent::Output { text } if text == "done"));
        let has_complete = collected.iter().any(|e| {
            matches!(
                e,
                PromptEvent::Complete {
                    outcome: PromptOutcome::EndTurn
                }
            )
        });

        assert!(has_tool_call, "expected ToolCall event");
        assert!(has_tool_result, "expected ToolResult(Completed) event");
        assert!(has_output, "expected Output event");
        assert!(has_complete, "expected Complete event");

        let complete_idx = collected
            .iter()
            .position(|e| matches!(e, PromptEvent::Complete { .. }))
            .unwrap();
        assert_eq!(
            complete_idx,
            collected.len() - 1,
            "Complete should be last event"
        );
    });
}

#[test]
fn stream_tool_call_precedes_tool_result() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("ord1", "t", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "fin".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.register_tool(FunctionTool::simple("t", "t", |_| Ok(json!(0))));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (_handle, mut events) = session.prompt_stream("go");

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event);
            if matches!(collected.last(), Some(PromptEvent::Complete { .. })) {
                break;
            }
        }

        let tool_call_pos = collected
            .iter()
            .position(|e| matches!(e, PromptEvent::ToolCall { call_id, .. } if call_id == "ord1"))
            .unwrap();
        let tool_result_pos = collected
            .iter()
            .position(|e| matches!(e, PromptEvent::ToolResult { call_id, .. } if call_id == "ord1"))
            .unwrap();
        assert!(
            tool_call_pos < tool_result_pos,
            "ToolCall must precede ToolResult"
        );
    });
}

#[test]
fn stream_approval_deny_resolves_as_tool_result() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("ap1", "risky", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "after".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(
            Config::default().with_approval_strategy(ApprovalStrategy::PerTool),
            provider,
        );
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("risky", "risky", json!({})).with_approval(true),
            |_| Ok(json!("should not run")),
        ));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (handle, mut events) = session.prompt_stream("go");

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event.clone());
            if let PromptEvent::ApprovalRequest { ref call_id, .. } = event {
                if call_id == "ap1" {
                    handle.deny(call_id).unwrap();
                }
            }
            if matches!(&event, PromptEvent::Complete { .. }) {
                break;
            }
        }

        let has_denied_result = collected.iter().any(|e| {
            matches!(e, PromptEvent::ToolResult { call_id, status: ToolResultStatus::Denied, .. } if call_id == "ap1")
        });
        assert!(
            has_denied_result,
            "denied approval should produce ToolResult(Denied)"
        );

        let has_complete = collected.iter().any(|e| {
            matches!(
                e,
                PromptEvent::Complete {
                    outcome: PromptOutcome::EndTurn
                }
            )
        });
        assert!(has_complete, "should complete with EndTurn");
    });
}

#[test]
fn stream_approval_approve_executes_tool() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("aa1", "risky", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "done".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(
            Config::default().with_approval_strategy(ApprovalStrategy::PerTool),
            provider,
        );
        let executions = Arc::new(AtomicUsize::new(0));
        let exec_clone = executions.clone();
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("risky", "risky", json!({})).with_approval(true),
            move |_| {
                exec_clone.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"ok": true}))
            },
        ));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (handle, mut events) = session.prompt_stream("go");

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event.clone());
            if let PromptEvent::ApprovalRequest { ref call_id, .. } = event {
                if call_id == "aa1" {
                    handle.approve(call_id).unwrap();
                }
            }
            if matches!(&event, PromptEvent::Complete { .. }) {
                break;
            }
        }

        assert_eq!(
            executions.load(Ordering::SeqCst),
            1,
            "tool should have been executed"
        );
        let has_completed_result = collected.iter().any(|e| {
            matches!(e, PromptEvent::ToolResult { call_id, status: ToolResultStatus::Completed, .. } if call_id == "aa1")
        });
        assert!(
            has_completed_result,
            "approved tool should produce ToolResult(Completed)"
        );
    });
}

#[test]
fn session_auth_flow_emits_auth_state_change_events() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("auth_hold", "risky", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![ProviderEvent::Complete],
        ]);
        let agent = IronAgent::new(
            Config::default().with_approval_strategy(ApprovalStrategy::PerTool),
            provider,
        );
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("risky", "risky", json!({})).with_approval(true),
            |_| Ok(json!("ok")),
        ));
        register_test_auth_plugin(&agent, "auth-plugin");

        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (handle, mut events) = session.prompt_stream("go");
        let mut collected = Vec::new();
        let mut auth_started = false;

        while let Some(event) = events.next().await {
            collected.push(event.clone());
            if let PromptEvent::ApprovalRequest { ref call_id, .. } = event {
                if !auth_started {
                    let request = session.start_auth_flow("auth-plugin").unwrap();
                    assert_eq!(request.plugin_id, "auth-plugin");
                    assert_eq!(request.provider, "github");

                    let transition = session
                        .complete_auth_flow(
                            "auth-plugin",
                            AuthInteractionResponse {
                                request_id: request.request_id,
                                result: AuthInteractionResult::Success {
                                    code: "auth-code-123".to_string(),
                                    state: None,
                                },
                            },
                        )
                        .unwrap();
                    assert_eq!(transition.previous_state, AuthState::Authenticating);
                    assert_eq!(transition.new_state, AuthState::Authenticated);

                    handle.approve(call_id).unwrap();
                    auth_started = true;
                }
            }
            if matches!(event, PromptEvent::Complete { .. }) {
                break;
            }
        }

        assert!(collected.iter().any(|event| {
            matches!(
                event,
                PromptEvent::AuthStateChange {
                    auth_id,
                    previous_state: AuthState::Unauthenticated,
                    new_state: AuthState::Authenticating,
                } if auth_id == "auth-plugin"
            )
        }));
        assert!(collected.iter().any(|event| {
            matches!(
                event,
                PromptEvent::AuthStateChange {
                    auth_id,
                    previous_state: AuthState::Authenticating,
                    new_state: AuthState::Authenticated,
                } if auth_id == "auth-plugin"
            )
        }));
    });
}

#[test]
fn session_auth_flow_updates_client_visible_availability() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![ProviderEvent::Complete]]);
        let agent = IronAgent::new(Config::default(), provider);
        register_test_auth_plugin(&agent, "auth-plugin");

        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let prompts = session.get_auth_prompts();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].state, AuthState::Unauthenticated);

        let request = session.start_auth_flow("auth-plugin").unwrap();
        let prompts = session.get_auth_prompts();
        assert_eq!(prompts[0].state, AuthState::Authenticating);

        session
            .complete_auth_flow(
                "auth-plugin",
                AuthInteractionResponse {
                    request_id: request.request_id,
                    result: AuthInteractionResult::Success {
                        code: "auth-code-456".to_string(),
                        state: None,
                    },
                },
            )
            .unwrap();

        let prompts = session.get_auth_prompts();
        assert_eq!(prompts[0].state, AuthState::Authenticated);

        let availability = agent.get_plugin_availability("auth-plugin").unwrap();
        assert!(availability.authenticated);
        assert_eq!(availability.available_tools, 1);

        let status = agent.get_plugin_status("auth-plugin").unwrap();
        assert_eq!(status.auth.state, AuthState::Authenticated);

        let stored_credentials = agent
            .plugin_registry()
            .get("auth-plugin")
            .and_then(|state| state.credentials)
            .unwrap();
        assert_eq!(stored_credentials.provider, "github");
        assert_eq!(stored_credentials.scopes, vec!["repo".to_string()]);
        assert_eq!(stored_credentials.access_token, "auth-code-456");
    });
}

#[test]
fn stream_cancel_emits_terminal_complete() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("cc1", "risky_op", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "should not appear".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("risky_op", "risky_op", json!({})).with_approval(true),
            |_| Ok(json!("should not run")),
        ));

        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (handle, mut events) = session.prompt_stream("go");

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event.clone());
            if let PromptEvent::ApprovalRequest { ref call_id, .. } = &event {
                if call_id == "cc1" {
                    handle.cancel().await;
                }
            }
            if matches!(&event, PromptEvent::Complete { .. }) {
                break;
            }
        }

        let completes: Vec<_> = collected
            .iter()
            .filter(|e| matches!(e, PromptEvent::Complete { .. }))
            .collect();
        assert_eq!(completes.len(), 1, "should have exactly one Complete event");

        let has_cancelled = collected.iter().any(|e| {
            matches!(
                e,
                PromptEvent::Complete {
                    outcome: PromptOutcome::Cancelled
                }
            )
        });
        assert!(has_cancelled, "should complete with Cancelled");
    });
}

#[test]
fn stream_single_terminal_complete_per_prompt() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "hello".into(),
            },
            ProviderEvent::Complete,
        ]]);
        let agent = IronAgent::new(Config::default(), provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (_handle, mut events) = session.prompt_stream("hi");

        let mut complete_count = 0;
        let mut total_events = 0;
        while let Some(event) = events.next().await {
            total_events += 1;
            if matches!(&event, PromptEvent::Complete { .. }) {
                complete_count += 1;
            }
        }

        assert!(total_events > 0, "should have some events");
        assert_eq!(complete_count, 1, "should have exactly one Complete event");
    });
}

#[test]
fn stream_approve_unknown_call_id_returns_error() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "hi".into(),
            },
            ProviderEvent::Complete,
        ]]);
        let agent = IronAgent::new(Config::default(), provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (handle, mut events) = session.prompt_stream("hi");

        let result = handle.approve("nonexistent_id");
        assert!(result.is_err(), "approving unknown call_id should fail");

        while events.next().await.is_some() {}
    });
}

#[test]
fn stream_deny_unknown_call_id_returns_error() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "hi".into(),
            },
            ProviderEvent::Complete,
        ]]);
        let agent = IronAgent::new(Config::default(), provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (handle, mut events) = session.prompt_stream("hi");

        let result = handle.deny("nonexistent_id");
        assert!(result.is_err(), "denying unknown call_id should fail");

        while events.next().await.is_some() {}
    });
}

// ===================================================================
// 5. Streaming isolation, idleness, and cancellation (task 1.4 / 2.3)
// ===================================================================

#[test]
fn stream_incremental_output_before_step_completion() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "chunk1 ".into(),
            },
            ProviderEvent::Output {
                content: "chunk2".into(),
            },
            ProviderEvent::Complete,
        ]]);
        let agent = IronAgent::new(Config::default(), provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (_handle, mut events) = session.prompt_stream("hi");

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event);
            if matches!(collected.last(), Some(PromptEvent::Complete { .. })) {
                break;
            }
        }

        let output_events: Vec<_> = collected
            .iter()
            .filter_map(|e| match e {
                PromptEvent::Output { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(output_events, vec!["chunk1 ", "chunk2"]);

        let complete_pos = collected
            .iter()
            .position(|e| matches!(e, PromptEvent::Complete { .. }))
            .unwrap();
        for (i, e) in collected.iter().enumerate() {
            if matches!(e, PromptEvent::Output { .. }) {
                assert!(
                    i < complete_pos,
                    "Output events must arrive before Complete"
                );
            }
        }
    });
}

#[test]
fn stream_multi_step_events_preserve_ordering() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::Output {
                    content: "step1 ".into(),
                },
                ProviderEvent::ToolCall {
                    call: ToolCall::new("ms1", "t", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "step2".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.register_tool(FunctionTool::simple("t", "t", |_| Ok(json!(0))));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (_handle, mut events) = session.prompt_stream("go");

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event);
            if matches!(collected.last(), Some(PromptEvent::Complete { .. })) {
                break;
            }
        }

        let step1_pos = collected
            .iter()
            .position(|e| matches!(e, PromptEvent::Output { text } if text == "step1 "))
            .unwrap();
        let tool_result_pos = collected
            .iter()
            .position(|e| matches!(e, PromptEvent::ToolResult { call_id, .. } if call_id == "ms1"))
            .unwrap();
        let step2_pos = collected
            .iter()
            .position(|e| matches!(e, PromptEvent::Output { text } if text == "step2"))
            .unwrap();

        assert!(
            step1_pos < tool_result_pos,
            "step1 output must arrive before tool result"
        );
        assert!(
            tool_result_pos < step2_pos,
            "tool result must arrive before step2 output"
        );
    });
}

#[test]
fn session_is_idle_when_no_active_prompts() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "hi".into(),
            },
            ProviderEvent::Complete,
        ]]);
        let agent = IronAgent::new(Config::default(), provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        assert!(session.is_idle(), "new session should be idle");

        session.prompt("hi").await;

        assert!(
            session.is_idle(),
            "session should be idle after prompt completes"
        );
    });
}

#[test]
fn session_is_not_idle_during_active_stream_prompt() {
    run_local(async {
        let tool_started = Arc::new(AtomicUsize::new(0));
        let tool_release = Arc::new(AtomicUsize::new(0));
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("c1", "slow", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![ProviderEvent::Complete],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        let started = tool_started.clone();
        let release = tool_release.clone();
        agent.register_tool(FunctionTool::simple("slow", "slow", move |_| {
            started.fetch_add(1, Ordering::SeqCst);
            while release.load(Ordering::SeqCst) == 0 {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok(json!("done"))
        }));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (_handle, mut events) = session.prompt_stream("hi");

        while tool_started.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }

        assert!(
            !session.is_idle(),
            "session should not be idle while prompt is active"
        );

        tool_release.store(1, Ordering::SeqCst);

        while let Some(event) = events.next().await {
            if matches!(event, PromptEvent::Complete { .. }) {
                break;
            }
        }

        assert!(
            session.is_idle(),
            "session should be idle after prompt completes"
        );
    });
}

#[test]
fn concurrent_streamed_prompts_on_one_connection() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::Output {
                    content: "session1".into(),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "session2".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        let conn = agent.connect();

        let session1 = conn.create_session().unwrap();
        let session2 = conn.create_session().unwrap();

        let (_h1, mut events1) = session1.prompt_stream("hi");
        let (_h2, mut events2) = session2.prompt_stream("hi");

        let mut collected1 = Vec::new();
        while let Some(event) = events1.next().await {
            collected1.push(event);
            if matches!(collected1.last(), Some(PromptEvent::Complete { .. })) {
                break;
            }
        }
        let mut collected2 = Vec::new();
        while let Some(event) = events2.next().await {
            collected2.push(event);
            if matches!(collected2.last(), Some(PromptEvent::Complete { .. })) {
                break;
            }
        }

        let s1_has_output = collected1
            .iter()
            .any(|e| matches!(e, PromptEvent::Output { text } if text == "session1"));
        let s2_has_output = collected2
            .iter()
            .any(|e| matches!(e, PromptEvent::Output { text } if text == "session2"));

        assert!(s1_has_output, "session1 should receive its output");
        assert!(s2_has_output, "session2 should receive its output");

        let s1_completes: Vec<_> = collected1
            .iter()
            .filter(|e| matches!(e, PromptEvent::Complete { .. }))
            .collect();
        let s2_completes: Vec<_> = collected2
            .iter()
            .filter(|e| matches!(e, PromptEvent::Complete { .. }))
            .collect();
        assert_eq!(s1_completes.len(), 1, "session1 should have one Complete");
        assert_eq!(s2_completes.len(), 1, "session2 should have one Complete");
    });
}

#[test]
fn cancel_between_tool_executions_skips_remaining() {
    run_local(async {
        let tool_started = Arc::new(AtomicUsize::new(0));
        let tool_release = Arc::new(AtomicUsize::new(0));
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("ct1", "slow", json!({})),
                },
                ProviderEvent::ToolCall {
                    call: ToolCall::new("ct2", "t2", json!({})),
                },
                ProviderEvent::ToolCall {
                    call: ToolCall::new("ct3", "t3", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![ProviderEvent::Complete],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        let started = tool_started.clone();
        let release = tool_release.clone();
        agent.register_tool(FunctionTool::simple("slow", "slow", move |_| {
            started.fetch_add(1, Ordering::SeqCst);
            while release.load(Ordering::SeqCst) == 0 {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok(json!("slow_done"))
        }));
        agent.register_tool(FunctionTool::simple("t2", "t2", |_| Ok(json!("t2_done"))));
        agent.register_tool(FunctionTool::simple("t3", "t3", |_| Ok(json!("t3_done"))));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (handle, mut events) = session.prompt_stream("go");

        while tool_started.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }

        handle.cancel().await;
        tool_release.store(1, Ordering::SeqCst);

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event.clone());
            if matches!(&event, PromptEvent::Complete { .. }) {
                break;
            }
        }

        assert_eq!(
            tool_started.load(Ordering::SeqCst),
            1,
            "only slow tool should execute"
        );

        let cancelled_results: Vec<_> = collected
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    PromptEvent::ToolResult {
                        status: ToolResultStatus::Failed,
                        ..
                    }
                )
            })
            .collect();
        assert!(
            cancelled_results.len() >= 2,
            "remaining tools should have failed/cancelled results, got {}",
            cancelled_results.len()
        );

        let has_cancelled_complete = collected.iter().any(|e| {
            matches!(
                e,
                PromptEvent::Complete {
                    outcome: PromptOutcome::Cancelled
                }
            )
        });
        assert!(has_cancelled_complete, "should complete with Cancelled");
    });
}

#[test]
fn tool_result_uses_canonical_tool_name_from_call() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("tn1", "my_named_tool", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![ProviderEvent::Complete],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.register_tool(FunctionTool::simple(
            "my_named_tool",
            "my_named_tool",
            |_| Ok(json!("result")),
        ));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (_handle, mut events) = session.prompt_stream("go");

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event);
            if matches!(collected.last(), Some(PromptEvent::Complete { .. })) {
                break;
            }
        }

        let tool_result = collected.iter().find_map(|e| {
            if let PromptEvent::ToolResult {
                call_id, tool_name, ..
            } = e
            {
                if call_id == "tn1" {
                    return Some(tool_name.clone());
                }
            }
            None
        });
        assert_eq!(
            tool_result,
            Some("my_named_tool".to_string()),
            "ToolResult should carry the canonical tool name from ToolCall"
        );
    });
}

// ===================================================================
// 8. One-active-prompt-per-session enforcement
// ===================================================================

#[test]
fn runtime_rejects_second_prompt_on_same_session() {
    let rt = IronRuntime::new(Config::default(), MockProvider::default());
    let conn_id = ConnectionId(1);
    rt.register_connection(conn_id);
    let (sid, _session) = rt.create_session(conn_id).unwrap();

    let first = rt.try_start_prompt(sid);
    assert!(first.is_ok(), "first prompt start should succeed");

    let second = rt.try_start_prompt(sid);
    assert!(
        second.is_err(),
        "second prompt start on same session should be rejected"
    );
}

#[test]
fn runtime_allows_prompt_after_previous_finishes() {
    let rt = IronRuntime::new(Config::default(), MockProvider::default());
    let conn_id = ConnectionId(1);
    rt.register_connection(conn_id);
    let (sid, _session) = rt.create_session(conn_id).unwrap();

    let first = rt.try_start_prompt(sid);
    assert!(first.is_ok(), "first prompt start should succeed");

    rt.finish_prompt(sid);

    let second = rt.try_start_prompt(sid);
    assert!(second.is_ok(), "prompt start after finish should succeed");
}

#[test]
fn facade_stream_rejects_second_prompt_while_active() {
    run_local(async {
        let tool_release = Arc::new(AtomicUsize::new(0));
        let tool_started = Arc::new(AtomicUsize::new(0));

        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("c1", "slow", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![ProviderEvent::Complete],
        ]);

        let agent = IronAgent::new(Config::default(), provider);
        let started = tool_started.clone();
        let release = tool_release.clone();
        agent.register_tool(FunctionTool::simple("slow", "slow", move |_| {
            started.fetch_add(1, Ordering::SeqCst);
            while release.load(Ordering::SeqCst) == 0 {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok(json!("done"))
        }));

        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (_handle, mut events) = session.prompt_stream("go");

        while tool_started.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }

        assert!(
            !session.is_idle(),
            "session should not be idle during active prompt"
        );

        let outcome = session.prompt("second prompt").await;
        assert_eq!(
            outcome,
            PromptOutcome::EndTurn,
            "second prompt while first is active should fail gracefully"
        );

        tool_release.store(1, Ordering::SeqCst);

        while let Some(event) = events.next().await {
            if matches!(event, PromptEvent::Complete { .. }) {
                break;
            }
        }
    });
}

#[test]
fn has_active_prompt_reflects_prompt_lifecycle() {
    let rt = IronRuntime::new(Config::default(), MockProvider::default());
    let conn_id = ConnectionId(1);
    rt.register_connection(conn_id);
    let (sid, _session) = rt.create_session(conn_id).unwrap();

    assert!(!rt.has_active_prompt(sid), "no active prompt before start");

    let _ephemeral = rt.try_start_prompt(sid).unwrap();
    assert!(rt.has_active_prompt(sid), "active prompt after start");

    rt.finish_prompt(sid);
    assert!(!rt.has_active_prompt(sid), "no active prompt after finish");
}

#[test]
fn cancel_active_prompt_signals_cancellation() {
    let rt = IronRuntime::new(Config::default(), MockProvider::default());
    let conn_id = ConnectionId(1);
    rt.register_connection(conn_id);
    let (sid, _session) = rt.create_session(conn_id).unwrap();

    let ephemeral = rt.try_start_prompt(sid).unwrap();
    assert!(!ephemeral.lock().is_cancel_requested());

    let cancelled = rt.cancel_active_prompt(sid);
    assert!(cancelled, "cancel should return true for active prompt");
    assert!(
        ephemeral.lock().is_cancel_requested(),
        "ephemeral should reflect cancellation"
    );

    let cancelled_again = rt.cancel_active_prompt(sid);
    assert!(
        cancelled_again,
        "cancel on still-active prompt should still return true"
    );
}

#[test]
fn semantic_event_denied_tool_parity() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("tc1", "dangerous", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![ProviderEvent::Complete],
        ]);
        let config = Config {
            default_approval_strategy: ApprovalStrategy::Always,
            ..Default::default()
        };
        let agent = IronAgent::new(config, provider);
        agent.register_tool(FunctionTool::simple("dangerous", "dangerous", |_| {
            Ok(json!("should not run"))
        }));
        let conn = agent.connect();

        let session = conn.create_session().unwrap();
        let (handle, mut events) = session.prompt_stream("do it");
        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            match &event {
                PromptEvent::ApprovalRequest { call_id, .. } => {
                    let _ = handle.deny(call_id);
                }
                PromptEvent::Complete { .. } => {
                    collected.push(event);
                    break;
                }
                _ => {}
            }
            collected.push(event);
        }

        let denied_result = collected.iter().find(|e| {
            matches!(
                e,
                PromptEvent::ToolResult {
                    status: ToolResultStatus::Denied,
                    ..
                }
            )
        });
        assert!(denied_result.is_some(), "expected Denied tool result");

        if let Some(PromptEvent::ToolResult {
            call_id, tool_name, ..
        }) = denied_result
        {
            assert_eq!(call_id, "tc1");
            assert_eq!(tool_name, "dangerous");
        }
    });
}

#[test]
fn prompt_iteration_uses_one_consistent_tool_snapshot() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("swap1", "swap_tool", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![ProviderEvent::Complete],
        ]);
        let agent = IronAgent::new(
            Config::default().with_approval_strategy(ApprovalStrategy::PerTool),
            provider.clone(),
        );
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("swap_tool", "original tool", json!({})).with_approval(true),
            |_| Ok(json!("old_result")),
        ));

        let conn = agent.connect();
        let session = conn.create_session().unwrap();
        let (handle, mut events) = session.prompt_stream("go");

        let mut saw_approval = false;
        let mut final_result = None;

        while let Some(event) = events.next().await {
            match event {
                PromptEvent::ApprovalRequest {
                    call_id, tool_name, ..
                } => {
                    assert_eq!(call_id, "swap1");
                    assert_eq!(tool_name, "swap_tool");
                    saw_approval = true;

                    // Replace the runtime tool after the prompt iteration has
                    // already advertised tools and decided approval. Execution
                    // should still use the original per-iteration snapshot.
                    agent.register_tool(FunctionTool::new(
                        ToolDefinition::new("swap_tool", "replacement tool", json!({})),
                        |_| Ok(json!("new_result")),
                    ));
                    handle.approve(&call_id).unwrap();
                }
                PromptEvent::ToolResult {
                    call_id,
                    tool_name,
                    status,
                    result,
                    ..
                } if call_id == "swap1" && tool_name == "swap_tool" => {
                    assert_eq!(status, ToolResultStatus::Completed);
                    final_result = result;
                }
                PromptEvent::Complete { outcome } => {
                    assert_eq!(outcome, PromptOutcome::EndTurn);
                    break;
                }
                _ => {}
            }
        }

        assert!(
            saw_approval,
            "approval should be based on the original snapshot"
        );
        assert_eq!(final_result, Some(json!("old_result")));

        let requests = provider.requests();
        assert!(
            !requests.is_empty(),
            "provider should receive at least one request"
        );
        assert!(
            requests[0]
                .tools
                .iter()
                .any(|tool| tool.name == "swap_tool"),
            "prompt construction should advertise the original tool snapshot"
        );
    });
}

#[test]
fn tool_execution_not_found_emits_failed_result() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("c1", "nonexistent", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![ProviderEvent::Complete],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let (handle, mut events) = session.prompt_stream("go");
        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event.clone());
            if matches!(collected.last(), Some(PromptEvent::Complete { .. })) {
                break;
            }
        }
        drop(handle);

        let failed = collected.iter().find(|e| {
            matches!(
                e,
                PromptEvent::ToolResult {
                    call_id,
                    status: ToolResultStatus::Failed,
                    ..
                } if call_id == "c1"
            )
        });
        assert!(
            failed.is_some(),
            "expected Failed result for nonexistent tool"
        );

        let records = session.tool_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ToolRecordStatus::Failed);
    });
}

// ---------------------------------------------------------------------------
// Streaming multimodal tests (prompt_stream_with_blocks)
// ---------------------------------------------------------------------------

#[test]
fn stream_blocks_emits_ordered_events() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("sb1", "my_tool", json!({"x": 1})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "done".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.register_tool(FunctionTool::simple("my_tool", "my_tool", |_| {
            Ok(json!(42))
        }));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let blocks = vec![
            ContentBlock::text("analyze this"),
            ContentBlock::Image {
                data: "iVBORw0KGgo=".into(),
                mime_type: "image/png".into(),
            },
        ];
        let (handle, mut events) = session.prompt_stream_with_blocks(&blocks);

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event);
            if matches!(collected.last(), Some(PromptEvent::Complete { .. })) {
                break;
            }
        }

        assert!(handle.status() == PromptStatus::Completed);

        let has_tool_call = collected
            .iter()
            .any(|e| matches!(e, PromptEvent::ToolCall { call_id, .. } if call_id == "sb1"));
        let has_tool_result = collected.iter().any(|e| matches!(e, PromptEvent::ToolResult { call_id, status: ToolResultStatus::Completed, .. } if call_id == "sb1"));
        let has_output = collected
            .iter()
            .any(|e| matches!(e, PromptEvent::Output { text } if text == "done"));
        let has_complete = collected.iter().any(|e| {
            matches!(
                e,
                PromptEvent::Complete {
                    outcome: PromptOutcome::EndTurn
                }
            )
        });

        assert!(has_tool_call, "expected ToolCall event");
        assert!(has_tool_result, "expected ToolResult(Completed) event");
        assert!(has_output, "expected Output event");
        assert!(has_complete, "expected Complete event");

        let complete_idx = collected
            .iter()
            .position(|e| matches!(e, PromptEvent::Complete { .. }))
            .unwrap();
        assert_eq!(
            complete_idx,
            collected.len() - 1,
            "Complete should be last event"
        );
    });
}

#[test]
fn stream_blocks_tool_call_precedes_tool_result() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("ord_blk", "t", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "fin".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(Config::default(), provider);
        agent.register_tool(FunctionTool::simple("t", "t", |_| Ok(json!(0))));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let blocks = vec![
            ContentBlock::text("go"),
            ContentBlock::Image {
                data: "AAAA".into(),
                mime_type: "image/jpeg".into(),
            },
        ];
        let (_handle, mut events) = session.prompt_stream_with_blocks(&blocks);

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event);
            if matches!(collected.last(), Some(PromptEvent::Complete { .. })) {
                break;
            }
        }

        let tool_call_pos = collected
            .iter()
            .position(
                |e| matches!(e, PromptEvent::ToolCall { call_id, .. } if call_id == "ord_blk"),
            )
            .unwrap();
        let tool_result_pos = collected
            .iter()
            .position(
                |e| matches!(e, PromptEvent::ToolResult { call_id, .. } if call_id == "ord_blk"),
            )
            .unwrap();
        assert!(
            tool_call_pos < tool_result_pos,
            "ToolCall must precede ToolResult"
        );

        let complete_idx = collected
            .iter()
            .position(|e| matches!(e, PromptEvent::Complete { .. }))
            .unwrap();
        assert_eq!(
            complete_idx,
            collected.len() - 1,
            "Complete should be last event"
        );
    });
}

#[test]
fn stream_blocks_approval_deny_resolves_as_tool_result() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![
            vec![
                ProviderEvent::ToolCall {
                    call: ToolCall::new("ap_blk", "risky", json!({})),
                },
                ProviderEvent::Complete,
            ],
            vec![
                ProviderEvent::Output {
                    content: "after".into(),
                },
                ProviderEvent::Complete,
            ],
        ]);
        let agent = IronAgent::new(
            Config::default().with_approval_strategy(ApprovalStrategy::PerTool),
            provider,
        );
        agent.register_tool(FunctionTool::new(
            ToolDefinition::new("risky", "risky", json!({})).with_approval(true),
            |_| Ok(json!("should not run")),
        ));
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let blocks = vec![
            ContentBlock::text("do it"),
            ContentBlock::Image {
                data: "iVBOR".into(),
                mime_type: "image/png".into(),
            },
        ];
        let (handle, mut events) = session.prompt_stream_with_blocks(&blocks);

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event.clone());
            if let PromptEvent::ApprovalRequest { ref call_id, .. } = event {
                if call_id == "ap_blk" {
                    handle.deny(call_id).unwrap();
                }
            }
            if matches!(&event, PromptEvent::Complete { .. }) {
                break;
            }
        }

        let has_denied_result = collected.iter().any(|e| {
            matches!(e, PromptEvent::ToolResult { call_id, status: ToolResultStatus::Denied, .. } if call_id == "ap_blk")
        });
        assert!(
            has_denied_result,
            "denied approval should produce ToolResult(Denied)"
        );

        let complete_idx = collected
            .iter()
            .position(|e| matches!(e, PromptEvent::Complete { .. }))
            .unwrap();
        assert_eq!(
            complete_idx,
            collected.len() - 1,
            "Complete should be last event"
        );
    });
}

#[test]
fn stream_blocks_cancel_emits_terminal_complete() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "thinking".into(),
            },
            ProviderEvent::Complete,
        ]]);
        let agent = IronAgent::new(Config::default(), provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        let blocks = vec![ContentBlock::text("start")];
        let (handle, mut events) = session.prompt_stream_with_blocks(&blocks);

        // Cancel immediately
        handle.cancel().await;

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event);
            if matches!(collected.last(), Some(PromptEvent::Complete { .. })) {
                break;
            }
        }

        let completes: Vec<_> = collected
            .iter()
            .filter(|e| matches!(e, PromptEvent::Complete { .. }))
            .collect();
        assert_eq!(
            completes.len(),
            1,
            "exactly one Complete event expected after cancel"
        );
    });
}

#[test]
fn stream_blocks_empty_slice_completes_normally() {
    run_local(async {
        let provider = MockProvider::with_infer_responses(vec![vec![
            ProviderEvent::Output {
                content: "ok".into(),
            },
            ProviderEvent::Complete,
        ]]);
        let agent = IronAgent::new(Config::default(), provider);
        let conn = agent.connect();
        let session = conn.create_session().unwrap();

        // Empty slice — should behave same as prompt_with_blocks(&[])
        let (handle, mut events) = session.prompt_stream_with_blocks(&[]);

        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event);
            if matches!(collected.last(), Some(PromptEvent::Complete { .. })) {
                break;
            }
        }

        let has_complete = collected.iter().any(|e| {
            matches!(
                e,
                PromptEvent::Complete {
                    outcome: PromptOutcome::EndTurn
                }
            )
        });
        assert!(has_complete, "empty blocks should still complete normally");
        assert!(handle.status() == PromptStatus::Completed);
    });
}
