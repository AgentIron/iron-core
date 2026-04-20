use crate::durable::{DurableSession, SessionId};
use crate::prompt_lifecycle::AcpPromptSink;
use crate::prompt_runner::PromptRunner;
use crate::runtime::{ConnectionId, IronRuntime};
use std::cell::RefCell;
use std::rc::Rc;
use tracing::{debug, info};

pub trait ClientChannel {
    fn send_notification(
        &self,
        notification: agent_client_protocol::SessionNotification,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = agent_client_protocol::Result<()>>>>;

    fn request_permission(
        &self,
        request: agent_client_protocol::RequestPermissionRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                Output = agent_client_protocol::Result<
                    agent_client_protocol::RequestPermissionResponse,
                >,
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
}

struct NopClientChannel;

impl ClientChannel for NopClientChannel {
    fn send_notification(
        &self,
        _notification: agent_client_protocol::SessionNotification,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = agent_client_protocol::Result<()>>>>
    {
        Box::pin(async { Ok(()) })
    }

    fn request_permission(
        &self,
        request: agent_client_protocol::RequestPermissionRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                Output = agent_client_protocol::Result<
                    agent_client_protocol::RequestPermissionResponse,
                >,
            >,
        >,
    > {
        let _tool_call_id = request.tool_call.tool_call_id.to_string();
        Box::pin(async move {
            Ok(agent_client_protocol::RequestPermissionResponse::new(
                agent_client_protocol::RequestPermissionOutcome::Selected(
                    agent_client_protocol::SelectedPermissionOutcome::new(
                        agent_client_protocol::PermissionOptionId::new("allow_once"),
                    ),
                ),
            ))
        })
    }
}

pub(crate) type SharedClientChannel = Rc<dyn ClientChannel>;

pub struct IronConnection {
    id: ConnectionId,
    runtime: IronRuntime,
    client: RefCell<Option<SharedClientChannel>>,
}

static CONNECTION_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl IronConnection {
    pub fn new(runtime: IronRuntime) -> Self {
        let id = ConnectionId(CONNECTION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
        runtime.register_connection(id);
        Self {
            id,
            runtime,
            client: RefCell::new(None),
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

    fn client_channel(&self) -> SharedClientChannel {
        self.client
            .borrow()
            .clone()
            .unwrap_or_else(|| Rc::new(NopClientChannel))
    }

    fn parse_session_id(&self, id: &agent_client_protocol::SessionId) -> Option<SessionId> {
        let s = id.to_string();
        s.strip_prefix("session-")
            .and_then(|n| n.parse::<u64>().ok())
            .map(SessionId)
    }

    fn resolve_owned_session(
        &self,
        acp_session_id: &agent_client_protocol::SessionId,
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
}

#[async_trait::async_trait(?Send)]
impl agent_client_protocol::Agent for IronConnection {
    async fn initialize(
        &self,
        _args: agent_client_protocol::InitializeRequest,
    ) -> agent_client_protocol::Result<agent_client_protocol::InitializeResponse> {
        info!("ACP initialize from client");

        let caps = agent_client_protocol::AgentCapabilities::default();
        Ok(agent_client_protocol::InitializeResponse::new(
            agent_client_protocol::ProtocolVersion::V1,
        )
        .agent_capabilities(caps))
    }

    async fn authenticate(
        &self,
        _args: agent_client_protocol::AuthenticateRequest,
    ) -> agent_client_protocol::Result<agent_client_protocol::AuthenticateResponse> {
        Ok(agent_client_protocol::AuthenticateResponse::new())
    }

    async fn new_session(
        &self,
        _args: agent_client_protocol::NewSessionRequest,
    ) -> agent_client_protocol::Result<agent_client_protocol::NewSessionResponse> {
        info!(connection_id = self.id.0, "ACP new_session");

        let (session_id, _session) = self
            .runtime
            .create_session(self.id)
            .map_err(|e| agent_client_protocol::Error::into_internal_error(&e))?;

        Ok(agent_client_protocol::NewSessionResponse::new(
            agent_client_protocol::SessionId::new(session_id.to_string()),
        ))
    }

    async fn prompt(
        &self,
        args: agent_client_protocol::PromptRequest,
    ) -> agent_client_protocol::Result<agent_client_protocol::PromptResponse> {
        debug!(session_id = %args.session_id, "ACP prompt received");

        let (iron_session_id, durable) = self.resolve_owned_session(&args.session_id)?;

        let user_blocks: Vec<crate::durable::ContentBlock> = args
            .prompt
            .iter()
            .map(crate::durable::ContentBlock::from_acp_content)
            .collect();
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

        let sink = AcpPromptSink::new(acp_session_id.clone(), client);

        let runner = PromptRunner::new(self.runtime.clone());
        let stop_reason = runner
            .run(&durable, &ephemeral, &sink, &config, max_iterations)
            .await;

        self.runtime.finish_prompt(iron_session_id);

        if config.context_management.enabled {
            runner.maybe_compact_post_turn(&durable, &config).await;
        }

        Ok(agent_client_protocol::PromptResponse::new(stop_reason))
    }

    async fn cancel(
        &self,
        args: agent_client_protocol::CancelNotification,
    ) -> agent_client_protocol::Result<()> {
        info!(session_id = %args.session_id, "ACP cancel received");

        let (iron_session_id, _) = self.resolve_owned_session(&args.session_id)?;

        self.runtime.cancel_active_prompt(iron_session_id);
        debug!(session_id = %args.session_id, "Turn cancellation through session-owned state");

        Ok(())
    }

    async fn close_session(
        &self,
        args: agent_client_protocol::CloseSessionRequest,
    ) -> agent_client_protocol::Result<agent_client_protocol::CloseSessionResponse> {
        info!(session_id = %args.session_id, "ACP close_session");

        let (iron_session_id, _) = self.resolve_owned_session(&args.session_id)?;
        self.runtime.finish_prompt(iron_session_id);
        self.runtime.close_session(iron_session_id);

        Ok(agent_client_protocol::CloseSessionResponse::new())
    }
}

pub(crate) fn notification(
    session_id: &agent_client_protocol::SessionId,
    update: agent_client_protocol::SessionUpdate,
) -> agent_client_protocol::SessionNotification {
    agent_client_protocol::SessionNotification::new(session_id.clone(), update)
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
