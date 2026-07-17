//! MCP support for runtime-managed servers and the canonical session tool catalog.
//!
//! New code should use [`SessionToolCatalog`] for session-effective inspection,
//! provider request construction, and execution.
//!
//! ```
//! use iron_core::{HttpConfig, McpServerConfig, McpServerRegistry, McpTransport};
//!
//! let registry = McpServerRegistry::new();
//! registry.register_server(McpServerConfig {
//!     id: "docs".into(),
//!     label: "Documentation".into(),
//!     description: Some("Internal documentation search".into()),
//!     transport: McpTransport::Http {
//!         config: HttpConfig::new("https://mcp.example.com".into()),
//!     },
//!     enabled_by_default: true,
//!     working_dir: None,
//!     inherited_env_vars: vec![],
//! });
//! assert_eq!(registry.list_servers().len(), 1);
//! ```

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
