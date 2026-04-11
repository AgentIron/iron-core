pub mod client;
pub mod connection;
pub mod effective_tools;
pub mod protocol;
pub mod server;
pub mod session_catalog;

pub use client::{create_transport_client, McpTransportClient};
pub use connection::{McpConnectionHandle, McpConnectionManager, ReconnectConfig};
#[allow(deprecated)]
pub use effective_tools::{EffectiveToolView, McpTool, ServerStatus, SessionMcpSummary};
pub use server::{
    McpServerConfig, McpServerHealth, McpServerRegistry, McpServerState, McpToolInfo, McpTransport,
};
pub use session_catalog::{McpServerSummary, SessionToolCatalog, ToolDiagnostic, ToolSource};
