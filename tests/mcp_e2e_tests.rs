use futures::StreamExt;
use iron_core::{
    config::{ApprovalStrategy, McpConfig},
    AgentEvent, Config, FacadeToolStatus, IronAgent, McpServerConfig, McpServerHealth,
    McpTransport, PermissionVerdict, PromptOutcome,
};
use iron_providers::{InferenceRequest, Provider, ProviderEvent, ToolCall};
use serde_json::json;
use std::collections::VecDeque;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

#[derive(Clone, Default)]
struct RecordingProvider {
    responses: Arc<Mutex<VecDeque<Vec<ProviderEvent>>>>,
    requests: Arc<Mutex<Vec<InferenceRequest>>>,
}

impl RecordingProvider {
    fn with_stream_responses(responses: Vec<Vec<ProviderEvent>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into())),
            ..Self::default()
        }
    }

    fn requests(&self) -> Vec<InferenceRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Provider for RecordingProvider {
    fn infer(
        &self,
        request: InferenceRequest,
    ) -> iron_providers::ProviderFuture<'_, Vec<ProviderEvent>> {
        self.requests.lock().unwrap().push(request);
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![ProviderEvent::Complete]);
        Box::pin(async move { Ok(response) })
    }

    fn infer_stream(
        &self,
        request: InferenceRequest,
    ) -> iron_providers::ProviderFuture<
        '_,
        futures::stream::BoxStream<'static, iron_providers::ProviderResult<ProviderEvent>>,
    > {
        self.requests.lock().unwrap().push(request);
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![ProviderEvent::Complete]);
        Box::pin(async move { Ok(futures::stream::iter(response.into_iter().map(Ok)).boxed()) })
    }
}

fn write_fake_stdio_mcp_server(tempdir: &TempDir) -> std::path::PathBuf {
    let script_path = tempdir.path().join("fake-mcp-server.sh");
    let script = r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"fake-mcp","version":"1.0.0"}}}'
      ;;
    *'"method":"tools/list"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"test_tool","description":"Test MCP tool","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}}]}}'
      ;;
    *'"method":"tools/call"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"mcp-tool-result"}],"isError":false}}'
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

    script_path
}

async fn wait_for_server_ready(agent: &IronAgent, server_id: &str) {
    for _ in 0..100 {
        let server = agent.runtime().mcp_registry().get_server(server_id);
        if let Some(server) = server {
            if server.health == McpServerHealth::Connected && !server.discovered_tools.is_empty() {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let server = agent.runtime().mcp_registry().get_server(server_id);
    panic!(
        "server never became ready: {:?}",
        server.map(|s| (s.health, s.discovered_tools.len(), s.last_error))
    );
}

#[tokio::test]
async fn register_server_connects_and_discovers_tools_end_to_end() {
    let tempdir = TempDir::new().unwrap();
    let script_path = write_fake_stdio_mcp_server(&tempdir);

    let agent = IronAgent::new(
        Config::new().with_mcp(
            McpConfig::new()
                .with_enabled(true)
                .with_enabled_by_default(true),
        ),
        RecordingProvider::default(),
    );

    agent.register_mcp_server(McpServerConfig {
        id: "stdio-server".to_string(),
        label: "Fake stdio server".to_string(),
        transport: McpTransport::Stdio {
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            env: Default::default(),
        },
        enabled_by_default: true,
        working_dir: None,
    });

    wait_for_server_ready(&agent, "stdio-server").await;

    let server = agent
        .runtime()
        .mcp_registry()
        .get_server("stdio-server")
        .unwrap();
    assert_eq!(server.health, McpServerHealth::Connected);
    assert_eq!(server.discovered_tools.len(), 1);
    assert_eq!(server.discovered_tools[0].name, "test_tool");
}

#[tokio::test]
async fn prompt_request_includes_visible_mcp_tools() {
    let tempdir = TempDir::new().unwrap();
    let script_path = write_fake_stdio_mcp_server(&tempdir);
    let provider = RecordingProvider::with_stream_responses(vec![vec![ProviderEvent::Complete]]);

    let agent = IronAgent::new(
        Config::new().with_mcp(
            McpConfig::new()
                .with_enabled(true)
                .with_enabled_by_default(true),
        ),
        provider.clone(),
    );

    agent.register_mcp_server(McpServerConfig {
        id: "stdio-server".to_string(),
        label: "Fake stdio server".to_string(),
        transport: McpTransport::Stdio {
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            env: Default::default(),
        },
        enabled_by_default: true,
        working_dir: None,
    });

    wait_for_server_ready(&agent, "stdio-server").await;

    let conn = agent.connect();
    let session = conn.create_session().unwrap();
    let outcome = session.prompt("hello").await;
    assert_eq!(outcome, PromptOutcome::EndTurn);

    let requests = provider.requests();
    assert!(
        !requests.is_empty(),
        "provider should have received a request"
    );
    assert!(
        requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == "mcp_stdio-server_test_tool"),
        "provider request should include the discovered MCP tool"
    );
}

#[tokio::test]
async fn model_issued_mcp_tool_call_executes_through_runtime() {
    let tempdir = TempDir::new().unwrap();
    let script_path = write_fake_stdio_mcp_server(&tempdir);
    let provider = RecordingProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new(
                    "mcp1",
                    "mcp_stdio-server_test_tool",
                    json!({"text": "hello"}),
                ),
            },
            ProviderEvent::Complete,
        ],
        vec![
            ProviderEvent::Output {
                content: "done".to_string(),
            },
            ProviderEvent::Complete,
        ],
    ]);

    let agent = IronAgent::new(
        Config::new()
            .with_mcp(
                McpConfig::new()
                    .with_enabled(true)
                    .with_enabled_by_default(true),
            )
            .with_approval_strategy(ApprovalStrategy::Never),
        provider,
    );

    agent.register_mcp_server(McpServerConfig {
        id: "stdio-server".to_string(),
        label: "Fake stdio server".to_string(),
        transport: McpTransport::Stdio {
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            env: Default::default(),
        },
        enabled_by_default: true,
        working_dir: None,
    });

    wait_for_server_ready(&agent, "stdio-server").await;

    let conn = agent.connect();
    let session = conn.create_session().unwrap();
    let outcome = session.prompt("go").await;
    assert_eq!(outcome, PromptOutcome::EndTurn);

    let mut saw_completed_result = false;
    for event in session.drain_events() {
        if let AgentEvent::ToolCallUpdate {
            call_id,
            status,
            output,
            ..
        } = event
        {
            if call_id == "mcp1" && status == FacadeToolStatus::Completed {
                assert_eq!(output, Some(json!({"result": "mcp-tool-result"})));
                saw_completed_result = true;
            }
        }
    }

    assert!(saw_completed_result, "expected completed MCP tool result");
}

#[tokio::test]
async fn model_issued_mcp_tool_call_respects_real_approval_flow() {
    let tempdir = TempDir::new().unwrap();
    let script_path = write_fake_stdio_mcp_server(&tempdir);
    let provider = RecordingProvider::with_stream_responses(vec![vec![
        ProviderEvent::ToolCall {
            call: ToolCall::new(
                "mcp1",
                "mcp_stdio-server_test_tool",
                json!({"text": "hello"}),
            ),
        },
        ProviderEvent::Complete,
    ]]);

    let agent = IronAgent::new(
        Config::new()
            .with_mcp(
                McpConfig::new()
                    .with_enabled(true)
                    .with_enabled_by_default(true),
            )
            .with_approval_strategy(ApprovalStrategy::Always),
        provider,
    );

    agent.register_mcp_server(McpServerConfig {
        id: "stdio-server".to_string(),
        label: "Fake stdio server".to_string(),
        transport: McpTransport::Stdio {
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            env: Default::default(),
        },
        enabled_by_default: true,
        working_dir: None,
    });

    wait_for_server_ready(&agent, "stdio-server").await;

    let conn = agent.connect();
    conn.on_permission(|_| PermissionVerdict::Deny);
    let session = conn.create_session().unwrap();
    let outcome = session.prompt("go").await;
    assert_eq!(outcome, PromptOutcome::EndTurn);

    let denied = session.drain_events().into_iter().any(|event| {
        matches!(
            event,
            AgentEvent::ToolCallUpdate {
                call_id,
                status: FacadeToolStatus::Failed,
                output: Some(output),
                ..
            } if call_id == "mcp1" && output == json!({"error": "denied by user"})
        )
    });

    assert!(
        denied,
        "expected MCP tool call to be denied through the real approval path"
    );
}

#[tokio::test]
async fn model_issued_unavailable_mcp_tool_uses_precise_canonical_diagnostics() {
    let tempdir = TempDir::new().unwrap();
    let script_path = write_fake_stdio_mcp_server(&tempdir);
    let provider = RecordingProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new(
                    "mcp-missing",
                    "mcp_stdio-server_missing_tool",
                    json!({"text": "hello"}),
                ),
            },
            ProviderEvent::Complete,
        ],
        vec![ProviderEvent::Complete],
    ]);

    let agent = IronAgent::new(
        Config::new()
            .with_mcp(
                McpConfig::new()
                    .with_enabled(true)
                    .with_enabled_by_default(true),
            )
            .with_approval_strategy(ApprovalStrategy::Never),
        provider,
    );

    agent.register_mcp_server(McpServerConfig {
        id: "stdio-server".to_string(),
        label: "Fake stdio server".to_string(),
        transport: McpTransport::Stdio {
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            env: Default::default(),
        },
        enabled_by_default: true,
        working_dir: None,
    });

    wait_for_server_ready(&agent, "stdio-server").await;

    let conn = agent.connect();
    let session = conn.create_session().unwrap();
    let outcome = session.prompt("go").await;
    assert_eq!(outcome, PromptOutcome::EndTurn);

    let precise_error = session
        .drain_events()
        .into_iter()
        .find_map(|event| match event {
            AgentEvent::ToolCallUpdate {
                call_id,
                status: FacadeToolStatus::Failed,
                output: Some(output),
                ..
            } if call_id == "mcp-missing" => output["error"].as_str().map(str::to_owned),
            _ => None,
        });

    let precise_error = precise_error.expect("expected a failed MCP tool call event");
    assert!(
        precise_error.contains("Tool 'missing_tool' not found on MCP server 'stdio-server'. Available tools: test_tool"),
        "expected precise canonical MCP diagnostic, got: {precise_error}"
    );
    assert!(
        !precise_error.contains("not found in session catalog"),
        "prompt path should not fall back to generic session-catalog miss diagnostics: {precise_error}"
    );
}

#[tokio::test]
async fn reconnect_and_rediscover_restore_tools() {
    let tempdir = TempDir::new().unwrap();
    let script_path = write_fake_stdio_mcp_server(&tempdir);

    let agent = IronAgent::new(
        Config::new().with_mcp(
            McpConfig::new()
                .with_enabled(true)
                .with_enabled_by_default(true),
        ),
        RecordingProvider::default(),
    );

    agent.register_mcp_server(McpServerConfig {
        id: "stdio-server".to_string(),
        label: "Fake stdio server".to_string(),
        transport: McpTransport::Stdio {
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            env: Default::default(),
        },
        enabled_by_default: true,
        working_dir: None,
    });

    wait_for_server_ready(&agent, "stdio-server").await;

    let conn = agent.connect();
    let session = conn.create_session().unwrap();
    let session_id = session.id();

    let initial_catalog = agent
        .runtime()
        .get_session_tool_catalog(session_id)
        .unwrap();
    assert!(initial_catalog.contains("mcp_stdio-server_test_tool"));

    agent
        .runtime()
        .mcp_connection_manager()
        .disconnect_server("stdio-server")
        .await;

    for _ in 0..50 {
        let server = agent
            .runtime()
            .mcp_registry()
            .get_server("stdio-server")
            .unwrap();
        if server.health == McpServerHealth::Configured {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let disconnected_catalog = agent
        .runtime()
        .get_session_tool_catalog(session_id)
        .unwrap();
    assert!(!disconnected_catalog.contains("mcp_stdio-server_test_tool"));

    agent
        .runtime()
        .mcp_connection_manager()
        .reconnect_server("stdio-server")
        .await;

    wait_for_server_ready(&agent, "stdio-server").await;

    let reconnected_catalog = agent
        .runtime()
        .get_session_tool_catalog(session_id)
        .unwrap();
    assert!(reconnected_catalog.contains("mcp_stdio-server_test_tool"));
}

#[cfg(feature = "embedded-python")]
#[tokio::test]
async fn python_exec_child_call_can_reach_visible_mcp_tool() {
    let tempdir = TempDir::new().unwrap();
    let script_path = write_fake_stdio_mcp_server(&tempdir);
    let provider = RecordingProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new(
                    "py1",
                    "python_exec",
                    json!({
                        "script": "result = await tools.call('mcp_stdio-server_test_tool', {'text': 'hello'})\nresult['result']",
                        "input": {}
                    }),
                ),
            },
            ProviderEvent::Complete,
        ],
        vec![
            ProviderEvent::Output {
                content: "done".to_string(),
            },
            ProviderEvent::Complete,
        ],
    ]);

    let agent = IronAgent::new(
        Config::new()
            .with_mcp(
                McpConfig::new()
                    .with_enabled(true)
                    .with_enabled_by_default(true),
            )
            .with_embedded_python(iron_core::EmbeddedPythonConfig::new().with_enabled(true))
            .with_approval_strategy(ApprovalStrategy::Never),
        provider,
    );
    agent.register_python_exec_tool();

    agent.register_mcp_server(McpServerConfig {
        id: "stdio-server".to_string(),
        label: "Fake stdio server".to_string(),
        transport: McpTransport::Stdio {
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            env: Default::default(),
        },
        enabled_by_default: true,
        working_dir: None,
    });

    wait_for_server_ready(&agent, "stdio-server").await;

    let conn = agent.connect();
    let session = conn.create_session().unwrap();
    let outcome = session.prompt("go").await;
    assert_eq!(outcome, PromptOutcome::EndTurn);

    let mut saw_completed_result = false;
    for event in session.drain_events() {
        if let AgentEvent::ToolCallUpdate {
            call_id,
            status,
            output,
            ..
        } = event
        {
            if call_id == "py1" && status == FacadeToolStatus::Completed {
                let result = output.expect("python_exec should produce a result");
                assert_eq!(result["result"], json!("mcp-tool-result"));
                saw_completed_result = true;
            }
        }
    }

    assert!(
        saw_completed_result,
        "expected python_exec to complete using the MCP child tool"
    );
}

#[cfg(feature = "embedded-python")]
#[tokio::test]
async fn python_exec_child_unavailable_mcp_tool_uses_precise_canonical_diagnostics() {
    let tempdir = TempDir::new().unwrap();
    let script_path = write_fake_stdio_mcp_server(&tempdir);
    let provider = RecordingProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new(
                    "py-missing",
                    "python_exec",
                    json!({
                        "script": "await tools.call('mcp_stdio-server_missing_tool', {'text': 'hello'})",
                        "input": {}
                    }),
                ),
            },
            ProviderEvent::Complete,
        ],
        vec![ProviderEvent::Complete],
    ]);

    let agent = IronAgent::new(
        Config::new()
            .with_mcp(
                McpConfig::new()
                    .with_enabled(true)
                    .with_enabled_by_default(true),
            )
            .with_embedded_python(iron_core::EmbeddedPythonConfig::new().with_enabled(true))
            .with_approval_strategy(ApprovalStrategy::Never),
        provider,
    );
    agent.register_python_exec_tool();

    agent.register_mcp_server(McpServerConfig {
        id: "stdio-server".to_string(),
        label: "Fake stdio server".to_string(),
        transport: McpTransport::Stdio {
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            env: Default::default(),
        },
        enabled_by_default: true,
        working_dir: None,
    });

    wait_for_server_ready(&agent, "stdio-server").await;

    let conn = agent.connect();
    let session = conn.create_session().unwrap();
    let outcome = session.prompt("go").await;
    assert_eq!(outcome, PromptOutcome::EndTurn);

    let precise_error = session
        .drain_events()
        .into_iter()
        .find_map(|event| match event {
            AgentEvent::ToolCallUpdate {
                call_id,
                output: Some(output),
                ..
            } if call_id == "py-missing" => output["child_outcomes"]
                .as_array()
                .and_then(|child_outcomes| child_outcomes.first())
                .and_then(|child| child["result"]["error"].as_str())
                .map(str::to_owned),
            _ => None,
        });

    let precise_error = precise_error.expect("expected failed child MCP diagnostic");
    assert!(
        precise_error.contains("Tool 'missing_tool' not found on MCP server 'stdio-server'. Available tools: test_tool"),
        "expected precise canonical child MCP diagnostic, got: {precise_error}"
    );
    assert!(
        !precise_error.contains(
            "tool 'mcp_stdio-server_missing_tool' is not present in the script tool catalog"
        ),
        "child path should not use the generic script-catalog diagnostic: {precise_error}"
    );
}
