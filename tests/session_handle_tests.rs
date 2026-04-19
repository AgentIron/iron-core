#![allow(deprecated)]
use futures::stream::{self, BoxStream};
use futures::StreamExt;
use iron_core::{
    tool::FunctionTool, Config, InteractionSource, LoopError, PendingInteractionPayload,
    Provider, ProviderEvent, Session, SessionHandle, SessionRuntime, ToolDefinition,
    ToolRegistry, TurnEvent, TurnEvents, TurnOutcome, TurnStatus,
};
use iron_providers::{InferenceRequest, ToolCall};
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
    fn with_stream_responses(responses: Vec<Vec<ProviderEvent>>) -> Self {
        Self {
            stream_responses: Arc::new(Mutex::new(responses.into())),
            ..Self::default()
        }
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

async fn collect_all(events: &mut TurnEvents) -> Vec<TurnEvent> {
    let mut collected = Vec::new();
    while let Some(event) = events.next_event().await {
        let is_finished = matches!(event, TurnEvent::Finished { .. });
        collected.push(event);
        if is_finished {
            break;
        }
    }
    collected
}

fn simple_handle(provider: MockProvider) -> SessionHandle {
    SessionHandle::new(Config::default(), provider, Session::new())
}

fn handle_with_tools(provider: MockProvider, registry: ToolRegistry) -> SessionHandle {
    SessionHandle::with_tools(Config::default(), provider, Session::new(), registry)
}

fn handle_in(runtime: &SessionRuntime, provider: MockProvider) -> SessionHandle {
    SessionHandle::new_in(runtime, Config::default(), provider, Session::new())
}

#[tokio::test]
async fn simple_text_response_completes() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "Hello world".into(),
        },
        ProviderEvent::Complete,
    ]]);
    let handle = simple_handle(provider);

    let (th, mut events) = handle.start_turn("hi").unwrap();
    let all = collect_all(&mut events).await;

    assert!(all
        .iter()
        .any(|e| matches!(e, TurnEvent::OutputDelta { content } if content == "Hello world")));
    assert!(all.iter().any(|e| matches!(
        e,
        TurnEvent::Finished {
            outcome: TurnOutcome::Completed
        }
    )));
    assert_eq!(
        th.status(),
        TurnStatus::Finished {
            outcome: TurnOutcome::Completed
        }
    );
}

#[tokio::test]
async fn incremental_output_deltas() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "Hel".into(),
        },
        ProviderEvent::Output {
            content: "lo".into(),
        },
        ProviderEvent::Complete,
    ]]);
    let handle = simple_handle(provider);

    let (_, mut events) = handle.start_turn("hi").unwrap();
    let all = collect_all(&mut events).await;

    let deltas: Vec<&str> = all
        .iter()
        .filter_map(|e| match e {
            TurnEvent::OutputDelta { content } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, vec!["Hel", "lo"]);

    assert_eq!(
        handle.session().messages.last(),
        Some(&iron_providers::Message::assistant("Hello"))
    );
}

#[tokio::test]
async fn tool_execution_and_continuation() {
    let provider = MockProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("c1", "calc", json!({"x": 1})),
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
    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::simple("calc", "calc", |_| Ok(json!(42))));

    let handle = handle_with_tools(provider, registry);
    let (_, mut events) = handle.start_turn("compute").unwrap();
    let all = collect_all(&mut events).await;

    assert!(all.iter().any(|e| matches!(
        e,
        TurnEvent::ToolCall { call_id, .. } if call_id == "c1"
    )));
    assert!(all.iter().any(|e| matches!(
        e,
        TurnEvent::ToolResult { call_id, result, .. }
        if call_id == "c1" && *result == json!(42)
    )));
    assert!(all.iter().any(|e| matches!(
        e,
        TurnEvent::Finished {
            outcome: TurnOutcome::Completed
        }
    )));
}

#[tokio::test]
async fn approval_flow_approve_tool() {
    let provider = MockProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("c1", "danger", json!({"path": "/etc"})),
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
    let executions = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    let counter = executions.clone();
    registry.register(FunctionTool::new(
        ToolDefinition::new("danger", "danger", json!({})).with_approval(true),
        move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"ok": true}))
        },
    ));

    let handle = handle_with_tools(provider, registry);
    let (th, mut events) = handle.start_turn("do it").unwrap();

    let mut collected = Vec::new();
    loop {
        let event = events.next_event().await.unwrap();
        collected.push(event.clone());
        if let TurnEvent::ApprovalRequired { call_id, .. } = &event {
            assert_eq!(call_id, "c1");
            // Verify the turn is waiting for an interaction with an approval payload
            let status = th.status();
            match &status {
                TurnStatus::WaitingForInteraction { pending } => {
                    assert_eq!(pending.source, InteractionSource::Runtime);
                    match &pending.payload {
                        PendingInteractionPayload::Approval(approval) => {
                            assert_eq!(approval.calls.len(), 1);
                            assert_eq!(approval.calls[0].call_id, "c1");
                            assert_eq!(approval.calls[0].tool_name, "danger");
                        }
                        other => panic!("Expected approval payload, got {:?}", other),
                    }
                }
                other => panic!("Expected WaitingForInteraction, got {:?}", other),
            }
            th.approve("c1").unwrap();
        }
        if matches!(event, TurnEvent::Finished { .. }) {
            break;
        }
    }

    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert!(collected.iter().any(|e| matches!(
        e,
        TurnEvent::ToolResult { call_id, .. } if call_id == "c1"
    )));
    assert!(collected.iter().any(|e| matches!(
        e,
        TurnEvent::Finished {
            outcome: TurnOutcome::Completed
        }
    )));
}

#[tokio::test]
async fn approval_flow_deny_tool() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::ToolCall {
            call: ToolCall::new("c1", "danger", json!({})),
        },
        ProviderEvent::Complete,
    ]]);
    let executions = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    let counter = executions.clone();
    registry.register(FunctionTool::new(
        ToolDefinition::new("danger", "danger", json!({})).with_approval(true),
        move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"ok": true}))
        },
    ));

    let handle = handle_with_tools(provider, registry);
    let (th, mut events) = handle.start_turn("do it").unwrap();

    let mut collected = Vec::new();
    loop {
        let event = events.next_event().await.unwrap();
        collected.push(event.clone());
        if let TurnEvent::ApprovalRequired { call_id, .. } = &event {
            th.deny(call_id).unwrap();
        }
        if matches!(event, TurnEvent::Finished { .. }) {
            break;
        }
    }

    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert!(collected.iter().any(|e| matches!(
        e,
        TurnEvent::ToolResult { call_id, result, .. }
        if call_id == "c1" && result["error"].as_str() == Some("Tool execution denied by user")
    )));
}

#[tokio::test]
async fn interrupt_stops_turn() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "start ".into(),
        },
        ProviderEvent::Output {
            content: "more".into(),
        },
        ProviderEvent::Complete,
    ]]);
    let handle = simple_handle(provider);

    let (th, mut events) = handle.start_turn("hi").unwrap();
    th.interrupt().unwrap();

    let all = collect_all(&mut events).await;
    assert!(all.iter().any(|e| matches!(
        e,
        TurnEvent::Finished {
            outcome: TurnOutcome::Interrupted
        }
    )));
}

#[tokio::test]
async fn cancel_stops_turn() {
    let provider = MockProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("c1", "slow_tool", json!({})),
            },
            ProviderEvent::Complete,
        ],
        vec![ProviderEvent::Complete],
    ]);
    let tool_started = Arc::new(AtomicUsize::new(0));
    let tool_release = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    let started = tool_started.clone();
    let release = tool_release.clone();
    registry.register(FunctionTool::simple("slow_tool", "slow_tool", move |_| {
        started.store(1, Ordering::SeqCst);
        while release.load(Ordering::SeqCst) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Ok(json!("done"))
    }));

    let handle = handle_with_tools(provider, registry);
    let (th, mut events) = handle.start_turn("hi").unwrap();

    while tool_started.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    th.cancel().unwrap();
    tool_release.store(1, Ordering::SeqCst);

    let all = collect_all(&mut events).await;
    assert!(all.iter().any(|e| matches!(
        e,
        TurnEvent::Finished {
            outcome: TurnOutcome::Cancelled
        }
    )));
}

#[tokio::test]
async fn max_iterations_reached() {
    let provider = MockProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("c1", "loop_tool", json!({})),
            },
            ProviderEvent::Complete,
        ],
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("c2", "loop_tool", json!({})),
            },
            ProviderEvent::Complete,
        ],
    ]);
    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::simple("loop_tool", "loop_tool", |_| {
        Ok(json!(0))
    }));

    let config = Config::default().with_max_iterations(2);
    let handle = SessionHandle::with_tools(config, provider, Session::new(), registry);

    let (_, mut events) = handle.start_turn("loop").unwrap();
    let all = collect_all(&mut events).await;

    assert!(all.iter().any(|e| matches!(
        e,
        TurnEvent::Finished {
            outcome: TurnOutcome::MaxIterationsReached { count: 2 }
        }
    )));
}

#[tokio::test]
async fn provider_error_results_in_failed_outcome() {
    let provider = MockProvider::with_stream_responses(vec![vec![ProviderEvent::Error {
        message: "rate limited".into(),
    }]]);
    let handle = simple_handle(provider);

    let (_, mut events) = handle.start_turn("hi").unwrap();
    let all = collect_all(&mut events).await;

    assert!(all.iter().any(|e| matches!(
        e,
        TurnEvent::Finished {
            outcome: TurnOutcome::Failed { message }
        } if message.contains("rate limited")
    )));
}

#[tokio::test]
async fn turn_already_active_prevents_second_turn() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::ToolCall {
            call: ToolCall::new("c1", "slow_tool", json!({})),
        },
        ProviderEvent::Complete,
    ]]);
    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::new(
        ToolDefinition::new("slow_tool", "slow_tool", json!({})).with_approval(true),
        |_| Ok(json!("ok")),
    ));
    let handle = handle_with_tools(provider, registry);

    let (th1, mut ev1) = handle.start_turn("first").unwrap();

    loop {
        let event = ev1.next_event().await.unwrap();
        if matches!(event, TurnEvent::ApprovalRequired { .. }) {
            break;
        }
    }

    let err = handle.start_turn("second").unwrap_err();
    assert!(matches!(err, LoopError::TurnAlreadyActive { turn_id: 1 }));

    th1.deny("c1").unwrap();
    collect_all(&mut ev1).await;
}

#[tokio::test]
async fn can_start_new_turn_after_previous_finishes() {
    let provider = MockProvider::with_stream_responses(vec![
        vec![ProviderEvent::Complete],
        vec![
            ProviderEvent::Output {
                content: "second response".into(),
            },
            ProviderEvent::Complete,
        ],
    ]);
    let handle = simple_handle(provider);

    let (_, mut events) = handle.start_turn("first").unwrap();
    collect_all(&mut events).await;

    let (_, mut events2) = handle.start_turn("second").unwrap();
    let all2 = collect_all(&mut events2).await;

    assert!(all2
        .iter()
        .any(|e| matches!(e, TurnEvent::OutputDelta { content } if content == "second response")));
}

#[tokio::test]
async fn approve_on_wrong_call_id_returns_error() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::ToolCall {
            call: ToolCall::new("c1", "danger", json!({})),
        },
        ProviderEvent::Complete,
    ]]);
    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::new(
        ToolDefinition::new("danger", "danger", json!({})).with_approval(true),
        |_| Ok(json!("ok")),
    ));

    let handle = handle_with_tools(provider, registry);
    let (th, mut events) = handle.start_turn("go").unwrap();

    loop {
        let event = events.next_event().await.unwrap();
        if matches!(event, TurnEvent::ApprovalRequired { .. }) {
            let err = th.approve("nonexistent").unwrap_err();
            assert!(matches!(err, LoopError::ApprovalNotFound { .. }));

            th.deny("c1").unwrap();
            break;
        }
    }
    collect_all(&mut events).await;
}

#[tokio::test]
async fn approve_when_not_waiting_returns_error() {
    let provider = MockProvider::with_stream_responses(vec![vec![ProviderEvent::Complete]]);
    let handle = simple_handle(provider);

    let (th, mut events) = handle.start_turn("hi").unwrap();

    let err = th.approve("c1").unwrap_err();
    assert!(matches!(
        err,
        LoopError::NotWaitingForApproval | LoopError::TurnFinished
    ));

    collect_all(&mut events).await;
}

#[tokio::test]
async fn operations_on_finished_turn_return_error() {
    let provider = MockProvider::with_stream_responses(vec![vec![ProviderEvent::Complete]]);
    let handle = simple_handle(provider);

    let (th, mut events) = handle.start_turn("hi").unwrap();
    collect_all(&mut events).await;

    assert!(matches!(
        th.interrupt().unwrap_err(),
        LoopError::TurnFinished
    ));
    assert!(matches!(th.cancel().unwrap_err(), LoopError::TurnFinished));
    assert!(matches!(
        th.approve("x").unwrap_err(),
        LoopError::TurnFinished
    ));
    assert!(matches!(th.deny("x").unwrap_err(), LoopError::TurnFinished));
}

#[tokio::test]
async fn active_turn_returns_none_when_finished() {
    let provider = MockProvider::with_stream_responses(vec![vec![ProviderEvent::Complete]]);
    let handle = simple_handle(provider);

    let (_, mut events) = handle.start_turn("hi").unwrap();
    collect_all(&mut events).await;

    assert!(handle.active_turn().is_none());
}

#[tokio::test]
async fn active_turn_returns_handle_while_running() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::ToolCall {
            call: ToolCall::new("c1", "slow_tool", json!({})),
        },
        ProviderEvent::Complete,
    ]]);
    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::new(
        ToolDefinition::new("slow_tool", "slow_tool", json!({})).with_approval(true),
        |_| Ok(json!("ok")),
    ));
    let handle = handle_with_tools(provider, registry);

    let (th, mut events) = handle.start_turn("hi").unwrap();

    loop {
        let event = events.next_event().await.unwrap();
        if matches!(event, TurnEvent::ApprovalRequired { .. }) {
            break;
        }
    }

    let recovered = handle.active_turn();
    assert!(recovered.is_some());
    assert_eq!(recovered.unwrap().id(), th.id());

    th.deny("c1").unwrap();
    collect_all(&mut events).await;
}

#[tokio::test]
async fn session_updated_across_turns() {
    let provider = MockProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::Output {
                content: "first".into(),
            },
            ProviderEvent::Complete,
        ],
        vec![
            ProviderEvent::Output {
                content: "second".into(),
            },
            ProviderEvent::Complete,
        ],
    ]);
    let handle = simple_handle(provider);

    let (_, mut ev1) = handle.start_turn("hi").unwrap();
    collect_all(&mut ev1).await;

    let (_, mut ev2) = handle.start_turn("hello").unwrap();
    collect_all(&mut ev2).await;

    let msgs = &handle.session().messages;
    assert_eq!(msgs.len(), 4);
    assert!(matches!(&msgs[0], iron_providers::Message::User { content } if content == "hi"));
    assert!(
        matches!(&msgs[1], iron_providers::Message::Assistant { content } if content == "first")
    );
    assert!(matches!(&msgs[2], iron_providers::Message::User { content } if content == "hello"));
    assert!(
        matches!(&msgs[3], iron_providers::Message::Assistant { content } if content == "second")
    );
}

#[tokio::test]
async fn dropped_shared_runtime_handle_does_not_kill_turn() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "still running".into(),
        },
        ProviderEvent::Complete,
    ]]);
    let runtime = SessionRuntime::new();
    let handle = handle_in(&runtime, provider);

    let (_, mut events) = handle.start_turn("hi").unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    drop(handle);

    let all = collect_all(&mut events).await;
    assert!(all.iter().any(|e| matches!(
        e,
        TurnEvent::Finished {
            outcome: TurnOutcome::Completed
        }
    )));

    runtime.shutdown();
}

#[tokio::test]
async fn dropped_private_runtime_handle_terminates_event_stream() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "still running".into(),
        },
        ProviderEvent::Complete,
    ]]);
    let handle = simple_handle(provider);

    let (_, mut events) = handle.start_turn("hi").unwrap();
    drop(handle);

    let all = collect_all(&mut events).await;
    let has_finished = all.iter().any(|e| matches!(e, TurnEvent::Finished { .. }));
    assert!(has_finished || all.is_empty());
}

#[tokio::test]
async fn close_rejects_future_turns() {
    let provider = MockProvider::with_stream_responses(vec![vec![ProviderEvent::Complete]]);
    let handle = simple_handle(provider);

    handle.close();

    let err = handle.start_turn("hi").unwrap_err();
    assert!(matches!(err, LoopError::SessionClosed));
}

#[tokio::test]
async fn close_terminates_active_turn() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "partial".into(),
        },
        ProviderEvent::Complete,
    ]]);
    let handle = simple_handle(provider);

    let (_, mut events) = handle.start_turn("hi").unwrap();
    handle.close();

    let all = collect_all(&mut events).await;
    let has_finished = all.iter().any(|e| matches!(e, TurnEvent::Finished { .. }));
    assert!(has_finished || all.is_empty());
}

#[tokio::test]
async fn shared_runtime_session_close_does_not_affect_other_sessions() {
    let provider1 = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "one".into(),
        },
        ProviderEvent::Complete,
    ]]);
    let provider2 = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "two".into(),
        },
        ProviderEvent::Complete,
    ]]);
    let runtime = SessionRuntime::new();
    let handle1 = handle_in(&runtime, provider1);
    let handle2 = handle_in(&runtime, provider2);

    let (_, mut events2) = handle2.start_turn("hi").unwrap();
    handle1.close();

    let all2 = collect_all(&mut events2).await;
    assert!(all2.iter().any(|e| matches!(
        e,
        TurnEvent::Finished {
            outcome: TurnOutcome::Completed
        }
    )));

    runtime.shutdown();
}

#[tokio::test]
async fn runtime_shutdown_cancels_active_turns() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "running".into(),
        },
        ProviderEvent::Complete,
    ]]);
    let runtime = SessionRuntime::new();
    let handle = handle_in(&runtime, provider);

    let (_, mut events) = handle.start_turn("hi").unwrap();
    runtime.shutdown();

    let all = collect_all(&mut events).await;
    let terminal = all.iter().any(|e| matches!(e, TurnEvent::Finished { .. }));
    assert!(terminal || all.is_empty());
}

#[tokio::test]
async fn runtime_shutdown_rejects_new_turns() {
    let provider = MockProvider::with_stream_responses(vec![vec![ProviderEvent::Complete]]);
    let runtime = SessionRuntime::new();
    let handle = handle_in(&runtime, provider);

    runtime.shutdown();
    let err = handle.start_turn("hi").unwrap_err();
    assert!(matches!(err, LoopError::RuntimeShutdown));
}

#[tokio::test]
async fn from_handle_uses_caller_runtime() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "borrowed".into(),
        },
        ProviderEvent::Complete,
    ]]);
    let handle = tokio::runtime::Handle::current();
    let runtime = SessionRuntime::from_handle(handle);
    let session_handle = handle_in(&runtime, provider);

    let (_, mut events) = session_handle.start_turn("hi").unwrap();
    let all = collect_all(&mut events).await;
    assert!(all
        .iter()
        .any(|e| matches!(e, TurnEvent::OutputDelta { content } if content == "borrowed")));
    assert!(all.iter().any(|e| matches!(
        e,
        TurnEvent::Finished {
            outcome: TurnOutcome::Completed
        }
    )));
}

#[tokio::test]
async fn mixed_approval_multiple_tools() {
    let provider = MockProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("c1", "safe", json!({})),
            },
            ProviderEvent::ToolCall {
                call: ToolCall::new("c2", "danger", json!({})),
            },
            ProviderEvent::Complete,
        ],
        vec![ProviderEvent::Complete],
    ]);
    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::simple("safe", "safe", |_| {
        Ok(json!("safe_result"))
    }));
    registry.register(FunctionTool::new(
        ToolDefinition::new("danger", "danger", json!({})).with_approval(true),
        |_| Ok(json!("danger_result")),
    ));

    let handle = handle_with_tools(provider, registry);
    let (th, mut events) = handle.start_turn("go").unwrap();

    let mut collected = Vec::new();
    loop {
        let event = events.next_event().await.unwrap();
        collected.push(event.clone());

        if let TurnEvent::ToolResult {
            call_id, result, ..
        } = &event
        {
            if call_id == "c1" {
                assert_eq!(*result, json!("safe_result"));
            }
        }

        if let TurnEvent::ApprovalRequired { call_id, .. } = &event {
            if call_id == "c2" {
                th.approve("c2").unwrap();
            }
        }

        if matches!(event, TurnEvent::Finished { .. }) {
            break;
        }
    }

    assert!(collected.iter().any(|e| matches!(
        e,
        TurnEvent::ToolResult { call_id, result, .. }
        if call_id == "c2" && *result == json!("danger_result")
    )));
}

#[tokio::test]
async fn provider_stream_error_results_in_failed_outcome() {
    let responses: Vec<Vec<ProviderEvent>> = vec![vec![
        ProviderEvent::Output {
            content: "start".into(),
        },
        ProviderEvent::Error {
            message: "connection lost".into(),
        },
    ]];
    let provider = MockProvider::with_stream_responses(responses);
    let handle = simple_handle(provider);

    let (_, mut events) = handle.start_turn("hi").unwrap();
    let all = collect_all(&mut events).await;

    assert!(all.iter().any(|e| matches!(
        e,
        TurnEvent::Finished {
            outcome: TurnOutcome::Failed { message }
        } if message.contains("connection lost")
    )));
}

#[test]
fn turn_works_without_ambient_tokio_runtime() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "no ambient runtime needed".into(),
        },
        ProviderEvent::Complete,
    ]]);
    let handle = simple_handle(provider);

    let (_, mut events) = handle.start_turn("hi").unwrap();

    let all = std::thread::scope(|s| {
        s.spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let mut collected = Vec::new();
                while let Some(event) = events.next_event().await {
                    let is_finished = matches!(event, TurnEvent::Finished { .. });
                    collected.push(event);
                    if is_finished {
                        break;
                    }
                }
                collected
            })
        })
        .join()
        .unwrap()
    });

    assert!(all
        .iter()
        .any(|e| matches!(e, TurnEvent::OutputDelta { content } if content == "no ambient runtime needed")));
    assert!(all.iter().any(|e| matches!(
        e,
        TurnEvent::Finished {
            outcome: TurnOutcome::Completed
        }
    )));
}
