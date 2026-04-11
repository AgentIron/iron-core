use futures::StreamExt;
use iron_core::{
    config::{ApprovalStrategy, McpConfig},
    mcp::create_transport_client,
    mcp::ReconnectConfig,
    AgentEvent, Config, FacadeToolStatus, IronAgent, McpConnectionManager, McpServerConfig,
    McpServerHealth, McpServerRegistry, McpTransport, PromptOutcome,
};
use iron_providers::{InferenceRequest, Provider, ProviderEvent, ToolCall};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};

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

fn write_fake_stdio_mcp_server(tempdir: &TempDir, tool_result: &str) -> std::path::PathBuf {
    let script_path = tempdir.path().join("fake-mcp-server.sh");
    let script = format!(
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocol_version":"2024-11-05","capabilities":{{}},"server_info":{{"name":"fake-mcp","version":"1.0.0"}}}}}}'
      ;;
    *'"method":"tools/list"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"tools":[{{"name":"test_tool","description":"Test MCP tool","input_schema":{{"type":"object","properties":{{"text":{{"type":"string"}}}},"required":["text"]}}}}]}}}}'
      ;;
    *'"method":"tools/call"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":3,"result":{{"content":[{{"type":"text","text":"{}"}}],"is_error":false}}}}'
      ;;
  esac
done
"#,
        tool_result
    );

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

fn write_review_stdio_mcp_server(tempdir: &TempDir) -> std::path::PathBuf {
    let script_path = tempdir.path().join("review-mcp-server.py");
    let script = r#"#!/usr/bin/env python3
import json
import os
import sys
import threading

# Force line buffering for stdin/stdout
sys.stdin = open(sys.stdin.fileno(), 'r', buffering=1)
sys.stdout = open(sys.stdout.fileno(), 'w', buffering=1)

stderr_flooded = False
stdout_lock = threading.Lock()


def send(payload):
    with stdout_lock:
        sys.stdout.write(json.dumps(payload) + "\n")
        sys.stdout.flush()


def send_notification(label):
    send({"jsonrpc": "2.0", "method": "notifications/test", "params": {"label": label}})


# Signal that server is ready
sys.stderr.write("Server starting\n")
sys.stderr.flush()

while True:
    try:
        raw_line = sys.stdin.readline()
        if not raw_line:
            break
        line = raw_line.strip()
        if not line:
            continue

        request = json.loads(line)
        request_id = request.get("id")
        method = request["method"]

        if method == "initialize":
            if not stderr_flooded:
                sys.stderr.write("stderr: initialization started\n")
                sys.stderr.flush()
                stderr_flooded = True
            send_notification("initialize")
            send({
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "protocol_version": "2024-11-05",
                    "capabilities": {},
                    "server_info": {"name": "review-stdio", "version": "1.0.0"}
                }
            })
        elif method == "tools/list":
            cursor = request.get("params", {}).get("cursor")
            send_notification(f"tools-list:{cursor}")
            if cursor is None:
                send({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "tools": [
                            {"name": "pwd_tool", "description": "Return cwd", "input_schema": {"type": "object", "properties": {}}},
                            {"name": "env_tool", "description": "Return env var", "input_schema": {"type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]}}
                        ],
                        "next_cursor": "page-2"
                    }
                })
            else:
                send({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "tools": [
                            {"name": "error_tool", "description": "Return call error", "input_schema": {"type": "object", "properties": {}}},
                            {"name": "delayed_tool", "description": "Return delayed result", "input_schema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}}
                        ]
                    }
                })
        elif method == "tools/call":
            params = request.get("params", {})
            name = params.get("name")
            arguments = params.get("arguments", {})
            send_notification(f"call:{name}")
            if name == "pwd_tool":
                send({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {"content": [{"type": "text", "text": os.getcwd()}], "is_error": False}
                })
            elif name == "env_tool":
                key = arguments.get("key", "")
                value = os.environ.get(key, "<missing>")
                send({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {"content": [{"type": "text", "text": value}], "is_error": False}
                })
            elif name == "error_tool":
                send({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {"content": [{"type": "text", "text": "server exploded: invalid token near secret-like-value"}], "is_error": True}
                })
            elif name == "delayed_tool":
                text = arguments.get("text", "unknown")
                delay = 0.15 if text == "alpha" else 0.01

                # Capture variables by value using default arguments
                def respond(req_id=request_id, captured_text=text):
                    send_notification(f"delayed:{captured_text}")
                    send({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {"content": [{"type": "text", "text": f"result-{captured_text}"}], "is_error": False}
                    })

                threading.Timer(delay, respond).start()
        else:
            send({
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {"code": -32601, "message": f"unknown method: {method}"}
            })
    except Exception as e:
        sys.stderr.write(f"Error: {e}\n")
        sys.stderr.flush()
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

struct FakeSseServer {
    url: String,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for FakeSseServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn read_http_request(stream: &mut TcpStream) -> (String, HashMap<String, String>, Vec<u8>) {
    let mut buffer = Vec::new();
    let headers_end = loop {
        let mut chunk = [0u8; 1024];
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
            headers.insert(key.to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);

    let mut body = buffer[headers_end..].to_vec();
    while body.len() < content_length {
        let mut chunk = vec![0u8; content_length - body.len()];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "unexpected EOF while reading HTTP body");
        body.extend_from_slice(&chunk[..read]);
    }

    (request_line, headers, body)
}

async fn start_fake_sse_server() -> FakeSseServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let current_sender: Arc<tokio::sync::Mutex<Option<mpsc::UnboundedSender<String>>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    let handle = tokio::spawn({
        let current_sender = Arc::clone(&current_sender);
        async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let current_sender = Arc::clone(&current_sender);
                tokio::spawn(async move {
                    let (request_line, _headers, body) = read_http_request(&mut socket).await;

                    if request_line.starts_with("GET ") {
                        let response = concat!(
                            "HTTP/1.1 200 OK\r\n",
                            "Content-Type: text/event-stream\r\n",
                            "Cache-Control: no-cache\r\n",
                            "Connection: keep-alive\r\n\r\n"
                        );
                        socket.write_all(response.as_bytes()).await.unwrap();

                        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
                        *current_sender.lock().await = Some(tx);

                        while let Some(message) = rx.recv().await {
                            if socket.write_all(message.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    } else if request_line.starts_with("POST ") {
                        let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                        let method = request["method"].as_str().unwrap();
                        let id = request["id"].as_u64().unwrap();
                        let cursor = request["params"]["cursor"].as_str();

                        let response_payload = match method {
                            "initialize" => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "protocol_version": "2024-11-05",
                                    "capabilities": {},
                                    "server_info": {"name": "fake-sse-mcp", "version": "1.0.0"}
                                }
                            }),
                            "tools/list" if cursor.is_none() => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "tools": [{
                                        "name": "test_tool",
                                        "description": "Test SSE MCP tool",
                                        "input_schema": {
                                            "type": "object",
                                            "properties": {"text": {"type": "string"}},
                                            "required": ["text"]
                                        }
                                    }],
                                    "next_cursor": "page-2"
                                }
                            }),
                            "tools/list" => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "tools": [{
                                        "name": "second_tool",
                                        "description": "Second SSE MCP tool",
                                        "input_schema": {
                                            "type": "object",
                                            "properties": {},
                                            "required": []
                                        }
                                    }]
                                }
                            }),
                            "tools/call" => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "content": [{"type": "text", "text": "sse-mcp-tool-result"}],
                                    "is_error": false
                                }
                            }),
                            other => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": {"code": -32601, "message": format!("unknown method: {}", other)}
                            }),
                        };

                        socket
                            .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
                            .await
                            .unwrap();

                        let sender = loop {
                            if let Some(sender) = current_sender.lock().await.clone() {
                                break sender;
                            }
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        };

                        let unrelated_payload = json!({
                            "jsonrpc": "2.0",
                            "id": id + 999,
                            "result": {"ignored": true}
                        });

                        sender
                            .send("event: ping\ndata: keepalive\n\n".to_string())
                            .unwrap();
                        sender
                            .send(format!("event: message\ndata: {}\n\n", unrelated_payload))
                            .unwrap();
                        sender
                            .send(format!("event: message\ndata: {}\n\n", response_payload))
                            .unwrap();
                    }
                });
            }
        }
    });

    FakeSseServer {
        url: format!("http://{}", addr),
        handle,
    }
}

#[tokio::test]
async fn public_effective_tools_match_prompt_visible_tools() {
    let tempdir = TempDir::new().unwrap();
    let script_path = write_fake_stdio_mcp_server(&tempdir, "mcp-tool-result");
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
    let session_id = session.id();
    assert_eq!(session.prompt("hello").await, PromptOutcome::EndTurn);

    let mut public_tools: Vec<String> = agent
        .get_effective_tools(session_id)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    public_tools.sort();

    let mut provider_tools: Vec<String> = provider.requests()[0]
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    provider_tools.sort();

    assert_eq!(public_tools, provider_tools);
    assert!(public_tools.contains(&"mcp_stdio-server_test_tool".to_string()));
}

#[tokio::test]
async fn connection_manager_stops_on_shutdown_signal() {
    let manager = McpConnectionManager::new(McpServerRegistry::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let task = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .start(
                    ReconnectConfig {
                        health_check_interval_secs: 60,
                        ..ReconnectConfig::default()
                    },
                    shutdown_rx,
                )
                .await;
        }
    });

    tokio::time::sleep(Duration::from_millis(20)).await;
    shutdown_tx.send(true).unwrap();

    tokio::time::timeout(Duration::from_millis(200), task)
        .await
        .expect("health monitor should stop after shutdown signal")
        .unwrap();
}

#[tokio::test]
async fn http_sse_transport_handles_framing_and_response_correlation() {
    let fake_sse_server = start_fake_sse_server().await;
    let provider = RecordingProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new("sse1", "mcp_sse-server_test_tool", json!({"text": "hello"})),
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
        provider.clone(),
    );

    agent.register_mcp_server(McpServerConfig {
        id: "sse-server".to_string(),
        label: "Fake SSE server".to_string(),
        transport: McpTransport::HttpSse {
            url: fake_sse_server.url.clone(),
        },
        enabled_by_default: true,
        working_dir: None,
    });

    wait_for_server_ready(&agent, "sse-server").await;

    let conn = agent.connect();
    let session = conn.create_session().unwrap();
    assert_eq!(session.prompt("go").await, PromptOutcome::EndTurn);

    let requests = provider.requests();
    assert!(requests[0]
        .tools
        .iter()
        .any(|tool| tool.name == "mcp_sse-server_test_tool"));

    let mut saw_completed_result = false;
    for event in session.drain_events() {
        if let AgentEvent::ToolCallUpdate {
            call_id,
            status,
            output,
            ..
        } = event
        {
            if call_id == "sse1" && status == FacadeToolStatus::Completed {
                assert_eq!(output, Some(json!({"result": "sse-mcp-tool-result"})));
                saw_completed_result = true;
            }
        }
    }

    assert!(
        saw_completed_result,
        "expected completed SSE MCP tool result"
    );
}

#[tokio::test]
async fn sse_tool_discovery_follows_pagination() {
    let fake_sse_server = start_fake_sse_server().await;

    let config = McpServerConfig {
        id: "paged-sse".to_string(),
        label: "Paged SSE".to_string(),
        transport: McpTransport::HttpSse {
            url: fake_sse_server.url.clone(),
        },
        enabled_by_default: true,
        working_dir: None,
    };

    let client = create_transport_client("paged-sse", &config).unwrap();
    client.initialize().await.unwrap();

    let tools = client.list_tools().await.unwrap();
    let tool_names: Vec<_> = tools.into_iter().map(|tool| tool.name).collect();
    assert_eq!(tool_names, vec!["test_tool", "second_tool"]);

    client.close().await;
}

#[tokio::test]
async fn mcp_name_resolution_is_unambiguous_for_server_ids_with_underscores() {
    let tempdir = TempDir::new().unwrap();
    let script_path = write_fake_stdio_mcp_server(&tempdir, "underscore-result");

    let agent = IronAgent::new(
        Config::new().with_mcp(
            McpConfig::new()
                .with_enabled(true)
                .with_enabled_by_default(true),
        ),
        RecordingProvider::default(),
    );

    agent.register_mcp_server(McpServerConfig {
        id: "stdio_server_id".to_string(),
        label: "Underscore stdio server".to_string(),
        transport: McpTransport::Stdio {
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            env: Default::default(),
        },
        enabled_by_default: true,
        working_dir: None,
    });

    wait_for_server_ready(&agent, "stdio_server_id").await;

    let conn = agent.connect();
    let session = conn.create_session().unwrap();
    let session_id = session.id();

    let catalog = agent
        .runtime()
        .get_session_tool_catalog(session_id)
        .unwrap();
    assert!(catalog.contains("mcp_stdio_server_id_test_tool"));

    let session_arc = agent.runtime().get_session(session_id).unwrap();
    let execute_future = {
        let session_guard = session_arc.lock().unwrap();
        catalog.execute(
            "missing1",
            "mcp_stdio_server_id_missing_tool",
            json!({"text": "hello"}),
            &session_guard,
        )
    };
    let err = execute_future.await.unwrap_err().to_string();

    assert!(err.contains("stdio_server_id"));
    assert!(!err.contains("MCP server 'stdio'"));
}

#[tokio::test]
async fn failed_stdio_spawn_surfaces_as_server_error_without_panic() {
    let agent = IronAgent::new(
        Config::new().with_mcp(
            McpConfig::new()
                .with_enabled(true)
                .with_enabled_by_default(true),
        ),
        RecordingProvider::default(),
    );

    // Register a server with a command that does not exist on the system.
    agent.register_mcp_server(McpServerConfig {
        id: "nonexistent-cmd".to_string(),
        label: "Bad command".to_string(),
        transport: McpTransport::Stdio {
            command: "/no/such/binary/exists_abc123".to_string(),
            args: vec![],
            env: Default::default(),
        },
        enabled_by_default: true,
        working_dir: None,
    });

    // Give the runtime a moment to attempt the connection.
    for _ in 0..50 {
        let server = agent.runtime().mcp_registry().get_server("nonexistent-cmd");
        if let Some(s) = server {
            if s.health == McpServerHealth::Error {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // The server should be in Error health state — not panic the runtime.
    let server = agent
        .runtime()
        .mcp_registry()
        .get_server("nonexistent-cmd")
        .expect("server should be registered");
    assert_eq!(
        server.health,
        McpServerHealth::Error,
        "expected Error health, got {:?}",
        server.health
    );
    assert!(
        server.last_error.is_some(),
        "expected a last_error message for the failed spawn"
    );
    assert!(
        server.discovered_tools.is_empty(),
        "no tools should be discovered for a failed server"
    );
}

#[tokio::test]
async fn stdio_transport_applies_working_dir_and_uses_safe_environment() {
    let tempdir = TempDir::new().unwrap();
    let script_path = write_review_stdio_mcp_server(&tempdir);
    let working_dir = tempdir.path().join("mcp-working-dir");
    fs::create_dir(&working_dir).unwrap();

    let inherited_key = std::env::vars()
        .map(|(key, _)| key)
        .find(|key| key.starts_with("CARGO_"))
        .expect("expected at least one CARGO_* env var during tests");

    let mut env = HashMap::new();
    env.insert("EXPLICIT_ALLOWED".to_string(), "present".to_string());

    let config = McpServerConfig {
        id: "review-stdio".to_string(),
        label: "Review stdio".to_string(),
        transport: McpTransport::Stdio {
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            env,
        },
        enabled_by_default: true,
        working_dir: Some(working_dir.clone()),
    };

    let client = create_transport_client("review-stdio", &config).unwrap();
    client.initialize().await.unwrap();

    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 4, "all paginated tools should be returned");
    assert!(tools.iter().any(|tool| tool.name == "pwd_tool"));
    assert!(tools.iter().any(|tool| tool.name == "delayed_tool"));

    let pwd = client.call_tool("pwd_tool", json!({})).await.unwrap();
    assert_eq!(
        pwd["result"],
        json!(working_dir.to_string_lossy().to_string())
    );

    let hidden_env = client
        .call_tool("env_tool", json!({"key": inherited_key}))
        .await
        .unwrap();
    assert_eq!(hidden_env["result"], json!("<missing>"));

    let explicit_env = client
        .call_tool("env_tool", json!({"key": "EXPLICIT_ALLOWED"}))
        .await
        .unwrap();
    assert_eq!(explicit_env["result"], json!("present"));

    client.close().await;
}

#[tokio::test]
async fn stdio_call_tool_preserves_server_error_details_and_correlates_responses() {
    let tempdir = TempDir::new().unwrap();
    let script_path = write_review_stdio_mcp_server(&tempdir);

    let config = McpServerConfig {
        id: "review-stdio".to_string(),
        label: "Review stdio".to_string(),
        transport: McpTransport::Stdio {
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            env: Default::default(),
        },
        enabled_by_default: true,
        working_dir: None,
    };

    let client = create_transport_client("review-stdio", &config).unwrap();
    client.initialize().await.unwrap();
    client.list_tools().await.unwrap();

    let error = client.call_tool("error_tool", json!({})).await.unwrap_err();
    assert!(error.contains("server exploded"));
    assert!(error.contains("invalid token"));

    let alpha = client.call_tool("delayed_tool", json!({"text": "alpha"}));
    let beta = client.call_tool("delayed_tool", json!({"text": "beta"}));
    let (alpha, beta) = tokio::join!(alpha, beta);

    assert_eq!(alpha.unwrap()["result"], json!("result-alpha"));
    assert_eq!(beta.unwrap()["result"], json!("result-beta"));

    client.close().await;
}

#[tokio::test]
async fn sse_startup_failure_fails_fast() {
    let unused_port = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    };

    let config = McpServerConfig {
        id: "broken-sse".to_string(),
        label: "Broken SSE".to_string(),
        transport: McpTransport::HttpSse {
            url: format!("http://127.0.0.1:{}", unused_port),
        },
        enabled_by_default: true,
        working_dir: None,
    };

    let client = create_transport_client("broken-sse", &config).unwrap();
    let start = Instant::now();
    let error = client.initialize().await.unwrap_err();

    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(
        error.contains("Failed to establish SSE endpoint")
            || error.contains("Timed out establishing SSE endpoint")
            || error.contains("SSE endpoint returned unsuccessful status"),
        "unexpected SSE startup error: {error}"
    );

    client.close().await;
}

/// Fake SSE server that can handle multiple concurrent POST requests and
/// deliver each response via SSE with correct id correlation.
async fn start_concurrent_sse_server() -> FakeSseServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let current_sender: Arc<tokio::sync::Mutex<Option<mpsc::UnboundedSender<String>>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    let handle = tokio::spawn({
        let current_sender = Arc::clone(&current_sender);
        async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let current_sender = Arc::clone(&current_sender);
                tokio::spawn(async move {
                    let (request_line, _headers, body) = read_http_request(&mut socket).await;

                    if request_line.starts_with("GET ") {
                        let response = concat!(
                            "HTTP/1.1 200 OK\r\n",
                            "Content-Type: text/event-stream\r\n",
                            "Cache-Control: no-cache\r\n",
                            "Connection: keep-alive\r\n\r\n"
                        );
                        socket.write_all(response.as_bytes()).await.unwrap();

                        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
                        *current_sender.lock().await = Some(tx);

                        while let Some(message) = rx.recv().await {
                            if socket.write_all(message.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    } else if request_line.starts_with("POST ") {
                        let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                        let method = request["method"].as_str().unwrap();
                        let id = request["id"].as_u64().unwrap();

                        // Return 202 immediately
                        socket
                            .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
                            .await
                            .unwrap();

                        let sender = loop {
                            if let Some(sender) = current_sender.lock().await.clone() {
                                break sender;
                            }
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        };

                        let response_payload = match method {
                            "initialize" => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "protocol_version": "2024-11-05",
                                    "capabilities": {},
                                    "server_info": {"name": "concurrent-sse-mcp", "version": "1.0.0"}
                                }
                            }),
                            "tools/list" => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "tools": [{
                                        "name": "test_tool",
                                        "description": "Concurrent SSE MCP tool",
                                        "input_schema": {
                                            "type": "object",
                                            "properties": {"text": {"type": "string"}},
                                            "required": ["text"]
                                        }
                                    }]
                                }
                            }),
                            "tools/call" => {
                                // Include the id in the result so the test can verify
                                // each caller gets the right response.
                                let args_text = request["params"]["arguments"]["text"]
                                    .as_str()
                                    .unwrap_or("unknown");
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "content": [{"type": "text", "text": format!("result-for-{}", args_text)}],
                                        "is_error": false
                                    }
                                })
                            }
                            other => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": {"code": -32601, "message": format!("unknown method: {}", other)}
                            }),
                        };

                        // Send a keepalive ping, then an unrelated response,
                        // then the real response — to verify filtering and correlation.
                        sender
                            .send("event: ping\ndata: keepalive\n\n".to_string())
                            .unwrap();
                        sender.send(format!(
                            "event: message\ndata: {}\n\n",
                            json!({"jsonrpc": "2.0", "id": id + 999, "result": {"unrelated": true}})
                        )).unwrap();
                        sender
                            .send(format!("event: message\ndata: {}\n\n", response_payload))
                            .unwrap();
                    }
                });
            }
        }
    });

    FakeSseServer {
        url: format!("http://{}", addr),
        handle,
    }
}

#[tokio::test]
async fn concurrent_sse_requests_are_correctly_correlated() {
    // This test verifies that multiple concurrent tool calls on the same
    // shared SSE server connection each receive their own correctly-correlated
    // response — not each other's.
    let fake_sse_server = start_concurrent_sse_server().await;

    let provider = RecordingProvider::with_stream_responses(vec![
        vec![
            ProviderEvent::ToolCall {
                call: ToolCall::new(
                    "c1",
                    "mcp_concurrent-sse_test_tool",
                    json!({"text": "alpha"}),
                ),
            },
            ProviderEvent::ToolCall {
                call: ToolCall::new(
                    "c2",
                    "mcp_concurrent-sse_test_tool",
                    json!({"text": "beta"}),
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
        id: "concurrent-sse".to_string(),
        label: "Concurrent SSE server".to_string(),
        transport: McpTransport::HttpSse {
            url: fake_sse_server.url.clone(),
        },
        enabled_by_default: true,
        working_dir: None,
    });

    wait_for_server_ready(&agent, "concurrent-sse").await;

    let conn = agent.connect();
    let session = conn.create_session().unwrap();
    assert_eq!(session.prompt("go").await, PromptOutcome::EndTurn);

    let mut alpha_completed = false;
    let mut beta_completed = false;
    for event in session.drain_events() {
        if let AgentEvent::ToolCallUpdate {
            call_id,
            status,
            output,
            ..
        } = event
        {
            if status == FacadeToolStatus::Completed {
                match call_id.as_str() {
                    "c1" => {
                        let result = output.expect("c1 should have output");
                        assert_eq!(
                            result["result"],
                            json!("result-for-alpha"),
                            "c1 should receive the alpha-correlated response"
                        );
                        alpha_completed = true;
                    }
                    "c2" => {
                        let result = output.expect("c2 should have output");
                        assert_eq!(
                            result["result"],
                            json!("result-for-beta"),
                            "c2 should receive the beta-correlated response"
                        );
                        beta_completed = true;
                    }
                    _ => {}
                }
            }
        }
    }

    assert!(alpha_completed, "expected alpha tool call to complete");
    assert!(beta_completed, "expected beta tool call to complete");
}
