//! Prompt sinks for delegated child runs.

use crate::connection::SharedClientChannel;
use crate::delegation::ChildApprovalMode;
use crate::prompt_lifecycle::{
    ApprovalRequest, ApprovalVerdict, PromptLifecycleEvent, PromptSink, ToolUpdateStatus,
};
use agent_client_protocol::schema as acp;
use std::pin::Pin;

/// A prompt sink for a delegated child session.
///
/// In `AutoApprove` mode, all approval requests resolve to `AllowOnce`.
/// In `PropagateToParent` mode, approval requests are forwarded to the parent
/// connection's client channel so the parent/main UI can resolve them.
pub(crate) struct DelegationPromptSink {
    mode: ChildApprovalMode,
    parent_client: SharedClientChannel,
    parent_session_id: acp::SessionId,
}

impl DelegationPromptSink {
    pub(crate) fn new(
        mode: ChildApprovalMode,
        parent_client: SharedClientChannel,
        parent_session_id: acp::SessionId,
    ) -> Self {
        Self {
            mode,
            parent_client,
            parent_session_id,
        }
    }
}

impl PromptSink for DelegationPromptSink {
    fn emit(&self, event: PromptLifecycleEvent) -> Pin<Box<dyn std::future::Future<Output = ()>>> {
        let parent_client = self.parent_client.clone();
        let parent_session_id = self.parent_session_id.clone();

        match event {
            PromptLifecycleEvent::Output { text } => {
                let prefixed = format!("[child] {}", text);
                Box::pin(async move {
                    let notif = crate::connection::notification(
                        &parent_session_id,
                        acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                            acp::ContentBlock::Text(acp::TextContent::new(&prefixed)),
                        )),
                    );
                    let _ = parent_client.send_notification(notif).await;
                })
            }
            PromptLifecycleEvent::ToolCallProposed {
                call_id,
                tool_name,
                arguments,
            } => {
                let child_call_id = format!("child-{}", call_id);
                Box::pin(async move {
                    let notif = crate::connection::notification(
                        &parent_session_id,
                        acp::SessionUpdate::ToolCall(
                            acp::ToolCall::new(acp::ToolCallId::new(child_call_id), &tool_name)
                                .raw_input(arguments)
                                .status(acp::ToolCallStatus::Pending),
                        ),
                    );
                    let _ = parent_client.send_notification(notif).await;
                })
            }
            PromptLifecycleEvent::ToolCallUpdate {
                call_id,
                tool_name,
                status,
                output,
            } => {
                let child_call_id = format!("child-{}", call_id);
                Box::pin(async move {
                    let acp_status = match status {
                        ToolUpdateStatus::InProgress => acp::ToolCallStatus::InProgress,
                        ToolUpdateStatus::Completed => acp::ToolCallStatus::Completed,
                        ToolUpdateStatus::Failed => acp::ToolCallStatus::Failed,
                    };
                    let mut fields = acp::ToolCallUpdateFields::new().status(acp_status);
                    if !tool_name.is_empty() {
                        fields = fields.title(&tool_name);
                    }
                    if let Some(out) = output {
                        fields = fields.raw_output(out);
                    }
                    let notif = crate::connection::notification(
                        &parent_session_id,
                        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                            acp::ToolCallId::new(child_call_id),
                            fields,
                        )),
                    );
                    let _ = parent_client.send_notification(notif).await;
                })
            }
            _ => Box::pin(async {}),
        }
    }

    fn request_approval(
        &self,
        request: ApprovalRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = ApprovalVerdict>>> {
        match self.mode {
            ChildApprovalMode::AutoApprove => Box::pin(async { ApprovalVerdict::AllowOnce }),
            ChildApprovalMode::PropagateToParent => {
                let parent_client = self.parent_client.clone();
                let parent_session_id = self.parent_session_id.clone();
                let call_id = request.call_id.clone();
                let tool_name = request.tool_name.clone();
                let arguments = request.arguments.clone();

                Box::pin(async move {
                    let tool_call_update = acp::ToolCallUpdate::new(
                        acp::ToolCallId::new(call_id),
                        acp::ToolCallUpdateFields::new()
                            .title(&tool_name)
                            .raw_input(arguments)
                            .status(acp::ToolCallStatus::InProgress),
                    );

                    let perm_request = acp::RequestPermissionRequest::new(
                        parent_session_id,
                        tool_call_update,
                        vec![
                            acp::PermissionOption::new(
                                acp::PermissionOptionId::new("allow_once"),
                                "Allow once",
                                acp::PermissionOptionKind::AllowOnce,
                            ),
                            acp::PermissionOption::new(
                                acp::PermissionOptionId::new("reject_once"),
                                "Deny",
                                acp::PermissionOptionKind::RejectOnce,
                            ),
                        ],
                    );

                    match parent_client.request_permission(perm_request).await {
                        Ok(response) => match response.outcome {
                            acp::RequestPermissionOutcome::Cancelled => ApprovalVerdict::Cancelled,
                            acp::RequestPermissionOutcome::Selected(sel) => {
                                let option_id = sel.option_id.to_string();
                                if option_id.contains("allow") {
                                    ApprovalVerdict::AllowOnce
                                } else {
                                    ApprovalVerdict::Denied
                                }
                            }
                            _ => ApprovalVerdict::Denied,
                        },
                        Err(_) => ApprovalVerdict::Denied,
                    }
                })
            }
        }
    }

    fn parent_client_channel(&self) -> Option<SharedClientChannel> {
        Some(self.parent_client.clone())
    }

    fn parent_session_acp_id(&self) -> Option<acp::SessionId> {
        Some(self.parent_session_id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::ClientChannel;

    enum TestPermissionOutcome {
        Allow,
        Deny,
        Cancel,
    }

    struct TestClientChannel {
        outcome: TestPermissionOutcome,
    }

    impl TestClientChannel {
        fn allow() -> Self {
            Self {
                outcome: TestPermissionOutcome::Allow,
            }
        }

        fn deny() -> Self {
            Self {
                outcome: TestPermissionOutcome::Deny,
            }
        }

        fn cancel() -> Self {
            Self {
                outcome: TestPermissionOutcome::Cancel,
            }
        }
    }

    impl ClientChannel for TestClientChannel {
        fn send_notification(
            &self,
            _notification: acp::SessionNotification,
        ) -> Pin<Box<dyn std::future::Future<Output = agent_client_protocol::Result<()>>>> {
            Box::pin(async { Ok(()) })
        }

        fn request_permission(
            &self,
            _request: acp::RequestPermissionRequest,
        ) -> Pin<
            Box<
                dyn std::future::Future<
                    Output = agent_client_protocol::Result<acp::RequestPermissionResponse>,
                >,
            >,
        > {
            let outcome = match self.outcome {
                TestPermissionOutcome::Allow => acp::RequestPermissionOutcome::Selected(
                    acp::SelectedPermissionOutcome::new(acp::PermissionOptionId::new("allow_once")),
                ),
                TestPermissionOutcome::Deny => {
                    acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                        acp::PermissionOptionId::new("reject_once"),
                    ))
                }
                TestPermissionOutcome::Cancel => acp::RequestPermissionOutcome::Cancelled,
            };
            Box::pin(async move { Ok(acp::RequestPermissionResponse::new(outcome)) })
        }
    }

    async fn propagated_verdict_for(channel: TestClientChannel) -> ApprovalVerdict {
        let sink = DelegationPromptSink::new(
            ChildApprovalMode::PropagateToParent,
            std::rc::Rc::new(channel),
            acp::SessionId::new("session-1"),
        );
        sink.request_approval(ApprovalRequest {
            call_id: "c1".to_string(),
            tool_name: "shell".to_string(),
            arguments: serde_json::json!({}),
        })
        .await
    }

    #[tokio::test]
    async fn auto_approve_returns_allow_once() {
        let sink = DelegationPromptSink::new(
            ChildApprovalMode::AutoApprove,
            std::rc::Rc::new(TestClientChannel::deny()),
            acp::SessionId::new("session-1"),
        );
        let verdict = sink
            .request_approval(ApprovalRequest {
                call_id: "c1".to_string(),
                tool_name: "shell".to_string(),
                arguments: serde_json::json!({}),
            })
            .await;
        assert_eq!(verdict, ApprovalVerdict::AllowOnce);
    }

    #[tokio::test]
    async fn propagate_returns_parent_verdict() {
        let verdict = propagated_verdict_for(TestClientChannel::allow()).await;
        assert_eq!(verdict, ApprovalVerdict::AllowOnce);
    }

    #[tokio::test]
    async fn propagate_returns_parent_denial() {
        let verdict = propagated_verdict_for(TestClientChannel::deny()).await;
        assert_eq!(verdict, ApprovalVerdict::Denied);
    }

    #[tokio::test]
    async fn propagate_returns_parent_cancellation() {
        let verdict = propagated_verdict_for(TestClientChannel::cancel()).await;
        assert_eq!(verdict, ApprovalVerdict::Cancelled);
    }
}
