use crate::config::Config;
use crate::connection::SharedClientChannel;
use crate::delegation::{
    validate_delegation_arguments, ChildApprovalMode, DelegationRequest, SubAgentToolPolicy,
};
use crate::durable::DurableSession;
use crate::ephemeral::{EphemeralTurn, TurnPhase};
use crate::mcp::SessionToolCatalog;
use crate::plugin::rich_output::transcript_text as plugin_transcript_text;

/// Determine whether a tool call requires permission, considering the session-effective
/// approval policy if present, otherwise falling back to the global config strategy.
fn session_approval_requires_permission(
    session: &DurableSession,
    config: &Config,
    tool_requires_approval: bool,
) -> bool {
    match session.effective_approval {
        Some(crate::profile::AgentApproval::AutoApprove) => false,
        Some(crate::profile::AgentApproval::PerTool) => tool_requires_approval,
        None => config
            .default_approval_strategy
            .is_approval_required(tool_requires_approval),
    }
}
use crate::profile::{AgentProfile, AgentProfileId};
use crate::prompt_lifecycle::{
    ApprovalRequest, ApprovalVerdict, PromptLifecycleEvent, PromptSink, ToolUpdateStatus,
};
use crate::provider_credential::ProviderPromptContext;
use crate::runtime::IronRuntime;
use agent_client_protocol::schema as acp;
use futures::StreamExt;
use iron_providers::{Provider, ProviderError, ProviderEvent};
use parking_lot::Mutex;
use std::sync::Arc;
use tracing::{trace, warn};

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
    managed_provider: Option<Box<dyn Provider>>,
    /// Provider context used for auth-failure retry.
    provider_context: Option<ProviderPromptContext>,
    #[cfg(test)]
    retry_provider_factory: Option<Arc<dyn Fn() -> Box<dyn Provider> + Send + Sync>>,
}

impl PromptRunner {
    pub(crate) fn new(runtime: IronRuntime) -> Self {
        Self {
            runtime,
            managed_provider: None,
            provider_context: None,
            #[cfg(test)]
            retry_provider_factory: None,
        }
    }

    pub(crate) fn new_managed(
        runtime: IronRuntime,
        provider: Box<dyn Provider>,
        context: ProviderPromptContext,
    ) -> Self {
        Self {
            runtime,
            managed_provider: Some(provider),
            provider_context: Some(context),
            #[cfg(test)]
            retry_provider_factory: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_managed_with_retry_provider_for_test(
        runtime: IronRuntime,
        provider: Box<dyn Provider>,
        context: ProviderPromptContext,
        retry_provider_factory: Arc<dyn Fn() -> Box<dyn Provider> + Send + Sync>,
    ) -> Self {
        Self {
            runtime,
            managed_provider: Some(provider),
            provider_context: Some(context),
            retry_provider_factory: Some(retry_provider_factory),
        }
    }

    fn provider(&self) -> &dyn Provider {
        self.managed_provider
            .as_ref()
            .map(|p| p.as_ref())
            .unwrap_or_else(|| self.runtime.provider())
    }

    async fn auth_failure_is_api_key_backed(&self) -> bool {
        let Some(context) = self.provider_context.as_ref() else {
            return false;
        };
        matches!(
            self.runtime.provider_auth_status_for_context(context).await,
            Some(crate::provider_credential::ProviderAuthStatus::ConfiguredApiKey)
        )
    }

    async fn refreshed_provider_for_retry(
        &self,
        context: &ProviderPromptContext,
    ) -> crate::provider_credential::ProviderAuthResult<Box<dyn Provider>> {
        #[cfg(test)]
        if let Some(factory) = &self.retry_provider_factory {
            return Ok(factory());
        }

        self.runtime.force_refresh_managed_provider(context).await
    }

    pub(crate) async fn run(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        ephemeral: &Arc<Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        config: &Config,
        max_iterations: u32,
    ) -> acp::StopReason {
        let mut iteration: u32 = 0;
        let turn_id = ephemeral.lock().turn_id.clone();

        loop {
            {
                let turn = ephemeral.lock();
                if turn.is_cancel_requested() {
                    tie_off_cancelled(durable);
                    return acp::StopReason::Cancelled;
                }
            }

            iteration += 1;
            if iteration > max_iterations {
                return acp::StopReason::MaxTurnRequests;
            }

            trace!(iteration, "Starting inference iteration");

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
                let profile_identity = session.profile_identity.clone();
                let compressed_blocks = session.compressed_blocks.clone();
                let repo_payload = session.repo_instruction_payload.clone();
                let messages = session.to_transcript_with_visible_ids(true).messages;
                let skill_instructions = session.active_skill_instructions();
                let workspace_roots = session.workspace_roots.clone();
                let effective_model = session.current_model.clone();
                drop(session);

                let tool_registry = self.runtime.tool_registry();
                let mut snapshot = {
                    let session = durable.lock();
                    crate::context::ActiveContextAccountant::estimate_snapshot(
                        session.instruction_text_for_estimate().as_deref(),
                        &compressed_blocks,
                        &messages,
                        &tool_registry,
                        None,
                        config.context_management.context_window_hint,
                        crate::context::SessionModelInfo {
                            current_model: None,
                            model_switch_count: 0,
                        },
                        Some(&session.token_tracker),
                    )
                };
                if config.context_management.enabled {
                    snapshot.compact_threshold_tokens =
                        Some(config.context_management.maintenance_threshold);
                }
                let context_pressure = snapshot.pressure_with_thresholds(
                    config.context_management.soft_threshold,
                    config.context_management.medium_threshold,
                    config.context_management.strong_threshold,
                    config.context_management.critical_threshold,
                );

                self.runtime.emit_debug(crate::debug::DebugEvent {
                    timestamp: chrono::Utc::now(),
                    sequence: self.runtime.next_debug_sequence(),
                    severity: crate::debug::DebugSeverity::Info,
                    scope: crate::debug::DebugScope {
                        session_id: Some(session_id),
                        turn_id: turn_id.clone(),
                        ..crate::debug::DebugScope::default()
                    },
                    payload: crate::debug::DebugPayload::Context(
                        crate::debug::ContextDebugEvent::SnapshotEstimated {
                            total_tokens: snapshot.total_tokens,
                            context_window_limit: snapshot.context_window_limit,
                            compact_threshold_tokens: snapshot.compact_threshold_tokens,
                            quality: snapshot.quality,
                            pressure: context_pressure.as_str().to_string(),
                            categories: snapshot
                                .categories
                                .iter()
                                .map(|usage| {
                                    (format!("{:?}", usage.category), usage.tokens, usage.quality)
                                })
                                .collect(),
                            accumulated_usage: snapshot.accumulated_usage.clone(),
                        },
                    ),
                });

                let request = if let Some(tool_catalog) = tool_catalog.as_ref() {
                    crate::request_builder::build_inference_request_with_effective_tools(
                        config,
                        &messages,
                        crate::request_builder::EffectiveToolRequestContext {
                            compressed_blocks: &compressed_blocks,
                            instructions: instructions.as_deref(),
                            repo_instruction_payload: repo_payload.as_ref(),
                            python_exec_available: tool_catalog.contains("python_exec"),
                            skill_instructions: Some(&skill_instructions),
                            context_pressure,
                            workspace_roots: Some(&workspace_roots),
                            effective_model: effective_model.as_deref(),
                            profile_identity: profile_identity.as_deref(),
                        },
                        tool_catalog.definitions(),
                    )
                } else {
                    crate::request_builder::build_inference_request_with_context_and_repo(
                        config,
                        &messages,
                        crate::request_builder::EffectiveToolRequestContext {
                            compressed_blocks: &compressed_blocks,
                            instructions: instructions.as_deref(),
                            repo_instruction_payload: repo_payload.as_ref(),
                            python_exec_available: tool_registry.contains("python_exec"),
                            skill_instructions: Some(&skill_instructions),
                            context_pressure,
                            workspace_roots: Some(&workspace_roots),
                            effective_model: effective_model.as_deref(),
                            profile_identity: profile_identity.as_deref(),
                        },
                        &tool_registry,
                    )
                };

                // Emit prompt debug events inside the block where variables are in scope
                if let Ok(ref req) = request {
                    if let Some(ref instructions) = req.instructions {
                        use std::collections::hash_map::DefaultHasher;
                        use std::hash::{Hash, Hasher};
                        let mut hasher = DefaultHasher::new();
                        instructions.hash(&mut hasher);
                        let fingerprint = format!("{:x}", hasher.finish());
                        let total_chars = instructions.len();
                        let sections: Vec<crate::debug::SectionSummary> =
                            crate::prompt::system::PROMPT_SECTION_ORDER
                                .iter()
                                .map(|section| {
                                    let meta = section.metadata();
                                    crate::debug::SectionSummary {
                                        name: meta.title.to_string(),
                                        owner: Some(format!("{:?}", meta.owner)),
                                        temperature: Some(format!("{:?}", meta.temperature)),
                                        chars: 0,
                                    }
                                })
                                .collect();

                        self.runtime.emit_debug(crate::debug::DebugEvent {
                            timestamp: chrono::Utc::now(),
                            sequence: self.runtime.next_debug_sequence(),
                            severity: crate::debug::DebugSeverity::Info,
                            scope: crate::debug::DebugScope {
                                session_id: Some(session_id),
                                turn_id: turn_id.clone(),
                                ..crate::debug::DebugScope::default()
                            },
                            payload: crate::debug::DebugPayload::Prompt(
                                crate::debug::PromptDebugEvent::SystemPromptRendered {
                                    fingerprint,
                                    total_chars,
                                    sections,
                                    changed: None,
                                },
                            ),
                        });

                        // Emit model input influence for context pressure
                        self.runtime.emit_debug(crate::debug::DebugEvent {
                            timestamp: chrono::Utc::now(),
                            sequence: self.runtime.next_debug_sequence(),
                            severity: crate::debug::DebugSeverity::Info,
                            scope: crate::debug::DebugScope {
                                session_id: Some(session_id),
                                turn_id: turn_id.clone(),
                                ..crate::debug::DebugScope::default()
                            },
                            payload: crate::debug::DebugPayload::Prompt(
                                crate::debug::PromptDebugEvent::ModelInputInfluence {
                                    source: crate::debug::InfluenceSource::ContextPressure,
                                    destination:
                                        crate::debug::InfluenceDestination::SystemPromptSection(
                                            "Tool Philosophy".to_string(),
                                        ),
                                    effect: crate::debug::InfluenceEffect::Added,
                                    reason: format!(
                                        "context pressure guidance: {}",
                                        context_pressure.as_str()
                                    ),
                                },
                            ),
                        });

                        // Emit model input influence for compression availability
                        self.runtime.emit_debug(crate::debug::DebugEvent {
                            timestamp: chrono::Utc::now(),
                            sequence: self.runtime.next_debug_sequence(),
                            severity: crate::debug::DebugSeverity::Info,
                            scope: crate::debug::DebugScope {
                                session_id: Some(session_id),
                                turn_id: turn_id.clone(),
                                ..crate::debug::DebugScope::default()
                            },
                            payload: crate::debug::DebugPayload::Prompt(
                                crate::debug::PromptDebugEvent::ModelInputInfluence {
                                    source: crate::debug::InfluenceSource::CompactionAvailability,
                                    destination:
                                        crate::debug::InfluenceDestination::SystemPromptSection(
                                            "Tool Philosophy".to_string(),
                                        ),
                                    effect: crate::debug::InfluenceEffect::Added,
                                    reason: "compression_available=true".to_string(),
                                },
                            ),
                        });
                    }
                }

                request
            };

            let request = match request {
                Ok(req) => req,
                Err(e) => {
                    warn!(error = %e, "Request building failed");
                    {
                        let mut session = durable.lock();
                        session.add_agent_text(format!("[Request error: {}]", e));
                    }
                    return acp::StopReason::EndTurn;
                }
            };

            let stream = match self.provider().infer_stream(request.clone()).await {
                Ok(s) => s,
                Err(e) => {
                    let retry_context = if e.is_authentication()
                        && self.managed_provider.is_some()
                        && !self.auth_failure_is_api_key_backed().await
                    {
                        self.provider_context.as_ref()
                    } else {
                        None
                    };

                    if let Some(context) = retry_context {
                        warn!(error = %e, "Provider auth failed, attempting force-refresh and retry");
                        match self.refreshed_provider_for_retry(context).await {
                            Ok(refreshed_provider) => {
                                match refreshed_provider.infer_stream(request.clone()).await {
                                    Ok(s) => s,
                                    Err(e2) => {
                                        warn!(error = %e2, "Provider retry failed after refresh");
                                        {
                                            let mut session = durable.lock();
                                            session.add_agent_text(format!(
                                                "[Provider auth error: {}]",
                                                e2
                                            ));
                                        }
                                        return acp::StopReason::EndTurn;
                                    }
                                }
                            }
                            Err(refresh_err) => {
                                warn!(error = %refresh_err, "Force-refresh failed, cannot retry");
                                {
                                    let mut session = durable.lock();
                                    session.add_agent_text(format!(
                                        "[Provider auth error: {} (refresh failed: {})]",
                                        e, refresh_err
                                    ));
                                }
                                return acp::StopReason::EndTurn;
                            }
                        }
                    } else {
                        warn!(error = %e, "Provider inference failed");
                        {
                            let mut session = durable.lock();
                            session.add_agent_text(format!("[Provider error: {}]", e));
                        }
                        return acp::StopReason::EndTurn;
                    }
                }
            };

            let step = match self.process_provider_stream(durable, sink, stream).await {
                ProviderStreamOutcome::Step(step) => step,
                ProviderStreamOutcome::Stop(reason) => return reason,
                ProviderStreamOutcome::AuthFailureBeforeOutput(error) => {
                    let retry_context = if self.managed_provider.is_some()
                        && !self.auth_failure_is_api_key_backed().await
                    {
                        self.provider_context.as_ref()
                    } else {
                        None
                    };

                    if let Some(context) = retry_context {
                        warn!(error = %error, "Provider stream auth failed before output, attempting force-refresh and retry");
                        let refreshed_provider = match self
                            .refreshed_provider_for_retry(context)
                            .await
                        {
                            Ok(provider) => provider,
                            Err(refresh_err) => {
                                warn!(error = %refresh_err, "Force-refresh failed, cannot retry stream auth failure");
                                let mut session = durable.lock();
                                session.add_agent_text(format!(
                                    "[Provider auth error: {} (refresh failed: {})]",
                                    error, refresh_err
                                ));
                                return acp::StopReason::EndTurn;
                            }
                        };

                        let retry_stream = match refreshed_provider.infer_stream(request).await {
                            Ok(stream) => stream,
                            Err(retry_error) => {
                                warn!(error = %retry_error, "Provider retry failed after refresh");
                                let mut session = durable.lock();
                                session.add_agent_text(format!(
                                    "[Provider auth error: {}]",
                                    retry_error
                                ));
                                return acp::StopReason::EndTurn;
                            }
                        };

                        match self
                            .process_provider_stream(durable, sink, retry_stream)
                            .await
                        {
                            ProviderStreamOutcome::Step(step) => step,
                            ProviderStreamOutcome::Stop(reason) => return reason,
                            ProviderStreamOutcome::AuthFailureBeforeOutput(retry_error) => {
                                let mut session = durable.lock();
                                session.add_agent_text(format!(
                                    "[Provider auth error: {}]",
                                    retry_error
                                ));
                                return acp::StopReason::EndTurn;
                            }
                        }
                    } else {
                        let mut session = durable.lock();
                        session.add_agent_text(format!("[Provider auth error: {}]", error));
                        return acp::StopReason::EndTurn;
                    }
                }
            };

            if step.tool_calls.is_empty() {
                return acp::StopReason::EndTurn;
            }

            let cancel_check = || {
                let turn = ephemeral.lock();
                turn.is_cancel_requested()
            };

            if cancel_check() {
                tie_off_cancelled(durable);
                return acp::StopReason::Cancelled;
            }

            let needs_permission = {
                if let Some(tool_catalog) = tool_catalog.as_ref() {
                    step.tool_calls.iter().any(|call| {
                        let tool_requires = tool_catalog.requires_approval(&call.tool_name);
                        session_approval_requires_permission(&durable.lock(), config, tool_requires)
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
                        if matches!(reason, acp::StopReason::Cancelled) {
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

    fn flush_tool_call_deltas(
        durable: &Arc<Mutex<DurableSession>>,
        tool_calls: &[iron_providers::ToolCall],
    ) {
        for call in tool_calls {
            let mut session = durable.lock();
            let tool_tokens =
                crate::durable::estimate_tool_call_tokens(&call.tool_name, &call.arguments);
            session.token_tracker.add_delta(tool_tokens);
            session.uncompacted_tokens += tool_tokens;
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
    ) -> ProviderStreamOutcome {
        let mut tool_calls = Vec::new();
        let mut assistant_output = String::new();
        let mut emitted_output = false;

        while let Some(result) = stream.next().await {
            let event = match result {
                Ok(e) => e,
                Err(error) => {
                    Self::flush_tool_call_deltas(durable, &tool_calls);
                    return Self::handle_provider_stream_error(
                        durable,
                        &mut assistant_output,
                        emitted_output,
                        error,
                    );
                }
            };

            match event {
                ProviderEvent::Output { content } => {
                    emitted_output = true;
                    assistant_output.push_str(&content);
                    sink.emit(PromptLifecycleEvent::Output { text: content })
                        .await;
                }
                ProviderEvent::ToolCall { call } => {
                    emitted_output = true;
                    {
                        let mut session = durable.lock();
                        session.propose_tool_call_without_delta(
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
                ProviderEvent::Usage { usage } => {
                    trace!(?usage, "Provider token usage");
                    {
                        let mut session = durable.lock();
                        session.token_tracker.record_provider_usage(&usage);
                    }
                }
                ProviderEvent::ChoiceRequest { request } => {
                    trace!(prompt = %request.prompt, "Choice requests are not supported by prompt runner");
                    if !assistant_output.is_empty() {
                        let mut session = durable.lock();
                        session.add_agent_text(&assistant_output);
                    }
                    Self::flush_tool_call_deltas(durable, &tool_calls);
                    return ProviderStreamOutcome::Stop(acp::StopReason::EndTurn);
                }
                ProviderEvent::Complete => {}
                ProviderEvent::Error { source } => {
                    Self::flush_tool_call_deltas(durable, &tool_calls);
                    return Self::handle_provider_stream_error(
                        durable,
                        &mut assistant_output,
                        emitted_output,
                        source,
                    );
                }
            }
        }

        // Add tool-call deltas after the stream so that any ProviderEvent::Usage
        // that arrived mid-stream does not wipe them.
        Self::flush_tool_call_deltas(durable, &tool_calls);

        if !assistant_output.is_empty() {
            let mut session = durable.lock();
            session.add_agent_text(&assistant_output);
        }

        ProviderStreamOutcome::Step(ProviderStep { tool_calls })
    }

    fn handle_provider_stream_error(
        durable: &Arc<Mutex<DurableSession>>,
        assistant_output: &mut String,
        emitted_output: bool,
        error: ProviderError,
    ) -> ProviderStreamOutcome {
        if !emitted_output && error.is_authentication() {
            return ProviderStreamOutcome::AuthFailureBeforeOutput(error);
        }

        let mut session = durable.lock();
        if !assistant_output.is_empty() {
            session.add_agent_text(assistant_output.as_str());
            assistant_output.clear();
        }
        if error.is_authentication() {
            session.add_agent_text(format!("[Provider auth error: {}]", error));
        } else {
            session.add_agent_text(format!("[Provider error: {}]", error));
        }
        ProviderStreamOutcome::Stop(acp::StopReason::EndTurn)
    }

    async fn handle_permission_flow(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        ephemeral: &Arc<Mutex<EphemeralTurn>>,
        sink: &dyn PromptSink,
        tool_calls: &[iron_providers::ToolCall],
        _config: &Config,
        tool_catalog: Option<Arc<SessionToolCatalog>>,
    ) -> Result<Vec<iron_providers::ToolCall>, acp::StopReason> {
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

        for call in tool_calls {
            let tool_requires = tool_catalog.requires_approval(&call.tool_name);
            let requires =
                session_approval_requires_permission(&durable.lock(), _config, tool_requires);

            // Emit tool approval evaluation debug event
            let decision_source = if tool_requires {
                "tool_catalog_requires_approval"
            } else {
                "approval_strategy"
            };
            let (turn_session_id, turn_id) = {
                let turn = ephemeral.lock();
                (turn.session_id, turn.turn_id.clone())
            };
            self.runtime.emit_debug(crate::debug::DebugEvent {
                timestamp: chrono::Utc::now(),
                sequence: self.runtime.next_debug_sequence(),
                severity: crate::debug::DebugSeverity::Info,
                scope: crate::debug::DebugScope {
                    session_id: Some(turn_session_id),
                    turn_id,
                    tool_call_id: Some(call.call_id.clone()),
                    ..crate::debug::DebugScope::default()
                },
                payload: crate::debug::DebugPayload::Tool(
                    crate::debug::ToolDebugEvent::ApprovalEvaluated {
                        tool_name: call.tool_name.clone(),
                        approved: !requires,
                        decision_source: decision_source.to_string(),
                        user_approval_requested: requires,
                        reason: if requires {
                            "Tool requires approval".to_string()
                        } else {
                            "Approval not required".to_string()
                        },
                    },
                ),
            });

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
                    return Err(acp::StopReason::Cancelled);
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

        let mut phase_rx = ephemeral.lock().phase_watcher();
        let approval_fut = sink.request_approval(ApprovalRequest {
            call_id: call_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments: arguments.clone(),
        });

        let verdict = tokio::select! {
            _ = phase_rx.wait_for(|phase| matches!(phase, TurnPhase::Cancelled)) => {
                ApprovalVerdict::Cancelled
            }
            verdict = approval_fut => verdict,
        };

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
        ephemeral: &Arc<Mutex<EphemeralTurn>>,
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
                ephemeral,
                sink,
                &call,
                cancel_token,
                tool_catalog.clone(),
            )
            .await;
            return;
        }

        if call.tool_name == "activate_skill" {
            let turn_id = ephemeral.lock().turn_id.clone();
            self.execute_activate_skill(durable, sink, call, turn_id)
                .await;
            return;
        }

        if call.tool_name == crate::context::compaction::COMPRESS_TOOL_NAME {
            let turn_id = ephemeral.lock().turn_id.clone();
            self.execute_compress_tool(durable, sink, call, turn_id)
                .await;
            return;
        }

        if call.tool_name == "delegate_task" {
            self.execute_delegate_task(durable, sink, call).await;
            return;
        }

        if call.tool_name == "run_task" {
            self.execute_run_task(durable, sink, call).await;
            return;
        }

        let turn_id = ephemeral.lock().turn_id.clone();
        self.execute_standard_tool(durable, sink, call, tool_catalog.as_ref(), turn_id)
            .await;
    }

    async fn execute_compress_tool(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        call: iron_providers::ToolCall,
        turn_id: Option<String>,
    ) {
        let call_id = call.call_id.clone();
        let tool_name = call.tool_name.clone();
        let config = self.runtime.config();
        let session_id = durable.lock().id;

        // Emit compaction requested debug event
        self.runtime.emit_debug(crate::debug::DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: self.runtime.next_debug_sequence(),
            severity: crate::debug::DebugSeverity::Info,
            scope: crate::debug::DebugScope {
                session_id: Some(session_id),
                turn_id: turn_id.clone(),
                tool_call_id: Some(call_id.clone()),
                ..crate::debug::DebugScope::default()
            },
            payload: crate::debug::DebugPayload::Compaction(
                crate::debug::CompactionDebugEvent::Requested {
                    topic_present: call.arguments.get("topic").is_some(),
                    range_count: call
                        .arguments
                        .get("ranges")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0),
                    thresholds: Some(format!(
                        "soft={:.2}, medium={:.2}, strong={:.2}, critical={:.2}",
                        config.context_management.soft_threshold,
                        config.context_management.medium_threshold,
                        config.context_management.strong_threshold,
                        config.context_management.critical_threshold,
                    )),
                },
            ),
        });

        let compaction_id = uuid::Uuid::new_v4().to_string();

        sink.emit(PromptLifecycleEvent::CompactionStarted {
            compaction_id: compaction_id.clone(),
            method: "model_summary".to_string(),
        })
        .await;

        let result = {
            let mut session = durable.lock();
            match crate::context::CompressTool::parse_arguments(&call.arguments).and_then(
                |(topic, ranges)| {
                    crate::context::CompressTool::execute(
                        &mut session,
                        topic,
                        ranges,
                        config.context_management.soft_threshold,
                        config.context_management.medium_threshold,
                        config.context_management.strong_threshold,
                        config.context_management.critical_threshold,
                    )
                },
            ) {
                Ok(result) => {
                    let output = serde_json::json!({
                        "status": "compressed",
                        "tokens_before": result.tokens_before,
                        "tokens_after": result.tokens_after,
                        "method": result.method,
                        "blocks_created": result.blocks_created.iter().map(|block| {
                            serde_json::json!({
                                "id": block.id,
                                "topic": block.topic,
                                "source_range": block.source_range,
                                "tokens_before": block.token_estimate_before,
                                "tokens_after": block.token_estimate_after,
                            })
                        }).collect::<Vec<_>>(),
                        "context_pressure": result.pressure_state,
                    });
                    session.complete_tool_call(&call_id, output.clone());
                    Ok((output, result))
                }
                Err(error) => {
                    let output = serde_json::json!({
                        "status": "rejected",
                        "error": error,
                    });
                    session.fail_tool_call(&call_id, output.clone());
                    Err((output, error.to_string()))
                }
            }
        };

        match result {
            Ok((output, result)) => {
                // Emit compaction applied debug event
                self.runtime.emit_debug(crate::debug::DebugEvent {
                    timestamp: chrono::Utc::now(),
                    sequence: self.runtime.next_debug_sequence(),
                    severity: crate::debug::DebugSeverity::Info,
                    scope: crate::debug::DebugScope {
                        session_id: Some(session_id),
                        turn_id: turn_id.clone(),
                        tool_call_id: Some(call_id.clone()),
                        ..crate::debug::DebugScope::default()
                    },
                    payload: crate::debug::DebugPayload::Compaction(
                        crate::debug::CompactionDebugEvent::Applied {
                            block_count: result.blocks_created.len(),
                            old_size_tokens: result.tokens_before,
                            new_size_tokens: result.tokens_after,
                            method: Some(result.method.clone()),
                            pressure_state: result.pressure_state.clone(),
                            reduction_pct: match (result.tokens_before, result.tokens_after) {
                                (Some(before), Some(after)) if before > 0 => Some(
                                    ((before.saturating_sub(after)) as f64 / before as f64) * 100.0,
                                ),
                                _ => None,
                            },
                        },
                    ),
                });

                sink.emit(PromptLifecycleEvent::CompactionFinished {
                    compaction_id: compaction_id.clone(),
                    tokens_before: result.tokens_before.map(|v| v as u32),
                    tokens_after: result.tokens_after.map(|v| v as u32),
                    method: result.method.clone(),
                })
                .await;

                sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                    call_id,
                    tool_name,
                    status: ToolUpdateStatus::Completed,
                    output: Some(output),
                })
                .await;
            }
            Err((output, error)) => {
                // Emit compaction rejected debug event
                self.runtime.emit_debug(crate::debug::DebugEvent {
                    timestamp: chrono::Utc::now(),
                    sequence: self.runtime.next_debug_sequence(),
                    severity: crate::debug::DebugSeverity::Warning,
                    scope: crate::debug::DebugScope {
                        session_id: Some(session_id),
                        turn_id: turn_id.clone(),
                        tool_call_id: Some(call_id.clone()),
                        ..crate::debug::DebugScope::default()
                    },
                    payload: crate::debug::DebugPayload::Compaction(
                        crate::debug::CompactionDebugEvent::Rejected {
                            reason: error.clone(),
                        },
                    ),
                });

                sink.emit(PromptLifecycleEvent::CompactionFailed {
                    compaction_id: compaction_id.clone(),
                    reason: error,
                })
                .await;

                sink.emit(PromptLifecycleEvent::ToolCallUpdate {
                    call_id,
                    tool_name,
                    status: ToolUpdateStatus::Failed,
                    output: Some(output),
                })
                .await;
            }
        }
    }

    async fn execute_standard_tool(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        call: iron_providers::ToolCall,
        tool_catalog: &SessionToolCatalog,
        turn_id: Option<String>,
    ) {
        let call_id = call.call_id.clone();
        let tool_name = call.tool_name.clone();
        let session_id = durable.lock().id;

        // Emit tool execution started debug event
        self.runtime.emit_debug(crate::debug::DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: self.runtime.next_debug_sequence(),
            severity: crate::debug::DebugSeverity::Info,
            scope: crate::debug::DebugScope {
                session_id: Some(session_id),
                turn_id: turn_id.clone(),
                tool_call_id: Some(call_id.clone()),
                ..crate::debug::DebugScope::default()
            },
            payload: crate::debug::DebugPayload::Tool(
                crate::debug::ToolDebugEvent::ExecutionStarted {
                    tool_name: tool_name.clone(),
                    tool_source: "session_catalog".to_string(),
                    call_id: call_id.clone(),
                },
            ),
        });

        let start_time = std::time::Instant::now();

        let call_id_owned = call.call_id.clone();
        let tool_name_owned = call.tool_name.clone();
        let arguments = call.arguments.clone();
        let execute_future = {
            let session_guard = durable.lock();
            let roots = session_guard.workspace_roots.clone();
            drop(session_guard);
            let builtin_config = self.runtime.session_builtin_tool_config(&roots);
            let session_guard = durable.lock();
            tool_catalog.execute_with_builtin_config(
                &call_id_owned,
                &tool_name_owned,
                arguments,
                &session_guard,
                builtin_config,
            )
        };

        let execute_result = execute_future.await;
        let duration_ms = start_time.elapsed().as_millis() as u64;

        match execute_result {
            Ok(result) => {
                let limited_result = limit_result_size(result);
                let truncated = limited_result
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                // Emit tool execution finished debug event (success)
                self.runtime.emit_debug(crate::debug::DebugEvent {
                    timestamp: chrono::Utc::now(),
                    sequence: self.runtime.next_debug_sequence(),
                    severity: crate::debug::DebugSeverity::Info,
                    scope: crate::debug::DebugScope {
                        session_id: Some(session_id),
                        turn_id: turn_id.clone(),
                        tool_call_id: Some(call_id.clone()),
                        ..crate::debug::DebugScope::default()
                    },
                    payload: crate::debug::DebugPayload::Tool(
                        crate::debug::ToolDebugEvent::ExecutionFinished {
                            tool_name: tool_name.clone(),
                            call_id: call_id.clone(),
                            status: "completed".to_string(),
                            duration_ms: Some(duration_ms),
                            truncated,
                            reason: None,
                        },
                    ),
                });

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

                // Emit tool execution finished debug event (failure)
                self.runtime.emit_debug(crate::debug::DebugEvent {
                    timestamp: chrono::Utc::now(),
                    sequence: self.runtime.next_debug_sequence(),
                    severity: crate::debug::DebugSeverity::Warning,
                    scope: crate::debug::DebugScope {
                        session_id: Some(session_id),
                        turn_id: turn_id.clone(),
                        tool_call_id: Some(call_id.clone()),
                        ..crate::debug::DebugScope::default()
                    },
                    payload: crate::debug::DebugPayload::Tool(
                        crate::debug::ToolDebugEvent::ExecutionFinished {
                            tool_name: tool_name.clone(),
                            call_id: call_id.clone(),
                            status: "failed".to_string(),
                            duration_ms: Some(duration_ms),
                            truncated: false,
                            reason: Some(error.to_string()),
                        },
                    ),
                });
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

    async fn execute_delegate_task(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        call: iron_providers::ToolCall,
    ) {
        let call_id = call.call_id.clone();
        let session_id = durable.lock().id;

        self.runtime.emit_debug(crate::debug::DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: self.runtime.next_debug_sequence(),
            severity: crate::debug::DebugSeverity::Info,
            scope: crate::debug::DebugScope {
                session_id: Some(session_id),
                tool_call_id: Some(call_id.clone()),
                ..crate::debug::DebugScope::default()
            },
            payload: crate::debug::DebugPayload::Tool(
                crate::debug::ToolDebugEvent::ExecutionStarted {
                    tool_name: "delegate_task".to_string(),
                    tool_source: "orchestrator".to_string(),
                    call_id: call_id.clone(),
                },
            ),
        });

        {
            let mut session = durable.lock();
            session.start_tool_call(&call_id, "delegate_task", call.arguments.clone());
        }

        let (result_value, status) = match self.run_delegation_from_call(durable, sink, &call).await
        {
            Ok(result) => {
                let value = limit_result_size(
                    serde_json::to_value(&result)
                        .unwrap_or(serde_json::json!({"error": "serialization failed"})),
                );
                {
                    let mut session = durable.lock();
                    session.complete_tool_call(&call_id, value.clone());
                }
                (value, ToolUpdateStatus::Completed)
            }
            Err(error) => {
                let value = serde_json::json!({"error": error});
                {
                    let mut session = durable.lock();
                    session.fail_tool_call(&call_id, value.clone());
                }
                (value, ToolUpdateStatus::Failed)
            }
        };

        sink.emit(PromptLifecycleEvent::ToolCallUpdate {
            call_id: call_id.clone(),
            tool_name: "delegate_task".to_string(),
            status,
            output: Some(result_value.clone()),
        })
        .await;

        self.runtime.emit_debug(crate::debug::DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: self.runtime.next_debug_sequence(),
            severity: crate::debug::DebugSeverity::Info,
            scope: crate::debug::DebugScope {
                session_id: Some(session_id),
                tool_call_id: Some(call_id.clone()),
                ..crate::debug::DebugScope::default()
            },
            payload: crate::debug::DebugPayload::Tool(
                crate::debug::ToolDebugEvent::ExecutionFinished {
                    tool_name: "delegate_task".to_string(),
                    call_id: call_id.clone(),
                    status: match status {
                        ToolUpdateStatus::Completed => "completed".to_string(),
                        _ => "failed".to_string(),
                    },
                    duration_ms: None,
                    truncated: false,
                    reason: None,
                },
            ),
        });
    }

    async fn run_delegation_from_call(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        call: &iron_providers::ToolCall,
    ) -> Result<crate::delegation::DelegationResult, String> {
        let request = validate_delegation_arguments(&call.arguments)
            .map_err(|e| format!("invalid delegate_task arguments: {}", e))?;

        let parent_session_id = durable.lock().id;

        let parent_client = sink
            .parent_client_channel()
            .ok_or_else(|| "delegate_task requires a client-backed prompt sink".to_string())?;
        let parent_session_acp_id = sink
            .parent_session_acp_id()
            .ok_or_else(|| "delegate_task requires an ACP session id".to_string())?;

        let profile = self
            .resolve_delegation_profile(durable, request.profile_id.clone())
            .await?;

        let (result, _metadata) = self
            .runtime
            .run_delegation(
                parent_session_id,
                Some(call.call_id.clone()),
                request,
                profile,
                parent_client,
                parent_session_acp_id,
            )
            .await?;
        Ok(result)
    }

    async fn resolve_delegation_profile(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        explicit_profile_id: Option<AgentProfileId>,
    ) -> Result<AgentProfile, String> {
        let profile_id = explicit_profile_id
            .or_else(|| durable.lock().profile_id.clone())
            .unwrap_or_else(|| AgentProfileId::from("default"));

        let registry = self
            .runtime
            .profile_registry()
            .ok_or_else(|| "profile registry not configured".to_string())?;
        let profile = {
            let guard = registry.read();
            guard.get(&profile_id).cloned()
        };
        profile.ok_or_else(|| format!("profile '{}' not found", profile_id.as_str()))
    }

    async fn execute_run_task(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        call: iron_providers::ToolCall,
    ) {
        let call_id = call.call_id.clone();
        let session_id = durable.lock().id;

        self.runtime.emit_debug(crate::debug::DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: self.runtime.next_debug_sequence(),
            severity: crate::debug::DebugSeverity::Info,
            scope: crate::debug::DebugScope {
                session_id: Some(session_id),
                tool_call_id: Some(call_id.clone()),
                ..crate::debug::DebugScope::default()
            },
            payload: crate::debug::DebugPayload::Tool(
                crate::debug::ToolDebugEvent::ExecutionStarted {
                    tool_name: "run_task".to_string(),
                    tool_source: "orchestrator".to_string(),
                    call_id: call_id.clone(),
                },
            ),
        });

        {
            let mut session = durable.lock();
            session.start_tool_call(&call_id, "run_task", call.arguments.clone());
        }

        let (result_value, status) = match self.run_stored_prompt_task(durable, sink, &call).await {
            Ok(result) => {
                let value = limit_result_size(
                    serde_json::to_value(&result)
                        .unwrap_or(serde_json::json!({"error": "serialization failed"})),
                );
                {
                    let mut session = durable.lock();
                    session.complete_tool_call(&call_id, value.clone());
                }
                (value, ToolUpdateStatus::Completed)
            }
            Err(error) => {
                let value = serde_json::json!({"error": error});
                {
                    let mut session = durable.lock();
                    session.fail_tool_call(&call_id, value.clone());
                }
                (value, ToolUpdateStatus::Failed)
            }
        };

        sink.emit(PromptLifecycleEvent::ToolCallUpdate {
            call_id: call_id.clone(),
            tool_name: "run_task".to_string(),
            status,
            output: Some(result_value.clone()),
        })
        .await;

        self.runtime.emit_debug(crate::debug::DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: self.runtime.next_debug_sequence(),
            severity: crate::debug::DebugSeverity::Info,
            scope: crate::debug::DebugScope {
                session_id: Some(session_id),
                tool_call_id: Some(call_id.clone()),
                ..crate::debug::DebugScope::default()
            },
            payload: crate::debug::DebugPayload::Tool(
                crate::debug::ToolDebugEvent::ExecutionFinished {
                    tool_name: "run_task".to_string(),
                    call_id: call_id.clone(),
                    status: match status {
                        ToolUpdateStatus::Completed => "completed".to_string(),
                        _ => "failed".to_string(),
                    },
                    duration_ms: None,
                    truncated: false,
                    reason: None,
                },
            ),
        });
    }

    async fn run_stored_prompt_task(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        call: &iron_providers::ToolCall,
    ) -> Result<crate::delegation::DelegationResult, String> {
        let task_id = call
            .arguments
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "run_task requires 'task_id'".to_string())?;

        let context = call
            .arguments
            .get("context")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let child_approval_mode = call
            .arguments
            .get("child_approval_mode")
            .and_then(|v| v.as_str())
            .and_then(|s| match s {
                "auto_approve" => Some(ChildApprovalMode::AutoApprove),
                "propagate_to_parent" => Some(ChildApprovalMode::PropagateToParent),
                _ => None,
            })
            .unwrap_or_default();

        let registry = self
            .runtime
            .stored_prompt_registry()
            .ok_or_else(|| "stored prompt registry not configured".to_string())?;
        let prompt = registry
            .read()
            .get(task_id)
            .cloned()
            .ok_or_else(|| format!("stored prompt '{}' not found", task_id))?;

        let request = DelegationRequest {
            goal: prompt.instructions,
            context,
            profile_id: prompt.profile,
            requested_skills: prompt.skills,
            max_iterations: 10,
            child_approval_mode,
            tool_policy: SubAgentToolPolicy::inherit_parent(),
        };

        let parent_session_id = durable.lock().id;
        let parent_client = sink
            .parent_client_channel()
            .ok_or_else(|| "run_task requires a client-backed prompt sink".to_string())?;
        let parent_session_acp_id = sink
            .parent_session_acp_id()
            .ok_or_else(|| "run_task requires an ACP session id".to_string())?;

        let profile = self
            .resolve_delegation_profile(durable, request.profile_id.clone())
            .await?;

        let (result, _metadata) = self
            .runtime
            .run_delegation(
                parent_session_id,
                Some(call.call_id.clone()),
                request,
                profile,
                parent_client,
                parent_session_acp_id,
            )
            .await?;
        Ok(result)
    }
    async fn execute_activate_skill(
        &self,
        durable: &Arc<Mutex<DurableSession>>,
        sink: &dyn PromptSink,
        call: iron_providers::ToolCall,
        turn_id: Option<String>,
    ) {
        let call_id = call.call_id.clone();
        let session_id = durable.lock().id;
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
            self.runtime.emit_debug(crate::debug::DebugEvent {
                timestamp: chrono::Utc::now(),
                sequence: self.runtime.next_debug_sequence(),
                severity: crate::debug::DebugSeverity::Warning,
                scope: crate::debug::DebugScope {
                    session_id: Some(session_id),
                    turn_id: turn_id.clone(),
                    tool_call_id: Some(call_id.clone()),
                    ..crate::debug::DebugScope::default()
                },
                payload: crate::debug::DebugPayload::Skill(
                    crate::debug::SkillDebugEvent::ActivationRejected {
                        skill_name: "".to_string(),
                        reason: "missing_argument".to_string(),
                    },
                ),
            });
            return;
        }

        let maybe_skill = {
            let session = durable.lock();
            session
                .load_available_skill(skill_name)
                .map(|skill| (skill, session.is_skill_active(skill_name)))
        };
        let (skill, already_active) = match maybe_skill {
            Some(skill) => skill,
            None => {
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
                self.runtime.emit_debug(crate::debug::DebugEvent {
                    timestamp: chrono::Utc::now(),
                    sequence: self.runtime.next_debug_sequence(),
                    severity: crate::debug::DebugSeverity::Warning,
                    scope: crate::debug::DebugScope {
                        session_id: Some(session_id),
                        turn_id: turn_id.clone(),
                        tool_call_id: Some(call_id.clone()),
                        ..crate::debug::DebugScope::default()
                    },
                    payload: crate::debug::DebugPayload::Skill(
                        crate::debug::SkillDebugEvent::ActivationRejected {
                            skill_name: skill_name.to_string(),
                            reason: "unavailable".to_string(),
                        },
                    ),
                });
                return;
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
            self.runtime.emit_debug(crate::debug::DebugEvent {
                timestamp: chrono::Utc::now(),
                sequence: self.runtime.next_debug_sequence(),
                severity: crate::debug::DebugSeverity::Warning,
                scope: crate::debug::DebugScope {
                    session_id: Some(session_id),
                    turn_id: turn_id.clone(),
                    tool_call_id: Some(call_id.clone()),
                    ..crate::debug::DebugScope::default()
                },
                payload: crate::debug::DebugPayload::Skill(
                    crate::debug::SkillDebugEvent::ActivationRejected {
                        skill_name: skill_name.to_string(),
                        reason: "trust_required".to_string(),
                    },
                ),
            });
            return;
        }

        {
            let mut session = durable.lock();
            session.start_tool_call(&call_id, "activate_skill", call.arguments.clone());
            if !already_active {
                session.activate_skill(&skill.metadata.id, &skill.body, skill.resources.clone());
            }
        }

        self.runtime.emit_debug(crate::debug::DebugEvent {
            timestamp: chrono::Utc::now(),
            sequence: self.runtime.next_debug_sequence(),
            severity: crate::debug::DebugSeverity::Info,
            scope: crate::debug::DebugScope {
                session_id: Some(session_id),
                turn_id,
                tool_call_id: Some(call_id.clone()),
                ..crate::debug::DebugScope::default()
            },
            payload: crate::debug::DebugPayload::Skill(
                crate::debug::SkillDebugEvent::ActivationSuccess {
                    skill_name: skill.metadata.id.clone(),
                    source_kind: format!("{:?}", skill.metadata.origin),
                    activation_source: if already_active {
                        "already_active".to_string()
                    } else {
                        "model_request".to_string()
                    },
                },
            ),
        });

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
                            let roots = session_guard.workspace_roots.clone();
                            drop(session_guard);
                            let builtin_config = self.runtime.session_builtin_tool_config(&roots);
                            let session_guard = durable.lock();
                            session_tool_catalog.execute_with_builtin_config(
                                &req.call_id,
                                &req.tool_name,
                                req.args.clone(),
                                &session_guard,
                                builtin_config,
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
                    let requires_permission = session_approval_requires_permission(
                        &durable.lock(),
                        self.runtime.config(),
                        tool_requires,
                    );
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
                        let roots = session_guard.workspace_roots.clone();
                        drop(session_guard);
                        let builtin_config = self.runtime.session_builtin_tool_config(&roots);
                        let session_guard = durable.lock();
                        session_tool_catalog.execute_with_builtin_config(
                            &req.call_id,
                            &req.tool_name,
                            req.args.clone(),
                            &session_guard,
                            builtin_config,
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
        client: &SharedClientChannel,
        acp_session_id: &acp::SessionId,
    ) {
        if !config.context_management.enabled {
            return;
        }

        let pressure = {
            let session = durable.lock();
            let messages = session.to_transcript().messages;
            let snapshot = crate::context::ActiveContextAccountant::estimate_snapshot(
                session.instruction_text_for_estimate().as_deref(),
                &session.compressed_blocks,
                &messages,
                &crate::tool::ToolRegistry::new(),
                None,
                config.context_management.context_window_hint,
                crate::context::SessionModelInfo {
                    current_model: session.current_model.as_deref(),
                    model_switch_count: session.model_switch_history.len(),
                },
                Some(&session.token_tracker),
            );
            snapshot.pressure_with_thresholds(
                config.context_management.soft_threshold,
                config.context_management.medium_threshold,
                config.context_management.strong_threshold,
                config.context_management.critical_threshold,
            )
        };

        if matches!(pressure, crate::context::ContextPressure::Critical) {
            let error_text = "Context pressure remains critical. Compression did not reduce usage below the threshold. Please start a new session to continue.";
            {
                let mut session = durable.lock();
                session.add_agent_text(error_text.to_string());
            }

            let update = acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                acp::ContentBlock::Text(acp::TextContent::new(error_text)),
            ));
            let notification = acp::SessionNotification::new(acp_session_id.clone(), update);
            let _ = client.send_notification(notification).await;
        }
    }
}

struct ProviderStep {
    tool_calls: Vec<iron_providers::ToolCall>,
}

enum ProviderStreamOutcome {
    Step(ProviderStep),
    Stop(acp::StopReason),
    AuthFailureBeforeOutput(ProviderError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::durable::{DurableSession, SessionId};
    use crate::ephemeral::EphemeralTurn;
    use crate::prompt_lifecycle::PromptLifecycleEvent;
    use crate::provider_credential::domain::ProviderPromptContext;
    use crate::runtime::IronRuntime;
    use futures::stream::{self, BoxStream};
    use futures::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Arc;

    struct NopSink;

    impl crate::prompt_lifecycle::PromptSink for NopSink {
        fn emit(
            &self,
            _event: PromptLifecycleEvent,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> {
            Box::pin(async {})
        }

        fn request_approval(
            &self,
            _request: crate::prompt_lifecycle::ApprovalRequest,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = crate::prompt_lifecycle::ApprovalVerdict>>,
        > {
            Box::pin(async { crate::prompt_lifecycle::ApprovalVerdict::AllowOnce })
        }
    }

    #[derive(Clone, Default)]
    struct AuthFailProvider {
        call_count: Arc<AtomicUsize>,
    }

    impl Provider for AuthFailProvider {
        fn infer(
            &self,
            _request: iron_providers::InferenceRequest,
        ) -> iron_providers::ProviderFuture<'_, Vec<iron_providers::ProviderEvent>> {
            Box::pin(async move { Ok(vec![iron_providers::ProviderEvent::Complete]) })
        }

        fn infer_stream(
            &self,
            _request: iron_providers::InferenceRequest,
        ) -> iron_providers::ProviderFuture<
            '_,
            BoxStream<'static, iron_providers::ProviderResult<iron_providers::ProviderEvent>>,
        > {
            let count = self.call_count.fetch_add(1, AtomicOrdering::SeqCst);
            Box::pin(async move {
                if count == 0 {
                    Err(iron_providers::ProviderError::auth("test auth failure"))
                } else {
                    Ok(stream::iter(vec![Ok(iron_providers::ProviderEvent::Complete)]).boxed())
                }
            })
        }
    }

    #[derive(Clone, Default)]
    struct TransportFailProvider;

    impl Provider for TransportFailProvider {
        fn infer(
            &self,
            _request: iron_providers::InferenceRequest,
        ) -> iron_providers::ProviderFuture<'_, Vec<iron_providers::ProviderEvent>> {
            Box::pin(async move { Ok(vec![iron_providers::ProviderEvent::Complete]) })
        }

        fn infer_stream(
            &self,
            _request: iron_providers::InferenceRequest,
        ) -> iron_providers::ProviderFuture<
            '_,
            BoxStream<'static, iron_providers::ProviderResult<iron_providers::ProviderEvent>>,
        > {
            Box::pin(async move {
                Err(iron_providers::ProviderError::transport(
                    "connection refused",
                ))
            })
        }
    }

    #[derive(Clone)]
    struct StreamEventsProvider {
        events: Arc<Vec<iron_providers::ProviderResult<iron_providers::ProviderEvent>>>,
        call_count: Arc<AtomicUsize>,
    }

    impl StreamEventsProvider {
        fn new(events: Vec<iron_providers::ProviderResult<iron_providers::ProviderEvent>>) -> Self {
            Self {
                events: Arc::new(events),
                call_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn call_count(&self) -> usize {
            self.call_count.load(AtomicOrdering::SeqCst)
        }
    }

    impl Provider for StreamEventsProvider {
        fn infer(
            &self,
            _request: iron_providers::InferenceRequest,
        ) -> iron_providers::ProviderFuture<'_, Vec<iron_providers::ProviderEvent>> {
            Box::pin(async move { Ok(vec![iron_providers::ProviderEvent::Complete]) })
        }

        fn infer_stream(
            &self,
            _request: iron_providers::InferenceRequest,
        ) -> iron_providers::ProviderFuture<
            '_,
            BoxStream<'static, iron_providers::ProviderResult<iron_providers::ProviderEvent>>,
        > {
            self.call_count.fetch_add(1, AtomicOrdering::SeqCst);
            let events = (*self.events).clone();
            Box::pin(async move { Ok(stream::iter(events).boxed()) })
        }
    }

    fn make_session_and_ephemeral() -> (Arc<Mutex<DurableSession>>, Arc<Mutex<EphemeralTurn>>) {
        let session_id = SessionId::new();
        let durable = Arc::new(Mutex::new(DurableSession::new(session_id)));
        let ephemeral = Arc::new(Mutex::new(EphemeralTurn::new(session_id, None)));
        ephemeral.lock().start();
        (durable, ephemeral)
    }

    fn agent_text(durable: &Arc<Mutex<DurableSession>>) -> String {
        let session = durable.lock();
        session
            .to_transcript()
            .messages
            .iter()
            .filter_map(|m| match m {
                iron_providers::Message::Assistant { content } => Some(content.clone()),
                _ => None,
            })
            .collect::<String>()
    }

    #[tokio::test]
    async fn debug_prompt_and_context_events_include_turn_id() {
        let runtime = IronRuntime::new(
            Config::default(),
            StreamEventsProvider::new(vec![Ok(iron_providers::ProviderEvent::Complete)]),
        );
        let debug_sink = Arc::new(crate::debug::test_helpers::RecordingDebugSink::new());
        runtime.set_debug_sink(Some(debug_sink.clone()));
        let _ = debug_sink.take_events();

        let runner = PromptRunner::new(runtime);
        let session_id = SessionId::new();
        let durable = Arc::new(Mutex::new(DurableSession::new(session_id)));
        let ephemeral = Arc::new(Mutex::new(EphemeralTurn::new(
            session_id,
            Some("turn-test".to_string()),
        )));
        ephemeral.lock().start();

        let _ = runner
            .run(&durable, &ephemeral, &NopSink, &Config::default(), 1)
            .await;

        let events = debug_sink.events();
        assert!(
            events.iter().any(|event| matches!(
                event.payload,
                crate::debug::DebugPayload::Context(
                    crate::debug::ContextDebugEvent::SnapshotEstimated { .. }
                ) if event.scope.turn_id.as_deref() == Some("turn-test")
            )),
            "context snapshot debug event should include the active turn_id"
        );
        assert!(
            events.iter().any(|event| matches!(
                event.payload,
                crate::debug::DebugPayload::Prompt(
                    crate::debug::PromptDebugEvent::SystemPromptRendered { .. }
                ) if event.scope.turn_id.as_deref() == Some("turn-test")
            )),
            "system prompt debug event should include the active turn_id"
        );
    }

    #[tokio::test]
    async fn debug_tool_events_include_turn_id() {
        let config =
            Config::default().with_approval_strategy(crate::config::ApprovalStrategy::Always);
        let runtime = IronRuntime::new(
            config.clone(),
            StreamEventsProvider::new(vec![
                Ok(iron_providers::ProviderEvent::ToolCall {
                    call: iron_providers::ToolCall {
                        call_id: "call-1".to_string(),
                        tool_name: "unknown_tool".to_string(),
                        arguments: serde_json::json!({}),
                    },
                }),
                Ok(iron_providers::ProviderEvent::Complete),
            ]),
        );
        let debug_sink = Arc::new(crate::debug::test_helpers::RecordingDebugSink::new());
        runtime.set_debug_sink(Some(debug_sink.clone()));
        let (session_id, durable) = runtime.create_session(crate::ConnectionId(1)).unwrap();
        let ephemeral = runtime.try_start_prompt(session_id).unwrap();
        let turn_id = ephemeral.lock().turn_id.clone();
        let _ = debug_sink.take_events();

        let runner = PromptRunner::new(runtime);
        let _ = runner.run(&durable, &ephemeral, &NopSink, &config, 1).await;

        let events = debug_sink.events();
        assert!(
            events.iter().any(|event| matches!(
                event.payload,
                crate::debug::DebugPayload::Tool(
                    crate::debug::ToolDebugEvent::ApprovalEvaluated { .. }
                ) if event.scope.turn_id == turn_id
            )),
            "tool approval debug event should include the active turn_id"
        );
    }

    #[tokio::test]
    async fn non_managed_no_retry_on_auth_error() {
        let provider = AuthFailProvider {
            call_count: Arc::new(AtomicUsize::new(0)),
        };
        let runtime = IronRuntime::new(Config::default(), provider);
        let runner = PromptRunner::new(runtime);
        let (durable, ephemeral) = make_session_and_ephemeral();
        let sink = NopSink;
        let config = Config::default();

        let stop = runner.run(&durable, &ephemeral, &sink, &config, 1).await;

        assert_eq!(stop, acp::StopReason::EndTurn);
        let agent_text = agent_text(&durable);
        assert!(agent_text.contains("Provider error"));
        assert!(!agent_text.contains("refresh"));
    }

    #[tokio::test]
    async fn managed_no_retry_on_transport_error() {
        let runtime = IronRuntime::new(Config::default(), TransportFailProvider);
        let managed_provider = Box::new(TransportFailProvider) as Box<dyn Provider>;
        let context = ProviderPromptContext {
            provider_slug: crate::provider_credential::ProviderSlug::new("codex"),
            model: "test".into(),
            api_key: None,
        };
        let runner = PromptRunner::new_managed(runtime, managed_provider, context);
        let (durable, ephemeral) = make_session_and_ephemeral();
        let sink = NopSink;
        let config = Config::default();

        let stop = runner.run(&durable, &ephemeral, &sink, &config, 1).await;

        assert_eq!(stop, acp::StopReason::EndTurn);
        let agent_text = agent_text(&durable);
        assert!(agent_text.contains("Provider error"));
        assert!(!agent_text.contains("auth"));
    }

    #[tokio::test]
    async fn managed_retry_attempted_on_auth_error_with_credential_store() {
        use crate::provider_credential::store::InMemoryCredentialStore;
        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let provider = AuthFailProvider {
            call_count: Arc::new(AtomicUsize::new(0)),
        };
        let runtime = IronRuntime::new_with_credential_store(Config::default(), provider, store);
        let managed_provider = Box::new(AuthFailProvider {
            call_count: Arc::new(AtomicUsize::new(0)),
        }) as Box<dyn Provider>;
        let context = ProviderPromptContext {
            provider_slug: crate::provider_credential::ProviderSlug::new("codex"),
            model: "test".into(),
            api_key: None,
        };
        let runner = PromptRunner::new_managed(runtime, managed_provider, context);
        let (durable, ephemeral) = make_session_and_ephemeral();
        let sink = NopSink;
        let config = Config::default();

        let stop = runner.run(&durable, &ephemeral, &sink, &config, 1).await;

        assert_eq!(stop, acp::StopReason::EndTurn);
        let agent_text = agent_text(&durable);
        assert!(
            agent_text.contains("auth error") || agent_text.contains("NotConfigured"),
            "expected auth error or NotConfigured in transcript, got: {}",
            agent_text
        );
    }

    #[tokio::test]
    async fn managed_retries_stream_auth_failure_before_output() {
        use crate::provider_credential::store::InMemoryCredentialStore;

        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let runtime = IronRuntime::new_with_credential_store(
            Config::default(),
            StreamEventsProvider::new(vec![Ok(iron_providers::ProviderEvent::Complete)]),
            store,
        );
        let initial_provider = StreamEventsProvider::new(vec![Err(
            iron_providers::ProviderError::auth("expired access token"),
        )]);
        let retry_provider = StreamEventsProvider::new(vec![
            Ok(iron_providers::ProviderEvent::Output {
                content: "retry ok".into(),
            }),
            Ok(iron_providers::ProviderEvent::Complete),
        ]);
        let retry_calls = retry_provider.call_count.clone();
        let context = ProviderPromptContext {
            provider_slug: crate::provider_credential::ProviderSlug::new("codex"),
            model: "test".into(),
            api_key: None,
        };
        let runner = PromptRunner::new_managed_with_retry_provider_for_test(
            runtime,
            Box::new(initial_provider.clone()),
            context,
            Arc::new(move || Box::new(retry_provider.clone())),
        );
        let (durable, ephemeral) = make_session_and_ephemeral();
        let sink = NopSink;

        let stop = runner
            .run(&durable, &ephemeral, &sink, &Config::default(), 1)
            .await;

        assert_eq!(stop, acp::StopReason::EndTurn);
        assert_eq!(initial_provider.call_count(), 1);
        assert_eq!(retry_calls.load(AtomicOrdering::SeqCst), 1);
        assert!(agent_text(&durable).contains("retry ok"));
    }

    #[tokio::test]
    async fn managed_does_not_retry_stream_auth_failure_after_output() {
        use crate::provider_credential::store::InMemoryCredentialStore;

        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let runtime = IronRuntime::new_with_credential_store(
            Config::default(),
            StreamEventsProvider::new(vec![Ok(iron_providers::ProviderEvent::Complete)]),
            store,
        );
        let initial_provider = StreamEventsProvider::new(vec![
            Ok(iron_providers::ProviderEvent::Output {
                content: "partial".into(),
            }),
            Err(iron_providers::ProviderError::auth("expired access token")),
        ]);
        let retry_calls = Arc::new(AtomicUsize::new(0));
        let retry_calls_for_factory = retry_calls.clone();
        let context = ProviderPromptContext {
            provider_slug: crate::provider_credential::ProviderSlug::new("codex"),
            model: "test".into(),
            api_key: None,
        };
        let runner = PromptRunner::new_managed_with_retry_provider_for_test(
            runtime,
            Box::new(initial_provider.clone()),
            context,
            Arc::new(move || {
                retry_calls_for_factory.fetch_add(1, AtomicOrdering::SeqCst);
                Box::new(StreamEventsProvider::new(vec![Ok(
                    iron_providers::ProviderEvent::Complete,
                )]))
            }),
        );
        let (durable, ephemeral) = make_session_and_ephemeral();
        let sink = NopSink;

        let stop = runner
            .run(&durable, &ephemeral, &sink, &Config::default(), 1)
            .await;

        let text = agent_text(&durable);
        assert_eq!(stop, acp::StopReason::EndTurn);
        assert_eq!(initial_provider.call_count(), 1);
        assert_eq!(retry_calls.load(AtomicOrdering::SeqCst), 0);
        assert!(text.contains("partial"));
        assert!(text.contains("Provider auth error"));
    }

    #[tokio::test]
    async fn managed_does_not_retry_api_key_backed_auth_failure() {
        use crate::provider_credential::store::InMemoryCredentialStore;

        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let runtime = IronRuntime::new_with_credential_store(
            Config::default(),
            StreamEventsProvider::new(vec![Ok(iron_providers::ProviderEvent::Complete)]),
            store,
        );
        let initial_provider = StreamEventsProvider::new(vec![Err(
            iron_providers::ProviderError::auth("bad api key"),
        )]);
        let retry_calls = Arc::new(AtomicUsize::new(0));
        let retry_calls_for_factory = retry_calls.clone();
        let context = ProviderPromptContext {
            provider_slug: crate::provider_credential::ProviderSlug::new("kimi-code"),
            model: "test".into(),
            api_key: Some("sk-test".into()),
        };
        let runner = PromptRunner::new_managed_with_retry_provider_for_test(
            runtime,
            Box::new(initial_provider.clone()),
            context,
            Arc::new(move || {
                retry_calls_for_factory.fetch_add(1, AtomicOrdering::SeqCst);
                Box::new(StreamEventsProvider::new(vec![Ok(
                    iron_providers::ProviderEvent::Complete,
                )]))
            }),
        );
        let (durable, ephemeral) = make_session_and_ephemeral();
        let sink = NopSink;

        let stop = runner
            .run(&durable, &ephemeral, &sink, &Config::default(), 1)
            .await;

        assert_eq!(stop, acp::StopReason::EndTurn);
        assert_eq!(initial_provider.call_count(), 1);
        assert_eq!(retry_calls.load(AtomicOrdering::SeqCst), 0);
        assert!(agent_text(&durable).contains("Provider auth error"));
    }

    #[tokio::test]
    async fn managed_reports_repeated_auth_failure_after_retry() {
        use crate::provider_credential::store::InMemoryCredentialStore;

        let store: std::sync::Arc<InMemoryCredentialStore> =
            std::sync::Arc::new(InMemoryCredentialStore::new());
        let runtime = IronRuntime::new_with_credential_store(
            Config::default(),
            StreamEventsProvider::new(vec![Ok(iron_providers::ProviderEvent::Complete)]),
            store,
        );
        let initial_provider = StreamEventsProvider::new(vec![Err(
            iron_providers::ProviderError::auth("expired access token"),
        )]);
        let retry_provider = StreamEventsProvider::new(vec![Err(
            iron_providers::ProviderError::auth("revoked token"),
        )]);
        let context = ProviderPromptContext {
            provider_slug: crate::provider_credential::ProviderSlug::new("codex"),
            model: "test".into(),
            api_key: None,
        };
        let runner = PromptRunner::new_managed_with_retry_provider_for_test(
            runtime,
            Box::new(initial_provider.clone()),
            context,
            Arc::new(move || Box::new(retry_provider.clone())),
        );
        let (durable, ephemeral) = make_session_and_ephemeral();
        let sink = NopSink;

        let stop = runner
            .run(&durable, &ephemeral, &sink, &Config::default(), 1)
            .await;

        let text = agent_text(&durable);
        assert_eq!(stop, acp::StopReason::EndTurn);
        assert!(text.contains("Provider auth error"));
        assert!(text.contains("revoked token"));
    }
}
