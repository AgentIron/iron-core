//! MCP transport client implementations for stdio, HTTP, and HTTP+SSE transports.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tracing::{error, warn};

use crate::mcp::protocol::messages as protocol_messages;
use crate::mcp::protocol::JsonRpcRequest as ProtocolJsonRpcRequest;
use crate::mcp::protocol::{tool_content_to_value, tool_error_to_string};
use crate::mcp::protocol::{JsonRpcError, JsonRpcResponse};
use crate::mcp::server::{HttpConfig, McpServerConfig, McpToolInfo, McpTransport};

/// Default Accept header required by remote MCP servers.
const MCP_ACCEPT_HEADER: &str = "application/json, text/event-stream";

/// Apply the default Accept header and any custom headers to a reqwest request builder.
/// Custom headers override the default Accept header if set.
fn apply_headers(
    builder: reqwest::RequestBuilder,
    custom_headers: &Option<HashMap<String, String>>,
) -> reqwest::RequestBuilder {
    let mut builder = builder.header("Accept", MCP_ACCEPT_HEADER);
    if let Some(headers) = custom_headers {
        for (key, value) in headers {
            builder = builder.header(key.as_str(), value.as_str());
        }
    }
    builder
}

/// Type alias for the pending-response waiter map shared across stdio client tasks.
type WaiterMap =
    Arc<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Result<JsonRpcResponse, String>>>>>;

/// Trait for MCP transport clients.
#[async_trait]
pub trait McpTransportClient: Send + Sync {
    /// Initialize the connection to the MCP server.
    async fn initialize(&self) -> Result<protocol_messages::InitializeResponse, String>;

    /// List available tools from the MCP server.
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String>;

    /// Call a tool on the MCP server.
    async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<Value, String>;

    /// Check if the client is connected.
    fn is_connected(&self) -> bool;

    /// Close the connection.
    async fn close(&self);
}

/// MCP transport client using stdio.
pub struct StdioMcpClient {
    server_id: String,
    _config: McpServerConfig,
    stdin: Arc<Mutex<ChildStdin>>,
    process: Arc<Mutex<Child>>,
    connected: AtomicBool,
    request_counter: Arc<Mutex<u64>>,
    waiters: WaiterMap,
    stdout_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    stderr_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    stderr_excerpt: Arc<Mutex<VecDeque<String>>>,
    /// When true, the reader may route id-less responses to the sole pending
    /// waiter as a bootstrap interoperability exception.
    bootstrap_mode: Arc<AtomicBool>,
}

impl StdioMcpClient {
    /// Create a new stdio MCP client.
    ///
    /// Returns an error if the subprocess cannot be spawned or its stdio
    /// pipes cannot be established, instead of panicking.
    pub fn new(server_id: String, config: McpServerConfig) -> Result<Self, String> {
        let (command, args, env) = match &config.transport {
            McpTransport::Stdio { command, args, env } => (command, args, env),
            other => {
                return Err(format!(
                    "StdioMcpClient requires Stdio transport, got {:?}",
                    other
                ))
            }
        };

        let mut cmd = Command::new(command);
        cmd.args(args)
            .envs(sanitized_stdio_env())
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(working_dir) = &config.working_dir {
            cmd.current_dir(working_dir);
        }

        let mut process = cmd.spawn().map_err(|e| {
            format!(
                "Failed to spawn MCP server process '{}' for server '{}': {}",
                redact_command(command),
                server_id,
                e
            )
        })?;

        let stdin = process.stdin.take().ok_or_else(|| {
            format!(
                "Failed to acquire stdin pipe for MCP server '{}'",
                server_id
            )
        })?;
        let stdout = process.stdout.take().ok_or_else(|| {
            format!(
                "Failed to acquire stdout pipe for MCP server '{}'",
                server_id
            )
        })?;
        let stderr = process.stderr.take().ok_or_else(|| {
            format!(
                "Failed to acquire stderr pipe for MCP server '{}'",
                server_id
            )
        })?;

        let waiters = Arc::new(Mutex::new(HashMap::<
            u64,
            tokio::sync::oneshot::Sender<Result<JsonRpcResponse, String>>,
        >::new()));
        let stderr_excerpt = Arc::new(Mutex::new(VecDeque::new()));
        let bootstrap_mode = Arc::new(AtomicBool::new(true));

        let stdout_task = tokio::spawn(start_stdio_stdout_reader(
            server_id.clone(),
            stdout,
            Arc::clone(&waiters),
            Arc::clone(&stderr_excerpt),
            Arc::clone(&bootstrap_mode),
        ));
        let stderr_task = tokio::spawn(start_stdio_stderr_reader(
            server_id.clone(),
            stderr,
            Arc::clone(&stderr_excerpt),
        ));

        Ok(Self {
            server_id,
            _config: config,
            stdin: Arc::new(Mutex::new(stdin)),
            process: Arc::new(Mutex::new(process)),
            connected: AtomicBool::new(false),
            request_counter: Arc::new(Mutex::new(0)),
            waiters,
            stdout_task: Arc::new(Mutex::new(Some(stdout_task))),
            stderr_task: Arc::new(Mutex::new(Some(stderr_task))),
            stderr_excerpt,
            bootstrap_mode,
        })
    }

    /// Send a request to the MCP server and wait for the response.
    async fn send_request<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, String> {
        let id = {
            let mut counter = self.request_counter.lock().await;
            *counter += 1;
            *counter
        };

        let request = ProtocolJsonRpcRequest::new(method, params, id);

        let request_json = serde_json::to_string(&request)
            .map_err(|e| format!("Failed to serialize request: {}", e))?;

        let (tx, rx) = tokio::sync::oneshot::channel::<Result<JsonRpcResponse, String>>();
        {
            let mut waiters = self.waiters.lock().await;
            waiters.insert(id, tx);
        }

        let mut stdin = self.stdin.lock().await;
        if let Err(e) = stdin
            .write_all(format!("{}\n", request_json).as_bytes())
            .await
        {
            self.cleanup_waiter(id).await;
            return Err(format!("Failed to write to stdin: {}", e));
        }
        if let Err(e) = stdin.flush().await {
            self.cleanup_waiter(id).await;
            return Err(format!("Failed to flush stdin: {}", e));
        }

        drop(stdin);

        // Wait for the response with a timeout to prevent hanging
        let response: JsonRpcResponse =
            match tokio::time::timeout(tokio::time::Duration::from_secs(30), rx).await {
                Ok(Ok(Ok(response))) => response,
                Ok(Ok(Err(error_msg))) => return Err(error_msg),
                Ok(Err(_)) => {
                    return Err(self.reader_terminated_error(
                        "stdout reader dropped before delivering response",
                    ));
                }
                Err(_) => {
                    self.cleanup_waiter(id).await;
                    return Err(format!(
                        "Timeout waiting for stdio response for request {} on server '{}'",
                        id, self.server_id
                    ));
                }
            };

        if response.id != Some(id) {
            // The reader may have routed an id-less bootstrap response to us.
            // Accept it only when the response is unambiguous.
            if response.id.is_none() && response.error.is_none() && response.result.is_some() {
                tracing::debug!(
                    "Accepting MCP response with missing id during bootstrap for server '{}'",
                    self.server_id
                );
            } else {
                return Err(format!(
                    "Response ID mismatch: expected {}, got {:?}",
                    id, response.id
                ));
            }
        }

        if let Some(error) = response.error {
            return Err(format_rpc_error(error));
        }

        response
            .result
            .ok_or_else(|| "Response missing result".to_string())
            .and_then(|v| {
                serde_json::from_value(v)
                    .map_err(|e| format!("Failed to deserialize result: {}", e))
            })
    }

    async fn cleanup_waiter(&self, id: u64) {
        self.waiters.lock().await.remove(&id);
    }

    fn reader_terminated_error(&self, prefix: &str) -> String {
        format!(
            "{} for server '{}'{}",
            prefix,
            self.server_id,
            stderr_suffix_blocking(&self.stderr_excerpt)
        )
    }
}

async fn start_stdio_stdout_reader(
    server_id: String,
    stdout: ChildStdout,
    waiters: WaiterMap,
    stderr_excerpt: Arc<Mutex<VecDeque<String>>>,
    bootstrap_mode: Arc<AtomicBool>,
) {
    let mut stdout = BufReader::new(stdout);
    let mut line = String::new();

    loop {
        line.clear();
        match stdout.read_line(&mut line).await {
            Ok(0) => {
                fail_pending_waiters(
                    &waiters,
                    format!(
                        "MCP stdio stdout closed for server '{}'{}",
                        server_id,
                        stderr_suffix_async(&stderr_excerpt).await
                    ),
                )
                .await;
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let response: JsonRpcResponse = match serde_json::from_str(trimmed) {
                    Ok(response) => response,
                    Err(e) => {
                        warn!(
                            "Ignoring non-JSON-RPC stdio message from MCP server '{}': {}",
                            server_id, e
                        );
                        continue;
                    }
                };

                let Some(id) = response.id else {
                    // During bootstrap, some MCP servers return a successful
                    // initialize response with a null or absent `id`. Route it
                    // to the sole pending waiter only when bootstrap_mode is
                    // still active and the response is unambiguous.
                    if bootstrap_mode.load(Ordering::SeqCst) {
                        let mut waiters_guard = waiters.lock().await;
                        if is_acceptable_bootstrap_response(&response, waiters_guard.len()) {
                            if let Some((_, sender)) = waiters_guard.drain().next() {
                                tracing::debug!(
                                    "Accepting MCP response with missing id during bootstrap for server '{}'",
                                    server_id
                                );
                                let _ = sender.send(Ok(response));
                            }
                            continue;
                        }
                    }
                    tracing::debug!(
                        "Received notification from MCP server '{}': {}",
                        server_id,
                        trimmed
                    );
                    continue;
                };

                let mut waiters_guard = waiters.lock().await;
                if let Some(sender) = waiters_guard.remove(&id) {
                    let _ = sender.send(Ok(response));
                } else {
                    tracing::debug!(
                        "No waiter found for response id {} from server '{}'",
                        id,
                        server_id
                    );
                }
            }
            Err(e) => {
                fail_pending_waiters(
                    &waiters,
                    format!(
                        "Failed reading stdio response from server '{}': {}{}",
                        server_id,
                        e,
                        stderr_suffix_async(&stderr_excerpt).await
                    ),
                )
                .await;
                break;
            }
        }
    }
}

async fn start_stdio_stderr_reader(
    server_id: String,
    stderr: tokio::process::ChildStderr,
    stderr_excerpt: Arc<Mutex<VecDeque<String>>>,
) {
    let mut stderr = BufReader::new(stderr);
    let mut line = String::new();

    loop {
        line.clear();
        match stderr.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => push_stderr_excerpt(&stderr_excerpt, line.trim_end()).await,
            Err(e) => {
                warn!(
                    "Failed reading stderr from MCP server '{}': {}",
                    server_id, e
                );
                break;
            }
        }
    }
}

async fn fail_pending_waiters(waiters: &WaiterMap, message: String) {
    let mut guard = waiters.lock().await;
    for (_, sender) in guard.drain() {
        let _ = sender.send(Err(message.clone()));
    }
}

/// Suffixes that indicate a sensitive environment variable when they appear
/// at the end of the var name (case-insensitive).
const SENSITIVE_SUFFIXES: &[&str] = &[
    "_API_KEY",
    "_SECRET",
    "_SECRET_KEY",
    "_TOKEN",
    "_PASSWORD",
    "_CREDENTIALS",
    "_AUTH_TOKEN",
    "_ACCESS_KEY",
    "_ACCESS_TOKEN",
];

/// Exact environment variable names that are always considered sensitive
/// (case-insensitive).
const SENSITIVE_EXACT_NAMES: &[&str] = &[
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AZURE_CLIENT_SECRET",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "DATABASE_URL",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
];

/// Returns true if the given environment variable name matches a known
/// sensitive pattern. Matching is case-insensitive.
fn is_sensitive_env_var(name: &str) -> bool {
    let upper = name.to_uppercase();
    SENSITIVE_EXACT_NAMES.iter().any(|exact| upper == *exact)
        || SENSITIVE_SUFFIXES
            .iter()
            .any(|suffix| upper.ends_with(suffix))
}

/// Build the environment for an MCP stdio subprocess by inheriting the full
/// parent environment and stripping vars whose names match sensitive patterns.
/// Logs the names (not values) of stripped vars at debug level.
fn sanitized_stdio_env() -> HashMap<String, String> {
    let mut env: HashMap<String, String> = std::env::vars().collect();
    let sensitive_keys: Vec<String> = env
        .keys()
        .filter(|k| is_sensitive_env_var(k))
        .cloned()
        .collect();

    if !sensitive_keys.is_empty() {
        tracing::debug!(
            "Stripping {} sensitive env var(s) from MCP stdio subprocess: {}",
            sensitive_keys.len(),
            sensitive_keys.join(", ")
        );
        for key in &sensitive_keys {
            env.remove(key);
        }
    }

    env
}

fn redact_command(command: &str) -> String {
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
        .to_string()
}

fn format_rpc_error(error: JsonRpcError) -> String {
    match error.data {
        Some(data) => format!(
            "RPC error {}: {} ({})",
            error.code,
            error.message,
            serde_json::to_string(&data)
                .unwrap_or_else(|_| "unserializable error data".to_string())
        ),
        None => format!("RPC error {}: {}", error.code, error.message),
    }
}

/// Decide whether a response with a missing or null JSON-RPC `id` can be
/// safely accepted as a bootstrap (`initialize`) reply.
///
/// The rule is intentionally narrow: accept only when there is exactly one
/// pending waiter, the response carries a result (not an error), and the
/// response `id` is absent. This preserves strict correlation for all
/// post-bootstrap traffic.
fn is_acceptable_bootstrap_response(response: &JsonRpcResponse, waiter_count: usize) -> bool {
    response.id.is_none()
        && response.error.is_none()
        && response.result.is_some()
        && waiter_count == 1
}

async fn push_stderr_excerpt(stderr_excerpt: &Arc<Mutex<VecDeque<String>>>, line: &str) {
    let sanitized = sanitize_stderr_line(line);
    if sanitized.is_empty() {
        return;
    }

    let mut excerpt = stderr_excerpt.lock().await;
    excerpt.push_back(sanitized);
    while excerpt.len() > 10 {
        excerpt.pop_front();
    }
}

async fn stderr_suffix_async(stderr_excerpt: &Arc<Mutex<VecDeque<String>>>) -> String {
    let excerpt = stderr_excerpt.lock().await;
    format_stderr_suffix(&excerpt)
}

fn stderr_suffix_blocking(stderr_excerpt: &Arc<Mutex<VecDeque<String>>>) -> String {
    stderr_excerpt
        .try_lock()
        .ok()
        .map_or_else(String::new, |excerpt| format_stderr_suffix(&excerpt))
}

fn format_stderr_suffix(excerpt: &VecDeque<String>) -> String {
    excerpt
        .back()
        .map(|line| format!(" [stderr excerpt: {}]", line))
        .unwrap_or_default()
}

fn sanitize_stderr_line(line: &str) -> String {
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated: String = collapsed.chars().take(240).collect();
    if collapsed.chars().count() > 240 {
        format!("{}…", truncated)
    } else {
        truncated
    }
}

#[async_trait]
impl McpTransportClient for StdioMcpClient {
    async fn initialize(&self) -> Result<protocol_messages::InitializeResponse, String> {
        let request = protocol_messages::InitializeRequest {
            protocol_version: "2024-11-05".to_string(),
            capabilities: serde_json::json!({}),
            client_info: protocol_messages::ClientInfo {
                name: "iron-core".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let result = self
            .send_request("initialize", serde_json::to_value(request).unwrap())
            .await?;

        self.bootstrap_mode.store(false, Ordering::SeqCst);
        self.connected.store(true, Ordering::SeqCst);
        Ok(result)
    }

    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String> {
        let mut cursor = None;
        let mut discovered = Vec::new();

        loop {
            let request = protocol_messages::ListToolsRequest {
                cursor: cursor.clone(),
            };

            let response: protocol_messages::ListToolsResponse = self
                .send_request("tools/list", serde_json::to_value(request).unwrap())
                .await?;

            discovered.extend(response.tools.into_iter().map(|tool| McpToolInfo {
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
            }));

            match response.next_cursor {
                Some(next_cursor) => cursor = Some(next_cursor),
                None => return Ok(discovered),
            }
        }
    }

    async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<Value, String> {
        let request = protocol_messages::CallToolRequest {
            name: tool_name.to_string(),
            arguments,
        };

        let response: protocol_messages::CallToolResponse = self
            .send_request("tools/call", serde_json::to_value(request).unwrap())
            .await?;

        if response.is_error {
            return Err(tool_error_to_string(response.content));
        }

        Ok(tool_content_to_value(response.content))
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn close(&self) {
        self.connected.store(false, Ordering::SeqCst);
        self.waiters.lock().await.clear();
        if let Some(handle) = self.stdout_task.lock().await.take() {
            handle.abort();
        }
        if let Some(handle) = self.stderr_task.lock().await.take() {
            handle.abort();
        }
        if let Ok(mut process) = self.process.try_lock() {
            let _ = process.kill().await;
        }
    }
}

/// MCP transport client using HTTP.
pub struct HttpMcpClient {
    #[allow(dead_code)]
    server_id: String,
    url: String,
    headers: Option<HashMap<String, String>>,
    client: reqwest::Client,
    connected: AtomicBool,
    request_counter: Arc<Mutex<u64>>,
}

impl HttpMcpClient {
    /// Create a new HTTP MCP client.
    pub fn new(server_id: String, config: HttpConfig) -> Self {
        Self {
            server_id,
            url: config.url,
            headers: config.headers,
            client: reqwest::Client::new(),
            connected: AtomicBool::new(false),
            request_counter: Arc::new(Mutex::new(0)),
        }
    }

    /// Send a request to the MCP server via HTTP POST.
    async fn send_request<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, String> {
        let id = {
            let mut counter = self.request_counter.lock().await;
            *counter += 1;
            *counter
        };

        let request = ProtocolJsonRpcRequest::new(method, params, id);

        let builder = self.client.post(&self.url).json(&request);
        let response = apply_headers(builder, &self.headers)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        let rpc_response: JsonRpcResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse JSON response: {}", e))?;

        if rpc_response.id != Some(id) {
            // During bootstrap (initialize), some MCP servers return a
            // successful response with a null or absent `id`. Accept it only
            // when the response is unambiguous: no error, has a result, and
            // there is no conflicting ID present.
            if rpc_response.id.is_none()
                && rpc_response.error.is_none()
                && rpc_response.result.is_some()
            {
                tracing::debug!(
                    "Accepting MCP response with missing id during bootstrap for server '{}'",
                    self.server_id
                );
            } else {
                return Err(format!(
                    "Response ID mismatch: expected {}, got {:?}",
                    id, rpc_response.id
                ));
            }
        }

        if let Some(error) = rpc_response.error {
            return Err(format_rpc_error(error));
        }

        rpc_response
            .result
            .ok_or_else(|| "Response missing result".to_string())
            .and_then(|v| {
                serde_json::from_value(v)
                    .map_err(|e| format!("Failed to deserialize result: {}", e))
            })
    }
}

#[async_trait]
impl McpTransportClient for HttpMcpClient {
    async fn initialize(&self) -> Result<protocol_messages::InitializeResponse, String> {
        let request = protocol_messages::InitializeRequest {
            protocol_version: "2024-11-05".to_string(),
            capabilities: serde_json::json!({}),
            client_info: protocol_messages::ClientInfo {
                name: "iron-core".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let result = self
            .send_request("initialize", serde_json::to_value(request).unwrap())
            .await?;

        self.connected.store(true, Ordering::SeqCst);
        Ok(result)
    }

    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String> {
        let mut cursor = None;
        let mut discovered = Vec::new();

        loop {
            let request = protocol_messages::ListToolsRequest {
                cursor: cursor.clone(),
            };

            let response: protocol_messages::ListToolsResponse = self
                .send_request("tools/list", serde_json::to_value(request).unwrap())
                .await?;

            discovered.extend(response.tools.into_iter().map(|tool| McpToolInfo {
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
            }));

            match response.next_cursor {
                Some(next_cursor) => cursor = Some(next_cursor),
                None => return Ok(discovered),
            }
        }
    }

    async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<Value, String> {
        let request = protocol_messages::CallToolRequest {
            name: tool_name.to_string(),
            arguments,
        };

        let response: protocol_messages::CallToolResponse = self
            .send_request("tools/call", serde_json::to_value(request).unwrap())
            .await?;

        if response.is_error {
            return Err(tool_error_to_string(response.content));
        }

        Ok(tool_content_to_value(response.content))
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn close(&self) {
        self.connected.store(false, Ordering::SeqCst);
    }
}

/// MCP transport client using HTTP+SSE (Server-Sent Events).
///
/// Uses a request-id-based dispatcher so that multiple concurrent in-flight
/// requests on the same shared SSE connection each receive their own response
/// without competing for the shared event stream.
pub struct HttpSseMcpClient {
    server_id: String,
    url: String,
    headers: Option<HashMap<String, String>>,
    client: reqwest::Client,
    connected: AtomicBool,
    request_counter: Arc<Mutex<u64>>,
    /// Per-request oneshot channels keyed by JSON-RPC request id.
    /// The SSE reader task dispatches each parsed response to the matching waiter.
    waiters: Arc<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<JsonRpcResponse>>>>,
    /// Handle to the SSE reader task so we can signal shutdown on `close()`.
    sse_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// When true, the SSE reader may route id-less responses to the sole
    /// pending waiter as a bootstrap interoperability exception.
    bootstrap_mode: Arc<AtomicBool>,
}

impl HttpSseMcpClient {
    /// Create a new HTTP+SSE MCP client.
    pub fn new(server_id: String, config: HttpConfig) -> Self {
        Self {
            server_id,
            url: config.url,
            headers: config.headers,
            client: reqwest::Client::new(),
            connected: AtomicBool::new(false),
            request_counter: Arc::new(Mutex::new(0)),
            waiters: Arc::new(Mutex::new(HashMap::new())),
            sse_task: Arc::new(Mutex::new(None)),
            bootstrap_mode: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Ensure the SSE reader task is running. Idempotent.
    async fn ensure_sse_reader(&self) -> Result<(), String> {
        let mut task_guard = self.sse_task.lock().await;
        if task_guard.is_some() {
            return Ok(());
        }

        let url = self.url.clone();
        let client = self.client.clone();
        let headers = self.headers.clone();
        let waiters: Arc<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<JsonRpcResponse>>>> =
            self.waiters.clone();
        let bootstrap_mode = self.bootstrap_mode.clone();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();

        let handle = tokio::spawn(async move {
            let get_builder = client.get(&url);
            let response = match tokio::time::timeout(
                tokio::time::Duration::from_secs(3),
                apply_headers(get_builder, &headers).send(),
            )
            .await
            {
                Ok(Ok(resp)) => resp,
                Ok(Err(e)) => {
                    let _ = ready_tx.send(Err(format!(
                        "Failed to establish SSE endpoint for MCP server: {}",
                        e
                    )));
                    error!("Failed to connect to SSE endpoint: {}", e);
                    return;
                }
                Err(_) => {
                    let _ = ready_tx.send(Err(
                        "Timed out establishing SSE endpoint for MCP server".to_string(),
                    ));
                    error!("Timed out connecting to SSE endpoint");
                    return;
                }
            };

            if !response.status().is_success() {
                let _ = ready_tx.send(Err(format!(
                    "SSE endpoint returned unsuccessful status: {}",
                    response.status()
                )));
                error!("SSE connection failed with status: {}", response.status());
                return;
            }

            if ready_tx.send(Ok(())).is_err() {
                return;
            }

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        // Process complete SSE event blocks (delimited by blank line)
                        while let Some(pos) = buffer.find("\n\n") {
                            let event_block = buffer[..pos].to_string();
                            buffer.drain(..pos + 2);

                            // Parse SSE fields
                            let mut event_type: Option<String> = None;
                            let mut data_lines: Vec<String> = Vec::new();
                            for line in event_block.lines() {
                                if line.starts_with(':') || line.is_empty() {
                                    // Comment or blank — skip
                                    continue;
                                }
                                if let Some(rest) = line.strip_prefix("event:") {
                                    event_type = Some(rest.trim_start().to_string());
                                    continue;
                                }
                                if let Some(rest) = line.strip_prefix("data:") {
                                    data_lines.push(rest.trim_start().to_string());
                                }
                            }

                            if data_lines.is_empty() {
                                continue;
                            }

                            // Skip non-payload events (keepalive, ping, etc.)
                            match event_type.as_deref() {
                                Some("ping") | Some("keepalive") => continue,
                                _ => {}
                            }

                            let payload = data_lines.join("\n");

                            // Try to parse as a JSON-RPC response
                            let rpc_response: JsonRpcResponse = match serde_json::from_str(&payload)
                            {
                                Ok(resp) => resp,
                                Err(_) => {
                                    // Not a JSON-RPC response; ignore (e.g. endpoint info)
                                    continue;
                                }
                            };

                            let Some(id) = rpc_response.id else {
                                // Bootstrap exception: route id-less response to
                                // the sole pending waiter only during bootstrap.
                                if bootstrap_mode.load(Ordering::SeqCst) {
                                    let mut waiters_guard = waiters.lock().await;
                                    if is_acceptable_bootstrap_response(
                                        &rpc_response,
                                        waiters_guard.len(),
                                    ) {
                                        if let Some((_, sender)) = waiters_guard.drain().next() {
                                            tracing::debug!(
                                                "Accepting MCP SSE response with missing id during bootstrap"
                                            );
                                            let _ = sender.send(rpc_response);
                                        }
                                    }
                                }
                                continue;
                            };

                            // Dispatch to the waiting request, if any
                            let mut waiters_guard = waiters.lock().await;
                            if let Some(sender) = waiters_guard.remove(&id) {
                                let _ = sender.send(rpc_response);
                            }
                            // If no waiter exists for this id, the response is dropped.
                            // This can happen for late-arriving or duplicate responses.
                        }
                    }
                    Err(e) => {
                        error!("SSE stream error: {}", e);
                        break;
                    }
                }
            }
        });

        *task_guard = Some(handle);

        drop(task_guard);

        tokio::time::timeout(tokio::time::Duration::from_secs(4), ready_rx)
            .await
            .map_err(|_| "Timed out waiting for SSE startup confirmation".to_string())?
            .map_err(|_| "SSE reader exited before startup confirmation".to_string())?
    }

    /// Send a request to the MCP server via HTTP POST and receive the response
    /// via the dispatcher-backed SSE stream.
    async fn send_request<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, String> {
        let id = {
            let mut counter = self.request_counter.lock().await;
            *counter += 1;
            *counter
        };

        let request = ProtocolJsonRpcRequest::new(method, params, id);

        // Ensure SSE reader is running
        self.ensure_sse_reader().await?;

        // Register a oneshot waiter for this request id
        let (tx, rx) = tokio::sync::oneshot::channel::<JsonRpcResponse>();
        {
            let mut waiters = self.waiters.lock().await;
            waiters.insert(id, tx);
        }

        // Send POST request
        let post_builder = self.client.post(&self.url).json(&request);
        let post_response = apply_headers(post_builder, &self.headers)
            .send()
            .await
            .map_err(|e| {
                self.cleanup_waiter(id);
                format!("HTTP+SSE post failed: {}", e)
            })?;

        if !post_response.status().is_success() && post_response.status() != 202 {
            self.cleanup_waiter(id);
            return Err(format!("HTTP+SSE post error: {}", post_response.status()));
        }

        // Wait for the SSE reader to dispatch our response
        let rpc_response = tokio::time::timeout(tokio::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| {
                self.cleanup_waiter(id);
                format!(
                    "Timeout waiting for SSE response for request {} on server '{}'",
                    id, self.server_id
                )
            })?
            .map_err(|_| {
                self.cleanup_waiter(id);
                format!(
                    "SSE reader dropped before delivering response for request {} on server '{}'",
                    id, self.server_id
                )
            })?;

        if let Some(error) = rpc_response.error {
            return Err(format_rpc_error(error));
        }

        rpc_response
            .result
            .ok_or_else(|| "Response missing result".to_string())
            .and_then(|v| {
                serde_json::from_value(v)
                    .map_err(|e| format!("Failed to deserialize result: {}", e))
            })
    }

    /// Remove a waiter from the map if it hasn't been consumed yet.
    fn cleanup_waiter(&self, id: u64) {
        // Use a blocking lock here because we are called from async context
        // after an error, and we don't want to hold the async mutex across
        // the error-path boundary.  The critical section is trivial.
        if let Ok(mut waiters) = self.waiters.try_lock() {
            waiters.remove(&id);
        }
    }
}

#[async_trait]
impl McpTransportClient for HttpSseMcpClient {
    async fn initialize(&self) -> Result<protocol_messages::InitializeResponse, String> {
        let request = protocol_messages::InitializeRequest {
            protocol_version: "2024-11-05".to_string(),
            capabilities: serde_json::json!({}),
            client_info: protocol_messages::ClientInfo {
                name: "iron-core".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let result = self
            .send_request("initialize", serde_json::to_value(request).unwrap())
            .await?;

        self.bootstrap_mode.store(false, Ordering::SeqCst);
        self.connected.store(true, Ordering::SeqCst);
        Ok(result)
    }

    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String> {
        let mut cursor = None;
        let mut discovered = Vec::new();

        loop {
            let request = protocol_messages::ListToolsRequest {
                cursor: cursor.clone(),
            };

            let response: protocol_messages::ListToolsResponse = self
                .send_request("tools/list", serde_json::to_value(request).unwrap())
                .await?;

            discovered.extend(response.tools.into_iter().map(|tool| McpToolInfo {
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
            }));

            match response.next_cursor {
                Some(next_cursor) => cursor = Some(next_cursor),
                None => return Ok(discovered),
            }
        }
    }

    async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<Value, String> {
        let request = protocol_messages::CallToolRequest {
            name: tool_name.to_string(),
            arguments,
        };

        let response: protocol_messages::CallToolResponse = self
            .send_request("tools/call", serde_json::to_value(request).unwrap())
            .await?;

        if response.is_error {
            return Err(tool_error_to_string(response.content));
        }

        Ok(tool_content_to_value(response.content))
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn close(&self) {
        self.connected.store(false, Ordering::SeqCst);
        // Abort the SSE reader task
        if let Some(handle) = self.sse_task.lock().await.take() {
            handle.abort();
        }
        // Drop any pending waiters so callers get a clean error
        self.waiters.lock().await.clear();
    }
}

/// Factory for creating transport clients based on configuration.
///
/// Returns an error if the transport client cannot be constructed (e.g. the
/// subprocess for a stdio transport fails to spawn or its pipes cannot be
/// established). This allows the connection manager to surface the failure as
/// an error health state rather than panicking.
pub fn create_transport_client(
    server_id: &str,
    config: &McpServerConfig,
) -> Result<Box<dyn McpTransportClient>, String> {
    match &config.transport {
        McpTransport::Stdio { .. } => {
            let client = StdioMcpClient::new(server_id.to_string(), config.clone())?;
            Ok(Box::new(client))
        }
        McpTransport::Http { config: http_config } => Ok(Box::new(HttpMcpClient::new(
            server_id.to_string(),
            http_config.clone(),
        ))),
        McpTransport::HttpSse { config: http_config } => Ok(Box::new(HttpSseMcpClient::new(
            server_id.to_string(),
            http_config.clone(),
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_api_key_is_sensitive() {
        assert!(is_sensitive_env_var("MY_SERVICE_API_KEY"));
        assert!(is_sensitive_env_var("SOMETHING_API_KEY"));
    }

    #[test]
    fn suffix_secret_is_sensitive() {
        assert!(is_sensitive_env_var("APP_SECRET"));
        assert!(is_sensitive_env_var("CLIENT_SECRET_KEY"));
    }

    #[test]
    fn suffix_token_is_sensitive() {
        assert!(is_sensitive_env_var("SESSION_TOKEN"));
        assert!(is_sensitive_env_var("AUTH_TOKEN"));
    }

    #[test]
    fn suffix_password_is_sensitive() {
        assert!(is_sensitive_env_var("DB_PASSWORD"));
    }

    #[test]
    fn suffix_credentials_is_sensitive() {
        assert!(is_sensitive_env_var("GOOGLE_CREDENTIALS"));
    }

    #[test]
    fn suffix_access_key_is_sensitive() {
        assert!(is_sensitive_env_var("S3_ACCESS_KEY"));
        assert!(is_sensitive_env_var("S3_ACCESS_TOKEN"));
    }

    #[test]
    fn exact_aws_keys_are_sensitive() {
        assert!(is_sensitive_env_var("AWS_ACCESS_KEY_ID"));
        assert!(is_sensitive_env_var("AWS_SECRET_ACCESS_KEY"));
        assert!(is_sensitive_env_var("AWS_SESSION_TOKEN"));
    }

    #[test]
    fn exact_cloud_credentials_are_sensitive() {
        assert!(is_sensitive_env_var("AZURE_CLIENT_SECRET"));
        assert!(is_sensitive_env_var("GOOGLE_APPLICATION_CREDENTIALS"));
    }

    #[test]
    fn exact_token_names_are_sensitive() {
        assert!(is_sensitive_env_var("GITHUB_TOKEN"));
        assert!(is_sensitive_env_var("GH_TOKEN"));
    }

    #[test]
    fn exact_api_keys_are_sensitive() {
        assert!(is_sensitive_env_var("ANTHROPIC_API_KEY"));
        assert!(is_sensitive_env_var("OPENAI_API_KEY"));
    }

    #[test]
    fn database_url_is_sensitive() {
        assert!(is_sensitive_env_var("DATABASE_URL"));
    }

    #[test]
    fn common_toolchain_vars_are_not_sensitive() {
        assert!(!is_sensitive_env_var("PATH"));
        assert!(!is_sensitive_env_var("HOME"));
        assert!(!is_sensitive_env_var("APPDATA"));
        assert!(!is_sensitive_env_var("LOCALAPPDATA"));
        assert!(!is_sensitive_env_var("USERPROFILE"));
        assert!(!is_sensitive_env_var("XDG_CONFIG_HOME"));
        assert!(!is_sensitive_env_var("XDG_DATA_HOME"));
        assert!(!is_sensitive_env_var("CARGO_HOME"));
        assert!(!is_sensitive_env_var("GOPATH"));
        assert!(!is_sensitive_env_var("NODE_PATH"));
        assert!(!is_sensitive_env_var("PYTHONPATH"));
        assert!(!is_sensitive_env_var("LANG"));
        assert!(!is_sensitive_env_var("TERM"));
        assert!(!is_sensitive_env_var("SYSTEMROOT"));
    }

    #[test]
    fn case_insensitive_suffix_matching() {
        assert!(is_sensitive_env_var("my_service_api_key"));
        assert!(is_sensitive_env_var("My_Service_Api_Key"));
        assert!(is_sensitive_env_var("APP_Secret"));
        assert!(is_sensitive_env_var("Session_Token"));
    }

    #[test]
    fn case_insensitive_exact_matching() {
        assert!(is_sensitive_env_var("github_token"));
        assert!(is_sensitive_env_var("Github_Token"));
        assert!(is_sensitive_env_var("anthropic_api_key"));
        assert!(is_sensitive_env_var("Anthropic_Api_Key"));
        assert!(is_sensitive_env_var("database_url"));
        assert!(is_sensitive_env_var("Database_Url"));
    }

    #[test]
    fn sanitized_env_preserves_non_sensitive_vars() {
        let env = sanitized_stdio_env();
        assert!(
            env.contains_key("PATH"),
            "PATH should be preserved in sanitized env"
        );
    }

    #[test]
    fn sanitized_env_strips_sensitive_test_vars() {
        let test_vars = vec![
            ("TEST_IRON_CORE_MY_API_KEY", "secret-key-123"),
            ("TEST_IRON_CORE_AUTH_TOKEN", "tok-456"),
            ("TEST_IRON_CORE_DB_PASSWORD", "pw-789"),
            ("TEST_IRON_CORE_GITHUB_TOKEN", "gh-token"),
        ];

        for (key, value) in &test_vars {
            std::env::set_var(key, value);
        }

        let env = sanitized_stdio_env();

        for (key, _) in &test_vars {
            assert!(
                !env.contains_key(*key),
                "sensitive var '{}' should have been stripped",
                key
            );
            std::env::remove_var(key);
        }
    }

    #[test]
    fn sanitized_env_preserves_toolchain_test_vars() {
        let test_vars = vec![
            ("TEST_IRON_CORE_CARGO_HOME", "/cargo"),
            ("TEST_IRON_CORE_GOPATH", "/go"),
            ("TEST_IRON_CORE_NODE_PATH", "/node"),
            ("TEST_IRON_CORE_XDG_CONFIG_HOME", "/xdg"),
        ];

        for (key, value) in &test_vars {
            std::env::set_var(key, value);
        }

        let env = sanitized_stdio_env();

        for (key, expected) in &test_vars {
            assert_eq!(
                env.get(*key),
                Some(&expected.to_string()),
                "non-sensitive var '{}' should be preserved",
                key
            );
            std::env::remove_var(key);
        }
    }
}
