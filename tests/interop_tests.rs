use iron_core::{
    runtime::IronRuntime,
    transport::{create_in_process_transport, InProcessClientHandler},
    Config, InProcessTransport,
};
use iron_providers::OpenAiProvider;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use agent_client_protocol::schema as acp;

struct MockClient {
    notifications: Rc<RefCell<Vec<acp::SessionNotification>>>,
}

impl MockClient {
    fn new(notifications: Rc<RefCell<Vec<acp::SessionNotification>>>) -> Self {
        Self { notifications }
    }
}

impl InProcessClientHandler for MockClient {
    fn session_notification(
        &self,
        args: acp::SessionNotification,
    ) -> Pin<Box<dyn Future<Output = agent_client_protocol::Result<()>>>> {
        let notifications = self.notifications.clone();
        Box::pin(async move {
            notifications.borrow_mut().push(args);
            Ok(())
        })
    }

    fn request_permission(
        &self,
        _args: acp::RequestPermissionRequest,
    ) -> Pin<Box<dyn Future<Output = agent_client_protocol::Result<acp::RequestPermissionResponse>>>>
    {
        Box::pin(async move {
            let outcome = acp::RequestPermissionOutcome::Selected(
                acp::SelectedPermissionOutcome::new(acp::PermissionOptionId::new("allow_once")),
            );
            Ok(acp::RequestPermissionResponse::new(outcome))
        })
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
    Rc<RefCell<Vec<acp::SessionNotification>>>,
) {
    let runtime = make_runtime();
    let notifications = Rc::new(RefCell::new(Vec::new()));
    let client = MockClient::new(notifications.clone());
    let (transport, agent_fut) = create_in_process_transport(runtime, client);

    tokio::task::spawn_local(agent_fut);

    let _ = transport
        .client()
        .initialize(acp::InitializeRequest::new(acp::ProtocolVersion::LATEST))
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
            .new_session(acp::NewSessionRequest::new("."))
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
            .new_session(acp::NewSessionRequest::new("."))
            .await
            .unwrap();
        let session_id = session_resp.session_id;

        let cancel_result = acp_client
            .cancel(acp::CancelNotification::new(session_id))
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
            .new_session(acp::NewSessionRequest::new("."))
            .await
            .unwrap();
        let session_id = session_resp.session_id;

        let close_result = acp_client
            .close_session(acp::CloseSessionRequest::new(session_id))
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
            .new_session(acp::NewSessionRequest::new("."))
            .await
            .unwrap()
            .session_id;
        let s2 = acp_client
            .new_session(acp::NewSessionRequest::new("."))
            .await
            .unwrap()
            .session_id;

        assert_ne!(s1, s2);

        let _ = acp_client
            .close_session(acp::CloseSessionRequest::new(s1))
            .await;
        let _ = acp_client
            .close_session(acp::CloseSessionRequest::new(s2))
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
            .new_session(acp::NewSessionRequest::new("."))
            .await
            .unwrap();
        let session_id = session_resp.session_id;

        let result = acp_client
            .prompt(acp::PromptRequest::new(
                session_id,
                vec![acp::ContentBlock::Text(acp::TextContent::new("hello"))],
            ))
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().stop_reason, acp::StopReason::EndTurn);
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
            .new_session(acp::NewSessionRequest::new("."))
            .await
            .unwrap()
            .session_id;
        let s2 = transport2
            .client()
            .new_session(acp::NewSessionRequest::new("."))
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
            .initialize(acp::InitializeRequest::new(acp::ProtocolVersion::LATEST))
            .await
            .unwrap();
        let _ = transport2
            .client()
            .initialize(acp::InitializeRequest::new(acp::ProtocolVersion::LATEST))
            .await
            .unwrap();

        let session_resp = transport1
            .client()
            .new_session(acp::NewSessionRequest::new("."))
            .await
            .unwrap();
        let session_id = session_resp.session_id;

        let result = transport2
            .client()
            .prompt(acp::PromptRequest::new(
                session_id,
                vec![acp::ContentBlock::Text(acp::TextContent::new("intrude"))],
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
            .initialize(acp::InitializeRequest::new(acp::ProtocolVersion::LATEST))
            .await
            .unwrap();
        let _ = transport2
            .client()
            .initialize(acp::InitializeRequest::new(acp::ProtocolVersion::LATEST))
            .await
            .unwrap();

        let session_resp = transport1
            .client()
            .new_session(acp::NewSessionRequest::new("."))
            .await
            .unwrap();
        let session_id = session_resp.session_id;

        let result = transport2
            .client()
            .close_session(acp::CloseSessionRequest::new(session_id))
            .await;

        assert!(result.is_err(), "expected cross-connection close to fail");
    });
}
