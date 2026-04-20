use crate::config::Config;
use crate::context::compaction::{CompactionEngine, CompactionReason};
use crate::durable::DurableSession;
use crate::ephemeral::EphemeralTurn;
use crate::mcp::SessionToolCatalog;
use crate::plugin::rich_output::transcript_text as plugin_transcript_text;
use crate::prompt_lifecycle::{
    ApprovalRequest, ApprovalVerdict, PromptLifecycleEvent, PromptSink, ToolUpdateStatus,
};
use crate::runtime::IronRuntime;
use futures::StreamExt;
use iron_providers::ProviderEvent;
use parking_lot::Mutex;
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

/// Transition any still-running or pending-approval tool records to
/// `Cancelled` under a single durable-mutex hold. Called from every cancel
/// exit point in the prompt loop so no record is left with a non-terminal
/// status after the run returns `Cancelled`.
fn tie_off_cancelled(durable: &Arc<Mutex<DurableSession>>) {
    let mut session = durable.lock();
    let cancelled = session.cancel_running_tool_calls("cancelled");
    if !cancelled.is_empty() {
        trace!(
            count = cancelled.len(),
            call_ids = ?cancelled,
            "Cancelled in-flight tool records on prompt cancel"
        );
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
        durable: &Arc<Mutex<DurableSession>>,
        ephemeral: &Arc<Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        config: &Config,
        max_iterations: u32,
    ) -> agent_client_protocol::StopReason {
        let mut iteration: u32 = 0;

        loop {
            {
                let turn = ephemeral.lock();
                if turn.is_cancel_requested() {
                    tie_off_cancelled(durable);
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

            let session_id = {
                let session = durable.lock();
                session.id
            };
            let tool_catalog = self
                .runtime
                .get_session_tool_catalog(session_id)
                .map(Arc::new);

            let request = {
                let session = durable.lock();
                let instructions = session.instructions.clone();
                let compacted_context = session.compacted_context.clone();
                let repo_payload = session.repo_instruction_payload.clone();
                let messages = session.to_transcript().messages;
                let skill_instructions = session.active_skill_instructions();
                drop(session);

                if let Some(tool_catalog) = tool_catalog.as_ref() {
                    crate::request_builder::build_inference_request_with_effective_tools(
                        config,
                        &messages,
                        compacted_context.as_ref(),
                        instructions.as_deref(),
                        repo_payload.as_ref(),
                        tool_catalog.definitions(),
                        tool_catalog.contains("python_exec"),
                        Some(&skill_instructions),
                    )
                } else {
                    let tool_registry = self.runtime.tool_registry();
                    crate::request_builder::build_inference_request_with_context_and_repo(
                        config,
                        &messages,
                        compacted_context.as_ref(),
                        instructions.as_deref(),
                        repo_payload.as_ref(),
                        &tool_registry,
                        Some(&skill_instructions),
                    )
                }
            };

            let request = match request {
                Ok(req) => req,
                Err(e) => {
                    warn!(error = %e, "Request building failed");
                    {
                        let mut session = durable.lock();
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
                        let mut session = durable.lock();
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
                let turn = ephemeral.lock();
                turn.is_cancel_requested()
            };

            if cancel_check() {
                tie_off_cancelled(durable);
                return agent_client_protocol::StopReason::Cancelled;
            }

            let needs_permission = {
                let approval_strategy = config.default_approval_strategy;
                if let Some(tool_catalog) = tool_catalog.as_ref() {
                    step.tool_calls.iter().any(|call| {
                        let tool_requires = tool_catalog.requires_approval(&call.tool_name);
                        approval_strategy.is_approval_required(tool_requires)
                    })
                } else {
                    // If we can't get the catalog, assume no tools need permission
                    // (they'll fail during execution with a clearer error)
                    false
                }
            };

            let cancel_token = {
                let turn = ephemeral.lock();
                turn.cancel_token()
            };

            if needs_permission {
                let approved_tool_calls = match self
                    .handle_permission_flow(
                        durable,
                        ephemeral,
                        sink,
                        &step.tool_calls,
                        config,
                        tool_catalog.clone(),
                    )
                    .await
                {
                    Ok(calls) => calls,
                    Err(reason) => {
                        if matches!(reason, agent_client_protocol::StopReason::Cancelled) {
                            tie_off_cancelled(durable);
                        }
                        return reason;
                    }
                };

                self.execute_tool_calls(
                    durable,
                    ephemeral,
                    sink,
                    approved_tool_calls,
                    cancel_token,
                    tool_catalog.clone(),
                )
                .await;
            } else {
                self.execute_tool_calls(
                    durable,
                    ephemeral,
                    sink,
                    step.tool_calls,
                    cancel_token,
                    tool_catalog.clone(),
                )
                .await;
            }
        }
    }

    async fn process_provider_stream(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
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
                        let mut session = durable.lock();
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
                        let mut session = durable.lock();
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
                ProviderEvent::ChoiceRequest { request } => {
                    trace!(prompt = %request.prompt, "Choice requests are not supported by prompt runner");
                    if !assistant_output.is_empty() {
                        let mut session = durable.lock();
                        session.add_agent_text(&assistant_output);
                    }
                    return Err(agent_client_protocol::StopReason::EndTurn);
                }
                ProviderEvent::Complete => {}
                ProviderEvent::Error { source: _ } => {
                    if !assistant_output.is_empty() {
                        let mut session = durable.lock();
                        session.add_agent_text(&assistant_output);
                    }
                    return Err(agent_client_protocol::StopReason::EndTurn);
                }
            }
        }

        if !assistant_output.is_empty() {
            let mut session = durable.lock();
            session.add_agent_text(&assistant_output);
        }

        Ok(ProviderStep { tool_calls })
    }

    async fn handle_permission_flow(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        ephemeral: &Arc<Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        tool_calls: &[iron_providers::ToolCall],
        _config: &Config,
        tool_catalog: Option<Arc<SessionToolCatalog>>,
    ) -> Result<Vec<iron_providers::ToolCall>, agent_client_protocol::StopReason> {
        let tool_catalog = match tool_catalog {
            Some(catalog) => catalog,
            None => {
                // If we can't get the catalog, deny all tools that require permission
                for call in tool_calls {
                    let error_result =
                        serde_json::json!({"error": "denied - session tool catalog unavailable"});
                    {
                        let mut session = durable.lock();
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
                return Ok(Vec::new());
            }
        };

        let mut approved = Vec::new();

        let approval_strategy = _config.default_approval_strategy;

        for call in tool_calls {
            let tool_requires = tool_catalog.requires_approval(&call.tool_name);
            let requires = approval_strategy.is_approval_required(tool_requires);

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
                        let mut session = durable.lock();
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
                            let mut session = durable.lock();
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
                        let mut session = durable.lock();
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
        ephemeral: &Arc<Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        call_id: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> ApprovalVerdict {
        {
            let mut turn = ephemeral.lock();
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
            let mut turn = ephemeral.lock();
            turn.resolve_permission(call_id);
        }

        verdict
    }

    async fn execute_tool_calls(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        ephemeral: &Arc<Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        tool_calls: Vec<iron_providers::ToolCall>,
        cancel_token: std::sync::Arc<std::sync::atomic::AtomicBool>,
        tool_catalog: Option<Arc<SessionToolCatalog>>,
    ) {
        use std::sync::atomic::Ordering;

        let mut calls = tool_calls.into_iter().peekable();

        while let Some(call) = calls.next() {
            if cancel_token.load(Ordering::SeqCst) {
                self.cancel_remaining_tool_calls(durable, sink, call, &mut calls)
                    .await;
                return;
            }

            self.execute_single_tool(
                durable,
                ephemeral,
                sink,
                call,
                cancel_token.clone(),
                tool_catalog.clone(),
            )
            .await;
        }
    }

    async fn validate_and_prepare(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        call: &iron_providers::ToolCall,
        tool_catalog: &SessionToolCatalog,
    ) -> bool {
        let tool_def = tool_catalog.get_definition(&call.tool_name).cloned();

        {
            let mut session = durable.lock();
            session.start_tool_call(&call.call_id, &call.tool_name, call.arguments.clone());
        }

        let Some(definition) = tool_def else {
            sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                status: ToolUpdateStatus::InProgress,
                output: None,
            })
            .await;

            return true;
        };

        match crate::schema::validate_arguments(&definition.input_schema, &call.arguments) {
            crate::schema::SchemaValidationOutcome::Valid => {}
            crate::schema::SchemaValidationOutcome::Invalid { errors } => {
                let error_detail = errors.join("; ");
                let error_result = serde_json::json!({
                    "error": format!("schema validation failed: {}", error_detail),
                    "validation_errors": errors,
                });
                {
                    let mut session = durable.lock();
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
                    let mut session = durable.lock();
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
        durable: &Arc<Mutex<DurableSession>>,
        _ephemeral: &Arc<Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        call: iron_providers::ToolCall,
        cancel_token: std::sync::Arc<std::sync::atomic::AtomicBool>,
        tool_catalog: Option<Arc<SessionToolCatalog>>,
    ) {
        #[cfg(not(feature = "embedded-python"))]
        let _ = &cancel_token;

        let tool_catalog = match tool_catalog {
            Some(catalog) => catalog,
            None => {
                let error_result =
                    serde_json::json!({"error": "Failed to get session tool catalog"});
                {
                    let mut session = durable.lock();
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

        if !self
            .validate_and_prepare(durable, sink, &call, &tool_catalog)
            .await
        {
            return;
        }

        #[cfg(feature = "embedded-python")]
        if call.tool_name == "python_exec" && self.runtime.config().embedded_python.enabled {
            self.execute_python_script(
                durable,
                _ephemeral,
                sink,
                &call,
                cancel_token,
                tool_catalog.clone(),
            )
            .await;
            return;
        }

        if call.tool_name == "activate_skill" {
            self.execute_activate_skill(durable, sink, call).await;
            return;
        }

        self.execute_standard_tool(durable, sink, call, tool_catalog.as_ref())
            .await;
    }

    async fn execute_standard_tool(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        call: iron_providers::ToolCall,
        tool_catalog: &SessionToolCatalog,
    ) {
        let call_id = call.call_id.clone();
        let tool_name = call.tool_name.clone();

        let call_id_owned = call.call_id.clone();
        let tool_name_owned = call.tool_name.clone();
        let arguments = call.arguments.clone();
        let execute_future = {
            let session_guard = durable.lock();
            tool_catalog.execute(&call_id_owned, &tool_name_owned, arguments, &session_guard)
        };

        let execute_result = execute_future.await;

        match execute_result {
            Ok(result) => {
                let limited_result = limit_result_size(result);
                if let Some(transcript_text) = plugin_transcript_text(&limited_result) {
                    sink.emit(PromptLifecycleEvent::Output {
                        text: transcript_text.to_string(),
                    })
                    .await;
                }
                {
                    let mut session = durable.lock();
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
                    let mut session = durable.lock();
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

    async fn execute_activate_skill(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        call: iron_providers::ToolCall,
    ) {
        let call_id = call.call_id.clone();
        let skill_name = call
            .arguments
            .get("skill_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if skill_name.is_empty() {
            let result = serde_json::json!({"error": "Missing 'skill_name' argument"});
            {
                let mut session = durable.lock();
                session.fail_tool_call(&call_id, result.clone());
            }
            sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                call_id: call_id.clone(),
                tool_name: "activate_skill".to_string(),
                status: ToolUpdateStatus::Failed,
                output: Some(result),
            })
            .await;
            return;
        }

        let (skill, already_active) = {
            let session = durable.lock();
            match session.load_available_skill(skill_name) {
                Some(skill) => (skill, session.is_skill_active(skill_name)),
                None => {
                    drop(session);
                    let result = serde_json::json!({
                        "error": format!("Skill '{}' is not available in this session", skill_name)
                    });
                    {
                        let mut session = durable.lock();
                        session.fail_tool_call(&call_id, result.clone());
                    }
                    sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                        call_id: call_id.clone(),
                        tool_name: "activate_skill".to_string(),
                        status: ToolUpdateStatus::Failed,
                        output: Some(result),
                    })
                    .await;
                    return;
                }
            }
        };

        if skill.metadata.requires_trust {
            let result = serde_json::json!({
                "error": format!(
                    "Skill '{}' requires elevated trust and cannot be activated by the model. Ask the user to activate it.",
                    skill_name
                )
            });
            {
                let mut session = durable.lock();
                session.fail_tool_call(&call_id, result.clone());
            }
            sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                call_id: call_id.clone(),
                tool_name: "activate_skill".to_string(),
                status: ToolUpdateStatus::Failed,
                output: Some(result),
            })
            .await;
            return;
        }

        {
            let mut session = durable.lock();
            session.start_tool_call(&call_id, "activate_skill", call.arguments.clone());
            if !already_active {
                session.activate_skill(&skill.metadata.id, &skill.body, skill.resources.clone());
            }
        }

        let result = if already_active {
            serde_json::json!({
                "status": "already_active",
                "skill": skill.metadata.id,
            })
        } else {
            serde_json::json!({
                "status": "activated",
                "skill": skill.metadata.id,
                "description": skill.metadata.description,
                "content": crate::skill::render_skill_content(&skill.metadata.id, &skill.body),
                "resources": skill.resources.iter().map(|r| &r.path).collect::<Vec<_>>(),
            })
        };
        let limited_result = limit_result_size(result);

        {
            let mut session = durable.lock();
            session.complete_tool_call(&call_id, limited_result.clone());
        }

        sink.emit(PromptLifecycleEvent::ToolCallUpdate {
            call_id: call_id.clone(),
            tool_name: "activate_skill".to_string(),
            status: ToolUpdateStatus::Completed,
            output: Some(limited_result),
        })
        .await;
    }

    async fn cancel_remaining_tool_calls(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        first: iron_providers::ToolCall,
        rest: &mut std::iter::Peekable<std::vec::IntoIter<iron_providers::ToolCall>>,
    ) {
        for call in std::iter::once(first).chain(rest) {
            {
                let mut session = durable.lock();
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

    /// Execute an embedded Python script, routing all child-tool calls
    /// through the canonical [`SessionToolCatalog`] execution path.
    ///
    /// ## Phase 8 guarantees
    ///
    /// * **Canonical execution path**: Every child-tool call (including
    ///   plugin-backed tools) goes through `SessionToolCatalog::execute()`,
    ///   which handles enablement, health, auth-gating, scope checks, and
    ///   WASM host execution identically to model-issued tool calls.
    ///
    /// * **Approval consistency**: The approval strategy is the single
    ///   arbiter for whether user confirmation is needed, applied uniformly
    ///   regardless of whether the call originates from the model or from
    ///   embedded Python.
    ///
    /// * **Visibility parity**: The Python-visible tool catalog is built
    ///   from `session_tool_catalog.definitions()`, which already excludes
    ///   tools from disabled, unhealthy, or auth-gated plugins.  Python
    ///   scripts see exactly the same set of tools as the model.
    #[cfg(feature = "embedded-python")]
    async fn execute_python_script(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        ephemeral: &Arc<Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        call: &iron_providers::ToolCall,
        cancel_token: std::sync::Arc<std::sync::atomic::AtomicBool>,
        session_tool_catalog: Arc<SessionToolCatalog>,
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
                    let mut session = durable.lock();
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
            let mut session = durable.lock();
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
        // Build the Python-visible tool catalog from the session-effective
        // definitions.  Because `SessionToolCatalog` already applies session
        // enablement, health, auth-gating, and scope checks for every tool
        // source (local, MCP, and plugin), the resulting catalog only
        // includes tools that are *actually usable* in this session.
        //
        // Phase 8 guarantee: embedded Python tool visibility matches
        // runtime-effective visibility.  A plugin tool that is disabled for
        // this session, unhealthy, or auth-gated will NOT appear in
        // `tools.available()` or be callable via `tools.call(...)` / alias
        // methods.
        let tool_catalog = crate::embedded_python::ToolCatalog::from_definitions(
            session_tool_catalog.definitions().to_vec(),
        );

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

                    let tool_def = session_tool_catalog.get_definition(&req.tool_name).cloned();

                    // Phase 8 guarantee: all child-tool execution routes
                    // through `SessionToolCatalog::execute()`, which is the
                    // same canonical path used for model-issued tool calls.
                    // For plugin-backed tools this means:
                    //   - Session enablement check (plugin enabled for this session)
                    //   - Health check (plugin is healthy)
                    //   - Auth-gating check (credentials present if required)
                    //   - WASM host execution via Extism
                    // No separate or legacy code path exists for plugin tools
                    // called from embedded Python.

                    let Some(definition) = tool_def else {
                        {
                            let mut session = durable.lock();
                            session.start_tool_call(&req.call_id, &req.tool_name, req.args.clone());
                            session.link_child_to_script(&script_id, &req.call_id);
                        }

                        let req_call_id = req.call_id.clone();
                        let req_tool_name = req.tool_name.clone();
                        let execute_future = {
                            let session_guard = durable.lock();
                            session_tool_catalog.execute(
                                &req.call_id,
                                &req.tool_name,
                                req.args.clone(),
                                &session_guard,
                            )
                        };

                        let result = match execute_future.await {
                            Ok(result) => {
                                let limited = limit_result_size(result);
                                {
                                    let mut session = durable.lock();
                                    session.complete_tool_call(&req_call_id, limited.clone());
                                }
                                (ChildCallStatus::Completed, Some(limited))
                            }
                            Err(error) => {
                                let result = serde_json::json!({"error": error.to_string()});
                                {
                                    let mut session = durable.lock();
                                    session.fail_tool_call(&req_call_id, result.clone());
                                }
                                (ChildCallStatus::Failed, Some(result))
                            }
                        };

                        sink.emit(PromptLifecycleEvent::ScriptActivity {
                            script_id: script_id.clone(),
                            parent_call_id: call.call_id.clone(),
                            activity_type: if matches!(result.0, ChildCallStatus::Completed) {
                                "child_tool_call_completed".to_string()
                            } else {
                                "child_tool_call_failed".to_string()
                            },
                            status: if matches!(result.0, ChildCallStatus::Completed) {
                                "completed".to_string()
                            } else {
                                "failed".to_string()
                            },
                            detail: Some(serde_json::json!({
                                "call_id": req_call_id,
                                "tool_name": req_tool_name,
                            })),
                        })
                        .await;

                        if let Err(err) = req.response_tx.send(result) {
                            tracing::debug!(
                                call_id = %req.call_id,
                                ?err,
                                "tool result receiver dropped (tool not found)"
                            );
                        }
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
                                let mut session = durable.lock();
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
                                let mut session = durable.lock();
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

                    // Phase 8 guarantee: child-tool approval uses the same
                    // approval-strategy logic as model-issued tool calls.
                    // `requires_approval` for plugin tools comes from the
                    // manifest's `requires_approval` field (set during
                    // canonical `SessionToolCatalog` construction), not from
                    // a separate or legacy code path.  The approval strategy
                    // is the single arbiter of whether user confirmation is
                    // needed, regardless of call origin (model or Python).
                    let tool_requires = session_tool_catalog.requires_approval(&req.tool_name);
                    let approval_strategy = self.runtime.config().default_approval_strategy;
                    let requires_permission = approval_strategy.is_approval_required(tool_requires);
                    if requires_permission {
                        {
                            let mut session = durable.lock();
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
                                    let mut session = durable.lock();
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
                                if let Err(err) =
                                    req.response_tx.send((ChildCallStatus::Denied, None))
                                {
                                    tracing::debug!(
                                        call_id = %req.call_id,
                                        ?err,
                                        "child tool result receiver dropped (denied)"
                                    );
                                }
                                continue;
                            }
                            ApprovalVerdict::Cancelled => {
                                {
                                    let mut session = durable.lock();
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
                                if let Err(err) =
                                    req.response_tx.send((ChildCallStatus::Cancelled, None))
                                {
                                    tracing::debug!(
                                        call_id = %req.call_id,
                                        ?err,
                                        "child tool result receiver dropped (cancelled)"
                                    );
                                }
                                continue;
                            }
                        }
                    }

                    {
                        let mut session = durable.lock();
                        session.start_tool_call(&req.call_id, &req.tool_name, req.args.clone());
                        session.link_child_to_script(&script_id, &req.call_id);
                    }

                    let req_call_id = req.call_id.clone();
                    let req_tool_name = req.tool_name.clone();
                    let execute_future = {
                        let session_guard = durable.lock();
                        session_tool_catalog.execute(
                            &req.call_id,
                            &req.tool_name,
                            req.args.clone(),
                            &session_guard,
                        )
                    };

                    let (status, result) = match execute_future.await {
                        Ok(result) => {
                            let limited = limit_result_size(result);
                            {
                                let mut session = durable.lock();
                                session.complete_tool_call(&req_call_id, limited.clone());
                            }
                            (ChildCallStatus::Completed, Some(limited))
                        }
                        Err(error) => {
                            let result = serde_json::json!({"error": error.to_string()});
                            {
                                let mut session = durable.lock();
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

                    if let Err(err) = req.response_tx.send((status, result)) {
                        tracing::debug!(
                            call_id = %req_call_id,
                            ?err,
                            "child tool result receiver dropped after execution"
                        );
                    }
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
            let mut session = durable.lock();
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
                    let mut session = durable.lock();
                    session.complete_tool_call(&call.call_id, result_json.clone());
                }
                ToolUpdateStatus::Completed
            }
            ScriptExecStatus::Failed => {
                {
                    let mut session = durable.lock();
                    session.fail_tool_call(&call.call_id, result_json.clone());
                }
                ToolUpdateStatus::Failed
            }
            ScriptExecStatus::Cancelled => {
                {
                    let mut session = durable.lock();
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
        durable: &Arc<Mutex<DurableSession>>,
        config: &Config,
    ) {
        let should = {
            let session = durable.lock();
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
            let session = durable.lock();
            CompactionEngine::prepare(
                &session,
                &config.context_management.tail_retention,
                CompactionReason::Maintenance,
            )
        };

        match CompactionEngine::execute(input, self.runtime.provider(), &config.model).await {
            Ok((compacted, tail)) => {
                let mut session = durable.lock();
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

    async fn maybe_compact_hard_fit(&self, durable: &Arc<Mutex<DurableSession>>, config: &Config) {
        let window = match config.context_management.context_window_hint {
            Some(w) => w,
            None => return,
        };

        let needs_compaction = {
            let session = durable.lock();
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
            let session = durable.lock();
            CompactionEngine::prepare(
                &session,
                &config.context_management.tail_retention,
                CompactionReason::HardFit,
            )
        };

        match CompactionEngine::execute(input, self.runtime.provider(), &config.model).await {
            Ok((compacted, tail)) => {
                let mut session = durable.lock();
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
