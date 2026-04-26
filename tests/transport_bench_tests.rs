use iron_core::{
    runtime::IronRuntime,
    transport::{create_in_process_transport, InProcessClientHandler, InProcessTransport},
    Config,
};
use iron_providers::{OpenAiConfig, OpenAiProvider};
use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

use agent_client_protocol::schema as acp;

struct NopClient;

impl InProcessClientHandler for NopClient {
    fn request_permission(
        &self,
        _args: acp::RequestPermissionRequest,
    ) -> Pin<Box<dyn Future<Output = agent_client_protocol::Result<acp::RequestPermissionResponse>>>>
    {
        Box::pin(async move {
            Ok(acp::RequestPermissionResponse::new(
                acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                    acp::PermissionOptionId::new("allow"),
                )),
            ))
        })
    }

    fn session_notification(
        &self,
        _args: acp::SessionNotification,
    ) -> Pin<Box<dyn Future<Output = agent_client_protocol::Result<()>>>> {
        Box::pin(async { Ok(()) })
    }
}

fn make_runtime() -> IronRuntime {
    let config = Config::new().with_model("test-model");
    let provider = OpenAiProvider::new(OpenAiConfig::new("test-key".into()))
        .expect("test provider config should be valid");
    IronRuntime::new(config, provider)
}

async fn setup() -> InProcessTransport {
    let runtime = make_runtime();
    let (transport, agent_fut) = create_in_process_transport(runtime, NopClient);
    tokio::task::spawn_local(agent_fut);
    let _ = transport
        .client()
        .initialize(acp::InitializeRequest::new(acp::ProtocolVersion::LATEST))
        .await
        .unwrap();
    transport
}

#[test]
fn bench_initialize_round_trip() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let mut durations = Vec::new();
        for _ in 0..100 {
            let runtime = make_runtime();
            let (transport, agent_fut) = create_in_process_transport(runtime, NopClient);
            tokio::task::spawn_local(agent_fut);
            let start = Instant::now();
            let _ = transport
                .client()
                .initialize(acp::InitializeRequest::new(acp::ProtocolVersion::LATEST))
                .await
                .unwrap();
            durations.push(start.elapsed());
        }
        let total: std::time::Duration = durations.iter().sum();
        let avg = total / 100;
        eprintln!(
            "bench_initialize: 100 calls, total={:?}, avg={:?}/call",
            total, avg
        );
        assert!(
            avg.as_millis() < 100,
            "initialize round-trip avg should be <100ms, got {:?}",
            avg
        );
    });
}

#[test]
fn bench_new_session_round_trip() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let transport = setup().await;
        let client = transport.client();
        let start = Instant::now();
        for _ in 0..100 {
            let _ = client
                .new_session(acp::NewSessionRequest::new("."))
                .await
                .unwrap();
        }
        let elapsed = start.elapsed();
        let avg = elapsed / 100;
        eprintln!(
            "bench_new_session: 100 calls, total={:?}, avg={:?}/call",
            elapsed, avg
        );
        assert!(
            avg.as_millis() < 100,
            "newSession round-trip avg should be <100ms, got {:?}",
            avg
        );
    });
}

#[test]
fn bench_prompt_round_trip_with_fake_provider() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let transport = setup().await;
        let client = transport.client();
        let session = client
            .new_session(acp::NewSessionRequest::new("."))
            .await
            .unwrap()
            .session_id;

        let start = Instant::now();
        for _ in 0..50 {
            let _ = client
                .prompt(acp::PromptRequest::new(
                    session.clone(),
                    vec![acp::ContentBlock::Text(acp::TextContent::new("hi"))],
                ))
                .await
                .unwrap();
        }
        let elapsed = start.elapsed();
        let avg = elapsed / 50;
        eprintln!(
            "bench_prompt (fake provider): 50 calls, total={:?}, avg={:?}/call",
            elapsed, avg
        );
        assert!(
            avg.as_millis() < 500,
            "prompt round-trip avg should be <500ms, got {:?}",
            avg
        );
    });
}
