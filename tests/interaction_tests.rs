#![allow(deprecated)]
//! Tests for the shared pending-interaction envelope model.
//!
//! Covers:
//! - 6.1 Single-choice and multi-choice interaction payloads
//! - 6.2 Approval batching under the shared pending-interaction envelope
//! - 6.3 Pausing and resuming a turn through a choice resolution
//! - 6.4 Cancelled, invalid, and mismatched interaction resolutions
//! - 6.5 Compatibility wrappers for approval-facing callers
//! - 6.6 Continuation context receives canonical choice_resolution record

use futures::stream::{self, BoxStream};
use futures::StreamExt;
use iron_core::{
    tool::FunctionTool, ApprovalCallInfo, ApprovalDecision, ApprovalInteractionInfo,
    ApprovalInteractionResolution, ApprovalVerdict, ChoiceInteractionInfo,
    ChoiceInteractionResolution, ChoiceItem, ChoiceResolutionItem, ChoiceResolutionRecord,
    ChoiceResolutionStatus, ChoiceSelectionMode, Config, InteractionResolution, InteractionSource,
    LoopError, PendingInteractionInfo, PendingInteractionPayload, Provider, ProviderEvent,
    Session, SessionHandle, ToolDefinition, ToolRegistry, TurnEvent, TurnEvents, TurnStatus,
};
use iron_providers::{
    ChoiceItem as ProviderChoiceItem, ChoiceRequest as ProviderChoiceRequest,
    ChoiceSelectionMode as ProviderChoiceSelectionMode, InferenceRequest, Message, ToolCall,
    CHOICE_REQUEST_TOOL_NAME,
};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

// ---------------------------------------------------------------------------
// Mock provider
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct MockProvider {
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
        let response = vec![ProviderEvent::Complete];
        Box::pin(async move { Ok(response) })
    }

    fn infer_stream(
        &self,
        request: InferenceRequest,
    ) -> iron_providers::ProviderFuture<'_, BoxStream<'static, iron_providers::ProviderResult<ProviderEvent>>>
    {
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

fn handle_with_tools(provider: MockProvider, registry: ToolRegistry) -> SessionHandle {
    SessionHandle::with_tools(Config::default(), provider, Session::new(), registry)
}

// ---------------------------------------------------------------------------
// 6.1 Single-choice and multi-choice interaction payload tests
// ---------------------------------------------------------------------------

#[test]
fn choice_interaction_info_single_serialization() {
    let info = ChoiceInteractionInfo {
        prompt: "Pick one".into(),
        selection_mode: ChoiceSelectionMode::Single,
        items: vec![
            ChoiceItem {
                id: "a".into(),
                label: "Option A".into(),
                description: None,
            },
            ChoiceItem {
                id: "b".into(),
                label: "Option B".into(),
                description: Some("The second option".into()),
            },
        ],
    };

    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["prompt"], "Pick one");
    assert_eq!(json["selection_mode"], "single");
    assert_eq!(json["items"][0]["id"], "a");
    assert_eq!(json["items"][1]["description"], "The second option");

    // Round-trip
    let deserialized: ChoiceInteractionInfo = serde_json::from_value(json).unwrap();
    assert_eq!(deserialized.prompt, "Pick one");
    assert_eq!(deserialized.selection_mode, ChoiceSelectionMode::Single);
    assert_eq!(deserialized.items.len(), 2);
}

#[test]
fn choice_interaction_info_multiple_serialization() {
    let info = ChoiceInteractionInfo {
        prompt: "Select all that apply".into(),
        selection_mode: ChoiceSelectionMode::Multiple,
        items: vec![
            ChoiceItem {
                id: "x".into(),
                label: "X".into(),
                description: None,
            },
            ChoiceItem {
                id: "y".into(),
                label: "Y".into(),
                description: None,
            },
            ChoiceItem {
                id: "z".into(),
                label: "Z".into(),
                description: None,
            },
        ],
    };

    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["selection_mode"], "multiple");
    assert_eq!(json["items"].as_array().unwrap().len(), 3);
}

#[test]
fn pending_interaction_info_choice_payload() {
    let info = PendingInteractionInfo {
        interaction_id: "choice_1".into(),
        source: InteractionSource::Model,
        payload: PendingInteractionPayload::Choice(ChoiceInteractionInfo {
            prompt: "Which repo?".into(),
            selection_mode: ChoiceSelectionMode::Single,
            items: vec![ChoiceItem {
                id: "r1".into(),
                label: "iron-core".into(),
                description: None,
            }],
        }),
    };

    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["interaction_id"], "choice_1");
    assert_eq!(json["source"], "model");
    assert_eq!(json["payload"]["kind"], "choice");
    assert_eq!(json["payload"]["prompt"], "Which repo?");
}

#[test]
fn pending_interaction_info_approval_payload() {
    let info = PendingInteractionInfo {
        interaction_id: "approval_1".into(),
        source: InteractionSource::Runtime,
        payload: PendingInteractionPayload::Approval(ApprovalInteractionInfo {
            calls: vec![
                ApprovalCallInfo {
                    call_id: "c1".into(),
                    tool_name: "bash".into(),
                    arguments: json!({"command": "ls"}),
                },
                ApprovalCallInfo {
                    call_id: "c2".into(),
                    tool_name: "write".into(),
                    arguments: json!({"path": "out.txt"}),
                },
            ],
        }),
    };

    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["payload"]["kind"], "approval");
    assert_eq!(json["payload"]["calls"].as_array().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// 6.2 Approval batching under shared pending-interaction envelope
// ---------------------------------------------------------------------------

#[tokio::test]
async fn approval_batch_exposes_single_interaction_envelope() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::ToolCall {
            call: ToolCall::new("c1", "danger", json!({"a": 1})),
        },
        ProviderEvent::ToolCall {
            call: ToolCall::new("c2", "danger", json!({"b": 2})),
        },
        ProviderEvent::Complete,
    ]]);
    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::new(
        ToolDefinition::new("danger", "danger", json!({})).with_approval(true),
        move |_| Ok(json!({"ok": true})),
    ));

    let handle = handle_with_tools(provider, registry);
    let (th, mut events) = handle.start_turn("do it").unwrap();

    let mut saw_interaction = false;
    let mut legacy_approval_count = 0;

    loop {
        let event = events.next_event().await.unwrap();

        match &event {
            TurnEvent::InteractionRequired { interaction } => {
                saw_interaction = true;
                // Verify it's an approval envelope with both calls
                assert_eq!(interaction.source, InteractionSource::Runtime);
                match &interaction.payload {
                    PendingInteractionPayload::Approval(approval) => {
                        assert_eq!(approval.calls.len(), 2);
                        let call_ids: Vec<&str> =
                            approval.calls.iter().map(|c| c.call_id.as_str()).collect();
                        assert!(call_ids.contains(&"c1"));
                        assert!(call_ids.contains(&"c2"));
                    }
                    other => panic!("Expected approval payload, got {:?}", other),
                }

                // Verify turn status is WaitingForInteraction
                match th.status() {
                    TurnStatus::WaitingForInteraction { .. } => {}
                    other => panic!("Expected WaitingForInteraction, got {:?}", other),
                }

                // Resolve with batch approval
                th.resolve_interaction(
                    &interaction.interaction_id,
                    InteractionResolution::Approval(ApprovalInteractionResolution {
                        decisions: vec![
                            ApprovalDecision {
                                call_id: "c1".into(),
                                verdict: ApprovalVerdict::AllowOnce,
                            },
                            ApprovalDecision {
                                call_id: "c2".into(),
                                verdict: ApprovalVerdict::AllowOnce,
                            },
                        ],
                    }),
                )
                .unwrap();
            }
            TurnEvent::ApprovalRequired { .. } => {
                legacy_approval_count += 1;
            }
            TurnEvent::Finished { .. } => break,
            _ => {}
        }
    }

    assert!(saw_interaction, "Should have seen InteractionRequired event");
    assert_eq!(
        legacy_approval_count, 2,
        "Should have seen 2 legacy ApprovalRequired events for compatibility"
    );
}

// ---------------------------------------------------------------------------
// 6.3 Pausing and resuming a turn through a choice resolution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn choice_interaction_submitted_resumes_turn() {
    let provider = MockProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::Output {
                content: "I found multiple repositories. ".into(),
            },
            ProviderEvent::ChoiceRequest {
                request: ProviderChoiceRequest {
                    prompt: "Please choose one repository.".into(),
                    selection_mode: ProviderChoiceSelectionMode::Single,
                    items: vec![
                        ProviderChoiceItem {
                            id: "repo_1".into(),
                            label: "agentiron/iron-core".into(),
                            description: Some("Core runtime".into()),
                        },
                        ProviderChoiceItem {
                            id: "repo_2".into(),
                            label: "agentiron/iron-tui".into(),
                            description: Some("TUI client".into()),
                        },
                    ],
                },
            },
            ProviderEvent::Complete,
        ],
        vec![
            ProviderEvent::Output {
                content: "Continuing with agentiron/iron-tui".into(),
            },
            ProviderEvent::Complete,
        ],
    ]);
    let handle = handle_with_tools(provider.clone(), ToolRegistry::new());
    let (th, mut events) = handle.start_turn("choose repo").unwrap();

    let mut interaction_id = None;
    let mut saw_partial_output = false;
    let mut saw_choice_event = false;
    loop {
        let event = events.next_event().await.unwrap();
        match event {
            TurnEvent::OutputDelta { content } => {
                if content.contains("multiple repositories") {
                    saw_partial_output = true;
                }
            }
            TurnEvent::InteractionRequired { interaction } => {
                saw_choice_event = true;
                interaction_id = Some(interaction.interaction_id.clone());
                assert_eq!(interaction.source, InteractionSource::Model);
                match interaction.payload {
                    PendingInteractionPayload::Choice(choice) => {
                        assert_eq!(choice.prompt, "Please choose one repository.");
                        assert_eq!(choice.selection_mode, ChoiceSelectionMode::Single);
                        assert_eq!(choice.items.len(), 2);
                    }
                    other => panic!("Expected choice payload, got {:?}", other),
                }

                th.resolve_interaction(
                    interaction.interaction_id,
                    InteractionResolution::Choice(ChoiceInteractionResolution::Submitted {
                        selected_ids: vec!["repo_2".into()],
                    }),
                )
                .unwrap();
            }
            TurnEvent::Finished { .. } => break,
            _ => {}
        }
    }

    assert!(saw_partial_output);
    assert!(saw_choice_event);

    let session = handle.session();
    assert!(matches!(
        session.messages[1],
        Message::Assistant { .. }
    ));
    assert!(matches!(
        session.messages[2],
        Message::SystemStructured { .. }
    ));
    match &session.messages[2] {
        Message::SystemStructured { kind, payload } => {
            assert_eq!(kind, "choice_resolution");
            assert_eq!(payload["status"], "submitted");
            assert_eq!(payload["interaction_id"], interaction_id.unwrap());
            assert_eq!(payload["selected_items"][0]["id"], "repo_2");
            assert_eq!(payload["selected_items"][0]["label"], "agentiron/iron-tui");
        }
        other => panic!("Expected structured runtime message, got {:?}", other),
    }

    // Verify the internal choice request tool was exposed to the provider.
    let requests = provider.requests.lock().unwrap();
    assert!(requests.iter().all(|request| request.tools.iter().any(|tool| tool.name == CHOICE_REQUEST_TOOL_NAME)));
}

#[tokio::test]
async fn choice_interaction_cancelled_resolves_correctly() {
    let provider = MockProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ChoiceRequest {
                request: ProviderChoiceRequest {
                    prompt: "Pick a deployment target".into(),
                    selection_mode: ProviderChoiceSelectionMode::Multiple,
                    items: vec![
                        ProviderChoiceItem {
                            id: "staging".into(),
                            label: "Staging".into(),
                            description: None,
                        },
                        ProviderChoiceItem {
                            id: "prod".into(),
                            label: "Production".into(),
                            description: None,
                        },
                    ],
                },
            },
            ProviderEvent::Complete,
        ],
        vec![ProviderEvent::Complete],
    ]);
    let handle = handle_with_tools(provider, ToolRegistry::new());
    let (th, mut events) = handle.start_turn("deploy").unwrap();

    loop {
        let event = events.next_event().await.unwrap();
        match event {
            TurnEvent::InteractionRequired { interaction } => {
                th.resolve_interaction(
                    interaction.interaction_id,
                    InteractionResolution::Choice(ChoiceInteractionResolution::Cancelled),
                )
                .unwrap();
            }
            TurnEvent::Finished { .. } => break,
            _ => {}
        }
    }

    let session = handle.session();
    match session.last_message().unwrap() {
        Message::SystemStructured { kind, payload } => {
            assert_eq!(kind, "choice_resolution");
            assert_eq!(payload["status"], "cancelled");
            assert!(payload["selected_items"].as_array().unwrap().is_empty());
        }
        other => panic!("Expected choice_resolution system message, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// 6.4 Cancelled, invalid, and mismatched interaction resolutions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_interaction_rejects_unknown_interaction_id() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::ToolCall {
            call: ToolCall::new("c1", "danger", json!({})),
        },
        ProviderEvent::Complete,
    ]]);
    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::new(
        ToolDefinition::new("danger", "danger", json!({})).with_approval(true),
        move |_| Ok(json!({"ok": true})),
    ));

    let handle = handle_with_tools(provider, registry);
    let (th, mut events) = handle.start_turn("do it").unwrap();

    // Wait for the interaction to be pending
    loop {
        let event = events.next_event().await.unwrap();
        if let TurnEvent::InteractionRequired { .. } = &event {
            break;
        }
        if matches!(event, TurnEvent::Finished { .. }) {
            panic!("Turn finished before we could test");
        }
    }

    // Try resolving with a wrong interaction_id
    let result = th.resolve_interaction(
        "nonexistent_id",
        InteractionResolution::Approval(ApprovalInteractionResolution {
            decisions: vec![ApprovalDecision {
                call_id: "c1".into(),
                verdict: ApprovalVerdict::AllowOnce,
            }],
        }),
    );

    assert!(result.is_err());
    match result.unwrap_err() {
        LoopError::InteractionNotFound { interaction_id } => {
            assert_eq!(interaction_id, "nonexistent_id");
        }
        other => panic!("Expected InteractionNotFound, got {:?}", other),
    }

    // Clean up: deny the actual pending call
    th.approve("c1").unwrap();
    let _ = collect_all(&mut events).await;
}

#[tokio::test]
async fn resolve_interaction_rejects_when_not_waiting() {
    let provider = MockProvider::with_stream_responses(vec![vec![ProviderEvent::Complete]]);
    let handle = handle_with_tools(provider, ToolRegistry::new());
    let (th, mut events) = handle.start_turn("hi").unwrap();

    // Wait for turn to finish
    let _ = collect_all(&mut events).await;

    let result = th.resolve_interaction(
        "any_id",
        InteractionResolution::Choice(ChoiceInteractionResolution::Submitted {
            selected_ids: vec!["a".into()],
        }),
    );

    assert!(result.is_err());
    match result.unwrap_err() {
        LoopError::TurnFinished => {}
        other => panic!("Expected TurnFinished, got {:?}", other),
    }
}

#[tokio::test]
async fn resolve_interaction_rejects_invalid_choice_selection_at_api_boundary() {
    let provider = MockProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ChoiceRequest {
                request: ProviderChoiceRequest {
                    prompt: "Choose one".into(),
                    selection_mode: ProviderChoiceSelectionMode::Single,
                    items: vec![
                        ProviderChoiceItem {
                            id: "a".into(),
                            label: "A".into(),
                            description: None,
                        },
                        ProviderChoiceItem {
                            id: "b".into(),
                            label: "B".into(),
                            description: None,
                        },
                    ],
                },
            },
            ProviderEvent::Complete,
        ],
        vec![ProviderEvent::Complete],
    ]);
    let handle = handle_with_tools(provider, ToolRegistry::new());
    let (th, mut events) = handle.start_turn("choose").unwrap();

    let interaction_id = loop {
        let event = events.next_event().await.unwrap();
        if let TurnEvent::InteractionRequired { interaction } = event {
            break interaction.interaction_id;
        }
    };

    let duplicate_err = th
        .resolve_interaction(
            interaction_id.clone(),
            InteractionResolution::Choice(ChoiceInteractionResolution::Submitted {
                selected_ids: vec!["a".into(), "a".into()],
            }),
        )
        .unwrap_err();
    assert!(matches!(duplicate_err, LoopError::InvalidInteractionResolution { .. }));

    let unknown_err = th
        .resolve_interaction(
            interaction_id.clone(),
            InteractionResolution::Choice(ChoiceInteractionResolution::Submitted {
                selected_ids: vec!["missing".into()],
            }),
        )
        .unwrap_err();
    assert!(matches!(unknown_err, LoopError::InvalidInteractionResolution { .. }));

    let mismatch_err = th
        .resolve_interaction(
            interaction_id.clone(),
            InteractionResolution::Approval(ApprovalInteractionResolution { decisions: vec![] }),
        )
        .unwrap_err();
    assert!(matches!(mismatch_err, LoopError::InteractionKindMismatch { .. }));

    // Cleanup with a valid resolution
    th.resolve_interaction(
        interaction_id,
        InteractionResolution::Choice(ChoiceInteractionResolution::Submitted {
            selected_ids: vec!["b".into()],
        }),
    )
    .unwrap();
    let _ = collect_all(&mut events).await;
}

#[tokio::test]
async fn provider_rejects_ambiguous_choice_and_tool_phase() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::ChoiceRequest {
            request: ProviderChoiceRequest {
                prompt: "Choose one".into(),
                selection_mode: ProviderChoiceSelectionMode::Single,
                items: vec![ProviderChoiceItem {
                    id: "a".into(),
                    label: "A".into(),
                    description: None,
                }],
            },
        },
        ProviderEvent::ToolCall {
            call: ToolCall::new("c1", "danger", json!({})),
        },
        ProviderEvent::Complete,
    ]]);
    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::new(
        ToolDefinition::new("danger", "danger", json!({})).with_approval(true),
        move |_| Ok(json!({"ok": true})),
    ));

    let handle = handle_with_tools(provider, registry);
    let (_, mut events) = handle.start_turn("do it").unwrap();
    let collected = collect_all(&mut events).await;
    assert!(collected.iter().any(|event| matches!(event, TurnEvent::Finished { outcome } if matches!(outcome, iron_core::TurnOutcome::Failed { message } if message.contains("choice request and tool calls")))));
}

#[tokio::test]
async fn provider_rejects_multiple_choice_requests_in_one_phase() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::ChoiceRequest {
            request: ProviderChoiceRequest {
                prompt: "Choose one".into(),
                selection_mode: ProviderChoiceSelectionMode::Single,
                items: vec![ProviderChoiceItem {
                    id: "a".into(),
                    label: "A".into(),
                    description: None,
                }],
            },
        },
        ProviderEvent::ChoiceRequest {
            request: ProviderChoiceRequest {
                prompt: "Choose two".into(),
                selection_mode: ProviderChoiceSelectionMode::Single,
                items: vec![ProviderChoiceItem {
                    id: "b".into(),
                    label: "B".into(),
                    description: None,
                }],
            },
        },
        ProviderEvent::Complete,
    ]]);
    let handle = handle_with_tools(provider, ToolRegistry::new());
    let (_, mut events) = handle.start_turn("choose").unwrap();
    let collected = collect_all(&mut events).await;
    assert!(collected.iter().any(|event| matches!(event, TurnEvent::Finished { outcome } if matches!(outcome, iron_core::TurnOutcome::Failed { message } if message.contains("multiple blocking choice requests")))));
}

#[tokio::test]
async fn provider_rejects_invalid_choice_request_payload() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::ChoiceRequest {
            request: ProviderChoiceRequest {
                prompt: " ".into(),
                selection_mode: ProviderChoiceSelectionMode::Single,
                items: vec![
                    ProviderChoiceItem {
                        id: "dup".into(),
                        label: "A".into(),
                        description: None,
                    },
                    ProviderChoiceItem {
                        id: "dup".into(),
                        label: "B".into(),
                        description: None,
                    },
                ],
            },
        },
        ProviderEvent::Complete,
    ]]);
    let handle = handle_with_tools(provider, ToolRegistry::new());
    let (_, mut events) = handle.start_turn("choose").unwrap();
    let collected = collect_all(&mut events).await;
    assert!(collected.iter().any(|event| matches!(event, TurnEvent::Finished { outcome } if matches!(outcome, iron_core::TurnOutcome::Failed { message } if message.contains("prompt must not be empty")))));
}

// ---------------------------------------------------------------------------
// 6.5 Compatibility wrappers for approval-facing callers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn legacy_approve_deny_still_works() {
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

    loop {
        let event = events.next_event().await.unwrap();
        if let TurnEvent::ApprovalRequired { call_id, .. } = &event {
            // Use the legacy approve() API
            th.approve(call_id).unwrap();
        }
        if matches!(event, TurnEvent::Finished { .. }) {
            break;
        }
    }

    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn legacy_deny_prevents_execution() {
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

    let mut saw_denied_result = false;
    loop {
        let event = events.next_event().await.unwrap();
        if let TurnEvent::ApprovalRequired { call_id, .. } = &event {
            th.deny(call_id).unwrap();
        }
        if let TurnEvent::ToolResult { call_id, result, .. } = &event {
            if call_id == "c1" {
                saw_denied_result = true;
                assert!(result["error"].is_string());
            }
        }
        if matches!(event, TurnEvent::Finished { .. }) {
            break;
        }
    }

    assert_eq!(executions.load(Ordering::SeqCst), 0, "Tool should not have executed");
    assert!(saw_denied_result, "Should have seen denied tool result");
}

#[tokio::test]
async fn legacy_approve_rejects_unknown_call_id() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::ToolCall {
            call: ToolCall::new("c1", "danger", json!({})),
        },
        ProviderEvent::Complete,
    ]]);
    let mut registry = ToolRegistry::new();
    registry.register(FunctionTool::new(
        ToolDefinition::new("danger", "danger", json!({})).with_approval(true),
        move |_| Ok(json!({"ok": true})),
    ));

    let handle = handle_with_tools(provider, registry);
    let (th, mut events) = handle.start_turn("do it").unwrap();

    loop {
        let event = events.next_event().await.unwrap();
        if matches!(event, TurnEvent::InteractionRequired { .. }) {
            let err = th.approve("nonexistent").unwrap_err();
            assert!(matches!(err, LoopError::ApprovalNotFound { .. }));
            th.approve("c1").unwrap();
        }
        if matches!(event, TurnEvent::Finished { .. }) {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// 6.6 Canonical choice_resolution record
// ---------------------------------------------------------------------------

#[test]
fn choice_resolution_record_submitted() {
    let record = ChoiceResolutionRecord::submitted(
        "interaction_123".into(),
        "Which repo?".into(),
        ChoiceSelectionMode::Single,
        vec![ChoiceResolutionItem {
            id: "repo_2".into(),
            label: "agentiron/iron-tui".into(),
        }],
    );

    assert_eq!(record.kind, "choice_resolution");
    assert_eq!(record.interaction_id, "interaction_123");
    assert_eq!(record.status, ChoiceResolutionStatus::Submitted);
    assert_eq!(record.prompt, "Which repo?");
    assert_eq!(record.selection_mode, ChoiceSelectionMode::Single);
    assert_eq!(record.selected_items.len(), 1);
    assert_eq!(record.selected_items[0].id, "repo_2");
    assert_eq!(record.selected_items[0].label, "agentiron/iron-tui");

    // Verify serialization
    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["kind"], "choice_resolution");
    assert_eq!(json["status"], "submitted");
    assert_eq!(json["selected_items"][0]["id"], "repo_2");
    assert_eq!(json["selected_items"][0]["label"], "agentiron/iron-tui");
}

#[test]
fn choice_resolution_record_cancelled() {
    let record = ChoiceResolutionRecord::cancelled(
        "interaction_456".into(),
        "Pick a color".into(),
        ChoiceSelectionMode::Multiple,
    );

    assert_eq!(record.kind, "choice_resolution");
    assert_eq!(record.status, ChoiceResolutionStatus::Cancelled);
    assert_eq!(record.prompt, "Pick a color");
    assert!(record.selected_items.is_empty(), "Cancelled should have empty selected_items");

    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["status"], "cancelled");
    assert!(json["selected_items"].as_array().unwrap().is_empty());
}

#[test]
fn choice_resolution_record_round_trip() {
    let record = ChoiceResolutionRecord::submitted(
        "ix_789".into(),
        "Choose deployment target".into(),
        ChoiceSelectionMode::Multiple,
        vec![
            ChoiceResolutionItem {
                id: "staging".into(),
                label: "Staging".into(),
            },
            ChoiceResolutionItem {
                id: "production".into(),
                label: "Production".into(),
            },
        ],
    );

    let json = serde_json::to_string(&record).unwrap();
    let restored: ChoiceResolutionRecord = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.kind, "choice_resolution");
    assert_eq!(restored.interaction_id, "ix_789");
    assert_eq!(restored.status, ChoiceResolutionStatus::Submitted);
    assert_eq!(restored.prompt, "Choose deployment target");
    assert_eq!(restored.selection_mode, ChoiceSelectionMode::Multiple);
    assert_eq!(restored.selected_items.len(), 2);
    assert_eq!(restored.selected_items[0].id, "staging");
    assert_eq!(restored.selected_items[1].label, "Production");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
