#![allow(deprecated)]
use crate::{
    config::{Config, ContextWindowPolicy},
    error::LoopError,
    events::{
        ApprovalCallInfo, ApprovalInteractionInfo, ApprovalVerdict, ChoiceInteractionInfo,
        ChoiceItem, ChoiceResolutionItem, ChoiceResolutionRecord, ChoiceSelectionMode,
        InteractionResolution, InteractionSource, PendingInteractionInfo,
        PendingInteractionPayload, TurnEvent, TurnId, TurnOutcome, TurnStatus,
    },
    session::Session,
    session_runtime::SessionRuntime,
    tool::ToolRegistry,
};
use futures::StreamExt;
use iron_providers::{
    ChoiceRequest as ProviderChoiceRequest, ChoiceSelectionMode as ProviderChoiceSelectionMode,
    InferenceRequest, Provider, ProviderEvent, ToolCall, ToolPolicy, CHOICE_REQUEST_TOOL_NAME,
};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};
use tracing::{trace, warn};

const MAX_CHOICE_ITEMS: usize = 25;
const MAX_CHOICE_PROMPT_LEN: usize = 4_000;
const MAX_CHOICE_LABEL_LEN: usize = 512;
const MAX_CHOICE_DESCRIPTION_LEN: usize = 2_000;

fn choice_request_tool_definition() -> iron_providers::ToolDefinition {
    iron_providers::ToolDefinition::new(
        CHOICE_REQUEST_TOOL_NAME,
        "Request a bounded user choice when you cannot proceed safely without a specific structured selection. Use this instead of guessing when the user must choose among explicit options.",
        serde_json::json!({
            "type": "object",
            "required": ["prompt", "selection_mode", "items"],
            "properties": {
                "prompt": { "type": "string", "minLength": 1 },
                "selection_mode": { "type": "string", "enum": ["single", "multiple"] },
                "items": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "required": ["id", "label"],
                        "properties": {
                            "id": { "type": "string", "minLength": 1 },
                            "label": { "type": "string", "minLength": 1 },
                            "description": { "type": "string" }
                        },
                        "additionalProperties": false
                    }
                }
            },
            "additionalProperties": false
        }),
    )
}

// ---------------------------------------------------------------------------
// Internal interaction state
// ---------------------------------------------------------------------------

/// Internal representation of a pending interaction, carrying the original
/// request data needed to validate resolutions and execute follow-up actions.
enum PendingInteractionRequest {
    Approval {
        calls: Vec<ToolCall>,
    },
    Choice {
        prompt: String,
        selection_mode: ChoiceSelectionMode,
        items: Vec<crate::events::ChoiceItem>,
    },
}

/// Tracks resolution state for a pending interaction.
struct PendingInteractionState {
    interaction_id: String,
    request: PendingInteractionRequest,
    /// For approval interactions, tracks per-call decisions as they arrive.
    /// For choice interactions, this is unused (resolution is atomic).
    approval_decisions: Vec<(String, ApprovalVerdict)>,
}

fn map_choice_selection_mode(selection_mode: ProviderChoiceSelectionMode) -> ChoiceSelectionMode {
    match selection_mode {
        ProviderChoiceSelectionMode::Single => ChoiceSelectionMode::Single,
        ProviderChoiceSelectionMode::Multiple => ChoiceSelectionMode::Multiple,
    }
}

fn validate_choice_request(request: &ProviderChoiceRequest) -> Result<(), LoopError> {
    if request.prompt.trim().is_empty() {
        return Err(LoopError::InvalidInteractionResolution {
            message: "Choice request prompt must not be empty".into(),
        });
    }
    if request.prompt.len() > MAX_CHOICE_PROMPT_LEN {
        return Err(LoopError::InvalidInteractionResolution {
            message: format!(
                "Choice request prompt exceeds {} characters",
                MAX_CHOICE_PROMPT_LEN
            ),
        });
    }
    if request.items.is_empty() {
        return Err(LoopError::InvalidInteractionResolution {
            message: "Choice request must include at least one item".into(),
        });
    }
    if request.items.len() > MAX_CHOICE_ITEMS {
        return Err(LoopError::InvalidInteractionResolution {
            message: format!(
                "Choice request exceeds maximum item count of {}",
                MAX_CHOICE_ITEMS
            ),
        });
    }

    let mut seen_item_ids = HashSet::new();
    for item in &request.items {
        if item.id.trim().is_empty() {
            return Err(LoopError::InvalidInteractionResolution {
                message: "Choice request item id must not be empty".into(),
            });
        }
        if !seen_item_ids.insert(item.id.clone()) {
            return Err(LoopError::InvalidInteractionResolution {
                message: format!("Choice request contains duplicate item id '{}'", item.id),
            });
        }
        if item.label.trim().is_empty() {
            return Err(LoopError::InvalidInteractionResolution {
                message: format!("Choice request item '{}' has an empty label", item.id),
            });
        }
        if item.label.len() > MAX_CHOICE_LABEL_LEN {
            return Err(LoopError::InvalidInteractionResolution {
                message: format!(
                    "Choice request item '{}' label exceeds {} characters",
                    item.id, MAX_CHOICE_LABEL_LEN
                ),
            });
        }
        if let Some(description) = &item.description {
            if description.len() > MAX_CHOICE_DESCRIPTION_LEN {
                return Err(LoopError::InvalidInteractionResolution {
                    message: format!(
                        "Choice request item '{}' description exceeds {} characters",
                        item.id, MAX_CHOICE_DESCRIPTION_LEN
                    ),
                });
            }
        }
    }

    Ok(())
}

fn choice_request_to_interaction(
    turn_id: TurnId,
    request: ProviderChoiceRequest,
) -> Result<(PendingInteractionInfo, PendingInteractionState), LoopError> {
    validate_choice_request(&request)?;

    let selection_mode = map_choice_selection_mode(request.selection_mode);
    let items: Vec<ChoiceItem> = request
        .items
        .into_iter()
        .map(|item| ChoiceItem {
            id: item.id,
            label: item.label,
            description: item.description,
        })
        .collect();

    let interaction_id = format!("choice_{}", turn_id.0);
    let payload = PendingInteractionPayload::Choice(ChoiceInteractionInfo {
        prompt: request.prompt.clone(),
        selection_mode,
        items: items.clone(),
    });

    Ok((
        PendingInteractionInfo {
            interaction_id: interaction_id.clone(),
            source: InteractionSource::Model,
            payload,
        },
        PendingInteractionState {
            interaction_id,
            request: PendingInteractionRequest::Choice {
                prompt: request.prompt,
                selection_mode,
                items,
            },
            approval_decisions: Vec::new(),
        },
    ))
}

fn validate_choice_resolution_against_request(
    selection_mode: ChoiceSelectionMode,
    items: &[ChoiceItem],
    resolution: &crate::events::ChoiceInteractionResolution,
) -> Result<(), LoopError> {
    match resolution {
        crate::events::ChoiceInteractionResolution::Submitted { selected_ids } => {
            let unique_selected_ids: HashSet<&String> = selected_ids.iter().collect();
            if unique_selected_ids.len() != selected_ids.len() {
                return Err(LoopError::InvalidInteractionResolution {
                    message: "Choice resolution contains duplicate selected_ids".into(),
                });
            }

            let valid_item_ids: HashSet<&String> = items.iter().map(|item| &item.id).collect();
            for selected_id in selected_ids {
                if !valid_item_ids.contains(selected_id) {
                    return Err(LoopError::InvalidInteractionResolution {
                        message: format!(
                            "Choice resolution references unknown item id '{}'",
                            selected_id
                        ),
                    });
                }
            }

            match selection_mode {
                ChoiceSelectionMode::Single if selected_ids.len() != 1 => {
                    Err(LoopError::InvalidInteractionResolution {
                        message: "Single-choice interaction requires exactly one selected id"
                            .into(),
                    })
                }
                ChoiceSelectionMode::Multiple if selected_ids.is_empty() => {
                    Err(LoopError::InvalidInteractionResolution {
                        message:
                            "Multi-choice interaction requires at least one selected id when submitted"
                                .into(),
                    })
                }
                _ => Ok(()),
            }
        }
        crate::events::ChoiceInteractionResolution::Cancelled => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Control commands
// ---------------------------------------------------------------------------

enum ControlCommand {
    /// Resolve a pending interaction with a typed resolution.
    ResolveInteraction {
        interaction_id: String,
        resolution: InteractionResolution,
    },
    /// Legacy: approve a single approval call by call_id.
    #[allow(dead_code)]
    Approve { call_id: String },
    /// Legacy: deny a single approval call by call_id.
    #[allow(dead_code)]
    Deny { call_id: String },
    Interrupt,
    Cancel,
}

// ---------------------------------------------------------------------------
// Shared turn state
// ---------------------------------------------------------------------------

struct TurnShared {
    status: TurnStatus,
    pending_interaction: Option<PendingInteractionState>,
}

// ---------------------------------------------------------------------------
// Public handle types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Turn driver (runs on the runtime task)
// ---------------------------------------------------------------------------

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
            let mut choice_request: Option<ProviderChoiceRequest> = None;
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
                        if choice_request.is_some() {
                            provider_error = Some(
                                "Provider emitted output after a blocking choice request in the same interaction phase"
                                    .into(),
                            );
                            break;
                        }
                        assistant_output.push_str(&content);
                        self.emit(TurnEvent::OutputDelta { content });
                    }
                    ProviderEvent::ToolCall { call } => {
                        if choice_request.is_some() {
                            provider_error = Some(
                                "Provider emitted both a choice request and tool calls in the same interaction phase"
                                    .into(),
                            );
                            break;
                        }
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
                    ProviderEvent::ChoiceRequest { request } => {
                        if choice_request.is_some() {
                            provider_error = Some(
                                "Provider emitted multiple blocking choice requests in one interaction phase"
                                    .into(),
                            );
                            break;
                        }
                        if !tool_calls.is_empty() {
                            provider_error = Some(
                                "Provider emitted a choice request after tool calls in the same interaction phase"
                                    .into(),
                            );
                            break;
                        }
                        choice_request = Some(request);
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

            if tool_calls.is_empty() && choice_request.is_none() {
                self.finish(TurnOutcome::Completed);
                return;
            }

            if let Some(request) = choice_request {
                match self.pause_for_choice_request(request).await {
                    Ok(Some(outcome)) => {
                        self.finish(outcome);
                        return;
                    }
                    Ok(None) => continue,
                    Err(error) => {
                        self.finish(TurnOutcome::Failed {
                            message: error.to_string(),
                        });
                        return;
                    }
                }
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

            // --- Create approval interaction envelope ---
            let interaction_id = format!("approval_{}", self.turn_id.0);
            let approval_calls: Vec<ApprovalCallInfo> = needs_approval
                .iter()
                .map(|call| ApprovalCallInfo {
                    call_id: call.call_id.clone(),
                    tool_name: call.tool_name.clone(),
                    arguments: call.arguments.clone(),
                })
                .collect();

            let pending_info = PendingInteractionInfo {
                interaction_id: interaction_id.clone(),
                source: InteractionSource::Runtime,
                payload: PendingInteractionPayload::Approval(ApprovalInteractionInfo {
                    calls: approval_calls,
                }),
            };

            {
                let mut shared = self.shared.lock().unwrap();
                shared.pending_interaction = Some(PendingInteractionState {
                    interaction_id: interaction_id.clone(),
                    request: PendingInteractionRequest::Approval {
                        calls: needs_approval,
                    },
                    approval_decisions: Vec::new(),
                });
                shared.status = TurnStatus::WaitingForInteraction {
                    pending: pending_info.clone(),
                };
            }

            // Emit the generalized interaction event
            self.emit(TurnEvent::InteractionRequired {
                interaction: pending_info.clone(),
            });

            // Emit legacy per-call ApprovalRequired events for compatibility
            #[allow(deprecated)]
            {
                if let PendingInteractionPayload::Approval(ref approval_info) = pending_info.payload
                {
                    for call_info in &approval_info.calls {
                        self.emit(TurnEvent::ApprovalRequired {
                            call_id: call_info.call_id.clone(),
                            tool_name: call_info.tool_name.clone(),
                            arguments: call_info.arguments.clone(),
                        });
                    }
                }
            }

            // Wait for resolution
            match self.wait_for_interaction_resolution().await {
                Some(outcome) => {
                    self.finish(outcome);
                    return;
                }
                None => {
                    // Process the resolved interaction
                    let state = {
                        let mut shared = self.shared.lock().unwrap();
                        shared.pending_interaction.take()
                    };

                    if let Some(pending) = state {
                        self.process_approval_resolution(pending).await;
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

    /// Process the resolved approval interaction: execute approved calls,
    /// deny the rest.
    async fn process_approval_resolution(&self, pending: PendingInteractionState) {
        let PendingInteractionRequest::Approval { calls } = pending.request else {
            return;
        };

        // Build a lookup from call_id to verdict
        let verdict_map: std::collections::HashMap<String, ApprovalVerdict> = pending
            .approval_decisions
            .into_iter()
            .collect();

        for call in calls {
            let verdict = verdict_map
                .get(&call.call_id)
                .copied()
                .unwrap_or(ApprovalVerdict::Deny);

            match verdict {
                ApprovalVerdict::AllowOnce => {
                    self.execute_tool(call).await;
                }
                ApprovalVerdict::Deny | ApprovalVerdict::Cancelled => {
                    let result = serde_json::json!({"error": "Tool execution denied by user"});
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
        }
    }

    async fn pause_for_choice_request(
        &mut self,
        request: ProviderChoiceRequest,
    ) -> Result<Option<TurnOutcome>, LoopError> {
        let (pending_info, pending_state) = choice_request_to_interaction(self.turn_id, request)?;

        {
            let mut shared = self.shared.lock().unwrap();
            if shared.pending_interaction.is_some() {
                return Err(LoopError::InvalidInteractionResolution {
                    message:
                        "A new choice interaction cannot be created while another interaction is pending"
                            .into(),
                });
            }
            shared.pending_interaction = Some(pending_state);
            shared.status = TurnStatus::WaitingForInteraction {
                pending: pending_info.clone(),
            };
        }

        self.emit(TurnEvent::InteractionRequired {
            interaction: pending_info,
        });

        match self.wait_for_interaction_resolution().await {
            Some(outcome) => Ok(Some(outcome)),
            None => {
                {
                    let mut shared = self.shared.lock().unwrap();
                    shared.pending_interaction.take();
                    shared.status = TurnStatus::Running;
                }
                Ok(None)
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
                ControlCommand::ResolveInteraction { .. }
                | ControlCommand::Approve { .. }
                | ControlCommand::Deny { .. } => {
                    warn!("Received interaction command during streaming phase, ignoring");
                }
            }
        }
        None
    }

    /// Wait for the pending interaction to be fully resolved.
    /// Returns `Some(outcome)` if the turn should terminate, `None` if the
    /// interaction was resolved and the turn should continue.
    async fn wait_for_interaction_resolution(&mut self) -> Option<TurnOutcome> {
        loop {
            {
                let shared = self.shared.lock().unwrap();
                if let Some(ref pending) = shared.pending_interaction {
                    if let PendingInteractionRequest::Approval { .. } = pending.request {
                        // For approval, check if all calls have decisions
                        if let PendingInteractionPayload::Approval(ref approval_info) =
                            shared.pending_status_payload().payload
                        {
                            if approval_info.calls.is_empty() {
                                // All calls decided
                                return None;
                            }
                        }
                    }
                    // For choice, resolution is atomic — handled in the command path below
                } else {
                    return None;
                }
            }

            if *self.shutdown_rx.borrow() {
                return Some(TurnOutcome::Cancelled);
            }

            match self.control_rx.recv().await {
                Some(ControlCommand::ResolveInteraction {
                    interaction_id,
                    resolution,
                }) => {
                    return self.handle_resolve_interaction(&interaction_id, resolution);
                }
                Some(ControlCommand::Approve { call_id }) => {
                    self.handle_legacy_approve(&call_id);
                    // Check if all decisions are in
                    if self.all_approval_decisions_in() {
                        return None;
                    }
                }
                Some(ControlCommand::Deny { call_id }) => {
                    self.handle_legacy_deny(&call_id);
                    if self.all_approval_decisions_in() {
                        return None;
                    }
                }
                Some(ControlCommand::Interrupt) => return Some(TurnOutcome::Interrupted),
                Some(ControlCommand::Cancel) => return Some(TurnOutcome::Cancelled),
                None => return Some(TurnOutcome::Interrupted),
            }
        }
    }

    fn handle_resolve_interaction(
        &mut self,
        interaction_id: &str,
        resolution: InteractionResolution,
    ) -> Option<TurnOutcome> {
        let mut shared = self.shared.lock().unwrap();

        let pending = match shared.pending_interaction.as_mut() {
            Some(p) => p,
            None => {
                warn!("Received resolution but no pending interaction");
                return None;
            }
        };

        if pending.interaction_id != interaction_id {
            warn!(
                "Received resolution for interaction {} but waiting for {}",
                interaction_id, pending.interaction_id
            );
            return None;
        }

        match (&pending.request, &resolution) {
            (
                PendingInteractionRequest::Approval { calls: _ },
                InteractionResolution::Approval(approval_res),
            ) => {
                if let Err(error) = validate_interaction_resolution_for_pending(pending, &resolution)
                {
                    warn!("{}", error);
                    return None;
                }

                pending.approval_decisions = approval_res
                    .decisions
                    .iter()
                    .map(|d| (d.call_id.clone(), d.verdict))
                    .collect();

                // All decisions received
                None
            }
            (
                PendingInteractionRequest::Choice {
                    prompt,
                    selection_mode,
                    items,
                },
                InteractionResolution::Choice(choice_res),
            ) => {
                if let Err(error) =
                    validate_choice_resolution_against_request(*selection_mode, items, choice_res)
                {
                    warn!("{}", error);
                    return None;
                }

                match choice_res {
                    crate::events::ChoiceInteractionResolution::Submitted { selected_ids } => {
                        // Build canonical resolution record and inject into session
                        let selected_items: Vec<ChoiceResolutionItem> = items
                            .iter()
                            .filter(|i| selected_ids.contains(&i.id))
                            .map(|i| ChoiceResolutionItem {
                                id: i.id.clone(),
                                label: i.label.clone(),
                            })
                            .collect();

                        let record = ChoiceResolutionRecord::submitted(
                            interaction_id.to_string(),
                            prompt.clone(),
                            *selection_mode,
                            selected_items,
                        );

                        drop(shared);
                        self.inject_choice_resolution(record);
                        None
                    }
                    crate::events::ChoiceInteractionResolution::Cancelled => {
                        let record = ChoiceResolutionRecord::cancelled(
                            interaction_id.to_string(),
                            prompt.clone(),
                            *selection_mode,
                        );

                        drop(shared);
                        self.inject_choice_resolution(record);
                        None
                    }
                }
            }
            _ => {
                warn!(
                    "Interaction resolution kind does not match pending interaction kind for {}",
                    interaction_id
                );
                None
            }
        }
    }

    fn handle_legacy_approve(&mut self, call_id: &str) {
        let mut shared = self.shared.lock().unwrap();
        if let Some(ref mut pending) = shared.pending_interaction {
            if let PendingInteractionRequest::Approval { .. } = pending.request {
                // Check if this call_id is already decided
                let already = pending
                    .approval_decisions
                    .iter()
                    .any(|(id, _)| id == call_id);
                if !already {
                    pending
                        .approval_decisions
                        .push((call_id.to_string(), ApprovalVerdict::AllowOnce));
                }
            }
        }
        // Update the status to reflect remaining pending calls
        update_interaction_status_locked(&mut shared);
    }

    fn handle_legacy_deny(&mut self, call_id: &str) {
        let mut shared = self.shared.lock().unwrap();
        if let Some(ref mut pending) = shared.pending_interaction {
            if let PendingInteractionRequest::Approval { .. } = pending.request {
                let already = pending
                    .approval_decisions
                    .iter()
                    .any(|(id, _)| id == call_id);
                if !already {
                    pending
                        .approval_decisions
                        .push((call_id.to_string(), ApprovalVerdict::Deny));
                }
            }
        }
        update_interaction_status_locked(&mut shared);
    }

    fn all_approval_decisions_in(&self) -> bool {
        let shared = self.shared.lock().unwrap();
        if let Some(ref pending) = shared.pending_interaction {
            if let PendingInteractionRequest::Approval { calls } = &pending.request {
                return pending.approval_decisions.len() == calls.len();
            }
        }
        false
    }

    fn inject_choice_resolution(&self, record: ChoiceResolutionRecord) {
        let payload = serde_json::to_value(&record).unwrap_or_else(|_| {
            serde_json::json!({"error": "failed to serialize choice resolution"})
        });
        self.session
            .lock()
            .unwrap()
            .add_system_structured_message(record.kind.clone(), payload);
    }

    fn prepare_request(&self) -> Result<InferenceRequest, LoopError> {
        let session = self.session.lock().unwrap();
        let messages = apply_context_policy(&self.config, &session.messages)?;
        let transcript = iron_providers::Transcript::with_messages(messages);

        let mut provider_tools = self.tools.provider_definitions();
        provider_tools.push(choice_request_tool_definition());

        let tool_policy = if provider_tools.is_empty() {
            ToolPolicy::None
        } else if self.tools.is_empty() {
            ToolPolicy::Auto
        } else {
            self.config.default_tool_policy.clone()
        };

        let mut request = InferenceRequest::new(self.config.model.clone(), transcript)
            .with_tools(provider_tools)
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

// ---------------------------------------------------------------------------
// Context policy helper
// ---------------------------------------------------------------------------

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
            "ContextWindowPolicy::SummarizeAfter is not implemented; use context_management compaction instead",
        )),
    }
}

// ---------------------------------------------------------------------------
// Status update helper
// ---------------------------------------------------------------------------

/// Update the turn status to reflect the current pending interaction state.
/// For approval interactions, this filters out already-decided calls.
fn update_interaction_status_locked(shared: &mut TurnShared) {
    let pending = match shared.pending_interaction.as_ref() {
        Some(p) => p,
        None => return,
    };

    match &pending.request {
        PendingInteractionRequest::Approval { calls } => {
            let decided_ids: std::collections::HashSet<String> = pending
                .approval_decisions
                .iter()
                .map(|(id, _)| id.clone())
                .collect();

            let remaining_calls: Vec<ApprovalCallInfo> = calls
                .iter()
                .filter(|c| !decided_ids.contains(&c.call_id))
                .map(|c| ApprovalCallInfo {
                    call_id: c.call_id.clone(),
                    tool_name: c.tool_name.clone(),
                    arguments: c.arguments.clone(),
                })
                .collect();

            if remaining_calls.is_empty() {
                // All decided — status will be updated when the turn resumes
                return;
            }

            shared.status = TurnStatus::WaitingForInteraction {
                pending: PendingInteractionInfo {
                    interaction_id: pending.interaction_id.clone(),
                    source: InteractionSource::Runtime,
                    payload: PendingInteractionPayload::Approval(ApprovalInteractionInfo {
                        calls: remaining_calls,
                    }),
                },
            };
        }
        PendingInteractionRequest::Choice { .. } => {
            // Choice interactions are atomic — status doesn't change until resolved
        }
    }
}

// ---------------------------------------------------------------------------
// TurnShared helper for extracting pending payload
// ---------------------------------------------------------------------------

impl TurnShared {
    fn pending_status_payload(&self) -> PendingInteractionInfo {
        match self.status {
            TurnStatus::WaitingForInteraction { ref pending } => pending.clone(),
            _ => PendingInteractionInfo {
                interaction_id: String::new(),
                source: InteractionSource::Runtime,
                payload: PendingInteractionPayload::Approval(ApprovalInteractionInfo {
                    calls: vec![],
                }),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Turn creation
// ---------------------------------------------------------------------------

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
        pending_interaction: None,
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

fn validate_interaction_resolution_for_pending(
    pending: &PendingInteractionState,
    resolution: &InteractionResolution,
) -> Result<(), LoopError> {
    match (&pending.request, resolution) {
        (PendingInteractionRequest::Approval { calls }, InteractionResolution::Approval(res)) => {
            let expected_call_ids: HashSet<&String> = calls.iter().map(|call| &call.call_id).collect();
            let decision_call_ids: HashSet<&String> =
                res.decisions.iter().map(|decision| &decision.call_id).collect();

            if res.decisions.len() != calls.len() {
                return Err(LoopError::InvalidInteractionResolution {
                    message: format!(
                        "Approval interaction requires decisions for all {} pending calls",
                        calls.len()
                    ),
                });
            }
            if decision_call_ids.len() != res.decisions.len() {
                return Err(LoopError::InvalidInteractionResolution {
                    message: "Approval interaction contains duplicate call_id decisions".into(),
                });
            }
            for decision in &res.decisions {
                if !expected_call_ids.contains(&decision.call_id) {
                    return Err(LoopError::InvalidInteractionResolution {
                        message: format!(
                            "Approval interaction references unknown call_id '{}'",
                            decision.call_id
                        ),
                    });
                }
            }
            Ok(())
        }
        (
            PendingInteractionRequest::Choice {
                selection_mode,
                items,
                ..
            },
            InteractionResolution::Choice(choice_resolution),
        ) => validate_choice_resolution_against_request(*selection_mode, items, choice_resolution),
        (PendingInteractionRequest::Approval { .. }, InteractionResolution::Choice(_))
        | (PendingInteractionRequest::Choice { .. }, InteractionResolution::Approval(_)) => {
            Err(LoopError::InteractionKindMismatch {
                interaction_id: pending.interaction_id.clone(),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// TurnHandle public API
// ---------------------------------------------------------------------------

impl TurnHandle {
    pub fn id(&self) -> TurnId {
        self.turn_id
    }

    pub fn status(&self) -> TurnStatus {
        self.shared.lock().unwrap().status.clone()
    }

    /// Resolve a pending interaction with a typed resolution.
    pub fn resolve_interaction(
        &self,
        interaction_id: impl Into<String>,
        resolution: InteractionResolution,
    ) -> Result<(), LoopError> {
        let interaction_id = interaction_id.into();
        let shared = self.shared.lock().unwrap();
        match &shared.status {
            TurnStatus::Finished { .. } => Err(LoopError::TurnFinished),
            TurnStatus::Running => Err(LoopError::NotWaitingForInteraction),
            TurnStatus::WaitingForInteraction { .. } => {
                if let Some(ref pending) = shared.pending_interaction {
                    if pending.interaction_id != interaction_id {
                        return Err(LoopError::InteractionNotFound { interaction_id });
                    }
                    validate_interaction_resolution_for_pending(pending, &resolution)?;
                } else {
                    return Err(LoopError::InteractionNotFound { interaction_id });
                }
                drop(shared);
                self.control_tx
                    .send(ControlCommand::ResolveInteraction {
                        interaction_id,
                        resolution,
                    })
                    .map_err(|_| LoopError::TurnFinished)
            }
        }
    }

    /// Approve a tool call that is waiting for permission.
    ///
    /// This is a legacy compatibility wrapper. New code should use
    /// `resolve_interaction` with an `ApprovalInteractionResolution`.
    #[deprecated(note = "Use resolve_interaction with ApprovalInteractionResolution instead")]
    pub fn approve(&self, call_id: impl Into<String>) -> Result<(), LoopError> {
        let call_id = call_id.into();
        let shared = self.shared.lock().unwrap();
        match &shared.status {
            TurnStatus::Finished { .. } => Err(LoopError::TurnFinished),
            TurnStatus::Running => Err(LoopError::NotWaitingForApproval),
            TurnStatus::WaitingForInteraction { .. } => {
                // Validate this is an approval interaction and the call_id exists
                if let Some(ref pending) = shared.pending_interaction {
                    if let PendingInteractionRequest::Approval { ref calls } = pending.request {
                        if !calls.iter().any(|c| c.call_id == call_id) {
                            return Err(LoopError::ApprovalNotFound { call_id });
                        }
                    } else {
                        return Err(LoopError::NotWaitingForApproval);
                    }
                } else {
                    return Err(LoopError::ApprovalNotFound { call_id });
                }
                drop(shared);
                self.control_tx
                    .send(ControlCommand::Approve { call_id })
                    .map_err(|_| LoopError::TurnFinished)
            }
        }
    }

    /// Deny a tool call that is waiting for permission.
    ///
    /// This is a legacy compatibility wrapper. New code should use
    /// `resolve_interaction` with an `ApprovalInteractionResolution`.
    #[deprecated(note = "Use resolve_interaction with ApprovalInteractionResolution instead")]
    pub fn deny(&self, call_id: impl Into<String>) -> Result<(), LoopError> {
        let call_id = call_id.into();
        let shared = self.shared.lock().unwrap();
        match &shared.status {
            TurnStatus::Finished { .. } => Err(LoopError::TurnFinished),
            TurnStatus::Running => Err(LoopError::NotWaitingForApproval),
            TurnStatus::WaitingForInteraction { .. } => {
                if let Some(ref pending) = shared.pending_interaction {
                    if let PendingInteractionRequest::Approval { ref calls } = pending.request {
                        if !calls.iter().any(|c| c.call_id == call_id) {
                            return Err(LoopError::ApprovalNotFound { call_id });
                        }
                    } else {
                        return Err(LoopError::NotWaitingForApproval);
                    }
                } else {
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
