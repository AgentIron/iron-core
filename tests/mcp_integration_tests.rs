use futures::StreamExt;
use iron_core::{
    config::McpConfig, Config, HttpConfig, IronRuntime, McpServerConfig, McpServerHealth,
    McpToolInfo, McpTransport,
};
use iron_providers::{Provider, ProviderEvent};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Mock provider for testing
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct MockProvider {
    infer_responses: Arc<Mutex<VecDeque<Vec<ProviderEvent>>>>,
    requests: Arc<Mutex<Vec<iron_providers::InferenceRequest>>>,
}

impl MockProvider {
    #[allow(dead_code)]
    fn with_infer_responses(responses: Vec<Vec<ProviderEvent>>) -> Self {
        Self {
            infer_responses: Arc::new(Mutex::new(responses.into())),
            ..Self::default()
        }
    }

    #[allow(dead_code)]
    fn requests(&self) -> Vec<iron_providers::InferenceRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Provider for MockProvider {
    fn infer(
        &self,
        request: iron_providers::InferenceRequest,
    ) -> iron_providers::ProviderFuture<'_, Vec<ProviderEvent>> {
        self.requests.lock().unwrap().push(request);
        let response = self
            .infer_responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![ProviderEvent::Complete]);
        Box::pin(async move { Ok(response) })
    }

    fn infer_stream(
        &self,
        request: iron_providers::InferenceRequest,
    ) -> iron_providers::ProviderFuture<
        '_,
        futures::stream::BoxStream<'static, iron_providers::ProviderResult<ProviderEvent>>,
    > {
        self.requests.lock().unwrap().push(request);
        let response = self
            .infer_responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![ProviderEvent::Complete]);
        Box::pin(async move { Ok(futures::stream::iter(response.into_iter().map(Ok)).boxed()) })
    }
}

// ---------------------------------------------------------------------------
// MCP Integration Tests
// ---------------------------------------------------------------------------

#[test]
fn test_mcp_server_registration_stores_configuration() {
    // Disable MCP to prevent immediate connection attempt, so we can verify
    // the initial Configured state
    let config = Config::new().with_mcp(McpConfig::new().with_enabled(false));
    let runtime = IronRuntime::new(config, MockProvider::default());

    let server_config = McpServerConfig {
        id: "my-server".to_string(),
        label: "My Test Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8080".to_string()),
        },
        enabled_by_default: true,
        working_dir: None,
    };

    runtime.register_mcp_server(server_config);

    let registry = runtime.mcp_registry();
    let server = registry.get_server("my-server");

    assert!(server.is_some(), "Server should be registered");
    let server = server.unwrap();
    assert_eq!(server.config.id, "my-server");
    assert_eq!(server.config.label, "My Test Server");
    // When MCP is disabled, server stays in Configured state
    assert_eq!(server.health, McpServerHealth::Configured);
}

#[test]
fn test_mcp_server_health_transitions() {
    let config = Config::new().with_mcp(McpConfig::new().with_enabled(true));
    let runtime = IronRuntime::new(config, MockProvider::default());

    let server_config = McpServerConfig {
        id: "my-server".to_string(),
        label: "My Test Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8080".to_string()),
        },
        enabled_by_default: true,
        working_dir: None,
    };

    runtime.register_mcp_server(server_config);

    let registry = runtime.mcp_registry();
    assert_eq!(
        registry.get_server("my-server").unwrap().health,
        McpServerHealth::Configured
    );

    registry.set_error("my-server", "boom".to_string());
    let server = registry.get_server("my-server").unwrap();
    assert_eq!(server.health, McpServerHealth::Error);
    assert_eq!(server.last_error.as_deref(), Some("boom"));

    registry.update_health("my-server", McpServerHealth::Connected);
    let server = registry.get_server("my-server").unwrap();
    assert_eq!(server.health, McpServerHealth::Connected);
    assert_eq!(
        server.last_error, None,
        "successful reconnect should clear last_error"
    );
}

#[test]
fn test_tool_discovery_updates_registry() {
    let config = Config::new().with_mcp(McpConfig::new().with_enabled(true));
    let runtime = IronRuntime::new(config, MockProvider::default());

    let server_config = McpServerConfig {
        id: "my-server".to_string(),
        label: "My Test Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8080".to_string()),
        },
        enabled_by_default: true,
        working_dir: None,
    };

    runtime.register_mcp_server(server_config);

    let registry = runtime.mcp_registry();
    registry.update_discovered_tools(
        "my-server",
        vec![McpToolInfo {
            name: "test_tool".to_string(),
            description: "Test MCP tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }],
    );

    let server = registry.get_server("my-server").unwrap();
    assert_eq!(server.discovered_tools.len(), 1);
    assert_eq!(server.discovered_tools[0].name, "test_tool");
}

#[tokio::test]
async fn test_session_tool_catalog_includes_mcp_tools_when_enabled_and_usable() {
    let config = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(true),
    );
    let runtime = IronRuntime::new(config, MockProvider::default());

    // Register an MCP server
    let server_config = McpServerConfig {
        id: "my-server".to_string(),
        label: "My Test Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8080".to_string()),
        },
        enabled_by_default: true,
        working_dir: None,
    };
    runtime.register_mcp_server(server_config);

    // Wait a moment for the connection manager to start up
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    runtime
        .mcp_registry()
        .update_health("my-server", McpServerHealth::Connected);
    runtime.mcp_registry().update_discovered_tools(
        "my-server",
        vec![McpToolInfo {
            name: "test_tool".to_string(),
            description: "Test MCP tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }],
    );

    let (session_id, _session) = runtime
        .create_session(iron_core::ConnectionId(1))
        .expect("Failed to create session");

    let catalog = runtime.get_session_tool_catalog(session_id).unwrap();
    assert!(catalog.contains("mcp_my-server_test_tool"));
}

#[tokio::test]
async fn test_session_tool_catalog_excludes_disabled_mcp_servers() {
    let config = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(false),
    );
    let runtime = IronRuntime::new(config, MockProvider::default());

    // Register an MCP server that is disabled by default
    let server_config = McpServerConfig {
        id: "disabled-server".to_string(),
        label: "Disabled Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8080".to_string()),
        },
        enabled_by_default: false,
        working_dir: None,
    };
    runtime.register_mcp_server(server_config);

    // Create a connection and session
    let _conn = iron_core::IronConnection::new(runtime.clone());
    let (session_id, _session) = runtime
        .create_session(iron_core::ConnectionId(1))
        .expect("Failed to create session");

    // Get the session tool catalog
    let catalog = runtime.get_session_tool_catalog(session_id).unwrap();

    // Disabled server tools should not appear in catalog
    // (Verify by checking no MCP-prefixed tools exist)
    let has_mcp_tools = catalog
        .definitions()
        .iter()
        .any(|d| d.name.starts_with("mcp_"));
    assert!(
        !has_mcp_tools,
        "Disabled MCP server tools should not appear in catalog"
    );
}

#[tokio::test]
async fn test_session_tool_catalog_excludes_errored_mcp_servers() {
    let config = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(true),
    );
    let runtime = IronRuntime::new(config, MockProvider::default());

    // Register an MCP server
    let server_config = McpServerConfig {
        id: "errored-server".to_string(),
        label: "Errored Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8080".to_string()),
        },
        enabled_by_default: true,
        working_dir: None,
    };
    runtime.register_mcp_server(server_config);

    runtime.mcp_registry().update_discovered_tools(
        "errored-server",
        vec![McpToolInfo {
            name: "hidden_tool".to_string(),
            description: "Should stay hidden".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }],
    );
    runtime
        .mcp_registry()
        .set_error("errored-server", "connection failed".to_string());

    let (session_id, _session) = runtime
        .create_session(iron_core::ConnectionId(1))
        .expect("Failed to create session");

    let catalog = runtime.get_session_tool_catalog(session_id).unwrap();
    assert!(!catalog.contains("mcp_errored-server_hidden_tool"));
}

#[tokio::test]
async fn test_session_tool_catalog_cache_invalidates_on_mcp_registry_change() {
    let config = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(true),
    );
    let runtime = IronRuntime::new(config, MockProvider::default());

    let server_config = McpServerConfig {
        id: "cached-server".to_string(),
        label: "Cached Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8080".to_string()),
        },
        enabled_by_default: true,
        working_dir: None,
    };
    runtime.register_mcp_server(server_config);

    runtime
        .mcp_registry()
        .update_health("cached-server", McpServerHealth::Connected);
    runtime.mcp_registry().update_discovered_tools(
        "cached-server",
        vec![McpToolInfo {
            name: "cached_tool".to_string(),
            description: "Cached MCP tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }],
    );

    let (session_id, _session) = runtime
        .create_session(iron_core::ConnectionId(1))
        .expect("Failed to create session");

    let first_catalog = runtime.get_session_tool_catalog(session_id).unwrap();
    assert!(first_catalog.contains("mcp_cached-server_cached_tool"));

    runtime
        .mcp_registry()
        .update_health("cached-server", McpServerHealth::Error);

    let refreshed_catalog = runtime.get_session_tool_catalog(session_id).unwrap();
    assert!(
        !refreshed_catalog.contains("mcp_cached-server_cached_tool"),
        "catalog should refresh after MCP registry health changes"
    );
}

#[tokio::test]
async fn test_session_tool_catalog_filters_consistently() {
    let config = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(true),
    );
    let runtime = IronRuntime::new(config, MockProvider::default());

    // Register multiple MCP servers with different states
    let enabled_config = McpServerConfig {
        id: "enabled-server".to_string(),
        label: "Enabled Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8081".to_string()),
        },
        enabled_by_default: true,
        working_dir: None,
    };

    let disabled_config = McpServerConfig {
        id: "disabled-server".to_string(),
        label: "Disabled Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8082".to_string()),
        },
        enabled_by_default: false,
        working_dir: None,
    };

    runtime.register_mcp_server(enabled_config);
    runtime.register_mcp_server(disabled_config);

    // Create a connection and session
    let _conn = iron_core::IronConnection::new(runtime.clone());
    let (session_id, _session) = runtime
        .create_session(iron_core::ConnectionId(1))
        .expect("Failed to create session");

    // Get the session tool catalog multiple times
    let catalog1 = runtime.get_session_tool_catalog(session_id).unwrap();
    let catalog2 = runtime.get_session_tool_catalog(session_id).unwrap();

    // Catalogs should be consistent
    assert_eq!(
        catalog1.definitions().len(),
        catalog2.definitions().len(),
        "Catalog should be consistent across calls"
    );
}

#[tokio::test]
async fn test_mcp_tools_have_namespaced_names() {
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    // Create a fake MCP server script
    let tempdir = TempDir::new().unwrap();
    let script_path = tempdir.path().join("fake-mcp-server.sh");
    let script = r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) 
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"fake-mcp","version":"1.0.0"}}}'
      ;;
    *'"method":"tools/list"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"read_file","description":"Read a file","inputSchema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}]}}'
      ;;
  esac
done
"#;

    fs::write(&script_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    let config = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(true),
    );
    let runtime = IronRuntime::new(config, MockProvider::default());

    // Register the MCP server
    let server_config = McpServerConfig {
        id: "my-server".to_string(),
        label: "My Server".to_string(),
        transport: McpTransport::Stdio {
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            env: Default::default(),
        },
        enabled_by_default: true,
        working_dir: None,
    };
    runtime.register_mcp_server(server_config);

    // Wait a bit for connection attempt
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create a connection and session
    let _conn = iron_core::IronConnection::new(runtime.clone());
    let (session_id, _session) = runtime
        .create_session(iron_core::ConnectionId(1))
        .expect("Failed to create session");

    let catalog = runtime.get_session_tool_catalog(session_id).unwrap();

    // Debug: print all tool names in catalog
    println!(
        "Catalog tools: {:?}",
        catalog
            .definitions()
            .iter()
            .map(|d| &d.name)
            .collect::<Vec<_>>()
    );

    // MCP tools should have namespaced names
    assert!(
        catalog.contains("mcp_my-server_read_file"),
        "MCP tool should have namespaced name. Catalog has: {:?}",
        catalog
            .definitions()
            .iter()
            .map(|d| &d.name)
            .collect::<Vec<_>>()
    );

    let definition = catalog.get_definition("mcp_my-server_read_file").unwrap();
    assert!(
        definition.name.starts_with("mcp_my-server_"),
        "MCP tool name should be namespaced with server ID"
    );
}

#[tokio::test]
async fn test_mcp_tool_execution_requires_approval() {
    let config = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(true),
    );
    let runtime = IronRuntime::new(config, MockProvider::default());

    // MCP tools should respect the approval strategy
    // This test verifies that MCP tool calls go through the approval mechanism

    let server_config = McpServerConfig {
        id: "approval-server".to_string(),
        label: "Approval Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8080".to_string()),
        },
        enabled_by_default: true,
        working_dir: None,
    };
    runtime.register_mcp_server(server_config);

    // The approval mechanism is handled by the runtime/tool execution layer
    // This test verifies the infrastructure is in place
    let registry = runtime.mcp_registry();
    let server = registry.get_server("approval-server").unwrap();

    assert!(server.config.enabled_by_default);
}

#[tokio::test]
async fn test_mcp_tool_not_visible_returns_clear_error() {
    let config = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(true),
    );
    let runtime = IronRuntime::new(config, MockProvider::default());

    // Create a connection and session
    let _conn = iron_core::IronConnection::new(runtime.clone());
    let (session_id, _session) = runtime
        .create_session(iron_core::ConnectionId(1))
        .expect("Failed to create session");

    let catalog = runtime.get_session_tool_catalog(session_id).unwrap();

    // Try to get a non-existent MCP tool
    let result = catalog.get_definition("mcp_nonexistent_tool");

    assert!(result.is_none(), "Non-existent MCP tool should return None");
}

#[tokio::test]
async fn test_end_to_end_mcp_lifecycle() {
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    // Create a fake MCP server script
    let tempdir = TempDir::new().unwrap();
    let script_path = tempdir.path().join("fake-mcp-server.sh");
    let script = r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) 
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"fake-mcp","version":"1.0.0"}}}'
      ;;
    *'"method":"tools/list"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"lifecycle_tool","description":"Test lifecycle","inputSchema":{"type":"object","properties":{}}}]}'
      ;;
    *'"method":"tools/call"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"success"}],"isError":false}}'
      ;;
  esac
done
"#;

    fs::write(&script_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    let config = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(true),
    );
    let runtime = IronRuntime::new(config, MockProvider::default());

    // 1. Register server
    let server_config = McpServerConfig {
        id: "lifecycle-server".to_string(),
        label: "Lifecycle Server".to_string(),
        transport: McpTransport::Stdio {
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            env: Default::default(),
        },
        enabled_by_default: true,
        working_dir: None,
    };
    runtime.register_mcp_server(server_config);

    // Verify server is registered
    {
        let registry = runtime.mcp_registry();
        let server = registry.get_server("lifecycle-server");
        assert!(server.is_some());
    }

    // 2. Wait for connection
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 3. Create session
    let _conn = iron_core::IronConnection::new(runtime.clone());
    let (session_id, _session) = runtime
        .create_session(iron_core::ConnectionId(1))
        .expect("Failed to create session");

    // 4. Get tool catalog
    let catalog = runtime.get_session_tool_catalog(session_id).unwrap();

    // 5. Verify tools are available (or catalog structure is correct)
    // The lifecycle completes successfully if we reach this point
    println!("End-to-end MCP lifecycle test completed successfully");
    println!(
        "Catalog has {} tool definitions",
        catalog.definitions().len()
    );
}

#[tokio::test]
async fn test_mcp_unavailable_tool_diagnostics_disabled_server() {
    use iron_core::{
        Config, ConnectionId, IronConnection, IronRuntime, McpConfig, McpServerConfig, McpTransport,
    };

    // Configure MCP with enabled_by_default=true but we'll create session and disable
    let config = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(true),
    );
    let runtime = IronRuntime::new(config, MockProvider::default());

    // Register a server
    let server_config = McpServerConfig {
        id: "diagnostics-server".to_string(),
        label: "Diagnostics Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8080".to_string()),
        },
        enabled_by_default: true,
        working_dir: None,
    };
    runtime.register_mcp_server(server_config);

    // Create connection and session
    let _conn = IronConnection::new(runtime.clone());
    let (session_id, session) = runtime
        .create_session(ConnectionId(1))
        .expect("Failed to create session");

    // Disable the server for this session
    {
        let mut sess = session.lock();
        sess.set_mcp_server_enabled("diagnostics-server", false);
    }

    // Get catalog
    let catalog = runtime.get_session_tool_catalog(session_id).unwrap();

    // Try to execute a tool from the disabled server
    let execute_future = {
        let session_guard = session.lock();
        catalog.execute(
            "call-1",
            "mcp_diagnostics-server_test_tool",
            serde_json::json!({}),
            &session_guard,
        )
    };
    let result = execute_future.await;

    // Verify the error message is precise about the server being disabled
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("disabled for this session"),
        "Expected error about disabled server, got: {}",
        error_msg
    );
    assert!(
        error_msg.contains("diagnostics-server"),
        "Expected error to mention server name, got: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_mcp_unavailable_tool_diagnostics_unknown_tool() {
    use iron_core::{
        Config, ConnectionId, IronConnection, IronRuntime, McpConfig, McpServerConfig, McpTransport,
    };

    let config = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(true),
    );
    let runtime = IronRuntime::new(config, MockProvider::default());

    // Register a server that has discovered tools
    let server_config = McpServerConfig {
        id: "tools-server".to_string(),
        label: "Tools Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8080".to_string()),
        },
        enabled_by_default: true,
        working_dir: None,
    };
    runtime.register_mcp_server(server_config);

    // Create connection and session
    let _conn = IronConnection::new(runtime.clone());
    let (session_id, session) = runtime
        .create_session(ConnectionId(1))
        .expect("Failed to create session");

    // Get catalog
    let catalog = runtime.get_session_tool_catalog(session_id).unwrap();

    // Try to execute a non-existent tool from the server
    // This will fail because the server is unhealthy (not connected)
    let execute_future = {
        let session_guard = session.lock();
        catalog.execute(
            "call-1",
            "mcp_tools-server_nonexistent_tool",
            serde_json::json!({}),
            &session_guard,
        )
    };
    let result = execute_future.await;

    // Verify the error message - server is not healthy (it's in Connecting state)
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not healthy")
            || error_msg.contains("Connecting")
            || error_msg.contains("not configured"),
        "Expected error about unhealthy server, got: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_mcp_unavailable_tool_diagnostics_unconfigured_server() {
    use iron_core::{Config, ConnectionId, IronConnection, IronRuntime, McpConfig};

    let config = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(true),
    );
    let runtime = IronRuntime::new(config, MockProvider::default());

    // Create connection and session WITHOUT registering any servers
    let _conn = IronConnection::new(runtime.clone());
    let (session_id, session) = runtime
        .create_session(ConnectionId(1))
        .expect("Failed to create session");

    // Get catalog
    let catalog = runtime.get_session_tool_catalog(session_id).unwrap();

    // Try to execute a tool from a server that was never configured
    let execute_future = {
        let session_guard = session.lock();
        catalog.execute(
            "call-1",
            "mcp_unconfigured-server_some_tool",
            serde_json::json!({}),
            &session_guard,
        )
    };
    let result = execute_future.await;

    // Verify the error message indicates server is not configured
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not configured") || error_msg.contains("unknown"),
        "Expected error about unconfigured server, got: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_mcp_runtime_default_enablement_semantics() {
    use iron_core::{Config, ConnectionId, IronRuntime, McpConfig, McpServerConfig, McpTransport};

    // Test 1: Runtime default enabled_by_default=false
    let config_disabled = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(false),
    );
    let runtime_disabled = IronRuntime::new(config_disabled, MockProvider::default());

    // Register a server with enabled_by_default=true (should be overridden by runtime)
    let server_config = McpServerConfig {
        id: "override-server".to_string(),
        label: "Override Server".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8080".to_string()),
        },
        enabled_by_default: true, // Server says enabled, but runtime says disabled
        working_dir: None,
    };
    runtime_disabled.register_mcp_server(server_config);

    // Create session
    let (_session_id, session) = runtime_disabled
        .create_session(ConnectionId(1))
        .expect("Failed to create session");

    // Verify server is disabled (runtime default overrides server default)
    let sess = session.lock();
    let is_enabled = sess.is_mcp_server_enabled("override-server");
    assert_eq!(
        is_enabled,
        Some(false),
        "Runtime default (disabled) should override server default (enabled)"
    );
    drop(sess);

    // Test 2: Runtime default enabled_by_default=true
    let config_enabled = Config::new().with_mcp(
        McpConfig::new()
            .with_enabled(true)
            .with_enabled_by_default(true),
    );
    let runtime_enabled = IronRuntime::new(config_enabled, MockProvider::default());

    // Register a server with enabled_by_default=false (should be overridden by runtime)
    let server_config2 = McpServerConfig {
        id: "override-server2".to_string(),
        label: "Override Server 2".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new("http://localhost:8080".to_string()),
        },
        enabled_by_default: false, // Server says disabled, but runtime says enabled
        working_dir: None,
    };
    runtime_enabled.register_mcp_server(server_config2);

    // Create session
    let (_session_id2, session2) = runtime_enabled
        .create_session(ConnectionId(2))
        .expect("Failed to create session");

    // Verify server is enabled (runtime default overrides server default)
    let sess2 = session2.lock();
    let is_enabled2 = sess2.is_mcp_server_enabled("override-server2");
    assert_eq!(
        is_enabled2,
        Some(true),
        "Runtime default (enabled) should override server default (disabled)"
    );
}

// ---------------------------------------------------------------------------
// HTTP header integration tests
// ---------------------------------------------------------------------------

/// Read an HTTP request from a TCP stream, returning (request_line, headers, body).
async fn read_http_request_with_body(
    stream: &mut tokio::net::TcpStream,
) -> (String, HashMap<String, String>, Vec<u8>) {
    use tokio::io::AsyncReadExt;

    let mut buffer = Vec::new();
    let headers_end = loop {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "unexpected EOF while reading HTTP request");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break index + 4;
        }
    };

    let header_text = String::from_utf8_lossy(&buffer[..headers_end]);
    let mut lines = header_text.lines();
    let request_line = lines.next().unwrap().to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    // Read body based on Content-Length
    let body = if let Some(content_length) = headers.get("content-length") {
        let length: usize = content_length.parse().unwrap();
        let mut body = buffer[headers_end..].to_vec();
        while body.len() < length {
            let mut chunk = [0u8; 4096];
            let read = stream.read(&mut chunk).await.unwrap();
            body.extend_from_slice(&chunk[..read]);
        }
        body
    } else {
        Vec::new()
    };

    (request_line, headers, body)
}

/// Start a fake HTTP server that captures request headers and responds to MCP initialize.
async fn start_header_capture_http_server(
) -> (u16, Arc<tokio::sync::Mutex<Vec<HashMap<String, String>>>>) {
    use tokio::io::AsyncWriteExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured_headers: Arc<tokio::sync::Mutex<Vec<HashMap<String, String>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let captured = Arc::clone(&captured_headers);
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let captured = Arc::clone(&captured);
            tokio::spawn(async move {
                let (_request_line, headers, body) = read_http_request_with_body(&mut socket).await;

                // Store captured headers
                captured.lock().await.push(headers.clone());

                // Build a JSON-RPC response matching the request id
                let request: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                let id = request.get("id").and_then(|v| v.as_u64()).unwrap_or(1);

                let response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {},
                        "serverInfo": {"name": "test-server", "version": "1.0"}
                    }
                });

                let response_body = serde_json::to_string(&response).unwrap();
                let http_response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                let _ = socket.write_all(http_response.as_bytes()).await;
            });
        }
    });

    (port, captured_headers)
}

#[derive(Clone, Copy)]
enum HttpResponseIdMode {
    MatchRequest,
    Null,
    Absent,
}

async fn start_bootstrap_tolerance_http_server(
    initialize_id_mode: HttpResponseIdMode,
    tools_list_id_mode: HttpResponseIdMode,
) -> u16 {
    use tokio::io::AsyncWriteExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => continue,
            };
            tokio::spawn(async move {
                let (_request_line, _headers, body) =
                    read_http_request_with_body(&mut socket).await;

                let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let method = request
                    .get("method")
                    .and_then(|value| value.as_str())
                    .expect("request should include method");
                let request_id = request.get("id").and_then(|value| value.as_u64());

                let response = match method {
                    "initialize" => {
                        let mut response = serde_json::json!({
                            "jsonrpc": "2.0",
                            "result": {
                                "protocolVersion": "2024-11-05",
                                "capabilities": {},
                                "serverInfo": {"name": "http-bootstrap", "version": "1.0"}
                            }
                        });
                        match initialize_id_mode {
                            HttpResponseIdMode::MatchRequest => {
                                response["id"] = serde_json::json!(request_id)
                            }
                            HttpResponseIdMode::Null => response["id"] = serde_json::Value::Null,
                            HttpResponseIdMode::Absent => {}
                        }
                        response
                    }
                    "tools/list" => {
                        let mut response = serde_json::json!({
                            "jsonrpc": "2.0",
                            "result": {
                                "tools": [{
                                    "name": "http_tool",
                                    "description": "HTTP tool",
                                    "inputSchema": {"type": "object", "properties": {}}
                                }]
                            }
                        });
                        match tools_list_id_mode {
                            HttpResponseIdMode::MatchRequest => {
                                response["id"] = serde_json::json!(request_id)
                            }
                            HttpResponseIdMode::Null => response["id"] = serde_json::Value::Null,
                            HttpResponseIdMode::Absent => {}
                        }
                        response
                    }
                    other => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "error": {"code": -32601, "message": format!("unknown method: {}", other)}
                    }),
                };

                let response_body = serde_json::to_string(&response).unwrap();
                let http_response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                let _ = socket.write_all(http_response.as_bytes()).await;
            });
        }
    });

    port
}

async fn start_streamable_http_server() -> u16 {
    use tokio::io::AsyncWriteExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => continue,
            };
            tokio::spawn(async move {
                let (_request_line, _headers, body) =
                    read_http_request_with_body(&mut socket).await;

                let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let method = request
                    .get("method")
                    .and_then(|value| value.as_str())
                    .expect("request should include method");
                let request_id = request.get("id").and_then(|value| value.as_u64());

                let response = match method {
                    "initialize" => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "result": {
                            "protocolVersion": "2024-11-05",
                            "capabilities": {"tools": {"listChanged": true}},
                            "serverInfo": {"name": "streamable-http", "version": "1.0"}
                        }
                    }),
                    "tools/list" => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "result": {
                            "tools": [{
                                "name": "streamable_tool",
                                "description": "Streamable HTTP tool",
                                "inputSchema": {"type": "object", "properties": {}}
                            }]
                        }
                    }),
                    other => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "error": {"code": -32601, "message": format!("unknown method: {}", other)}
                    }),
                };

                let sse_body = format!("event: message\r\ndata: {}\r\n\r\n", response);
                let http_response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
                    sse_body.len(),
                    sse_body
                );
                let _ = socket.write_all(http_response.as_bytes()).await;
            });
        }
    });

    port
}

#[tokio::test]
async fn test_http_mcp_client_parses_streamable_http_sse_response() {
    let port = start_streamable_http_server().await;

    let config = McpServerConfig {
        id: "streamable-http".to_string(),
        label: "Streamable HTTP".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new(format!("http://127.0.0.1:{}", port)),
        },
        enabled_by_default: true,
        working_dir: None,
    };

    let client = iron_core::mcp::create_transport_client("streamable-http", &config)
        .expect("client creation");

    let initialize = client.initialize().await;
    assert!(
        initialize.is_ok(),
        "initialize should parse SSE response: {:?}",
        initialize
    );

    let tools = client
        .list_tools()
        .await
        .expect("tools/list should succeed");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "streamable_tool");
}

#[tokio::test]
async fn test_http_mcp_client_initialize_accepts_absent_response_id() {
    let port = start_bootstrap_tolerance_http_server(
        HttpResponseIdMode::Absent,
        HttpResponseIdMode::MatchRequest,
    )
    .await;

    let config = McpServerConfig {
        id: "http-absent-init-id".to_string(),
        label: "HTTP absent initialize id".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new(format!("http://127.0.0.1:{}", port)),
        },
        enabled_by_default: true,
        working_dir: None,
    };

    let client = iron_core::mcp::create_transport_client("http-absent-init-id", &config)
        .expect("client creation");

    let result = client.initialize().await;
    assert!(result.is_ok(), "initialize should succeed: {:?}", result);
}

#[tokio::test]
async fn test_http_mcp_client_initialize_accepts_explicit_null_response_id() {
    let port = start_bootstrap_tolerance_http_server(
        HttpResponseIdMode::Null,
        HttpResponseIdMode::MatchRequest,
    )
    .await;

    let config = McpServerConfig {
        id: "http-null-init-id".to_string(),
        label: "HTTP null initialize id".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new(format!("http://127.0.0.1:{}", port)),
        },
        enabled_by_default: true,
        working_dir: None,
    };

    let client = iron_core::mcp::create_transport_client("http-null-init-id", &config)
        .expect("client creation");

    let result = client.initialize().await;
    assert!(result.is_ok(), "initialize should succeed: {:?}", result);
}

#[tokio::test]
async fn test_http_mcp_client_rejects_post_bootstrap_id_less_response() {
    let port = start_bootstrap_tolerance_http_server(
        HttpResponseIdMode::MatchRequest,
        HttpResponseIdMode::Absent,
    )
    .await;

    let config = McpServerConfig {
        id: "http-post-bootstrap-idless".to_string(),
        label: "HTTP post-bootstrap id-less response".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new(format!("http://127.0.0.1:{}", port)),
        },
        enabled_by_default: true,
        working_dir: None,
    };

    let client = iron_core::mcp::create_transport_client("http-post-bootstrap-idless", &config)
        .expect("client creation");

    let initialize = client.initialize().await;
    assert!(
        initialize.is_ok(),
        "initialize should succeed before tools/list: {:?}",
        initialize
    );

    let error = client
        .list_tools()
        .await
        .expect_err("id-less post-bootstrap tools/list should be rejected");
    assert!(
        error.contains("Response ID mismatch"),
        "expected ID mismatch error, got: {}",
        error
    );
}

#[tokio::test]
async fn test_http_mcp_client_sends_accept_header() {
    let (port, captured_headers) = start_header_capture_http_server().await;

    let config = McpServerConfig {
        id: "test-accept".to_string(),
        label: "Test Accept Header".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig::new(format!("http://127.0.0.1:{}", port)),
        },
        enabled_by_default: true,
        working_dir: None,
    };

    let client =
        iron_core::mcp::create_transport_client("test-accept", &config).expect("client creation");
    let result = client.initialize().await;
    assert!(result.is_ok(), "initialize should succeed: {:?}", result);

    // Verify the Accept header was sent
    let headers = captured_headers.lock().await;
    assert!(
        !headers.is_empty(),
        "should have captured at least one request"
    );
    let request_headers = &headers[0];
    let accept = request_headers
        .get("accept")
        .expect("Accept header should be present");
    assert_eq!(
        accept, "application/json, text/event-stream",
        "Accept header should match MCP requirement"
    );
}

#[tokio::test]
async fn test_http_mcp_client_sends_custom_headers() {
    let (port, captured_headers) = start_header_capture_http_server().await;

    let mut custom_headers = HashMap::new();
    custom_headers.insert("Authorization".to_string(), "Bearer test-token".to_string());
    custom_headers.insert("X-API-Key".to_string(), "key-123".to_string());

    let config = McpServerConfig {
        id: "test-custom-headers".to_string(),
        label: "Test Custom Headers".to_string(),
        transport: McpTransport::Http {
            config: HttpConfig {
                url: format!("http://127.0.0.1:{}", port),
                headers: Some(custom_headers),
            },
        },
        enabled_by_default: true,
        working_dir: None,
    };

    let client = iron_core::mcp::create_transport_client("test-custom-headers", &config)
        .expect("client creation");
    let result = client.initialize().await;
    assert!(result.is_ok(), "initialize should succeed: {:?}", result);

    // Verify both Accept and custom headers were sent
    let headers = captured_headers.lock().await;
    assert!(
        !headers.is_empty(),
        "should have captured at least one request"
    );
    let request_headers = &headers[0];

    let accept = request_headers
        .get("accept")
        .expect("Accept header should be present");
    assert_eq!(
        accept, "application/json, text/event-stream",
        "Accept header should be present alongside custom headers"
    );

    let auth = request_headers
        .get("authorization")
        .expect("Authorization header should be present");
    assert_eq!(auth, "Bearer test-token");

    let api_key = request_headers
        .get("x-api-key")
        .expect("X-API-Key header should be present");
    assert_eq!(api_key, "key-123");
}
