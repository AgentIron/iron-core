//! Behavioral tests for the AgentLoop state machine.
#![allow(deprecated)]

use futures::stream::{self, BoxStream};
use futures::StreamExt;
use iron_core::{
    config::ContextWindowPolicy, tool::FunctionTool, AgentLoop, Config, Provider, ProviderEvent,
    Session, ToolDefinition, ToolRegistry,
};
use iron_providers::{GenerationConfig, InferenceRequest, ToolCall, ToolPolicy};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

#[derive(Clone, Default)]
struct MockProvider {
    infer_responses: Arc<Mutex<VecDeque<Vec<ProviderEvent>>>>,
    stream_responses: Arc<Mutex<VecDeque<Vec<ProviderEvent>>>>,
    requests: Arc<Mutex<Vec<InferenceRequest>>>,
}

impl MockProvider {
    fn with_infer_responses(responses: Vec<Vec<ProviderEvent>>) -> Self {
        Self {
            infer_responses: Arc::new(Mutex::new(responses.into())),
            ..Self::default()
        }
    }

    fn with_stream_responses(responses: Vec<Vec<ProviderEvent>>) -> Self {
        Self {
            stream_responses: Arc::new(Mutex::new(responses.into())),
            ..Self::default()
        }
    }

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
            .stream_responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![ProviderEvent::Complete]);
        Box::pin(async move { Ok(stream::iter(response.into_iter().map(Ok)).boxed()) })
    }
}

#[tokio::test]
async fn run_turn_executes_tool_and_continues_iteration() {
    let provider = MockProvider::with_infer_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("call_1", "lookup", json!({"q": "rust"})),
            },
            ProviderEvent::Complete,
        ],
        vec![
            ProviderEvent::Output {
                content: "done".to_string(),
            },
            ProviderEvent::Complete,
        ],
    ]);
    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::simple("lookup", "lookup", |_| {
        Ok(json!({"answer": 42}))
    }));

    let loop_state = AgentLoop::with_tools(Config::default(), provider.clone(), registry);
    let mut session = Session::new();
    let events = loop_state.run_turn(&mut session, "find it").await.unwrap();

    assert!(events
        .iter()
        .any(|event| matches!(event, iron_core::StreamEvent::ToolCall { call_id, .. } if call_id == "call_1")));
    assert!(events.iter().any(|event| matches!(
        event,
        iron_core::StreamEvent::ToolResult { call_id, .. } if call_id == "call_1"
    )));
    assert!(matches!(
        events.last(),
        Some(iron_core::StreamEvent::Complete)
    ));
    assert_eq!(provider.requests().len(), 2);
    assert!(matches!(
        &provider.requests()[1].transcript.messages[1],
        iron_providers::Message::Tool { call_id, .. } if call_id == "call_1"
    ));
    assert_eq!(
        session.messages.last(),
        Some(&iron_providers::Message::assistant("done"))
    );
}

#[tokio::test]
async fn approval_requests_pause_until_resume() {
    let provider = MockProvider::with_infer_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("call_1", "secure", json!({"path": "/tmp"})),
            },
            ProviderEvent::Complete,
        ],
        vec![
            ProviderEvent::Output {
                content: "approved".to_string(),
            },
            ProviderEvent::Complete,
        ],
    ]);
    let executions = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    let counter = executions.clone();
    registry.register(FunctionTool::new(
        ToolDefinition::new("secure", "secure", json!({})).with_approval(true),
        move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"ok": true}))
        },
    ));

    let loop_state = AgentLoop::with_tools(Config::default(), provider.clone(), registry);
    let mut session = Session::new();
    let events = loop_state.run_turn(&mut session, "do it").await.unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        iron_core::StreamEvent::ApprovalRequest { call_id, .. } if call_id == "call_1"
    )));
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert!(session.metadata.contains_key("agent_loop_pending_approval"));

    let resumed = loop_state.resume_turn(&mut session, true).await.unwrap();
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert!(resumed.iter().any(|event| matches!(
        event,
        iron_core::StreamEvent::ToolResult { call_id, .. } if call_id == "call_1"
    )));
    assert!(matches!(
        resumed.last(),
        Some(iron_core::StreamEvent::Complete)
    ));
}

#[tokio::test]
async fn streaming_turn_emits_all_chunks_once_and_updates_session() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "Hel".to_string(),
        },
        ProviderEvent::Output {
            content: "lo".to_string(),
        },
        ProviderEvent::Complete,
    ]]);
    let loop_state = AgentLoop::new(Config::default(), provider);
    let mut session = Session::new();
    let events: Vec<_> = loop_state
        .run_turn_stream(&mut session, "hi")
        .await
        .unwrap()
        .collect()
        .await;

    let outputs: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            iron_core::StreamEvent::Output { content } => Some(content.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(outputs, vec!["Hel".to_string(), "lo".to_string()]);
    assert_eq!(
        session.messages.last(),
        Some(&iron_providers::Message::assistant("Hello"))
    );
}

#[tokio::test]
async fn streaming_turn_executes_tools_across_iterations() {
    let provider = MockProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("call_stream", "lookup", json!({"q": "rust"})),
            },
            ProviderEvent::Complete,
        ],
        vec![
            ProviderEvent::Output {
                content: "streamed".to_string(),
            },
            ProviderEvent::Complete,
        ],
    ]);
    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::simple("lookup", "lookup", |_| {
        Ok(json!({"answer": "ok"}))
    }));

    let loop_state = AgentLoop::with_tools(Config::default(), provider, registry);
    let mut session = Session::new();
    let events: Vec<_> = loop_state
        .run_turn_stream(&mut session, "hi")
        .await
        .unwrap()
        .collect()
        .await;

    assert!(events.iter().any(|event| matches!(
        event,
        iron_core::StreamEvent::ToolCall { call_id, .. } if call_id == "call_stream"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        iron_core::StreamEvent::ToolResult { call_id, .. } if call_id == "call_stream"
    )));
    assert!(matches!(
        events.last(),
        Some(iron_core::StreamEvent::Complete)
    ));
}

#[tokio::test]
async fn missing_tool_uses_original_call_id() {
    let provider = MockProvider::with_infer_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("call_missing", "unknown", json!({"q": 1})),
            },
            ProviderEvent::Complete,
        ],
        vec![
            ProviderEvent::Output {
                content: "fallback".to_string(),
            },
            ProviderEvent::Complete,
        ],
    ]);
    let loop_state = AgentLoop::new(Config::default(), provider);
    let mut session = Session::new();
    let _ = loop_state.run_turn(&mut session, "hi").await.unwrap();

    assert!(matches!(
        &session.messages[1],
        iron_providers::Message::Tool { call_id, .. } if call_id == "call_missing"
    ));
}

#[tokio::test]
async fn keep_recent_policy_is_applied_to_requests() {
    let provider = MockProvider::with_infer_responses(vec![vec![ProviderEvent::Complete]]);
    let loop_state = AgentLoop::new(
        Config::default().with_context_window_policy(ContextWindowPolicy::KeepRecent(2)),
        provider.clone(),
    );
    let mut session = Session::new();
    session.add_user_message("one");
    session.add_assistant_message("two");
    session.add_user_message("three");

    let _ = loop_state.run_turn(&mut session, "four").await.unwrap();
    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].transcript.messages.len(), 2);
    assert_eq!(
        requests[0].transcript.messages,
        vec![
            iron_providers::Message::user("three"),
            iron_providers::Message::user("four"),
        ]
    );
}

#[tokio::test]
async fn summarize_after_is_explicitly_rejected() {
    let provider = MockProvider::with_infer_responses(vec![]);
    let loop_state = AgentLoop::new(
        Config::default().with_context_window_policy(ContextWindowPolicy::SummarizeAfter(3)),
        provider,
    );
    let mut session = Session::new();

    let error = loop_state.run_turn(&mut session, "hi").await.unwrap_err();
    assert!(matches!(error, iron_core::LoopError::InvalidConfig { .. }));
}

#[tokio::test]
async fn max_iterations_emit_terminal_event() {
    let provider = MockProvider::with_infer_responses(vec![vec![], vec![]]);
    let loop_state = AgentLoop::new(Config::default().with_max_iterations(2), provider);
    let mut session = Session::new();
    let events = loop_state.run_turn(&mut session, "loop").await.unwrap();

    assert!(matches!(
        events.last(),
        Some(iron_core::StreamEvent::MaxIterationsReached { count: 2 })
    ));
}

#[tokio::test]
async fn generation_defaults_applied_to_requests() {
    let provider = MockProvider::with_infer_responses(vec![vec![ProviderEvent::Complete]]);
    let config = Config::default().with_default_generation(
        GenerationConfig::new()
            .with_temperature(0.5)
            .with_max_tokens(100),
    );
    let loop_state = AgentLoop::new(config, provider.clone());
    let mut session = Session::new();
    let _ = loop_state.run_turn(&mut session, "hi").await.unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].generation.temperature, Some(0.5));
    assert_eq!(requests[0].generation.max_tokens, Some(100));
}

#[tokio::test]
async fn default_tool_policy_applied_when_tools_registered() {
    let provider = MockProvider::with_infer_responses(vec![vec![ProviderEvent::Complete]]);
    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::simple("t", "t", |_| Ok(serde_json::json!(0))));

    let config = Config::default().with_default_tool_policy(ToolPolicy::Required);
    let loop_state = AgentLoop::with_tools(config, provider.clone(), registry);
    let mut session = Session::new();
    let _ = loop_state.run_turn(&mut session, "hi").await.unwrap();

    let requests = provider.requests();
    assert_eq!(requests[0].tool_policy, ToolPolicy::Required);
}

#[tokio::test]
async fn tool_policy_coerced_to_none_when_no_tools_registered() {
    let provider = MockProvider::with_infer_responses(vec![vec![ProviderEvent::Complete]]);
    let config = Config::default().with_default_tool_policy(ToolPolicy::Required);
    let loop_state = AgentLoop::new(config, provider.clone());
    let mut session = Session::new();
    let _ = loop_state.run_turn(&mut session, "hi").await.unwrap();

    let requests = provider.requests();
    assert_eq!(requests[0].tool_policy, ToolPolicy::None);
}
