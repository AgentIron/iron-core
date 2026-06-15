use crate::connection::{notification, SharedClientChannel};
use agent_client_protocol::schema as acp;
use std::pin::Pin;

pub enum PromptLifecycleEvent {
    Output {
        text: String,
    },
    ToolCallProposed {
        call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
    },
    ToolCallUpdate {
        call_id: String,
        tool_name: String,
        status: ToolUpdateStatus,
        output: Option<serde_json::Value>,
    },
    ScriptActivity {
        script_id: String,
        parent_call_id: String,
        activity_type: String,
        status: String,
        detail: Option<serde_json::Value>,
    },
    CompactionStarted {
        compaction_id: String,
        method: String,
    },
    CompactionFinished {
        compaction_id: String,
        tokens_before: Option<u32>,
        tokens_after: Option<u32>,
        method: String,
    },
    CompactionFailed {
        compaction_id: String,
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolUpdateStatus {
    InProgress,
    Completed,
    Failed,
}

pub struct ApprovalRequest {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalVerdict {
    AllowOnce,
    Denied,
    Cancelled,
}

pub trait PromptSink {
    fn emit(&self, event: PromptLifecycleEvent) -> Pin<Box<dyn std::future::Future<Output = ()>>>;

    fn request_approval(
        &self,
        request: ApprovalRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = ApprovalVerdict>>>;

    /// Return the underlying client channel if this sink is backed by one.
    ///
    /// Used by delegation to forward child approval requests to the parent UI.
    fn parent_client_channel(&self) -> Option<SharedClientChannel> {
        None
    }

    /// Return the parent ACP session id used to route delegation UI events.
    fn parent_session_acp_id(&self) -> Option<acp::SessionId> {
        None
    }
}

pub(crate) struct AcpPromptSink {
    session_id: acp::SessionId,
    client: SharedClientChannel,
}

impl AcpPromptSink {
    pub(crate) fn new(session_id: acp::SessionId, client: SharedClientChannel) -> Self {
        Self { session_id, client }
    }
}

impl PromptSink for AcpPromptSink {
    fn emit(&self, event: PromptLifecycleEvent) -> Pin<Box<dyn std::future::Future<Output = ()>>> {
        match event {
            PromptLifecycleEvent::Output { text } => {
                let notif = notification(
                    &self.session_id,
                    acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                        acp::ContentBlock::Text(acp::TextContent::new(&text)),
                    )),
                );
                let client = self.client.clone();
                Box::pin(async move {
                    let _ = client.send_notification(notif).await;
                })
            }
            PromptLifecycleEvent::ToolCallProposed {
                call_id,
                tool_name,
                arguments,
            } => {
                let notif = notification(
                    &self.session_id,
                    acp::SessionUpdate::ToolCall(
                        acp::ToolCall::new(acp::ToolCallId::new(call_id), &tool_name)
                            .raw_input(arguments)
                            .status(acp::ToolCallStatus::Pending),
                    ),
                );
                let client = self.client.clone();
                Box::pin(async move {
                    let _ = client.send_notification(notif).await;
                })
            }
            PromptLifecycleEvent::ToolCallUpdate {
                call_id,
                tool_name,
                status,
                output,
            } => {
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
                let notif = notification(
                    &self.session_id,
                    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                        acp::ToolCallId::new(call_id),
                        fields,
                    )),
                );
                let client = self.client.clone();
                Box::pin(async move {
                    let _ = client.send_notification(notif).await;
                })
            }
            PromptLifecycleEvent::ScriptActivity {
                script_id,
                parent_call_id,
                activity_type,
                status,
                detail,
            } => {
                let client = self.client.clone();
                Box::pin(async move {
                    let _ = client
                        .emit_script_activity(
                            &script_id,
                            &parent_call_id,
                            &activity_type,
                            &status,
                            detail,
                        )
                        .await;
                })
            }
            PromptLifecycleEvent::CompactionStarted {
                compaction_id,
                method,
            } => {
                let client = self.client.clone();
                Box::pin(async move {
                    let _ = client
                        .emit_compaction_event("started", None, None, &method, None, &compaction_id)
                        .await;
                })
            }
            PromptLifecycleEvent::CompactionFinished {
                compaction_id,
                tokens_before,
                tokens_after,
                method,
            } => {
                let client = self.client.clone();
                Box::pin(async move {
                    let _ = client
                        .emit_compaction_event(
                            "finished",
                            tokens_before,
                            tokens_after,
                            &method,
                            None,
                            &compaction_id,
                        )
                        .await;
                })
            }
            PromptLifecycleEvent::CompactionFailed {
                compaction_id,
                reason,
            } => {
                let client = self.client.clone();
                Box::pin(async move {
                    let _ = client
                        .emit_compaction_event(
                            "failed",
                            None,
                            None,
                            "",
                            Some(&reason),
                            &compaction_id,
                        )
                        .await;
                })
            }
        }
    }

    fn request_approval(
        &self,
        request: ApprovalRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = ApprovalVerdict>>> {
        let tool_call_update = acp::ToolCallUpdate::new(
            acp::ToolCallId::new(request.call_id.clone()),
            acp::ToolCallUpdateFields::new()
                .title(&request.tool_name)
                .raw_input(request.arguments)
                .status(acp::ToolCallStatus::InProgress),
        );

        let perm_request = acp::RequestPermissionRequest::new(
            self.session_id.clone(),
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

        let client = self.client.clone();
        Box::pin(async move {
            match client.request_permission(perm_request).await {
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

    fn parent_client_channel(&self) -> Option<SharedClientChannel> {
        Some(self.client.clone())
    }

    fn parent_session_acp_id(&self) -> Option<acp::SessionId> {
        Some(self.session_id.clone())
    }
}
