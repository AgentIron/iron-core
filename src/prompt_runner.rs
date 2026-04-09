use crate::config::Config;
use crate::context::compaction::{CompactionEngine, CompactionReason};
use crate::durable::DurableSession;
use crate::ephemeral::EphemeralTurn;
use crate::prompt_lifecycle::{
    ApprovalRequest, ApprovalVerdict, PromptLifecycleEvent, PromptSink, ToolUpdateStatus,
};
use crate::runtime::IronRuntime;
use futures::StreamExt;
use iron_providers::ProviderEvent;
use std::sync::Arc;
use tracing::{info, trace, warn};

const MAX_TOOL_RESULT_SIZE: usize = 10 * 1024 * 1024;

fn limit_result_size(result: serde_json::Value) -> serde_json::Value {
    let size_estimate = serde_json::to_string(&result).map(|s| s.len()).unwrap_or(0);

    if size_estimate > MAX_TOOL_RESULT_SIZE {
        serde_json::json!({
            "error": format!("Tool result exceeded maximum size of {} bytes", MAX_TOOL_RESULT_SIZE),
            "actual_size": size_estimate,
            "truncated": true
        })
    } else {
        result
    }
}

pub(crate) struct PromptRunner {
    runtime: IronRuntime,
}

impl PromptRunner {
    pub(crate) fn new(runtime: IronRuntime) -> Self {
        Self { runtime }
    }

    pub(crate) async fn run(
        &self,
        durable: &Arc<std::sync::Mutex<DurableSession>>,
        ephemeral: &Arc<std::sync::Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        config: &Config,
        max_iterations: u32,
    ) -> agent_client_protocol::StopReason {
        let mut iteration: u32 = 0;

        loop {
            {
                let turn = ephemeral.lock().unwrap();
                if turn.is_cancel_requested() {
                    return agent_client_protocol::StopReason::Cancelled;
                }
            }

            iteration += 1;
            if iteration > max_iterations {
                return agent_client_protocol::StopReason::MaxTurnRequests;
            }

            trace!(iteration, "Starting inference iteration");

            if config.context_management.enabled {
                self.maybe_compact_hard_fit(durable, config).await;
            }

            let request = {
                let session = durable.lock().unwrap();
                let instructions = session.instructions.clone();
                let compacted_context = session.compacted_context.clone();
                let repo_payload = session.repo_instruction_payload.clone();
                let messages = session.to_transcript().messages;
                let tool_registry = self.runtime.tool_registry();
                crate::request_builder::build_inference_request_with_context_and_repo(
                    config,
                    &messages,
                    compacted_context.as_ref(),
                    instructions.as_deref(),
                    repo_payload.as_ref(),
                    &tool_registry,
                )
            };

            let request = match request {
                Ok(req) => req,
                Err(e) => {
                    warn!(error = %e, "Request building failed");
                    {
                        let mut session = durable.lock().unwrap();
                        session.add_agent_text(format!("[Request error: {}]", e));
                    }
                    return agent_client_protocol::StopReason::EndTurn;
                }
            };

            let stream = match self.runtime.provider().infer_stream(request).await {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "Provider inference failed");
                    {
                        let mut session = durable.lock().unwrap();
                        session.add_agent_text(format!("[Provider error: {}]", e));
                    }
                    return agent_client_protocol::StopReason::EndTurn;
                }
            };

            let step = match self.process_provider_stream(durable, sink, stream).await {
                Ok(s) => s,
                Err(e) => return e,
            };

            if step.tool_calls.is_empty() {
                return agent_client_protocol::StopReason::EndTurn;
            }

            let cancel_check = || {
                let turn = ephemeral.lock().unwrap();
                turn.is_cancel_requested()
            };

            if cancel_check() {
                return agent_client_protocol::StopReason::Cancelled;
            }

            let needs_permission = {
                let tool_registry = self.runtime.tool_registry();
                step.tool_calls.iter().any(|call| {
                    tool_registry
                        .get(&call.tool_name)
                        .map(|tool| {
                            config
                                .default_approval_strategy
                                .is_approval_required(tool.requires_approval())
                        })
                        .unwrap_or(false)
                })
            };

            let cancel_token = {
                let turn = ephemeral.lock().unwrap();
                turn.cancel_token()
            };

            if needs_permission {
                let approved_tool_calls = match self
                    .handle_permission_flow(durable, ephemeral, sink, &step.tool_calls, config)
                    .await
                {
                    Ok(calls) => calls,
                    Err(reason) => return reason,
                };

                self.execute_tool_calls(
                    durable,
                    ephemeral,
                    sink,
                    approved_tool_calls,
                    cancel_token,
                )
                .await;
            } else {
                self.execute_tool_calls(durable, ephemeral, sink, step.tool_calls, cancel_token)
                    .await;
            }
        }
    }

    async fn process_provider_stream(
        &self,
        durable: &Arc<std::sync::Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        mut stream: futures::stream::BoxStream<
            'static,
            iron_providers::ProviderResult<ProviderEvent>,
        >,
    ) -> Result<ProviderStep, agent_client_protocol::StopReason> {
        let mut tool_calls = Vec::new();
        let mut assistant_output = String::new();

        while let Some(result) = stream.next().await {
            let event = match result {
                Ok(e) => e,
                Err(_) => {
                    if !assistant_output.is_empty() {
                        let mut session = durable.lock().unwrap();
                        session.add_agent_text(&assistant_output);
                    }
                    return Err(agent_client_protocol::StopReason::EndTurn);
                }
            };

            match event {
                ProviderEvent::Output { content } => {
                    assistant_output.push_str(&content);
                    sink.emit(PromptLifecycleEvent::Output { text: content })
                        .await;
                }
                ProviderEvent::ToolCall { call } => {
                    {
                        let mut session = durable.lock().unwrap();
                        session.propose_tool_call(
                            &call.call_id,
                            &call.tool_name,
                            call.arguments.clone(),
                        );
                    }
                    sink.emit(PromptLifecycleEvent::ToolCallProposed {
                        call_id: call.call_id.clone(),
                        tool_name: call.tool_name.clone(),
                        arguments: call.arguments.clone(),
                    })
                    .await;
                    tool_calls.push(call);
                }
                ProviderEvent::Status { message } => {
                    trace!(%message, "Provider status");
                }
                ProviderEvent::Complete => {}
                ProviderEvent::Error { message: _ } => {
                    if !assistant_output.is_empty() {
                        let mut session = durable.lock().unwrap();
                        session.add_agent_text(&assistant_output);
                    }
                    return Err(agent_client_protocol::StopReason::EndTurn);
                }
            }
        }

        if !assistant_output.is_empty() {
            let mut session = durable.lock().unwrap();
            session.add_agent_text(&assistant_output);
        }

        Ok(ProviderStep { tool_calls })
    }

    async fn handle_permission_flow(
        &self,
        durable: &Arc<std::sync::Mutex<DurableSession>>,
        ephemeral: &Arc<std::sync::Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        tool_calls: &[iron_providers::ToolCall],
        config: &Config,
    ) -> Result<Vec<iron_providers::ToolCall>, agent_client_protocol::StopReason> {
        let mut approved = Vec::new();

        for call in tool_calls {
            let requires = {
                let tool_registry = self.runtime.tool_registry();
                tool_registry
                    .get(&call.tool_name)
                    .map(|t| {
                        config
                            .default_approval_strategy
                            .is_approval_required(t.requires_approval())
                    })
                    .unwrap_or(false)
            };

            if !requires {
                approved.push(call.clone());
                continue;
            }

            let verdict = self
                .request_tool_permission(
                    ephemeral,
                    sink,
                    &call.call_id,
                    &call.tool_name,
                    &call.arguments,
                )
                .await;

            match verdict {
                ApprovalVerdict::Cancelled => {
                    {
                        let mut session = durable.lock().unwrap();
                        session.cancel_tool_call(&call.call_id);
                    }
                    sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                        call_id: call.call_id.clone(),
                        tool_name: call.tool_name.clone(),
                        status: ToolUpdateStatus::Failed,
                        output: Some(serde_json::json!({"error": "cancelled"})),
                    })
                    .await;
                    for remaining in tool_calls
                        .iter()
                        .skip_while(|c| c.call_id != call.call_id)
                        .skip(1)
                    {
                        {
                            let mut session = durable.lock().unwrap();
                            session.cancel_tool_call(&remaining.call_id);
                        }
                        sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                            call_id: remaining.call_id.clone(),
                            tool_name: remaining.tool_name.clone(),
                            status: ToolUpdateStatus::Failed,
                            output: Some(serde_json::json!({"error": "cancelled"})),
                        })
                        .await;
                    }
                    return Err(agent_client_protocol::StopReason::Cancelled);
                }
                ApprovalVerdict::AllowOnce => {
                    approved.push(call.clone());
                }
                ApprovalVerdict::Denied => {
                    let error_result = serde_json::json!({"error": "denied by user"});
                    {
                        let mut session = durable.lock().unwrap();
                        session.deny_tool_call(&call.call_id);
                    }
                    sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                        call_id: call.call_id.clone(),
                        tool_name: call.tool_name.clone(),
                        status: ToolUpdateStatus::Failed,
                        output: Some(error_result),
                    })
                    .await;
                }
            }
        }

        Ok(approved)
    }

    async fn request_tool_permission(
        &self,
        ephemeral: &Arc<std::sync::Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        call_id: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> ApprovalVerdict {
        {
            let mut turn = ephemeral.lock().unwrap();
            turn.request_permission(
                call_id.to_string(),
                tool_name.to_string(),
                arguments.clone(),
            );
        }

        let verdict = sink
            .request_approval(ApprovalRequest {
                call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                arguments: arguments.clone(),
            })
            .await;

        {
            let mut turn = ephemeral.lock().unwrap();
            turn.resolve_permission(call_id);
        }

        verdict
    }

    async fn execute_tool_calls(
        &self,
        durable: &Arc<std::sync::Mutex<DurableSession>>,
        ephemeral: &Arc<std::sync::Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        tool_calls: Vec<iron_providers::ToolCall>,
        cancel_token: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        use std::sync::atomic::Ordering;

        let mut calls = tool_calls.into_iter().peekable();

        while let Some(call) = calls.next() {
            if cancel_token.load(Ordering::SeqCst) {
                self.cancel_remaining_tool_calls(durable, sink, call, &mut calls)
                    .await;
                return;
            }

            self.execute_single_tool(durable, ephemeral, sink, call, cancel_token.clone())
                .await;
        }
    }

    async fn validate_and_prepare(
        &self,
        durable: &Arc<std::sync::Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        call: &iron_providers::ToolCall,
    ) -> bool {
        let tool_def = {
            let tool_registry = self.runtime.tool_registry();
            tool_registry
                .get(&call.tool_name)
                .map(|tool| tool.definition())
        };

        let Some(definition) = tool_def else {
            let error_result =
                serde_json::json!({"error": format!("Tool '{}' not found", call.tool_name)});
            {
                let mut session = durable.lock().unwrap();
                session.start_tool_call(&call.call_id, &call.tool_name, call.arguments.clone());
                session.fail_tool_call(&call.call_id, error_result.clone());
            }
            sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                status: ToolUpdateStatus::Failed,
                output: Some(error_result),
            })
            .await;
            return false;
        };

        {
            let mut session = durable.lock().unwrap();
            session.start_tool_call(&call.call_id, &call.tool_name, call.arguments.clone());
        }

        match crate::schema::validate_arguments(&definition.input_schema, &call.arguments) {
            crate::schema::SchemaValidationOutcome::Valid => {}
            crate::schema::SchemaValidationOutcome::Invalid { errors } => {
                let error_detail = errors.join("; ");
                let error_result = serde_json::json!({
                    "error": format!("schema validation failed: {}", error_detail),
                    "validation_errors": errors,
                });
                {
                    let mut session = durable.lock().unwrap();
                    session.fail_tool_call(&call.call_id, error_result.clone());
                }
                sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                    call_id: call.call_id.clone(),
                    tool_name: call.tool_name.clone(),
                    status: ToolUpdateStatus::Failed,
                    output: Some(error_result),
                })
                .await;
                return false;
            }
            crate::schema::SchemaValidationOutcome::BadSchema { error } => {
                let error_result = serde_json::json!({
                    "error": format!("invalid tool schema: {}", error),
                });
                {
                    let mut session = durable.lock().unwrap();
                    session.fail_tool_call(&call.call_id, error_result.clone());
                }
                sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                    call_id: call.call_id.clone(),
                    tool_name: call.tool_name.clone(),
                    status: ToolUpdateStatus::Failed,
                    output: Some(error_result),
                })
                .await;
                return false;
            }
        }

        sink.emit(PromptLifecycleEvent::ToolCallUpdate {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            status: ToolUpdateStatus::InProgress,
            output: None,
        })
        .await;

        true
    }

    async fn execute_single_tool(
        &self,
        durable: &Arc<std::sync::Mutex<DurableSession>>,
        _ephemeral: &Arc<std::sync::Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        call: iron_providers::ToolCall,
        cancel_token: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        #[cfg(not(feature = "embedded-python"))]
        let _ = &cancel_token;

        if !self.validate_and_prepare(durable, sink, &call).await {
            return;
        }

        #[cfg(feature = "embedded-python")]
        if call.tool_name == "python_exec" && self.runtime.config().embedded_python.enabled {
            self.execute_python_script(durable, _ephemeral, sink, &call, cancel_token)
                .await;
            return;
        }

        self.execute_standard_tool(durable, sink, call).await;
    }

    async fn execute_standard_tool(
        &self,
        durable: &Arc<std::sync::Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        call: iron_providers::ToolCall,
    ) {
        let call_id = call.call_id.clone();
        let tool_name = call.tool_name.clone();

        let execute_future = {
            let tool_registry = self.runtime.tool_registry();
            match tool_registry.get(&call.tool_name) {
                Some(tool) => tool.execute(&call.call_id, call.arguments.clone()),
                None => Box::pin(async move {
                    Err(crate::error::LoopError::tool_execution(format!(
                        "Tool '{}' no longer available",
                        call.tool_name
                    )))
                }),
            }
        };

        let execute_result = execute_future.await;

        match execute_result {
            Ok(result) => {
                let limited_result = limit_result_size(result);
                {
                    let mut session = durable.lock().unwrap();
                    session.complete_tool_call(&call_id, limited_result.clone());
                }
                sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                    call_id: call_id.clone(),
                    tool_name: tool_name.clone(),
                    status: ToolUpdateStatus::Completed,
                    output: Some(limited_result),
                })
                .await;
            }
            Err(error) => {
                let result = serde_json::json!({"error": error.to_string()});
                {
                    let mut session = durable.lock().unwrap();
                    session.fail_tool_call(&call_id, result.clone());
                }
                sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                    call_id: call_id.clone(),
                    tool_name: tool_name.clone(),
                    status: ToolUpdateStatus::Failed,
                    output: Some(result),
                })
                .await;
            }
        }
    }

    async fn cancel_remaining_tool_calls(
        &self,
        durable: &Arc<std::sync::Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        first: iron_providers::ToolCall,
        rest: &mut std::iter::Peekable<std::vec::IntoIter<iron_providers::ToolCall>>,
    ) {
        for call in std::iter::once(first).chain(rest) {
            {
                let mut session = durable.lock().unwrap();
                session.cancel_tool_call(&call.call_id);
            }
            sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                status: ToolUpdateStatus::Failed,
                output: Some(serde_json::json!({"error": "cancelled"})),
            })
            .await;
        }
    }

    #[cfg(feature = "embedded-python")]
    async fn execute_python_script(
        &self,
        durable: &Arc<std::sync::Mutex<DurableSession>>,
        ephemeral: &Arc<std::sync::Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        call: &iron_providers::ToolCall,
        cancel_token: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        use crate::embedded_python::{
            script_output_to_json, ChildCallStatus, ScriptError, ScriptExecStatus, ScriptInput,
            ScriptOutput, ToolExecutorFn,
        };
        use std::sync::atomic::Ordering;

        let script = match call.arguments.get("script").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                let error_result = serde_json::json!({"error": "missing 'script' argument"});
                {
                    let mut session = durable.lock().unwrap();
                    session.fail_tool_call(&call.call_id, error_result.clone());
                }
                sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                    call_id: call.call_id.clone(),
                    tool_name: call.tool_name.clone(),
                    status: ToolUpdateStatus::Failed,
                    output: Some(error_result),
                })
                .await;
                return;
            }
        };
        let input = call
            .arguments
            .get("input")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let config = self.runtime.config().embedded_python.clone();
        let script_id = uuid::Uuid::new_v4().to_string();

        {
            let mut session = durable.lock().unwrap();
            session.record_script_start(&script_id, &call.call_id, &script, Some(input.clone()));
        }

        sink.emit(PromptLifecycleEvent::ScriptActivity {
            script_id: script_id.clone(),
            parent_call_id: call.call_id.clone(),
            activity_type: "script_started".to_string(),
            status: "running".to_string(),
            detail: None,
        })
        .await;

        struct ChildReq {
            call_id: String,
            tool_name: String,
            args: serde_json::Value,
            response_tx: std::sync::mpsc::Sender<(ChildCallStatus, Option<serde_json::Value>)>,
        }

        let (req_tx, req_rx) = std::sync::mpsc::channel::<ChildReq>();
        let timeout_secs = config.max_script_timeout_secs;
        let tool_catalog = {
            let tool_registry = self.runtime.tool_registry();
            crate::embedded_python::ToolCatalog::from_registry(&tool_registry)
        };

        let executor: std::sync::Arc<ToolExecutorFn> = std::sync::Arc::new({
            let req_tx = req_tx.clone();
            move |call_id: &str, name: &str, args: serde_json::Value| {
                let (resp_tx, resp_rx) = std::sync::mpsc::channel();
                if req_tx
                    .send(ChildReq {
                        call_id: call_id.to_string(),
                        tool_name: name.to_string(),
                        args,
                        response_tx: resp_tx,
                    })
                    .is_err()
                {
                    return (
                        ChildCallStatus::Failed,
                        Some(serde_json::json!({"error": "script execution channel closed"})),
                    );
                }
                match resp_rx.recv_timeout(std::time::Duration::from_secs(timeout_secs)) {
                    Ok(result) => result,
                    Err(_) => (
                        ChildCallStatus::Failed,
                        Some(serde_json::json!({"error": "child tool call timed out"})),
                    ),
                }
            }
        });

        let script_input = ScriptInput { script, input };
        let run =
            crate::embedded_python::ScriptRun::new(script_input, &config, cancel_token.clone())
                .with_tool_catalog(tool_catalog)
                .with_tool_executor(executor);

        let handle = std::thread::spawn(move || run.execute());

        loop {
            match req_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(req) => {
                    sink.emit(PromptLifecycleEvent::ScriptActivity {
                        script_id: script_id.clone(),
                        parent_call_id: call.call_id.clone(),
                        activity_type: "child_tool_call_started".to_string(),
                        status: "running".to_string(),
                        detail: Some(serde_json::json!({
                            "call_id": req.call_id,
                            "tool_name": req.tool_name,
                        })),
                    })
                    .await;

                    let tool_def = {
                        let tool_registry = self.runtime.tool_registry();
                        tool_registry
                            .get(&req.tool_name)
                            .map(|tool| tool.definition())
                    };

                    let Some(definition) = tool_def else {
                        let error_result = serde_json::json!({"error": format!("Tool '{}' not found", req.tool_name)});
                        {
                            let mut session = durable.lock().unwrap();
                            session.start_tool_call(&req.call_id, &req.tool_name, req.args.clone());
                            session.fail_tool_call(&req.call_id, error_result.clone());
                            session.link_child_to_script(&script_id, &req.call_id);
                        }
                        sink.emit(PromptLifecycleEvent::ScriptActivity {
                            script_id: script_id.clone(),
                            parent_call_id: call.call_id.clone(),
                            activity_type: "child_tool_call_failed".to_string(),
                            status: "failed".to_string(),
                            detail: Some(serde_json::json!({
                                "call_id": req.call_id,
                                "tool_name": req.tool_name,
                            })),
                        })
                        .await;
                        let _ = req
                            .response_tx
                            .send((ChildCallStatus::Failed, Some(error_result)));
                        continue;
                    };

                    let validation =
                        crate::schema::validate_arguments(&definition.input_schema, &req.args);
                    match validation {
                        crate::schema::SchemaValidationOutcome::Valid => {}
                        crate::schema::SchemaValidationOutcome::Invalid { errors } => {
                            let error_result = serde_json::json!({
                                "error": format!("schema validation failed: {}", errors.join("; ")),
                                "validation_errors": errors,
                            });
                            {
                                let mut session = durable.lock().unwrap();
                                session.start_tool_call(
                                    &req.call_id,
                                    &req.tool_name,
                                    req.args.clone(),
                                );
                                session.fail_tool_call(&req.call_id, error_result.clone());
                                session.link_child_to_script(&script_id, &req.call_id);
                            }
                            let _ = req
                                .response_tx
                                .send((ChildCallStatus::Failed, Some(error_result)));
                            continue;
                        }
                        crate::schema::SchemaValidationOutcome::BadSchema { error } => {
                            let error_result = serde_json::json!({"error": format!("invalid tool schema: {}", error)});
                            {
                                let mut session = durable.lock().unwrap();
                                session.start_tool_call(
                                    &req.call_id,
                                    &req.tool_name,
                                    req.args.clone(),
                                );
                                session.fail_tool_call(&req.call_id, error_result.clone());
                                session.link_child_to_script(&script_id, &req.call_id);
                            }
                            let _ = req
                                .response_tx
                                .send((ChildCallStatus::Failed, Some(error_result)));
                            continue;
                        }
                    }

                    let requires_permission = {
                        let tool_registry = self.runtime.tool_registry();
                        tool_registry
                            .get(&req.tool_name)
                            .map(|tool| {
                                self.runtime
                                    .config()
                                    .default_approval_strategy
                                    .is_approval_required(tool.requires_approval())
                            })
                            .unwrap_or(false)
                    };
                    if requires_permission {
                        {
                            let mut session = durable.lock().unwrap();
                            session.propose_tool_call(
                                &req.call_id,
                                &req.tool_name,
                                req.args.clone(),
                            );
                            session.link_child_to_script(&script_id, &req.call_id);
                        }

                        match self
                            .request_tool_permission(
                                ephemeral,
                                sink,
                                &req.call_id,
                                &req.tool_name,
                                &req.args,
                            )
                            .await
                        {
                            ApprovalVerdict::AllowOnce => {}
                            ApprovalVerdict::Denied => {
                                {
                                    let mut session = durable.lock().unwrap();
                                    session.deny_tool_call(&req.call_id);
                                }
                                sink.emit(PromptLifecycleEvent::ScriptActivity {
                                    script_id: script_id.clone(),
                                    parent_call_id: call.call_id.clone(),
                                    activity_type: "child_tool_call_failed".to_string(),
                                    status: "denied".to_string(),
                                    detail: Some(serde_json::json!({
                                        "call_id": req.call_id,
                                        "tool_name": req.tool_name,
                                    })),
                                })
                                .await;
                                let _ = req.response_tx.send((ChildCallStatus::Denied, None));
                                continue;
                            }
                            ApprovalVerdict::Cancelled => {
                                {
                                    let mut session = durable.lock().unwrap();
                                    session.cancel_tool_call(&req.call_id);
                                }
                                cancel_token.store(true, Ordering::SeqCst);
                                sink.emit(PromptLifecycleEvent::ScriptActivity {
                                    script_id: script_id.clone(),
                                    parent_call_id: call.call_id.clone(),
                                    activity_type: "child_tool_call_failed".to_string(),
                                    status: "cancelled".to_string(),
                                    detail: Some(serde_json::json!({
                                        "call_id": req.call_id,
                                        "tool_name": req.tool_name,
                                    })),
                                })
                                .await;
                                let _ = req.response_tx.send((ChildCallStatus::Cancelled, None));
                                continue;
                            }
                        }
                    }

                    {
                        let mut session = durable.lock().unwrap();
                        session.start_tool_call(&req.call_id, &req.tool_name, req.args.clone());
                        session.link_child_to_script(&script_id, &req.call_id);
                    }

                    let req_call_id = req.call_id.clone();
                    let req_tool_name = req.tool_name.clone();
                    let req_tool_name_for_err = req_tool_name.clone();

                    let execute_future = {
                        let tool_registry = self.runtime.tool_registry();
                        match tool_registry.get(&req.tool_name) {
                            Some(tool) => tool.execute(&req.call_id, req.args.clone()),
                            None => Box::pin(async move {
                                Err(crate::error::LoopError::tool_execution(format!(
                                    "Tool '{}' no longer available",
                                    req_tool_name_for_err
                                )))
                            }) as crate::tool::ToolFuture,
                        }
                    };

                    let (status, result) = match execute_future.await {
                        Ok(result) => {
                            let limited = limit_result_size(result);
                            {
                                let mut session = durable.lock().unwrap();
                                session.complete_tool_call(&req_call_id, limited.clone());
                            }
                            (ChildCallStatus::Completed, Some(limited))
                        }
                        Err(error) => {
                            let result = serde_json::json!({"error": error.to_string()});
                            {
                                let mut session = durable.lock().unwrap();
                                session.fail_tool_call(&req_call_id, result.clone());
                            }
                            (ChildCallStatus::Failed, Some(result))
                        }
                    };

                    let activity_type = match status {
                        ChildCallStatus::Completed => "child_tool_call_completed",
                        _ => "child_tool_call_failed",
                    };
                    sink.emit(PromptLifecycleEvent::ScriptActivity {
                        script_id: script_id.clone(),
                        parent_call_id: call.call_id.clone(),
                        activity_type: activity_type.to_string(),
                        status: match status {
                            ChildCallStatus::Completed => "completed".to_string(),
                            _ => "failed".to_string(),
                        },
                        detail: Some(serde_json::json!({
                            "call_id": req_call_id,
                            "tool_name": req_tool_name,
                        })),
                    })
                    .await;

                    let _ = req.response_tx.send((status, result));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if handle.is_finished() {
                        break;
                    }
                    if cancel_token.load(Ordering::SeqCst) {
                        break;
                    }
                }
            }
        }

        let output = handle.join().unwrap_or_else(|_| {
            ScriptOutput::failed(ScriptError::runtime("interpreter thread panicked"))
        });

        {
            let mut session = durable.lock().unwrap();
            let child_ids: Vec<String> = output
                .child_outcomes
                .iter()
                .map(|o| o.call_id.clone())
                .collect();
            match output.status {
                ScriptExecStatus::Completed => session.record_script_complete(
                    &script_id,
                    output.result.clone().unwrap_or(serde_json::json!(null)),
                    child_ids,
                ),
                ScriptExecStatus::CompletedWithFailures => session
                    .record_script_complete_with_failures(
                        &script_id,
                        output.result.clone().unwrap_or(serde_json::json!(null)),
                        child_ids,
                    ),
                ScriptExecStatus::Failed => {
                    session.record_script_failed(&script_id, serde_json::json!(output.error))
                }
                ScriptExecStatus::Cancelled => session.record_script_cancelled(&script_id),
            }
        }

        let final_status = match output.status {
            ScriptExecStatus::Completed => "completed",
            ScriptExecStatus::CompletedWithFailures => "completed_with_failures",
            ScriptExecStatus::Failed => "failed",
            ScriptExecStatus::Cancelled => "cancelled",
        };
        sink.emit(PromptLifecycleEvent::ScriptActivity {
            script_id: script_id.clone(),
            parent_call_id: call.call_id.clone(),
            activity_type: "script_completed".to_string(),
            status: final_status.to_string(),
            detail: None,
        })
        .await;

        let result_json = script_output_to_json(&output);

        let acp_status = match output.status {
            ScriptExecStatus::Completed | ScriptExecStatus::CompletedWithFailures => {
                {
                    let mut session = durable.lock().unwrap();
                    session.complete_tool_call(&call.call_id, result_json.clone());
                }
                ToolUpdateStatus::Completed
            }
            ScriptExecStatus::Failed => {
                {
                    let mut session = durable.lock().unwrap();
                    session.fail_tool_call(&call.call_id, result_json.clone());
                }
                ToolUpdateStatus::Failed
            }
            ScriptExecStatus::Cancelled => {
                {
                    let mut session = durable.lock().unwrap();
                    session.cancel_tool_call(&call.call_id);
                }
                ToolUpdateStatus::Failed
            }
        };

        sink.emit(PromptLifecycleEvent::ToolCallUpdate {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            status: acp_status,
            output: Some(result_json),
        })
        .await;
    }

    pub(crate) async fn maybe_compact_post_turn(
        &self,
        durable: &Arc<std::sync::Mutex<DurableSession>>,
        config: &Config,
    ) {
        let should = {
            let session = durable.lock().unwrap();
            CompactionEngine::should_compact(
                session.uncompacted_tokens,
                config.context_management.maintenance_threshold,
                config.context_management.enabled,
            ) && session.is_idle()
        };

        if !should {
            return;
        }

        let input = {
            let session = durable.lock().unwrap();
            CompactionEngine::prepare(
                &session,
                &config.context_management.tail_retention,
                CompactionReason::Maintenance,
            )
        };

        match CompactionEngine::execute(input, self.runtime.provider(), &config.model).await {
            Ok((compacted, tail)) => {
                let mut session = durable.lock().unwrap();
                session.apply_compaction(compacted, tail);
                info!(
                    session_id = %session.id,
                    tokens = session.uncompacted_tokens,
                    "Post-turn maintenance compaction applied"
                );
            }
            Err(e) => {
                warn!("Post-turn compaction failed: {}", e);
            }
        }
    }

    async fn maybe_compact_hard_fit(
        &self,
        durable: &Arc<std::sync::Mutex<DurableSession>>,
        config: &Config,
    ) {
        let window = match config.context_management.context_window_hint {
            Some(w) => w,
            None => return,
        };

        let needs_compaction = {
            let session = durable.lock().unwrap();
            if !session.is_idle() {
                return;
            }
            let instructions = session.instructions.clone();
            let compacted = session.compacted_context.clone();
            let transcript = session.to_transcript();
            drop(session);

            let tool_registry = self.runtime.tool_registry();
            let snapshot = crate::context::ContextTelemetry::for_session(
                instructions.as_deref(),
                compacted.as_ref(),
                &transcript.messages,
                &tool_registry,
                None,
                Some(window),
            );
            snapshot.total_tokens > window
        };

        if !needs_compaction {
            return;
        }

        let input = {
            let session = durable.lock().unwrap();
            CompactionEngine::prepare(
                &session,
                &config.context_management.tail_retention,
                CompactionReason::HardFit,
            )
        };

        match CompactionEngine::execute(input, self.runtime.provider(), &config.model).await {
            Ok((compacted, tail)) => {
                let mut session = durable.lock().unwrap();
                session.apply_compaction(compacted, tail);
                info!(
                    session_id = %session.id,
                    tokens = session.uncompacted_tokens,
                    "Hard-fit compaction applied"
                );
            }
            Err(e) => {
                warn!("Hard-fit compaction failed: {}", e);
            }
        }
    }
}

struct ProviderStep {
    tool_calls: Vec<iron_providers::ToolCall>,
}
