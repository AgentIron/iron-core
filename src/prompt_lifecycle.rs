//! Provider- and tool-runner lifecycle events delivered to prompt frontends.

use crate::connection::{notification, SharedClientChannel};
use agent_client_protocol::schema::v1 as acp;
use std::pin::Pin;

/// A semantic event emitted while a prompt is running.
///
/// Output and tool events are emitted in execution order. A proposed tool call
/// precedes its updates, and compaction starts before it finishes or fails.
pub enum PromptLifecycleEvent {
    /// Incremental model or transcript-safe tool output.
    Output {
        /// Text chunk to append to client-visible output.
        text: String,
    },
    /// A provider proposed a tool call that has not yet completed.
    ToolCallProposed {
        /// Provider-assigned tool call identifier.
        call_id: String,
        /// Requested tool name.
        tool_name: String,
        /// Requested JSON arguments.
        arguments: serde_json::Value,
    },
    /// A tool call changed execution state.
    ToolCallUpdate {
        /// Provider-assigned tool call identifier.
        call_id: String,
        /// Tool name, or an empty string when unavailable.
        tool_name: String,
        /// New execution status.
        status: ToolUpdateStatus,
        /// Optional result or structured error payload.
        output: Option<serde_json::Value>,
    },
    /// An embedded script or one of its child calls changed state.
    ScriptActivity {
        /// Runtime-assigned script identifier.
        script_id: String,
        /// Tool call that launched the script.
        parent_call_id: String,
        /// Machine-readable activity type.
        activity_type: String,
        /// Machine-readable activity status.
        status: String,
        /// Optional activity-specific data.
        detail: Option<serde_json::Value>,
    },
    /// Context compaction began.
    CompactionStarted {
        /// Correlation identifier for the compaction operation.
        compaction_id: String,
        /// Compaction strategy name.
        method: String,
    },
    /// Context compaction completed successfully.
    CompactionFinished {
        /// Correlation identifier from the start event.
        compaction_id: String,
        /// Estimated token count before compaction, when known.
        tokens_before: Option<u32>,
        /// Estimated token count after compaction, when known.
        tokens_after: Option<u32>,
        /// Compaction strategy used.
        method: String,
    },
    /// Context compaction failed.
    CompactionFailed {
        /// Correlation identifier from the start event.
        compaction_id: String,
        /// Human-readable failure reason.
        reason: String,
    },
}

/// Client-visible execution state for a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolUpdateStatus {
    /// Execution has started but is not terminal.
    InProgress,
    /// Execution completed successfully.
    Completed,
    /// Execution failed, was denied, or was cancelled.
    Failed,
}

/// Information presented to a prompt sink for a tool-approval decision.
pub struct ApprovalRequest {
    /// Provider-assigned tool call identifier.
    pub call_id: String,
    /// Requested tool name.
    pub tool_name: String,
    /// Requested JSON arguments.
    pub arguments: serde_json::Value,
}

/// Decision returned by a prompt sink for one permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalVerdict {
    /// Permit this invocation once.
    AllowOnce,
    /// Deny this invocation while allowing the prompt loop to continue.
    Denied,
    /// Cancel the prompt and remaining tool calls.
    Cancelled,
}

/// Receives ordered prompt events and resolves tool-approval requests.
pub trait PromptSink {
    /// Emits one lifecycle event and resolves after the sink processes it.
    fn emit(&self, event: PromptLifecycleEvent) -> Pin<Box<dyn std::future::Future<Output = ()>>>;

    /// Requests an approval verdict for a proposed tool call.
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

/// Adapts prompt lifecycle events to ACP notifications and permission requests.
pub(crate) struct AcpPromptSink {
    session_id: acp::SessionId,
    client: SharedClientChannel,
}

impl AcpPromptSink {
    /// Creates a sink routing events to `client` under `session_id`.
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
