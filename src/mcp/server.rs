use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tracing::{info, warn};

/// Configuration shared by HTTP-based MCP transports.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HttpConfig {
    /// Server URL for the MCP endpoint.
    pub url: String,
    /// Optional custom HTTP headers to send with every request.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

impl HttpConfig {
    /// Create a new `HttpConfig` with the given URL and no custom headers.
    pub fn new(url: String) -> Self {
        Self { url, headers: None }
    }
}

/// Transport type for MCP servers
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum McpTransport {
    /// Local stdio transport (spawns subprocess)
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
    /// Remote HTTP transport
    Http {
        #[serde(flatten)]
        config: HttpConfig,
    },
    /// Remote HTTP with SSE transport
    HttpSse {
        #[serde(flatten)]
        config: HttpConfig,
    },
}

/// Health state of an MCP server connection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpServerHealth {
    /// Initial state before connection attempt
    Configured,
    /// Currently connecting
    Connecting,
    /// Connected and operational
    Connected,
    /// Connection lost or failed
    Error,
    /// Explicitly disabled by configuration
    Disabled,
}

impl McpServerHealth {
    /// Returns true if the server is currently connected and usable
    pub fn is_usable(&self) -> bool {
        matches!(self, McpServerHealth::Connected)
    }
}

/// Metadata about a discovered MCP tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    /// JSON schema for the tool's input
    pub input_schema: serde_json::Value,
}

/// Configuration for an MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Stable runtime identity for this server
    pub id: String,
    /// Display label for this server
    pub label: String,
    /// Transport configuration
    pub transport: McpTransport,
    /// Whether this server should be enabled by default for new sessions
    pub enabled_by_default: bool,
    /// Optional working directory for stdio transports
    pub working_dir: Option<PathBuf>,
}

/// Runtime state for a configured MCP server
#[derive(Debug, Clone)]
pub struct McpServerState {
    pub config: McpServerConfig,
    pub health: McpServerHealth,
    /// Discovered tools from this server, if connected
    pub discovered_tools: Vec<McpToolInfo>,
    /// Last error message, if any
    pub last_error: Option<String>,
}

impl McpServerState {
    pub fn new(config: McpServerConfig) -> Self {
        Self {
            config,
            health: McpServerHealth::Configured,
            discovered_tools: Vec::new(),
            last_error: None,
        }
    }
}

/// Registry for MCP servers and their state
#[derive(Debug, Clone, Default)]
pub struct McpServerRegistry {
    servers: Arc<RwLock<HashMap<String, McpServerState>>>,
    version: Arc<AtomicU64>,
}

impl McpServerRegistry {
    pub fn new() -> Self {
        Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
            version: Arc::new(AtomicU64::new(0)),
        }
    }

    fn bump_version(&self) {
        self.version.fetch_add(1, Ordering::SeqCst);
    }

    /// Return the current mutation version for cache invalidation.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    /// Register a new MCP server configuration
    pub fn register_server(&self, config: McpServerConfig) {
        let mut servers = self.servers.write();
        let id = config.id.clone();
        let state = McpServerState::new(config);
        servers.insert(id.clone(), state);
        drop(servers);
        self.bump_version();
        info!("Registered MCP server: {}", id);
    }

    /// Remove a server from the registry
    pub fn unregister_server(&self, server_id: &str) -> Option<McpServerState> {
        let mut servers = self.servers.write();
        let removed = servers.remove(server_id);
        drop(servers);
        if removed.is_some() {
            self.bump_version();
            info!("Unregistered MCP server: {}", server_id);
        }
        removed
    }

    /// Get all configured servers
    pub fn list_servers(&self) -> Vec<McpServerState> {
        let servers = self.servers.read();
        servers.values().cloned().collect()
    }

    /// Get a specific server by ID
    pub fn get_server(&self, server_id: &str) -> Option<McpServerState> {
        let servers = self.servers.read();
        servers.get(server_id).cloned()
    }

    /// Update server health state
    pub fn update_health(&self, server_id: &str, health: McpServerHealth) {
        let mut servers = self.servers.write();
        if let Some(state) = servers.get_mut(server_id) {
            if matches!(health, McpServerHealth::Connecting)
                && matches!(
                    state.health,
                    McpServerHealth::Connected | McpServerHealth::Error
                )
            {
                return;
            }

            state.health = health;
            if health.is_usable() {
                state.last_error = None;
            }
            drop(servers);
            self.bump_version();
        }
    }

    /// Update server health to error state with message
    pub fn set_error(&self, server_id: &str, error: String) {
        let mut servers = self.servers.write();
        if let Some(state) = servers.get_mut(server_id) {
            let log_message = error.clone();
            state.health = McpServerHealth::Error;
            state.last_error = Some(error);
            drop(servers);
            self.bump_version();
            warn!(
                "MCP server {} entered error state: {}",
                server_id, log_message
            );
        }
    }

    /// Update discovered tools for a server
    pub fn update_discovered_tools(&self, server_id: &str, tools: Vec<McpToolInfo>) {
        let mut servers = self.servers.write();
        if let Some(state) = servers.get_mut(server_id) {
            state.discovered_tools = tools;
            let tool_count = state.discovered_tools.len();
            drop(servers);
            self.bump_version();
            info!(
                "Updated MCP server {} with {} discovered tools",
                server_id, tool_count
            );
        }
    }

    /// Check if a server is currently usable
    pub fn is_server_usable(&self, server_id: &str) -> bool {
        let servers = self.servers.read();
        servers
            .get(server_id)
            .map(|s| s.health.is_usable())
            .unwrap_or(false)
    }

    /// Get all tools from all usable servers
    pub fn get_all_usable_tools(&self) -> Vec<(String, McpToolInfo)> {
        let servers = self.servers.read();
        let mut all_tools = Vec::new();
        for (server_id, state) in servers.iter() {
            if state.health.is_usable() {
                for tool in &state.discovered_tools {
                    all_tools.push((server_id.clone(), tool.clone()));
                }
            }
        }
        all_tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_config_serializes_without_headers_when_none() {
        let config = HttpConfig::new("https://example.com/mcp".to_string());
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            !json.contains("headers"),
            "serialized output should not contain 'headers' when None: {}",
            json
        );
        assert!(
            json.contains("\"url\""),
            "serialized output should contain 'url': {}",
            json
        );
    }

    #[test]
    fn http_config_round_trip_with_headers() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer token123".to_string());
        headers.insert("X-Custom".to_string(), "value".to_string());
        let config = HttpConfig {
            url: "https://example.com/mcp".to_string(),
            headers: Some(headers),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: HttpConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config, deserialized);
        assert_eq!(
            deserialized.headers.as_ref().unwrap().get("Authorization"),
            Some(&"Bearer token123".to_string())
        );
    }

    #[test]
    fn http_config_deserializes_without_headers() {
        let json = r#"{"url":"https://example.com/mcp"}"#;
        let config: HttpConfig = serde_json::from_str(json).unwrap();

        assert_eq!(config.url, "https://example.com/mcp");
        assert_eq!(config.headers, None);
    }

    #[test]
    fn mcp_transport_http_flattens_http_config() {
        let json = r#"{"Http":{"url":"https://example.com/mcp","headers":{"X-Key":"val"}}}"#;
        let transport: McpTransport = serde_json::from_str(json).unwrap();

        match transport {
            McpTransport::Http { config } => {
                assert_eq!(config.url, "https://example.com/mcp");
                assert_eq!(
                    config.headers.as_ref().unwrap().get("X-Key"),
                    Some(&"val".to_string())
                );
            }
            other => panic!("expected Http variant, got {:?}", other),
        }
    }

    #[test]
    fn mcp_transport_http_deserializes_without_headers() {
        let json = r#"{"Http":{"url":"https://example.com/mcp"}}"#;
        let transport: McpTransport = serde_json::from_str(json).unwrap();

        match transport {
            McpTransport::Http { config } => {
                assert_eq!(config.url, "https://example.com/mcp");
                assert_eq!(config.headers, None);
            }
            other => panic!("expected Http variant, got {:?}", other),
        }
    }
}
