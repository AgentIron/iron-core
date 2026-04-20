//! MCP support for runtime-managed servers and the canonical session tool catalog.
//!
//! New code should use [`SessionToolCatalog`] for session-effective inspection,
//! provider request construction, and execution.

pub mod client;
pub mod connection;
pub mod protocol;
pub mod server;
pub mod session_catalog;

pub use client::{create_transport_client, McpTransportClient};
pub use connection::{McpConnectionHandle, McpConnectionManager, ReconnectConfig};
pub use server::{
    HttpConfig, McpServerConfig, McpServerHealth, McpServerRegistry, McpServerState, McpToolInfo,
    McpTransport,
};
pub use session_catalog::{McpServerSummary, SessionToolCatalog, ToolDiagnostic, ToolSource};
