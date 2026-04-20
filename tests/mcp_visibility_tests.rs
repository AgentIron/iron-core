//! Tests for MCP tool visibility through the canonical session catalog

use iron_core::{
    mcp::McpConnectionManager, plugin::wasm_host::WasmHost, plugin::PluginRegistry,
    tool::ToolRegistry, HttpConfig, McpServerConfig, McpServerHealth, McpServerRegistry,
    McpToolInfo, McpTransport, SessionToolCatalog,
};
use std::sync::Arc;

fn create_test_registry() -> Arc<McpServerRegistry> {
    let registry = McpServerRegistry::new();

    // Register a test server
    let server_config = McpServerConfig {
        id: "test-server".to_string(),
        label: "Test Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8080".to_string()),
        },
        enabled_by_default: true,
        working_dir: None,
    };
    registry.register_server(server_config);

    Arc::new(registry)
}

fn create_test_session_with_mcp() -> iron_core::durable::DurableSession {
    let session_id = iron_core::SessionId::new();
    let mut session = iron_core::durable::DurableSession::new(session_id);

    // Enable the test MCP server
    session.set_mcp_server_enabled("test-server", true);

    session
}

fn create_test_session_without_mcp() -> iron_core::durable::DurableSession {
    let session_id = iron_core::SessionId::new();
    let mut session = iron_core::durable::DurableSession::new(session_id);

    // Explicitly disable the test MCP server
    session.set_mcp_server_enabled("test-server", false);

    session
}

fn build_catalog(
    local_registry: Arc<ToolRegistry>,
    mcp_registry: Arc<McpServerRegistry>,
    session: &iron_core::durable::DurableSession,
) -> SessionToolCatalog {
    let plugin_registry = Arc::new(PluginRegistry::new());
    let wasm_host = Arc::new(WasmHost::new());
    let connection_manager = Arc::new(McpConnectionManager::new((*mcp_registry).clone()));

    SessionToolCatalog::new(
        local_registry,
        mcp_registry,
        plugin_registry,
        wasm_host,
        connection_manager,
        session,
    )
}

#[test]
fn disabled_server_tools_are_hidden() {
    let local_registry = Arc::new(ToolRegistry::new());
    let mcp_registry = create_test_registry();

    // Simulate the server being connected and having tools
    mcp_registry.update_discovered_tools(
        "test-server",
        vec![McpToolInfo {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            input_schema: serde_json::json!({}),
        }],
    );
    mcp_registry.update_health("test-server", McpServerHealth::Connected);

    // Session with MCP disabled
    let session_without = create_test_session_without_mcp();
    let tools_without = build_catalog(
        local_registry.clone(),
        mcp_registry.clone(),
        &session_without,
    )
    .definitions()
    .to_vec();

    // Session with MCP enabled
    let session_with = create_test_session_with_mcp();
    let tools_with = build_catalog(local_registry, mcp_registry, &session_with)
        .definitions()
        .to_vec();

    // The disabled session should have fewer tools
    assert!(
        tools_without.len() < tools_with.len(),
        "Disabled MCP server should result in fewer visible tools"
    );

    // Check that the MCP tool is not in the disabled session
    let has_mcp_tool = tools_without.iter().any(|t| t.name.contains("test_tool"));
    assert!(
        !has_mcp_tool,
        "MCP tool should not be visible when server is disabled"
    );
}

#[test]
fn errored_server_tools_are_hidden() {
    let local_registry = Arc::new(ToolRegistry::new());
    let mcp_registry = create_test_registry();

    // Simulate the server being connected initially
    mcp_registry.update_discovered_tools(
        "test-server",
        vec![McpToolInfo {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            input_schema: serde_json::json!({}),
        }],
    );
    mcp_registry.update_health("test-server", McpServerHealth::Connected);

    // Session with MCP enabled
    let session = create_test_session_with_mcp();
    let tools_connected = build_catalog(local_registry.clone(), mcp_registry.clone(), &session)
        .definitions()
        .to_vec();

    // Now simulate server going into error state
    mcp_registry.update_health("test-server", McpServerHealth::Error);

    let tools_errored = build_catalog(local_registry, mcp_registry, &session)
        .definitions()
        .to_vec();

    // Tools should be hidden when server is in error state
    assert!(
        tools_errored.len() < tools_connected.len(),
        "Errored MCP server should result in fewer visible tools"
    );

    let has_mcp_tool = tools_errored.iter().any(|t| t.name.contains("test_tool"));
    assert!(
        !has_mcp_tool,
        "MCP tool should not be visible when server is in error state"
    );
}

#[test]
fn reconnected_server_tools_return_for_enabled_sessions() {
    let local_registry = Arc::new(ToolRegistry::new());
    let mcp_registry = create_test_registry();

    // Simulate the server being connected
    mcp_registry.update_discovered_tools(
        "test-server",
        vec![McpToolInfo {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            input_schema: serde_json::json!({}),
        }],
    );
    mcp_registry.update_health("test-server", McpServerHealth::Connected);

    // Session with MCP enabled
    let session = create_test_session_with_mcp();

    // Initial connected state
    let tools_connected = build_catalog(local_registry.clone(), mcp_registry.clone(), &session)
        .definitions()
        .to_vec();

    // Server goes to error
    mcp_registry.update_health("test-server", McpServerHealth::Error);
    let tools_errored = build_catalog(local_registry.clone(), mcp_registry.clone(), &session)
        .definitions()
        .to_vec();

    // Server reconnects
    mcp_registry.update_health("test-server", McpServerHealth::Connected);
    let tools_reconnected = build_catalog(local_registry, mcp_registry, &session)
        .definitions()
        .to_vec();

    // Tools should return after reconnection
    assert_eq!(
        tools_connected.len(),
        tools_reconnected.len(),
        "Tools should return after server reconnects for enabled sessions"
    );

    assert!(
        tools_connected.len() > tools_errored.len(),
        "Connected state should have more tools than errored state"
    );
}
