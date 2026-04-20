use agent_client_protocol::{Agent as _, Client};
use iron_core::{
    runtime::IronRuntime, transport::create_in_process_transport, Config, InProcessTransport,
};
use iron_providers::OpenAiProvider;
use std::cell::RefCell;
use std::rc::Rc;

struct MockClient {
    notifications: Rc<RefCell<Vec<agent_client_protocol::SessionNotification>>>,
}

impl MockClient {
    fn new(notifications: Rc<RefCell<Vec<agent_client_protocol::SessionNotification>>>) -> Self {
        Self { notifications }
    }
}

#[async_trait::async_trait(?Send)]
impl Client for MockClient {
    async fn session_notification(
        &self,
        args: agent_client_protocol::SessionNotification,
    ) -> agent_client_protocol::Result<()> {
        self.notifications.borrow_mut().push(args);
        Ok(())
    }

    async fn request_permission(
        &self,
        _args: agent_client_protocol::RequestPermissionRequest,
    ) -> agent_client_protocol::Result<agent_client_protocol::RequestPermissionResponse> {
        let outcome = agent_client_protocol::RequestPermissionOutcome::Selected(
            agent_client_protocol::SelectedPermissionOutcome::new(
                agent_client_protocol::PermissionOptionId::new("allow_once"),
            ),
        );
        Ok(agent_client_protocol::RequestPermissionResponse::new(
            outcome,
        ))
    }
}

fn make_runtime() -> IronRuntime {
    let config = Config::new().with_model("test-model");
    let provider = OpenAiProvider::new(iron_providers::OpenAiConfig::new("test-key".into()))
        .expect("test provider config should be valid");
    IronRuntime::new(config, provider)
}

async fn setup_transport() -> (
    InProcessTransport,
    Rc<RefCell<Vec<agent_client_protocol::SessionNotification>>>,
) {
    let runtime = make_runtime();
    let notifications = Rc::new(RefCell::new(Vec::new()));
    let client = MockClient::new(notifications.clone());
    let (transport, agent_fut) = create_in_process_transport(runtime, client);

    tokio::task::spawn_local(agent_fut);

    let _ = transport
        .client()
        .initialize(agent_client_protocol::InitializeRequest::new(
            agent_client_protocol::ProtocolVersion::LATEST,
        ))
        .await
        .unwrap();

    (transport, notifications)
}

#[test]
fn inprocess_initialize_and_new_session() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let (transport, _notifications) = setup_transport().await;
        let acp_client = transport.client();

        let session_resp = acp_client
            .new_session(agent_client_protocol::NewSessionRequest::new("."))
            .await
            .unwrap();
        assert!(!session_resp.session_id.to_string().is_empty());
    });
}

#[test]
fn inprocess_cancel() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let (transport, _notifications) = setup_transport().await;
        let acp_client = transport.client();

        let session_resp = acp_client
            .new_session(agent_client_protocol::NewSessionRequest::new("."))
            .await
            .unwrap();
        let session_id = session_resp.session_id;

        let cancel_result = acp_client
            .cancel(agent_client_protocol::CancelNotification::new(session_id))
            .await;
        assert!(cancel_result.is_ok());
    });
}

#[test]
fn inprocess_close_session() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let (transport, _notifications) = setup_transport().await;
        let acp_client = transport.client();

        let session_resp = acp_client
            .new_session(agent_client_protocol::NewSessionRequest::new("."))
            .await
            .unwrap();
        let session_id = session_resp.session_id;

        let close_result = acp_client
            .close_session(agent_client_protocol::CloseSessionRequest::new(session_id))
            .await;
        assert!(close_result.is_ok());
    });
}

#[test]
fn inprocess_multiple_sessions() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let (transport, _notifications) = setup_transport().await;
        let acp_client = transport.client();

        let s1 = acp_client
            .new_session(agent_client_protocol::NewSessionRequest::new("."))
            .await
            .unwrap()
            .session_id;
        let s2 = acp_client
            .new_session(agent_client_protocol::NewSessionRequest::new("."))
            .await
            .unwrap()
            .session_id;

        assert_ne!(s1, s2);

        let _ = acp_client
            .close_session(agent_client_protocol::CloseSessionRequest::new(s1))
            .await;
        let _ = acp_client
            .close_session(agent_client_protocol::CloseSessionRequest::new(s2))
            .await;
    });
}

#[test]
fn inprocess_prompt_with_fake_provider_returns_end_turn() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let (transport, _notifications) = setup_transport().await;
        let acp_client = transport.client();

        let session_resp = acp_client
            .new_session(agent_client_protocol::NewSessionRequest::new("."))
            .await
            .unwrap();
        let session_id = session_resp.session_id;

        let result = acp_client
            .prompt(agent_client_protocol::PromptRequest::new(
                session_id,
                vec![agent_client_protocol::ContentBlock::Text(
                    agent_client_protocol::TextContent::new("hello"),
                )],
            ))
            .await;

        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().stop_reason,
            agent_client_protocol::StopReason::EndTurn
        );
    });
}

#[test]
fn inprocess_reinitialize_fresh_transport() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let (transport1, _) = setup_transport().await;
        let (transport2, _) = setup_transport().await;

        let s1 = transport1
            .client()
            .new_session(agent_client_protocol::NewSessionRequest::new("."))
            .await
            .unwrap()
            .session_id;
        let s2 = transport2
            .client()
            .new_session(agent_client_protocol::NewSessionRequest::new("."))
            .await
            .unwrap()
            .session_id;

        assert_ne!(s1, s2);
    });
}

#[test]
fn inprocess_cross_connection_prompt_rejected() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let runtime = make_runtime();
        let notifications = Rc::new(RefCell::new(Vec::new()));
        let client = MockClient::new(notifications.clone());
        let (transport1, agent_fut1) = create_in_process_transport(runtime.clone(), client);
        let notifications2 = Rc::new(RefCell::new(Vec::new()));
        let client2 = MockClient::new(notifications2.clone());
        let (transport2, agent_fut2) = create_in_process_transport(runtime.clone(), client2);

        tokio::task::spawn_local(agent_fut1);
        tokio::task::spawn_local(agent_fut2);

        let _ = transport1
            .client()
            .initialize(agent_client_protocol::InitializeRequest::new(
                agent_client_protocol::ProtocolVersion::LATEST,
            ))
            .await
            .unwrap();
        let _ = transport2
            .client()
            .initialize(agent_client_protocol::InitializeRequest::new(
                agent_client_protocol::ProtocolVersion::LATEST,
            ))
            .await
            .unwrap();

        let session_resp = transport1
            .client()
            .new_session(agent_client_protocol::NewSessionRequest::new("."))
            .await
            .unwrap();
        let session_id = session_resp.session_id;

        let result = transport2
            .client()
            .prompt(agent_client_protocol::PromptRequest::new(
                session_id,
                vec![agent_client_protocol::ContentBlock::Text(
                    agent_client_protocol::TextContent::new("intrude"),
                )],
            ))
            .await;

        assert!(result.is_err(), "expected cross-connection prompt to fail");
    });
}

#[test]
fn inprocess_cross_connection_close_session_rejected() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let runtime = make_runtime();
        let notifications = Rc::new(RefCell::new(Vec::new()));
        let client = MockClient::new(notifications.clone());
        let (transport1, agent_fut1) = create_in_process_transport(runtime.clone(), client);
        let notifications2 = Rc::new(RefCell::new(Vec::new()));
        let client2 = MockClient::new(notifications2.clone());
        let (transport2, agent_fut2) = create_in_process_transport(runtime.clone(), client2);

        tokio::task::spawn_local(agent_fut1);
        tokio::task::spawn_local(agent_fut2);

        let _ = transport1
            .client()
            .initialize(agent_client_protocol::InitializeRequest::new(
                agent_client_protocol::ProtocolVersion::LATEST,
            ))
            .await
            .unwrap();
        let _ = transport2
            .client()
            .initialize(agent_client_protocol::InitializeRequest::new(
                agent_client_protocol::ProtocolVersion::LATEST,
            ))
            .await
            .unwrap();

        let session_resp = transport1
            .client()
            .new_session(agent_client_protocol::NewSessionRequest::new("."))
            .await
            .unwrap();
        let session_id = session_resp.session_id;

        let result = transport2
            .client()
            .close_session(agent_client_protocol::CloseSessionRequest::new(session_id))
            .await;

        assert!(result.is_err(), "expected cross-connection close to fail");
    });
}
