//! Experimental WASM integration-plugin surface.
//!
//! This module contains the runtime-facing data model for plugin inventory,
//! manifests, auth state, status reporting, session enablement, and lifecycle
//! scaffolding.
//!
//! The current implementation is intentionally incomplete: remote artifact
//! fetching, manifest extraction from WASM binaries, OAuth exchange/refresh,
//! and WASM tool execution are not implemented yet.

pub mod auth;
pub mod config;
pub mod effective_tools;
pub mod lifecycle;
pub mod manifest;
pub mod network;
pub mod registry;
pub mod session;
pub mod status;
pub mod wasm_host;

pub use auth::{AuthAvailability, AuthState, CredentialBinding, OAuthProvider};
pub use config::{PluginConfig, PluginSource, Checksum, ChecksumAlgorithm};
pub use manifest::{PluginManifest, PluginIdentity, PluginPublisher, ExportedTool};
pub use network::NetworkPolicy;
pub use registry::{PluginRegistry, PluginState, PluginId};
pub use status::{PluginStatus, PluginRuntimeStatus, PerToolAvailability, PluginHealth};
pub use effective_tools::{EffectivePluginToolView, PluginTool};
