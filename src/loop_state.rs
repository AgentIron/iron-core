//! AgentLoop state machine
//!
//! The core loop implementing the sequence from AGENTS.md:
//! 1. Context Loading
//! 2. Provider Inference
//! 3. Action Selection
//! 4. Approval Gating
//! 5. Tool Execution
//! 6. Loop Continuation

#![allow(deprecated)]
use crate::{
    config::Config,
    error::{LoopError, LoopResult},
    events::StreamEvent,
    session::Session,
    tool::ToolRegistry,
};
use futures::stream::{self, BoxStream, StreamExt};
use iron_providers::{InferenceRequest, Provider, ProviderEvent, ToolCall};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, trace};

const PENDING_APPROVAL_KEY: &str = "agent_loop_pending_approval";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingApproval {
    iteration: u32,
    calls: Vec<ToolCall>,
}

#[derive(Debug)]
struct ProviderStep {
    turn_complete: bool,
    tool_calls: Vec<ToolCall>,
}

/// The AgentLoop state machine.
pub struct AgentLoop {
    config: Config,
    provider: Arc<dyn Provider>,
    tool_registry: ToolRegistry,
}

impl AgentLoop {
    /// Create a new AgentLoop with the given configuration and provider.
    pub fn new<P>(config: Config, provider: P) -> Self
    where
        P: Provider + 'static,
    {
        Self {
            config,
            provider: Arc::new(provider),
            tool_registry: ToolRegistry::new(),
        }
    }

    /// Create a new AgentLoop with a tool registry.
    pub fn with_tools<P>(config: Config, provider: P, tool_registry: ToolRegistry) -> Self
    where
        P: Provider + 'static,
    {
        Self {
            config,
            provider: Arc::new(provider),
            tool_registry,
        }
    }

    /// Create a new AgentLoop from an existing provider handle.
    pub fn from_shared_provider(
        config: Config,
        provider: Arc<dyn Provider>,
        tool_registry: ToolRegistry,
    ) -> Self {
        Self {
            config,
            provider,
            tool_registry,
        }
    }

    /// Get the configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Get the provider.
    pub fn provider(&self) -> &dyn Provider {
        self.provider.as_ref()
    }

    /// Get the tool registry (mutable).
    pub fn tools_mut(&mut self) -> &mut ToolRegistry {
        &mut self.tool_registry
    }

    /// Get the tool registry (immutable).
    pub fn tools(&self) -> &ToolRegistry {
        &self.tool_registry
    }

    /// Run a single turn with the given user input.
    pub async fn run_turn(
        &self,
        session: &mut Session,
        user_input: &str,
    ) -> LoopResult<Vec<StreamEvent>> {
        if Self::has_pending_approval(session) {
            return Err(LoopError::session(
                "Cannot start a new turn while awaiting approval",
            ));
        }

        session.add_user_message(user_input);
        self.run_loop(session, 0, None, false).await
    }

    /// Resume a paused turn after an approval decision.
    pub async fn resume_turn(
        &self,
        session: &mut Session,
        approved: bool,
    ) -> LoopResult<Vec<StreamEvent>> {
        let pending = Self::take_pending_approval(session)?;
        self.run_loop(
            session,
            pending.iteration,
            Some((pending.calls, approved)),
            false,
        )
        .await
    }

    /// Run a turn using the provider streaming API.
    ///
    /// This method preserves session consistency by materializing the stream events first,
    /// then returning them as a stream to the caller.
    pub async fn run_turn_stream(
        &self,
        session: &mut Session,
        user_input: &str,
    ) -> LoopResult<BoxStream<'static, StreamEvent>> {
        if Self::has_pending_approval(session) {
            return Err(LoopError::session(
                "Cannot start a new turn while awaiting approval",
            ));
        }

        session.add_user_message(user_input);
        let events = self.run_loop(session, 0, None, true).await?;
        Ok(stream::iter(events).boxed())
    }

    /// Resume a paused streaming turn after an approval decision.
    pub async fn resume_turn_stream(
        &self,
        session: &mut Session,
        approved: bool,
    ) -> LoopResult<BoxStream<'static, StreamEvent>> {
        let pending = Self::take_pending_approval(session)?;
        let events = self
            .run_loop(
                session,
                pending.iteration,
                Some((pending.calls, approved)),
                true,
            )
            .await?;
        Ok(stream::iter(events).boxed())
    }

    async fn run_loop(
        &self,
        session: &mut Session,
        mut iteration: u32,
        mut approval_resume: Option<(Vec<ToolCall>, bool)>,
        use_streaming_provider: bool,
    ) -> LoopResult<Vec<StreamEvent>> {
        let mut events = Vec::new();

        loop {
            if let Some((calls, approved)) = approval_resume.take() {
                self.execute_tool_batch(session, &mut events, calls, approved)
                    .await;
            }

            if iteration >= self.config.max_iterations {
                events.push(StreamEvent::max_iterations(iteration));
                return Ok(events);
            }
            iteration += 1;

            trace!("Starting iteration {}", iteration);

            let request = self.prepare_request(session)?;
            debug!(
                "Prepared inference request with {} messages",
                request.transcript.messages.len()
            );

            events.push(StreamEvent::status("Thinking..."));
            let provider_events = if use_streaming_provider {
                self.collect_streaming_events(request).await?
            } else {
                self.provider
                    .infer(request)
                    .await
                    .map_err(|e| LoopError::provider(e.to_string()))?
            };

            let step = self
                .process_provider_events(session, &mut events, provider_events, iteration)
                .await?;

            if step.tool_calls.is_empty() {
                if step.turn_complete {
                    events.push(StreamEvent::complete());
                    return Ok(events);
                }
                continue;
            }

            let requires_approval = step.tool_calls.iter().any(|call| {
                self.tool_registry
                    .get(&call.tool_name)
                    .map(|tool| {
                        self.config
                            .default_approval_strategy
                            .is_approval_required(tool.requires_approval())
                    })
                    .unwrap_or(false)
            });

            if requires_approval {
                self.store_pending_approval(session, iteration, &step.tool_calls)?;
                for call in &step.tool_calls {
                    if let Some(tool) = self.tool_registry.get(&call.tool_name) {
                        if self
                            .config
                            .default_approval_strategy
                            .is_approval_required(tool.requires_approval())
                        {
                            events.push(StreamEvent::approval_request(
                                call.call_id.clone(),
                                call.tool_name.clone(),
                                call.arguments.clone(),
                            ));
                        }
                    }
                }
                return Ok(events);
            }

            self.execute_tool_batch(session, &mut events, step.tool_calls, true)
                .await;
        }
    }

    async fn collect_streaming_events(
        &self,
        request: InferenceRequest,
    ) -> LoopResult<Vec<ProviderEvent>> {
        let mut stream = self
            .provider
            .infer_stream(request)
            .await
            .map_err(|e| LoopError::provider(e.to_string()))?;
        let mut events = Vec::new();

        while let Some(result) = stream.next().await {
            let event = result.map_err(|e| LoopError::provider(e.to_string()))?;
            events.push(event);
        }

        Ok(events)
    }

    async fn process_provider_events(
        &self,
        session: &mut Session,
        events: &mut Vec<StreamEvent>,
        provider_events: Vec<ProviderEvent>,
        iteration: u32,
    ) -> LoopResult<ProviderStep> {
        let mut turn_complete = false;
        let mut tool_calls = Vec::new();
        let mut assistant_output = String::new();

        for event in provider_events {
            match event {
                ProviderEvent::Status { message } => {
                    events.push(StreamEvent::Status { message });
                }
                ProviderEvent::Output { content } => {
                    assistant_output.push_str(&content);
                    events.push(StreamEvent::Output { content });
                }
                ProviderEvent::ToolCall { call } => {
                    events.push(StreamEvent::tool_call(
                        call.call_id.clone(),
                        call.tool_name.clone(),
                        call.arguments.clone(),
                    ));

                    if self.tool_registry.get(&call.tool_name).is_some() {
                        tool_calls.push(call);
                    } else {
                        let error_result = serde_json::json!({
                            "error": format!("Tool '{}' not found", call.tool_name)
                        });
                        events.push(StreamEvent::tool_result(
                            call.call_id.clone(),
                            call.tool_name.clone(),
                            error_result.clone(),
                        ));
                        session.add_tool_result(call.call_id, call.tool_name, error_result);
                    }
                }
                ProviderEvent::Complete => {
                    turn_complete = true;
                }
                ProviderEvent::Error { message } => {
                    events.push(StreamEvent::error(message));
                    if !assistant_output.is_empty() {
                        session.add_assistant_message(assistant_output);
                    }
                    return Ok(ProviderStep {
                        turn_complete: true,
                        tool_calls: Vec::new(),
                    });
                }
            }
        }

        if !assistant_output.is_empty() {
            session.add_assistant_message(assistant_output);
        }

        let _ = iteration;

        Ok(ProviderStep {
            turn_complete,
            tool_calls,
        })
    }

    async fn execute_tool_batch(
        &self,
        session: &mut Session,
        events: &mut Vec<StreamEvent>,
        calls: Vec<ToolCall>,
        approved: bool,
    ) {
        for call in calls {
            if !approved {
                let denied = serde_json::json!({
                    "error": "Tool execution denied by user"
                });
                events.push(StreamEvent::tool_result(
                    call.call_id.clone(),
                    call.tool_name.clone(),
                    denied.clone(),
                ));
                session.add_tool_result(call.call_id, call.tool_name, denied);
                continue;
            }

            let Some(tool) = self.tool_registry.get(&call.tool_name) else {
                let error_result = serde_json::json!({
                    "error": format!("Tool '{}' not found", call.tool_name)
                });
                events.push(StreamEvent::tool_result(
                    call.call_id.clone(),
                    call.tool_name.clone(),
                    error_result.clone(),
                ));
                session.add_tool_result(call.call_id, call.tool_name, error_result);
                continue;
            };

            match tool.execute(&call.call_id, call.arguments.clone()).await {
                Ok(result) => {
                    events.push(StreamEvent::tool_result(
                        call.call_id.clone(),
                        call.tool_name.clone(),
                        result.clone(),
                    ));
                    session.add_tool_result(call.call_id, call.tool_name, result);
                }
                Err(error) => {
                    let result = serde_json::json!({
                        "error": error.to_string()
                    });
                    events.push(StreamEvent::tool_result(
                        call.call_id.clone(),
                        call.tool_name.clone(),
                        result.clone(),
                    ));
                    session.add_tool_result(call.call_id, call.tool_name, result);
                }
            }
        }
    }

    /// Prepare an inference request from the current session state.
    fn prepare_request(&self, session: &Session) -> LoopResult<InferenceRequest> {
        let instructions = session.instructions.as_deref();
        let messages = &session.to_transcript().messages;
        crate::request_builder::build_inference_request(
            &self.config,
            messages,
            instructions,
            &self.tool_registry,
        )
    }

    fn has_pending_approval(session: &Session) -> bool {
        session.metadata.contains_key(PENDING_APPROVAL_KEY)
    }

    fn store_pending_approval(
        &self,
        session: &mut Session,
        iteration: u32,
        calls: &[ToolCall],
    ) -> LoopResult<()> {
        let pending = PendingApproval {
            iteration,
            calls: calls.to_vec(),
        };
        let value = serde_json::to_value(pending)
            .map_err(|e| LoopError::session(format!("Failed to store pending approval: {}", e)))?;
        session
            .metadata
            .insert(PENDING_APPROVAL_KEY.to_string(), value);
        Ok(())
    }

    fn take_pending_approval(session: &mut Session) -> LoopResult<PendingApproval> {
        let value = session
            .metadata
            .remove(PENDING_APPROVAL_KEY)
            .ok_or_else(|| LoopError::approval_required("no pending approval"))?;
        serde_json::from_value(value)
            .map_err(|e| LoopError::session(format!("Failed to load pending approval: {}", e)))
    }
}
