//! Tests for MCP (Model Context Protocol) session-scoped support

use futures::StreamExt;
use iron_core::{
    config::McpConfig, Config, IronAgent, McpServerConfig, McpServerHealth, McpTransport,
    SessionId,
};
use iron_providers::{Provider, ProviderEvent};

// Mock provider for testing
#[derive(Default)]
struct MockProvider;

impl Provider for MockProvider {
    fn infer(
        &self,
        _request: iron_providers::InferenceRequest,
    ) -> iron_providers::ProviderFuture<'_, Vec<ProviderEvent>> {
        Box::pin(async { Ok(vec![ProviderEvent::Complete]) })
    }

    fn infer_stream(
        &self,
        _request: iron_providers::InferenceRequest,
    ) -> iron_providers::ProviderFuture<'_, futures::stream::BoxStream<'static, iron_providers::ProviderResult<ProviderEvent>>>
    {
        Box::pin(async { 
            Ok(futures::stream::iter(vec![Ok(ProviderEvent::Complete)]).boxed()) 
        })
    }
}

#[test]
fn new_session_uses_runtime_default_enablement_enabled() {
    let config = Config::new()
        .with_mcp(
            McpConfig::new()
                .with_enabled(true)
                .with_enabled_by_default(true),
        );

    let runtime = iron_core::IronRuntime::new(config, MockProvider::default());

    // Register an MCP server
    let server_config = McpServerConfig {
        id: "test-server".to_string(),
        label: "Test Server".to_string(),
        transport: McpTransport::Http {
            url: "http://localhost:8080".to_string(),
        },
        enabled_by_default: true,
        working_dir: None,
    };
    runtime.register_mcp_server(server_config);

    // Create a connection and session
    let conn = iron_core::IronConnection::new(runtime.clone());
    let (session_id, session) = runtime
        .create_session(iron_core::ConnectionId(1))
        .expect("Failed to create session");

    // Check that the MCP server is enabled by default
    let session_guard = session.lock().unwrap();
    assert_eq!(
        session_guard.is_mcp_server_enabled("test-server"),
        Some(true),
        "MCP server should be enabled by default when runtime has enabled_by_default=true"
    );
}

#[test]
fn new_session_uses_runtime_default_enablement_disabled() {
    let config = Config::new()
        .with_mcp(
            McpConfig::new()
                .with_enabled(true)
                .with_enabled_by_default(false),
        );

    let runtime = iron_core::IronRuntime::new(config, MockProvider::default());

    // Register an MCP server
    let server_config = McpServerConfig {
        id: "test-server".to_string(),
        label: "Test Server".to_string(),
        transport: McpTransport::Http {
            url: "http://localhost:8080".to_string(),
        },
        enabled_by_default: true,
        working_dir: None,
    };
    runtime.register_mcp_server(server_config);

    // Create a connection and session
    let (session_id, session) = runtime
        .create_session(iron_core::ConnectionId(1))
        .expect("Failed to create session");

    // Check that the MCP server is disabled (runtime default overrides server default)
    let session_guard = session.lock().unwrap();
    assert_eq!(
        session_guard.is_mcp_server_enabled("test-server"),
        Some(false),
        "MCP server should be disabled when runtime has enabled_by_default=false"
    );
}

#[test]
fn session_toggle_does_not_affect_another_session() {
    let config = Config::new()
        .with_mcp(
            McpConfig::new()
                .with_enabled(true)
                .with_enabled_by_default(true),
        );

    let runtime = iron_core::IronRuntime::new(config, MockProvider::default());

    // Register an MCP server
    let server_config = McpServerConfig {
        id: "test-server".to_string(),
        label: "Test Server".to_string(),
        transport: McpTransport::Http {
            url: "http://localhost:8080".to_string(),
        },
        enabled_by_default: true,
        working_dir: None,
    };
    runtime.register_mcp_server(server_config);

    // Create two sessions
    let (session1_id, session1) = runtime
        .create_session(iron_core::ConnectionId(1))
        .expect("Failed to create session 1");

    let (session2_id, session2) = runtime
        .create_session(iron_core::ConnectionId(2))
        .expect("Failed to create session 2");

    // Disable the MCP server for session 1
    {
        let mut session1_guard = session1.lock().unwrap();
        session1_guard.set_mcp_server_enabled("test-server", false);
    }

    // Verify session 1 has the server disabled
    let session1_guard = session1.lock().unwrap();
    assert_eq!(
        session1_guard.is_mcp_server_enabled("test-server"),
        Some(false),
        "Session 1 should have MCP server disabled"
    );
    drop(session1_guard);

    // Verify session 2 still has the server enabled
    let session2_guard = session2.lock().unwrap();
    assert_eq!(
        session2_guard.is_mcp_server_enabled("test-server"),
        Some(true),
        "Session 2 should still have MCP server enabled"
    );
}

#[test]
fn mcp_state_not_included_in_handoff() {
    use iron_core::context::{HandoffExporter, HandoffImporter};
    use iron_core::context::config::ContextManagementConfig;

    let config = Config::new()
        .with_mcp(
            McpConfig::new()
                .with_enabled(true)
                .with_enabled_by_default(true),
        )
        .with_context_management(ContextManagementConfig::default());

    let runtime = iron_core::IronRuntime::new(config.clone(), MockProvider::default());

    // Register an MCP server
    let server_config = McpServerConfig {
        id: "test-server".to_string(),
        label: "Test Server".to_string(),
        transport: McpTransport::Http {
            url: "http://localhost:8080".to_string(),
        },
        enabled_by_default: true,
        working_dir: None,
    };
    runtime.register_mcp_server(server_config);

    // Create a session with MCP enabled
    let (session_id, session) = runtime
        .create_session(iron_core::ConnectionId(1))
        .expect("Failed to create session");

    // Verify MCP is enabled
    {
        let session_guard = session.lock().unwrap();
        assert_eq!(
            session_guard.is_mcp_server_enabled("test-server"),
            Some(true)
        );
    }

    // Export handoff bundle
    let session_guard = session.lock().unwrap();
    let bundle = HandoffExporter::export(
        &session_guard,
        "test-model",
        None,
        vec![],
        &config.context_management,
        None,
    )
    .expect("Failed to export handoff");
    drop(session_guard);

    // Create a new session and import the bundle
    let new_session_id = SessionId::new();
    let mut new_durable = iron_core::durable::DurableSession::new(new_session_id);
    
    HandoffImporter::hydrate(&mut new_durable, bundle)
        .expect("Failed to import handoff");

    // Verify MCP enablement state was NOT imported (should be empty)
    assert!(
        new_durable.mcp_server_enablement.is_empty(),
        "MCP enablement state should not be included in handoff"
    );
    assert_eq!(
        new_durable.is_mcp_server_enabled("test-server"),
        None,
        "MCP server enablement should be None after handoff import"
    );
}
