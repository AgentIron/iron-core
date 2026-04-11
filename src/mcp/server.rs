use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

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
    Http { url: String },
    /// Remote HTTP with SSE transport
    HttpSse { url: String },
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
}

impl McpServerRegistry {
    pub fn new() -> Self {
        Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new MCP server configuration
    pub fn register_server(&self, config: McpServerConfig) {
        let mut servers = self.servers.write().unwrap();
        let id = config.id.clone();
        let state = McpServerState::new(config);
        servers.insert(id.clone(), state);
        info!("Registered MCP server: {}", id);
    }

    /// Remove a server from the registry
    pub fn unregister_server(&self, server_id: &str) -> Option<McpServerState> {
        let mut servers = self.servers.write().unwrap();
        let removed = servers.remove(server_id);
        if removed.is_some() {
            info!("Unregistered MCP server: {}", server_id);
        }
        removed
    }

    /// Get all configured servers
    pub fn list_servers(&self) -> Vec<McpServerState> {
        let servers = self.servers.read().unwrap();
        servers.values().cloned().collect()
    }

    /// Get a specific server by ID
    pub fn get_server(&self, server_id: &str) -> Option<McpServerState> {
        let servers = self.servers.read().unwrap();
        servers.get(server_id).cloned()
    }

    /// Update server health state
    pub fn update_health(&self, server_id: &str, health: McpServerHealth) {
        let mut servers = self.servers.write().unwrap();
        if let Some(state) = servers.get_mut(server_id) {
            state.health = health;
            if health.is_usable() {
                state.last_error = None;
            }
        }
    }

    /// Update server health to error state with message
    pub fn set_error(&self, server_id: &str, error: String) {
        let mut servers = self.servers.write().unwrap();
        if let Some(state) = servers.get_mut(server_id) {
            let log_message = error.clone();
            state.health = McpServerHealth::Error;
            state.last_error = Some(error);
            warn!(
                "MCP server {} entered error state: {}",
                server_id, log_message
            );
        }
    }

    /// Update discovered tools for a server
    pub fn update_discovered_tools(&self, server_id: &str, tools: Vec<McpToolInfo>) {
        let mut servers = self.servers.write().unwrap();
        if let Some(state) = servers.get_mut(server_id) {
            state.discovered_tools = tools;
            info!(
                "Updated MCP server {} with {} discovered tools",
                server_id,
                state.discovered_tools.len()
            );
        }
    }

    /// Check if a server is currently usable
    pub fn is_server_usable(&self, server_id: &str) -> bool {
        let servers = self.servers.read().unwrap();
        servers
            .get(server_id)
            .map(|s| s.health.is_usable())
            .unwrap_or(false)
    }

    /// Get all tools from all usable servers
    pub fn get_all_usable_tools(&self) -> Vec<(String, McpToolInfo)> {
        let servers = self.servers.read().unwrap();
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
