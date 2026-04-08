use crate::connection::{notification, SharedClientChannel};
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
}

pub(crate) struct AcpPromptSink {
    session_id: agent_client_protocol::SessionId,
    client: SharedClientChannel,
}

impl AcpPromptSink {
    pub(crate) fn new(
        session_id: agent_client_protocol::SessionId,
        client: SharedClientChannel,
    ) -> Self {
        Self { session_id, client }
    }
}

impl PromptSink for AcpPromptSink {
    fn emit(&self, event: PromptLifecycleEvent) -> Pin<Box<dyn std::future::Future<Output = ()>>> {
        match event {
            PromptLifecycleEvent::Output { text } => {
                let notif = notification(
                    &self.session_id,
                    agent_client_protocol::SessionUpdate::AgentMessageChunk(
                        agent_client_protocol::ContentChunk::new(
                            agent_client_protocol::ContentBlock::Text(
                                agent_client_protocol::TextContent::new(&text),
                            ),
                        ),
                    ),
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
                    agent_client_protocol::SessionUpdate::ToolCall(
                        agent_client_protocol::ToolCall::new(
                            agent_client_protocol::ToolCallId::new(call_id),
                            &tool_name,
                        )
                        .raw_input(arguments)
                        .status(agent_client_protocol::ToolCallStatus::Pending),
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
                    ToolUpdateStatus::InProgress => {
                        agent_client_protocol::ToolCallStatus::InProgress
                    }
                    ToolUpdateStatus::Completed => agent_client_protocol::ToolCallStatus::Completed,
                    ToolUpdateStatus::Failed => agent_client_protocol::ToolCallStatus::Failed,
                };
                let mut fields =
                    agent_client_protocol::ToolCallUpdateFields::new().status(acp_status);
                if !tool_name.is_empty() {
                    fields = fields.title(&tool_name);
                }
                if let Some(out) = output {
                    fields = fields.raw_output(out);
                }
                let notif = notification(
                    &self.session_id,
                    agent_client_protocol::SessionUpdate::ToolCallUpdate(
                        agent_client_protocol::ToolCallUpdate::new(
                            agent_client_protocol::ToolCallId::new(call_id),
                            fields,
                        ),
                    ),
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
        }
    }

    fn request_approval(
        &self,
        request: ApprovalRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = ApprovalVerdict>>> {
        let tool_call_update = agent_client_protocol::ToolCallUpdate::new(
            agent_client_protocol::ToolCallId::new(request.call_id.clone()),
            agent_client_protocol::ToolCallUpdateFields::new()
                .title(&request.tool_name)
                .raw_input(request.arguments)
                .status(agent_client_protocol::ToolCallStatus::InProgress),
        );

        let perm_request = agent_client_protocol::RequestPermissionRequest::new(
            self.session_id.clone(),
            tool_call_update,
            vec![
                agent_client_protocol::PermissionOption::new(
                    agent_client_protocol::PermissionOptionId::new("allow_once"),
                    "Allow once",
                    agent_client_protocol::PermissionOptionKind::AllowOnce,
                ),
                agent_client_protocol::PermissionOption::new(
                    agent_client_protocol::PermissionOptionId::new("reject_once"),
                    "Deny",
                    agent_client_protocol::PermissionOptionKind::RejectOnce,
                ),
            ],
        );

        let client = self.client.clone();
        Box::pin(async move {
            match client.request_permission(perm_request).await {
                Ok(response) => match response.outcome {
                    agent_client_protocol::RequestPermissionOutcome::Cancelled => {
                        ApprovalVerdict::Cancelled
                    }
                    agent_client_protocol::RequestPermissionOutcome::Selected(sel) => {
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
