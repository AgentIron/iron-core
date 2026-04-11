use crate::mcp::client::{create_transport_client, McpTransportClient};
use crate::mcp::server::{McpServerHealth, McpServerRegistry, McpTransport};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, RwLock};
use tokio::time::interval;
use tracing::{error, info, warn};

/// Manages MCP server connections and their lifecycle with automatic reconnection
#[derive(Debug, Clone)]
pub struct McpConnectionManager {
    registry: McpServerRegistry,
    connections: Arc<RwLock<HashMap<String, McpConnectionHandle>>>,
    reconnect_attempts: Arc<RwLock<HashMap<String, u32>>>,
    in_flight_connections: Arc<RwLock<HashSet<String>>>,
}

/// Handle to an active MCP connection
pub struct McpConnectionHandle {
    pub server_id: String,
    pub transport: McpTransport,
    client: Box<dyn McpTransportClient>,
}

impl std::fmt::Debug for McpConnectionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpConnectionHandle")
            .field("server_id", &self.server_id)
            .field("transport", &self.transport)
            .finish()
    }
}

impl Clone for McpConnectionHandle {
    fn clone(&self) -> Self {
        panic!("McpConnectionHandle cannot be cloned")
    }
}

/// Configuration for reconnection behavior
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub health_check_interval_secs: u64,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay_ms: 1000,
            max_delay_ms: 30000,
            health_check_interval_secs: 30,
        }
    }
}

impl McpConnectionManager {
    /// Create a new connection manager bound to a registry
    pub fn new(registry: McpServerRegistry) -> Self {
        Self {
            registry,
            connections: Arc::new(RwLock::new(HashMap::new())),
            reconnect_attempts: Arc::new(RwLock::new(HashMap::new())),
            in_flight_connections: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    async fn begin_connect_attempt(&self, server_id: &str) -> bool {
        if self.is_connected(server_id).await {
            return false;
        }

        let mut in_flight = self.in_flight_connections.write().await;
        if in_flight.contains(server_id) {
            return false;
        }

        if self.is_connected(server_id).await {
            return false;
        }

        in_flight.insert(server_id.to_string());
        true
    }

    async fn finish_connect_attempt(&self, server_id: &str) {
        self.in_flight_connections.write().await.remove(server_id);
    }

    /// Start the connection manager with background health monitoring
    pub async fn start(&self, config: ReconnectConfig, mut shutdown_rx: watch::Receiver<bool>) {
        // Connect to all configured servers
        self.connect_all().await;

        self.health_monitor_loop(config, &mut shutdown_rx).await;
    }

    /// Background task that monitors connection health and attempts reconnection
    async fn health_monitor_loop(
        &self,
        config: ReconnectConfig,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) {
        let mut ticker = interval(Duration::from_secs(config.health_check_interval_secs));

        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                changed = shutdown_rx.changed() => {
                    match changed {
                        Ok(()) if *shutdown_rx.borrow() => {
                            info!("Stopping MCP health monitor due to runtime shutdown");
                            break;
                        }
                        Ok(()) => {
                            continue;
                        }
                        Err(_) => {
                            info!("Stopping MCP health monitor because shutdown channel closed");
                            break;
                        }
                    }
                }
            }

            // Check all servers and reconnect if needed
            let servers = self.registry.list_servers();
            for server in servers {
                let server_id = server.config.id.clone();

                // Skip if already connected
                if self.is_connected(&server_id).await {
                    continue;
                }

                // Check if server is in error state and should be reconnected
                match server.health {
                    McpServerHealth::Error | McpServerHealth::Configured => {
                        // Attempt reconnection with exponential backoff
                        let attempts = {
                            let attempts = self.reconnect_attempts.read().await;
                            *attempts.get(&server_id).unwrap_or(&0)
                        };

                        if attempts < config.max_attempts {
                            let delay = calculate_backoff(
                                attempts,
                                config.base_delay_ms,
                                config.max_delay_ms,
                            );
                            info!(
                                "Scheduling reconnection for MCP server {} after {}ms (attempt {} of {})",
                                server_id, delay, attempts + 1, config.max_attempts
                            );

                            tokio::time::sleep(Duration::from_millis(delay)).await;

                            // Double-check we're still not connected before attempting
                            if !self.is_connected(&server_id).await {
                                self.connect_server(&server_id).await;

                                // Update attempt counter
                                let mut attempts_map = self.reconnect_attempts.write().await;
                                if self.is_connected(&server_id).await {
                                    // Success - reset counter
                                    attempts_map.remove(&server_id);
                                } else {
                                    // Failed - increment counter
                                    *attempts_map.entry(server_id).or_insert(0) += 1;
                                }
                            }
                        } else {
                            warn!(
                                "MCP server {} has exceeded max reconnection attempts ({}). Giving up.",
                                server_id, config.max_attempts
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Connect to all configured MCP servers
    pub async fn connect_all(&self) {
        let servers = self.registry.list_servers();
        for server in servers {
            // Only connect to servers that aren't disabled
            if server.health != McpServerHealth::Disabled {
                self.connect_server(&server.config.id).await;
            }
        }
    }

    /// Connect to a specific MCP server
    pub async fn connect_server(&self, server_id: &str) {
        let server = self.registry.get_server(server_id);
        if let Some(server) = server {
            if !self.begin_connect_attempt(server_id).await {
                return;
            }

            info!("Connecting to MCP server: {}", server_id);

            self.registry
                .update_health(server_id, McpServerHealth::Connecting);

            // Create transport client (may fail if subprocess cannot spawn)
            let client = match create_transport_client(server_id, &server.config) {
                Ok(client) => client,
                Err(e) => {
                    error!(
                        "Failed to create transport for MCP server {}: {}",
                        server_id, e
                    );
                    self.registry.set_error(server_id, e);
                    self.finish_connect_attempt(server_id).await;
                    return;
                }
            };

            // Initialize the connection
            match client.initialize().await {
                Ok(init_response) => {
                    info!(
                        "MCP server {} initialized: {} {}",
                        server_id,
                        init_response.server_info.name,
                        init_response.server_info.version
                    );

                    let handle = McpConnectionHandle {
                        server_id: server_id.to_string(),
                        transport: server.config.transport.clone(),
                        client,
                    };

                    {
                        let mut connections = self.connections.write().await;
                        connections.insert(server_id.to_string(), handle);
                    }

                    // Reset reconnection attempts on success
                    {
                        let mut attempts = self.reconnect_attempts.write().await;
                        attempts.remove(server_id);
                    }

                    self.registry
                        .update_health(server_id, McpServerHealth::Connected);
                    info!("Successfully connected to MCP server: {}", server_id);

                    // Discover tools from the server
                    self.discover_tools(server_id).await;
                }
                Err(e) => {
                    error!("Failed to initialize MCP server {}: {}", server_id, e);
                    self.registry.set_error(server_id, e);
                }
            }

            self.finish_connect_attempt(server_id).await;
        }
    }

    /// Disconnect from a specific MCP server
    pub async fn disconnect_server(&self, server_id: &str) {
        let mut connections = self.connections.write().await;
        if let Some(handle) = connections.remove(server_id) {
            handle.client.close().await;
            info!("Disconnected from MCP server: {}", server_id);
            self.registry
                .update_health(server_id, McpServerHealth::Configured);
        }
    }

    /// Check if a server is currently connected and healthy
    pub async fn is_connected(&self, server_id: &str) -> bool {
        let connections = self.connections.read().await;
        if let Some(handle) = connections.get(server_id) {
            handle.client.is_connected()
        } else {
            false
        }
    }

    /// Reconnect to a server (force disconnect and reconnect)
    pub async fn reconnect_server(&self, server_id: &str) {
        info!("Forcing reconnection of MCP server: {}", server_id);
        self.disconnect_server(server_id).await;

        // Reset attempt counter for manual reconnect
        {
            let mut attempts = self.reconnect_attempts.write().await;
            attempts.remove(server_id);
        }

        self.connect_server(server_id).await;
    }

    /// Discover tools from a connected MCP server
    async fn discover_tools(&self, server_id: &str) {
        let connections = self.connections.read().await;
        let handle = match connections.get(server_id) {
            Some(h) => h,
            None => {
                warn!("Cannot discover tools: server {} not connected", server_id);
                return;
            }
        };

        info!("Discovering tools for MCP server: {}", server_id);

        match handle.client.list_tools().await {
            Ok(tools) => {
                info!(
                    "Discovered {} tools from MCP server {}",
                    tools.len(),
                    server_id
                );
                self.registry.update_discovered_tools(server_id, tools);
            }
            Err(e) => {
                error!("Failed to discover tools from {}: {}", server_id, e);
                // Mark server as errored - health monitor will attempt reconnection
                self.registry
                    .set_error(server_id, format!("Tool discovery failed: {}", e));
            }
        }
    }

    /// Call a tool on an MCP server
    pub async fn call_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let connections = self.connections.read().await;
        let handle = connections.get(server_id);

        // If no connection handle exists, check registry for detailed status
        if handle.is_none() {
            drop(connections);
            return Err(self.build_unavailable_error(server_id, tool_name).await);
        }

        let handle = handle.unwrap();

        if !handle.client.is_connected() {
            drop(connections);
            return Err(format!(
                "MCP server '{}' connection was lost while calling tool '{}'. \
                 Automatic reconnection is in progress. Please try again shortly.",
                server_id, tool_name
            ));
        }

        info!(
            "Calling tool '{}' on MCP server '{}' with argument keys: {:?}",
            tool_name,
            server_id,
            summarize_argument_keys(&arguments)
        );

        match handle.client.call_tool(tool_name, arguments).await {
            Ok(result) => Ok(result),
            Err(e) => {
                error!(
                    "Tool '{}' call failed on MCP server '{}': {}",
                    tool_name, server_id, e
                );
                // Check if the error indicates a connection issue
                if is_connection_error(&e) {
                    // Mark server as errored to trigger reconnection
                    drop(connections); // Release read lock before updating
                    self.registry.set_error(
                        server_id,
                        format!("Connection lost during tool call to '{}': {}", tool_name, e),
                    );
                }
                Err(format!(
                    "Tool '{}' on MCP server '{}' failed: {}",
                    tool_name, server_id, e
                ))
            }
        }
    }

    /// Build a detailed error message when a server is unavailable
    async fn build_unavailable_error(&self, server_id: &str, tool_name: &str) -> String {
        match self.registry.get_server(server_id) {
            Some(server_state) => {
                match server_state.health {
                    McpServerHealth::Disabled => {
                        format!(
                            "MCP server '{}' is currently disabled. \
                             Tool '{}' cannot be executed. \
                             Please enable the server in your configuration and try again.",
                            server_id, tool_name
                        )
                    }
                    McpServerHealth::Error => {
                        let error_detail = server_state
                            .last_error
                            .as_ref()
                            .map(|e| format!(" Last error: {}", e))
                            .unwrap_or_default();
                        format!(
                            "MCP server '{}' is currently unavailable due to an error. \
                             Tool '{}' cannot be executed.{} \
                             The system is attempting to reconnect automatically. \
                             Please try again in a few moments.",
                            server_id, tool_name, error_detail
                        )
                    }
                    McpServerHealth::Connecting => {
                        format!(
                            "MCP server '{}' is currently starting up (in 'Connecting' state). \
                             Tool '{}' cannot be executed yet. \
                             Please wait a moment and try again.",
                            server_id, tool_name
                        )
                    }
                    McpServerHealth::Configured => {
                        format!(
                            "MCP server '{}' is currently unavailable. \
                             Tool '{}' cannot be executed. \
                             The server may be starting up, restarting, or experiencing connectivity issues. \
                             Please try again in a few moments.",
                            server_id, tool_name
                        )
                    }
                    McpServerHealth::Connected => {
                        // This shouldn't happen if handle is None, but handle gracefully
                        format!(
                            "MCP server '{}' appears to be connected but the connection handle is missing. \
                             Tool '{}' cannot be executed. \
                             This may indicate a synchronization issue. Please try again.",
                            server_id, tool_name
                        )
                    }
                }
            }
            None => {
                format!(
                    "MCP server '{}' is not configured. \
                     Tool '{}' cannot be executed. \
                     Please check your MCP server configuration.",
                    server_id, tool_name
                )
            }
        }
    }

    /// Shutdown all connections
    pub async fn shutdown(&self) {
        let server_ids: Vec<String> = {
            let connections = self.connections.read().await;
            connections.keys().cloned().collect()
        };

        for server_id in server_ids {
            self.disconnect_server(&server_id).await;
        }

        info!("All MCP connections shutdown");
    }

    /// Get the registry reference
    pub fn registry(&self) -> &McpServerRegistry {
        &self.registry
    }
}

fn summarize_argument_keys(arguments: &serde_json::Value) -> Vec<String> {
    arguments
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
}

/// Calculate exponential backoff delay
fn calculate_backoff(attempt: u32, base_ms: u64, max_ms: u64) -> u64 {
    let delay = base_ms * 2_u64.pow(attempt.min(10)); // Cap at 2^10 to avoid overflow
    delay.min(max_ms)
}

/// Check if an error indicates a connection issue that warrants reconnection
fn is_connection_error(error: &str) -> bool {
    let error_lower = error.to_lowercase();
    error_lower.contains("connection")
        || error_lower.contains("disconnected")
        || error_lower.contains("broken pipe")
        || error_lower.contains("reset")
        || error_lower.contains("timeout")
        || error_lower.contains("eof")
}
