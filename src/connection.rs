use crate::durable::{DurableSession, SessionId};
use crate::profile::{
    default_identity_prompt, managed_profile_prompt_context, AgentApproval, AgentProfile,
    AgentProfileId, AgentProfileProvider, ResolvedProfileProvider, SkillFilter, ToolFilter,
};
use crate::prompt_lifecycle::AcpPromptSink;
use crate::prompt_runner::PromptRunner;
use crate::provider_credential::domain::ProviderPromptContext;
use crate::runtime::{ConnectionId, IronRuntime};
use agent_client_protocol::schema::v1 as acp;
use parking_lot::RwLock;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use tracing::{debug, info, warn};

pub trait ClientChannel {
    fn send_notification(
        &self,
        notification: acp::SessionNotification,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = agent_client_protocol::Result<()>>>>;

    fn request_permission(
        &self,
        request: acp::RequestPermissionRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                Output = agent_client_protocol::Result<acp::RequestPermissionResponse>,
            >,
        >,
    >;

    fn emit_script_activity(
        &self,
        _script_id: &str,
        _parent_call_id: &str,
        _activity_type: &str,
        _status: &str,
        _detail: Option<serde_json::Value>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> {
        Box::pin(async {})
    }

    fn emit_compaction_event(
        &self,
        _event_type: &str,
        _tokens_before: Option<u32>,
        _tokens_after: Option<u32>,
        _method: &str,
        _reason: Option<&str>,
        _compaction_id: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> {
        Box::pin(async {})
    }
}

struct NopClientChannel;

impl ClientChannel for NopClientChannel {
    fn send_notification(
        &self,
        _notification: acp::SessionNotification,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = agent_client_protocol::Result<()>>>>
    {
        Box::pin(async { Ok(()) })
    }

    fn request_permission(
        &self,
        request: acp::RequestPermissionRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                Output = agent_client_protocol::Result<acp::RequestPermissionResponse>,
            >,
        >,
    > {
        let _tool_call_id = request.tool_call.tool_call_id.to_string();
        Box::pin(async move {
            Ok(acp::RequestPermissionResponse::new(
                acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                    acp::PermissionOptionId::new("allow_once"),
                )),
            ))
        })
    }
}

pub(crate) type SharedClientChannel = Rc<dyn ClientChannel>;

pub(crate) type ProfileRegistry = HashMap<AgentProfileId, AgentProfile>;

fn default_connection_profile_registry() -> ProfileRegistry {
    let mut registry = HashMap::new();
    let default = AgentProfile {
        name: "default".to_string(),
        provider: AgentProfileProvider::RuntimeDefault,
        tools: ToolFilter::Inherit,
        skills: SkillFilter::Inherit,
        approval: AgentApproval::PerTool,
        identity_prompt: Some(default_identity_prompt().to_string()),
    };
    registry.insert(AgentProfileId::from("default"), default);
    registry
}

pub struct IronConnection {
    id: ConnectionId,
    runtime: IronRuntime,
    client: RefCell<Option<SharedClientChannel>>,
    profile_registry: Arc<RwLock<ProfileRegistry>>,
}

impl IronConnection {
    pub fn new(runtime: IronRuntime) -> Self {
        Self::new_with_profile_registry(
            runtime,
            Arc::new(RwLock::new(default_connection_profile_registry())),
        )
    }

    pub fn new_with_profile_registry(
        runtime: IronRuntime,
        profile_registry: Arc<RwLock<ProfileRegistry>>,
    ) -> Self {
        let id = crate::runtime::next_connection_id();
        runtime.register_connection(id);
        Self {
            id,
            runtime,
            client: RefCell::new(None),
            profile_registry,
        }
    }

    pub fn id(&self) -> ConnectionId {
        self.id
    }

    pub fn runtime(&self) -> &IronRuntime {
        &self.runtime
    }

    pub fn set_client(&self, client: SharedClientChannel) {
        *self.client.borrow_mut() = Some(client);
    }

    pub fn profile_registry(&self) -> &Arc<RwLock<ProfileRegistry>> {
        &self.profile_registry
    }

    fn selected_profile(
        &self,
        session_profile_id: Option<&AgentProfileId>,
    ) -> Result<AgentProfile, agent_client_protocol::Error> {
        let registry = self.profile_registry.read();
        match session_profile_id {
            Some(id) => registry.get(id).cloned().ok_or_else(|| {
                agent_client_protocol::Error::invalid_params().data(serde_json::json!({
                    "profile_id": id.as_str(),
                    "error": "profile not found"
                }))
            }),
            None => Ok(registry
                .get(&AgentProfileId::from("default"))
                .cloned()
                .unwrap_or_else(|| AgentProfile {
                    name: "default".to_string(),
                    provider: AgentProfileProvider::RuntimeDefault,
                    tools: ToolFilter::Inherit,
                    skills: SkillFilter::Inherit,
                    approval: AgentApproval::PerTool,
                    identity_prompt: Some(default_identity_prompt().to_string()),
                })),
        }
    }

    fn client_channel(&self) -> SharedClientChannel {
        self.client
            .borrow()
            .clone()
            .unwrap_or_else(|| Rc::new(NopClientChannel))
    }

    fn parse_session_id(&self, id: &acp::SessionId) -> Option<SessionId> {
        let s = id.to_string();
        s.strip_prefix("session-")
            .and_then(|n| n.parse::<u64>().ok())
            .map(SessionId)
    }

    fn resolve_owned_session(
        &self,
        acp_session_id: &acp::SessionId,
    ) -> Result<
        (
            SessionId,
            std::sync::Arc<parking_lot::Mutex<DurableSession>>,
        ),
        agent_client_protocol::Error,
    > {
        let session_id_str = acp_session_id.to_string();
        let iron_session_id = self.parse_session_id(acp_session_id).ok_or_else(|| {
            agent_client_protocol::Error::invalid_params()
                .data(serde_json::json!({"session_id": session_id_str}))
        })?;

        let owner = self.runtime.get_session_connection(iron_session_id);
        if owner != Some(self.id) {
            return Err(
                agent_client_protocol::Error::invalid_params().data(serde_json::json!({
                    "session_id": session_id_str,
                    "error": "session not owned by this connection"
                })),
            );
        }

        let durable = self.runtime.get_session(iron_session_id).ok_or_else(|| {
            agent_client_protocol::Error::invalid_params().data(serde_json::json!({
                "session_id": session_id_str,
                "error": "session not found"
            }))
        })?;

        Ok((iron_session_id, durable))
    }

    fn is_compact_command(blocks: &[crate::durable::ContentBlock]) -> bool {
        blocks.iter().any(|block| {
            matches!(block, crate::durable::ContentBlock::Text { text } if text.trim() == "/compact")
        })
    }
}

impl IronConnection {
    pub async fn handle_initialize(
        &self,
        _args: acp::InitializeRequest,
    ) -> agent_client_protocol::Result<acp::InitializeResponse> {
        info!("ACP initialize from client");

        let caps = acp::AgentCapabilities::default();
        Ok(
            acp::InitializeResponse::new(agent_client_protocol::schema::ProtocolVersion::V1)
                .agent_capabilities(caps),
        )
    }

    pub async fn handle_authenticate(
        &self,
        _args: acp::AuthenticateRequest,
    ) -> agent_client_protocol::Result<acp::AuthenticateResponse> {
        Ok(acp::AuthenticateResponse::new())
    }

    pub async fn handle_new_session(
        &self,
        _args: acp::NewSessionRequest,
    ) -> agent_client_protocol::Result<acp::NewSessionResponse> {
        info!(connection_id = self.id.0, "ACP new_session");

        let (session_id, _session) = self
            .runtime
            .create_session(self.id)
            .map_err(|e| agent_client_protocol::Error::into_internal_error(&e))?;

        Ok(acp::NewSessionResponse::new(acp::SessionId::new(
            session_id.to_string(),
        )))
    }

    pub async fn handle_prompt(
        &self,
        args: acp::PromptRequest,
        session_profile_id: Option<&AgentProfileId>,
    ) -> agent_client_protocol::Result<acp::PromptResponse> {
        debug!(session_id = %args.session_id, "ACP prompt received");

        let (iron_session_id, durable) = self.resolve_owned_session(&args.session_id)?;

        let user_blocks: Vec<crate::durable::ContentBlock> = args
            .prompt
            .iter()
            .map(crate::durable::ContentBlock::from_acp_content)
            .collect();

        let is_compact = Self::is_compact_command(&user_blocks);
        let user_blocks = if is_compact {
            vec![crate::durable::ContentBlock::Text {
                text: "The user has requested context compaction. Please use the compress tool to replace resolved older context with durable summaries. Preserve all important facts, decisions, constraints, file paths, errors, tool results, and user intent needed for future work.".to_string(),
            }]
        } else {
            user_blocks
        };

        {
            let mut session = durable.lock();
            session.add_user_message(user_blocks);
        }

        let ephemeral = self
            .runtime
            .try_start_prompt(iron_session_id)
            .map_err(|e| {
                agent_client_protocol::Error::invalid_params().data(serde_json::json!({
                    "session_id": args.session_id.to_string(),
                    "error": e.to_string()
                }))
            })?;

        let acp_session_id = args.session_id.clone();
        let client = self.client_channel();
        let config = self.runtime.config().clone();
        let max_iterations = config.max_iterations;

        let sink = AcpPromptSink::new(acp_session_id.clone(), client.clone());

        // Apply the selected profile identity layer unless the session already
        // has explicit instructions. Snapshot profile policy fields so later
        // edits/deletion of the stored profile do not affect this session.
        let selected_profile = self.selected_profile(session_profile_id)?;
        {
            let mut session = durable.lock();
            if let Some(profile_id) = session_profile_id {
                session.profile_id = Some(profile_id.clone());
                // Snapshot tool filter and approval only when a profile is explicitly selected.
                session.effective_tool_filter = Some(selected_profile.tools.clone());
                session.effective_approval = Some(selected_profile.approval);
            }
            if session.profile_identity.is_none() {
                if let Some(ref identity) = selected_profile.identity_prompt {
                    if !identity.trim().is_empty() {
                        session.set_profile_identity(identity.clone());
                    }
                }
            }
        }

        // Check if the session has a stored managed provider from a prior model switch
        let stored_context = {
            let session = durable.lock();
            session
                .current_provider_slug
                .clone()
                .map(|slug| ProviderPromptContext {
                    provider_slug: crate::provider_credential::domain::ProviderSlug::new(slug),
                    model: session.current_model.clone().unwrap_or_default(),
                    api_key: session.current_provider_api_key.clone(),
                })
        };

        // Resolve provider for this turn. If the profile specifies an unavailable
        // provider/model, warn once and fall back to the runtime default.
        let (runner, stop_reason) = if let Some(ref ctx) = stored_context {
            match self.runtime.resolve_managed_provider(ctx).await {
                Ok(provider) => {
                    let runner =
                        PromptRunner::new_managed(self.runtime.clone(), provider, ctx.clone());
                    let stop_reason = runner
                        .run(&durable, &ephemeral, &sink, &config, max_iterations)
                        .await;
                    (Some(runner), stop_reason)
                }
                Err(e) => {
                    warn!(error = %e, "Failed to resolve stored managed provider");
                    {
                        let mut session = durable.lock();
                        session.add_agent_text(format!("[Auth error: {}]", e));
                    }
                    (None, acp::StopReason::EndTurn)
                }
            }
        } else {
            match self
                .runtime
                .resolve_profile_provider(&selected_profile)
                .await
            {
                Ok(ResolvedProfileProvider::RuntimeDefault(_)) => {
                    let runner = PromptRunner::new(self.runtime.clone());
                    let stop_reason = runner
                        .run(&durable, &ephemeral, &sink, &config, max_iterations)
                        .await;
                    (Some(runner), stop_reason)
                }
                Ok(ResolvedProfileProvider::Managed(provider)) => {
                    let context = managed_profile_prompt_context(&selected_profile.provider)
                        .expect("managed profile always yields a prompt context");
                    let runner = PromptRunner::new_managed(self.runtime.clone(), provider, context);
                    let stop_reason = runner
                        .run(&durable, &ephemeral, &sink, &config, max_iterations)
                        .await;
                    (Some(runner), stop_reason)
                }
                Ok(ResolvedProfileProvider::Fallback {
                    provider: _,
                    diagnostic,
                }) => {
                    warn!(diagnostic = %diagnostic, "Profile provider fallback to runtime default");
                    {
                        let mut session = durable.lock();
                        session.profile_unavailable = Some(diagnostic.clone());
                    }
                    let runner = PromptRunner::new(self.runtime.clone());
                    let stop_reason = runner
                        .run(&durable, &ephemeral, &sink, &config, max_iterations)
                        .await;
                    (Some(runner), stop_reason)
                }
                Err(e) => {
                    warn!(error = %e, "Failed to resolve selected profile provider");
                    {
                        let mut session = durable.lock();
                        session.add_agent_text(format!("[Provider error: {}]", e));
                        session.profile_unavailable = Some(format!("Provider unavailable: {}", e));
                    }
                    (None, acp::StopReason::EndTurn)
                }
            }
        };

        self.runtime.finish_prompt(iron_session_id);

        self.runtime
            .check_and_apply_pending_workspace_roots(iron_session_id);

        match self
            .runtime
            .check_and_apply_pending_model_switch(iron_session_id)
        {
            Ok(Some((_, Some(compaction)))) => {
                let compaction_id = uuid::Uuid::new_v4().to_string();
                let _ = client
                    .emit_compaction_event(
                        "started",
                        Some(compaction.tokens_before),
                        Some(compaction.tokens_after),
                        &compaction.method,
                        None,
                        &compaction_id,
                    )
                    .await;
                let _ = client
                    .emit_compaction_event(
                        "finished",
                        Some(compaction.tokens_before),
                        Some(compaction.tokens_after),
                        &compaction.method,
                        None,
                        &compaction_id,
                    )
                    .await;
            }
            Ok(_) => {}
            Err(e) => {
                warn!(error = %e, "Pending model switch failed");
            }
        }

        if config.context_management.enabled {
            if let Some(ref runner) = runner {
                runner
                    .maybe_compact_post_turn(&durable, &config, &client, &acp_session_id)
                    .await;
            }
        }

        Ok(acp::PromptResponse::new(stop_reason))
    }

    pub async fn handle_prompt_managed(
        &self,
        args: acp::PromptRequest,
        provider_context: ProviderPromptContext,
        session_profile_id: Option<&AgentProfileId>,
    ) -> agent_client_protocol::Result<acp::PromptResponse> {
        debug!(session_id = %args.session_id, provider = %provider_context.provider_slug.as_str(), "Managed prompt received");

        let (iron_session_id, durable) = self.resolve_owned_session(&args.session_id)?;

        let user_blocks: Vec<crate::durable::ContentBlock> = args
            .prompt
            .iter()
            .map(crate::durable::ContentBlock::from_acp_content)
            .collect();

        let is_compact = Self::is_compact_command(&user_blocks);
        let user_blocks = if is_compact {
            vec![crate::durable::ContentBlock::Text {
                text: "The user has requested context compaction. Please use the compress tool to replace resolved older context with durable summaries. Preserve all important facts, decisions, constraints, file paths, errors, tool results, and user intent needed for future work.".to_string(),
            }]
        } else {
            user_blocks
        };

        {
            let mut session = durable.lock();
            session.add_user_message(user_blocks);
        }

        let ephemeral = self
            .runtime
            .try_start_prompt(iron_session_id)
            .map_err(|e| {
                agent_client_protocol::Error::invalid_params().data(serde_json::json!({
                    "session_id": args.session_id.to_string(),
                    "error": e.to_string()
                }))
            })?;

        let acp_session_id = args.session_id.clone();
        let client = self.client_channel();
        let config = self.runtime.config().clone();
        let max_iterations = config.max_iterations;

        let sink = AcpPromptSink::new(acp_session_id.clone(), client.clone());

        // Apply the selected profile identity layer unless the session already
        // has explicit instructions. Snapshot profile policy fields so later
        // edits/deletion of the stored profile do not affect this session.
        let selected_profile = self.selected_profile(session_profile_id)?;
        {
            let mut session = durable.lock();
            if let Some(profile_id) = session_profile_id {
                session.profile_id = Some(profile_id.clone());
                // Snapshot tool filter and approval only when a profile is explicitly selected.
                session.effective_tool_filter = Some(selected_profile.tools.clone());
                session.effective_approval = Some(selected_profile.approval);
            }
            if session.profile_identity.is_none() {
                if let Some(ref identity) = selected_profile.identity_prompt {
                    if !identity.trim().is_empty() {
                        session.set_profile_identity(identity.clone());
                    }
                }
            }
        }

        // Resolve the managed provider for this prompt
        let provider = match self
            .runtime
            .resolve_managed_provider(&provider_context)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, "Failed to resolve managed provider");
                {
                    let mut session = durable.lock();
                    session.add_agent_text(format!("[Auth error: {}]", e));
                }
                self.runtime.finish_prompt(iron_session_id);
                // Check for pending workspace roots at turn boundary
                self.runtime
                    .check_and_apply_pending_workspace_roots(iron_session_id);
                // Check for pending model switches at turn boundary
                match self
                    .runtime
                    .check_and_apply_pending_model_switch(iron_session_id)
                {
                    Ok(Some((_, Some(compaction)))) => {
                        let compaction_id = uuid::Uuid::new_v4().to_string();
                        let _ = client
                            .emit_compaction_event(
                                "started",
                                Some(compaction.tokens_before),
                                Some(compaction.tokens_after),
                                &compaction.method,
                                None,
                                &compaction_id,
                            )
                            .await;
                        let _ = client
                            .emit_compaction_event(
                                "finished",
                                Some(compaction.tokens_before),
                                Some(compaction.tokens_after),
                                &compaction.method,
                                None,
                                &compaction_id,
                            )
                            .await;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!(error = %e, "Pending model switch failed");
                    }
                }
                return Ok(acp::PromptResponse::new(acp::StopReason::EndTurn));
            }
        };

        let runner = PromptRunner::new_managed(self.runtime.clone(), provider, provider_context);

        let stop_reason = runner
            .run(&durable, &ephemeral, &sink, &config, max_iterations)
            .await;

        self.runtime.finish_prompt(iron_session_id);

        // Check for pending workspace roots at turn boundary
        self.runtime
            .check_and_apply_pending_workspace_roots(iron_session_id);

        // Check for pending model switches at turn boundary
        match self
            .runtime
            .check_and_apply_pending_model_switch(iron_session_id)
        {
            Ok(Some((_, Some(compaction)))) => {
                let compaction_id = uuid::Uuid::new_v4().to_string();
                let _ = client
                    .emit_compaction_event(
                        "started",
                        Some(compaction.tokens_before),
                        Some(compaction.tokens_after),
                        &compaction.method,
                        None,
                        &compaction_id,
                    )
                    .await;
                let _ = client
                    .emit_compaction_event(
                        "finished",
                        Some(compaction.tokens_before),
                        Some(compaction.tokens_after),
                        &compaction.method,
                        None,
                        &compaction_id,
                    )
                    .await;
            }
            Ok(_) => {}
            Err(e) => {
                warn!(error = %e, "Pending model switch failed");
            }
        }

        if config.context_management.enabled {
            runner
                .maybe_compact_post_turn(&durable, &config, &client, &acp_session_id)
                .await;
        }

        Ok(acp::PromptResponse::new(stop_reason))
    }

    pub async fn handle_cancel(
        &self,
        args: acp::CancelNotification,
    ) -> agent_client_protocol::Result<()> {
        info!(session_id = %args.session_id, "ACP cancel received");

        let (iron_session_id, _) = self.resolve_owned_session(&args.session_id)?;

        self.runtime.cancel_active_prompt(iron_session_id);
        debug!(session_id = %args.session_id, "Turn cancellation through session-owned state");

        Ok(())
    }

    pub async fn handle_close_session(
        &self,
        args: acp::CloseSessionRequest,
    ) -> agent_client_protocol::Result<acp::CloseSessionResponse> {
        info!(session_id = %args.session_id, "ACP close_session");

        let (iron_session_id, _) = self.resolve_owned_session(&args.session_id)?;
        self.runtime.finish_prompt(iron_session_id);
        self.runtime.close_session(iron_session_id);

        Ok(acp::CloseSessionResponse::new())
    }
}

pub(crate) fn notification(
    session_id: &acp::SessionId,
    update: acp::SessionUpdate,
) -> acp::SessionNotification {
    acp::SessionNotification::new(session_id.clone(), update)
}

impl Drop for IronConnection {
    fn drop(&mut self) {
        self.runtime.close_connection(self.id);
    }
}

impl std::fmt::Debug for IronConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IronConnection")
            .field("id", &self.id)
            .finish()
    }
}
