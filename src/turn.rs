#![allow(deprecated)]
use crate::{
    config::{Config, ContextWindowPolicy},
    error::LoopError,
    events::{PendingCallInfo, TurnEvent, TurnId, TurnOutcome, TurnStatus},
    session::Session,
    session_runtime::SessionRuntime,
    tool::ToolRegistry,
};
use futures::StreamExt;
use iron_providers::{InferenceRequest, Provider, ProviderEvent, ToolCall, ToolPolicy};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};
use tracing::{trace, warn};

enum ApprovalDecision {
    Approved,
    Denied,
}

enum ControlCommand {
    Approve { call_id: String },
    Deny { call_id: String },
    Interrupt,
    Cancel,
}

struct TurnShared {
    status: TurnStatus,
    pending_approvals: HashMap<String, (ToolCall, Option<ApprovalDecision>)>,
}

pub struct TurnHandle {
    turn_id: TurnId,
    control_tx: mpsc::UnboundedSender<ControlCommand>,
    shared: Arc<Mutex<TurnShared>>,
}

impl Clone for TurnHandle {
    fn clone(&self) -> Self {
        Self {
            turn_id: self.turn_id,
            control_tx: self.control_tx.clone(),
            shared: self.shared.clone(),
        }
    }
}

pub struct TurnEvents {
    event_rx: mpsc::UnboundedReceiver<TurnEvent>,
}

struct TurnDriver {
    turn_id: TurnId,
    provider: Arc<dyn Provider>,
    config: Config,
    tools: Arc<ToolRegistry>,
    session: Arc<Mutex<Session>>,
    event_tx: mpsc::UnboundedSender<TurnEvent>,
    control_rx: mpsc::UnboundedReceiver<ControlCommand>,
    shared: Arc<Mutex<TurnShared>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl TurnDriver {
    async fn run(mut self) {
        let mut iteration: u32 = 0;

        loop {
            if iteration >= self.config.max_iterations {
                self.finish(TurnOutcome::MaxIterationsReached { count: iteration });
                return;
            }
            iteration += 1;

            trace!(turn_id = self.turn_id.0, iteration, "Starting iteration");

            let request = match self.prepare_request() {
                Ok(r) => r,
                Err(e) => {
                    self.finish(TurnOutcome::Failed {
                        message: e.to_string(),
                    });
                    return;
                }
            };

            let mut stream = match self.provider.infer_stream(request).await {
                Ok(s) => s,
                Err(e) => {
                    self.finish(TurnOutcome::Failed {
                        message: e.to_string(),
                    });
                    return;
                }
            };

            self.emit(TurnEvent::Status {
                message: "Thinking...".into(),
            });

            let mut tool_calls = Vec::new();
            let mut assistant_output = String::new();
            let mut provider_error: Option<String> = None;

            while let Some(result) = stream.next().await {
                if let Some(outcome) = self.poll_control() {
                    self.commit_output(&mut assistant_output);
                    self.finish(outcome);
                    return;
                }

                let event = match result {
                    Ok(e) => e,
                    Err(e) => {
                        provider_error = Some(e.to_string());
                        break;
                    }
                };

                match event {
                    ProviderEvent::Status { message } => {
                        self.emit(TurnEvent::Status { message });
                    }
                    ProviderEvent::Output { content } => {
                        assistant_output.push_str(&content);
                        self.emit(TurnEvent::OutputDelta { content });
                    }
                    ProviderEvent::ToolCall { call } => {
                        self.emit(TurnEvent::ToolCall {
                            call_id: call.call_id.clone(),
                            tool_name: call.tool_name.clone(),
                            arguments: call.arguments.clone(),
                        });
                        if self.tools.get(&call.tool_name).is_some() {
                            tool_calls.push(call);
                        } else {
                            let err = serde_json::json!({
                                "error": format!("Tool '{}' not found", call.tool_name)
                            });
                            self.emit(TurnEvent::ToolResult {
                                call_id: call.call_id.clone(),
                                tool_name: call.tool_name.clone(),
                                result: err.clone(),
                            });
                            self.session.lock().unwrap().add_tool_result(
                                call.call_id.clone(),
                                call.tool_name.clone(),
                                err,
                            );
                        }
                    }
                    ProviderEvent::Complete => break,
                    ProviderEvent::Error { message } => {
                        provider_error = Some(message);
                        break;
                    }
                }
            }

            self.commit_output(&mut assistant_output);

            if let Some(message) = provider_error {
                self.finish(TurnOutcome::Failed { message });
                return;
            }

            if tool_calls.is_empty() {
                self.finish(TurnOutcome::Completed);
                return;
            }

            {
                let mut session = self.session.lock().unwrap();
                for call in &tool_calls {
                    session.add_tool_call(
                        call.call_id.clone(),
                        call.tool_name.clone(),
                        call.arguments.clone(),
                    );
                }
            }

            let (auto_approved, needs_approval) = self.partition_by_approval(tool_calls);

            for call in auto_approved {
                self.execute_tool(call).await;
                if let Some(outcome) = self.poll_control() {
                    self.finish(outcome);
                    return;
                }
            }

            if needs_approval.is_empty() {
                continue;
            }

            let pending_info: Vec<PendingCallInfo> = needs_approval
                .iter()
                .map(|call| PendingCallInfo {
                    call_id: call.call_id.clone(),
                    tool_name: call.tool_name.clone(),
                    arguments: call.arguments.clone(),
                })
                .collect();

            {
                let mut shared = self.shared.lock().unwrap();
                for call in &needs_approval {
                    shared
                        .pending_approvals
                        .insert(call.call_id.clone(), (call.clone(), None));
                }
                shared.status = TurnStatus::WaitingForApproval {
                    pending: pending_info.clone(),
                };
            }

            for info in &pending_info {
                self.emit(TurnEvent::ApprovalRequired {
                    call_id: info.call_id.clone(),
                    tool_name: info.tool_name.clone(),
                    arguments: info.arguments.clone(),
                });
            }

            match self.wait_for_approvals().await {
                Some(outcome) => {
                    self.finish(outcome);
                    return;
                }
                None => {
                    let decisions: Vec<(String, (ToolCall, Option<ApprovalDecision>))> = {
                        let mut shared = self.shared.lock().unwrap();
                        shared.pending_approvals.drain().collect()
                    };

                    for (call_id, (call, decision)) in decisions {
                        match decision {
                            Some(ApprovalDecision::Approved) => {
                                self.execute_tool(call).await;
                            }
                            Some(ApprovalDecision::Denied) | None => {
                                let result =
                                    serde_json::json!({"error": "Tool execution denied by user"});
                                self.emit(TurnEvent::ToolResult {
                                    call_id: call.call_id.clone(),
                                    tool_name: call.tool_name.clone(),
                                    result: result.clone(),
                                });
                                self.session.lock().unwrap().add_tool_result(
                                    call.call_id,
                                    call.tool_name,
                                    result,
                                );
                            }
                        }
                        let _ = call_id;
                    }

                    {
                        let mut shared = self.shared.lock().unwrap();
                        shared.status = TurnStatus::Running;
                    }

                    continue;
                }
            }
        }
    }

    fn emit(&self, event: TurnEvent) {
        let _ = self.event_tx.send(event);
    }

    fn finish(&self, outcome: TurnOutcome) {
        {
            let mut shared = self.shared.lock().unwrap();
            shared.status = TurnStatus::Finished {
                outcome: outcome.clone(),
            };
        }
        self.emit(TurnEvent::Finished { outcome });
    }

    fn commit_output(&self, output: &mut String) {
        if !output.is_empty() {
            self.session
                .lock()
                .unwrap()
                .add_assistant_message(output.clone());
            output.clear();
        }
    }

    fn poll_control(&mut self) -> Option<TurnOutcome> {
        if *self.shutdown_rx.borrow() {
            return Some(TurnOutcome::Cancelled);
        }
        while let Ok(cmd) = self.control_rx.try_recv() {
            match cmd {
                ControlCommand::Interrupt => return Some(TurnOutcome::Interrupted),
                ControlCommand::Cancel => return Some(TurnOutcome::Cancelled),
                ControlCommand::Approve { .. } | ControlCommand::Deny { .. } => {
                    warn!("Received approval command during streaming phase, ignoring");
                }
            }
        }
        None
    }

    async fn wait_for_approvals(&mut self) -> Option<TurnOutcome> {
        loop {
            {
                let shared = self.shared.lock().unwrap();
                if shared.pending_approvals.values().all(|(_, d)| d.is_some()) {
                    return None;
                }
            }

            if *self.shutdown_rx.borrow() {
                return Some(TurnOutcome::Cancelled);
            }

            match self.control_rx.recv().await {
                Some(ControlCommand::Approve { call_id }) => {
                    let all_decided = {
                        let mut shared = self.shared.lock().unwrap();
                        if let Some(entry) = shared.pending_approvals.get_mut(&call_id) {
                            entry.1 = Some(ApprovalDecision::Approved);
                        }
                        update_pending_status_locked(&mut shared);
                        shared.pending_approvals.values().all(|(_, d)| d.is_some())
                    };
                    if all_decided {
                        return None;
                    }
                }
                Some(ControlCommand::Deny { call_id }) => {
                    let all_decided = {
                        let mut shared = self.shared.lock().unwrap();
                        if let Some(entry) = shared.pending_approvals.get_mut(&call_id) {
                            entry.1 = Some(ApprovalDecision::Denied);
                        }
                        update_pending_status_locked(&mut shared);
                        shared.pending_approvals.values().all(|(_, d)| d.is_some())
                    };
                    if all_decided {
                        return None;
                    }
                }
                Some(ControlCommand::Interrupt) => return Some(TurnOutcome::Interrupted),
                Some(ControlCommand::Cancel) => return Some(TurnOutcome::Cancelled),
                None => return Some(TurnOutcome::Interrupted),
            }
        }
    }

    fn prepare_request(&self) -> Result<InferenceRequest, LoopError> {
        let session = self.session.lock().unwrap();
        let messages = apply_context_policy(&self.config, &session.messages)?;
        let transcript = iron_providers::Transcript::with_messages(messages);

        let tool_policy = if self.tools.is_empty() {
            ToolPolicy::None
        } else {
            self.config.default_tool_policy.clone()
        };

        let mut request = InferenceRequest::new(self.config.model.clone(), transcript)
            .with_tools(self.tools.provider_definitions())
            .with_tool_policy(tool_policy)
            .with_generation(self.config.default_generation.clone());

        if let Some(ref instructions) = session.instructions {
            request = request.with_instructions(instructions.clone());
        }

        Ok(request)
    }

    fn partition_by_approval(&self, calls: Vec<ToolCall>) -> (Vec<ToolCall>, Vec<ToolCall>) {
        let mut auto = Vec::new();
        let mut needs = Vec::new();

        for call in calls {
            let requires = self
                .tools
                .get(&call.tool_name)
                .map(|t| {
                    self.config
                        .default_approval_strategy
                        .is_approval_required(t.requires_approval())
                })
                .unwrap_or(false);

            if requires {
                needs.push(call);
            } else {
                auto.push(call);
            }
        }

        (auto, needs)
    }

    async fn execute_tool(&self, call: ToolCall) {
        let Some(tool) = self.tools.get(&call.tool_name) else {
            let err = serde_json::json!({"error": format!("Tool '{}' not found", call.tool_name)});
            self.emit(TurnEvent::ToolResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                result: err.clone(),
            });
            self.session
                .lock()
                .unwrap()
                .add_tool_result(call.call_id, call.tool_name, err);
            return;
        };

        match tool.execute(&call.call_id, call.arguments.clone()).await {
            Ok(result) => {
                self.emit(TurnEvent::ToolResult {
                    call_id: call.call_id.clone(),
                    tool_name: call.tool_name.clone(),
                    result: result.clone(),
                });
                self.session
                    .lock()
                    .unwrap()
                    .add_tool_result(call.call_id, call.tool_name, result);
            }
            Err(error) => {
                let result = serde_json::json!({"error": error.to_string()});
                self.emit(TurnEvent::ToolResult {
                    call_id: call.call_id.clone(),
                    tool_name: call.tool_name.clone(),
                    result: result.clone(),
                });
                self.session
                    .lock()
                    .unwrap()
                    .add_tool_result(call.call_id, call.tool_name, result);
            }
        }
    }
}

fn apply_context_policy(
    config: &Config,
    messages: &[iron_providers::Message],
) -> Result<Vec<iron_providers::Message>, LoopError> {
    match config.context_window_policy {
        ContextWindowPolicy::KeepAll => Ok(messages.to_vec()),
        ContextWindowPolicy::KeepRecent(count) => {
            if messages.len() <= count {
                Ok(messages.to_vec())
            } else {
                Ok(messages[messages.len() - count..].to_vec())
            }
        }
        ContextWindowPolicy::SummarizeAfter(_) => Err(LoopError::invalid_config(
            "ContextWindowPolicy::SummarizeAfter is not implemented",
        )),
    }
}

fn update_pending_status_locked(shared: &mut TurnShared) {
    let still_pending: Vec<PendingCallInfo> = shared
        .pending_approvals
        .iter()
        .filter(|(_, (_, d))| d.is_none())
        .map(|(_, (call, _))| PendingCallInfo {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            arguments: call.arguments.clone(),
        })
        .collect();

    if !still_pending.is_empty() {
        shared.status = TurnStatus::WaitingForApproval {
            pending: still_pending,
        };
    }
}

pub(crate) fn create_turn(
    turn_id: TurnId,
    provider: Arc<dyn Provider>,
    config: Config,
    tools: Arc<ToolRegistry>,
    session: Arc<Mutex<Session>>,
    runtime: &SessionRuntime,
) -> Result<(TurnHandle, TurnEvents), LoopError> {
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (control_tx, control_rx) = mpsc::unbounded_channel();
    let shared = Arc::new(Mutex::new(TurnShared {
        status: TurnStatus::Running,
        pending_approvals: HashMap::new(),
    }));

    let driver = TurnDriver {
        turn_id,
        provider,
        config,
        tools,
        session,
        event_tx,
        control_rx,
        shared: shared.clone(),
        shutdown_rx: runtime.shutdown_token(),
    };

    if !runtime.spawn(async move {
        driver.run().await;
    }) {
        return Err(LoopError::RuntimeShutdown);
    }

    let handle = TurnHandle {
        turn_id,
        control_tx,
        shared,
    };

    let events = TurnEvents { event_rx };

    Ok((handle, events))
}

impl TurnHandle {
    pub fn id(&self) -> TurnId {
        self.turn_id
    }

    pub fn status(&self) -> TurnStatus {
        self.shared.lock().unwrap().status.clone()
    }

    pub fn approve(&self, call_id: impl Into<String>) -> Result<(), LoopError> {
        let call_id = call_id.into();
        let shared = self.shared.lock().unwrap();
        match &shared.status {
            TurnStatus::Finished { .. } => Err(LoopError::TurnFinished),
            TurnStatus::Running => Err(LoopError::NotWaitingForApproval),
            TurnStatus::WaitingForApproval { .. } => {
                if !shared.pending_approvals.contains_key(&call_id) {
                    return Err(LoopError::ApprovalNotFound { call_id });
                }
                drop(shared);
                self.control_tx
                    .send(ControlCommand::Approve { call_id })
                    .map_err(|_| LoopError::TurnFinished)
            }
        }
    }

    pub fn deny(&self, call_id: impl Into<String>) -> Result<(), LoopError> {
        let call_id = call_id.into();
        let shared = self.shared.lock().unwrap();
        match &shared.status {
            TurnStatus::Finished { .. } => Err(LoopError::TurnFinished),
            TurnStatus::Running => Err(LoopError::NotWaitingForApproval),
            TurnStatus::WaitingForApproval { .. } => {
                if !shared.pending_approvals.contains_key(&call_id) {
                    return Err(LoopError::ApprovalNotFound { call_id });
                }
                drop(shared);
                self.control_tx
                    .send(ControlCommand::Deny { call_id })
                    .map_err(|_| LoopError::TurnFinished)
            }
        }
    }

    pub fn interrupt(&self) -> Result<(), LoopError> {
        let shared = self.shared.lock().unwrap();
        if let TurnStatus::Finished { .. } = &shared.status {
            return Err(LoopError::TurnFinished);
        }
        drop(shared);
        self.control_tx
            .send(ControlCommand::Interrupt)
            .map_err(|_| LoopError::TurnFinished)
    }

    pub fn cancel(&self) -> Result<(), LoopError> {
        let shared = self.shared.lock().unwrap();
        if let TurnStatus::Finished { .. } = &shared.status {
            return Err(LoopError::TurnFinished);
        }
        drop(shared);
        self.control_tx
            .send(ControlCommand::Cancel)
            .map_err(|_| LoopError::TurnFinished)
    }
}

impl std::fmt::Debug for TurnHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnHandle")
            .field("turn_id", &self.turn_id)
            .finish()
    }
}

impl TurnEvents {
    pub async fn next_event(&mut self) -> Option<TurnEvent> {
        self.event_rx.recv().await
    }
}

impl std::fmt::Debug for TurnEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnEvents").finish()
    }
}
