//! WASM integration-plugin surface.
//!
//! This module contains the runtime-facing data model for plugin inventory,
//! manifests, auth state, status reporting, session enablement, and lifecycle
//! management.
//!
//! ## Install lifecycle
//!
//! The lifecycle manager (`plugin::lifecycle::PluginLifecycle`) handles the
//! full install pipeline: artifact fetch → cache → manifest extraction →
//! validation → WASM host load.  The pipeline is parameterised over a
//! `PluginLoader` trait so the lifecycle state machine can be tested
//! independently of Extism.

pub mod auth;
pub mod config;
pub mod effective_tools;
pub mod lifecycle;
pub mod manifest;
pub mod network;
pub mod registry;
pub mod rich_output;
pub mod session;
pub mod status;
pub mod wasm_host;

pub use auth::{AuthAvailability, AuthState, CredentialBinding, OAuthProvider};
pub use config::{Checksum, ChecksumAlgorithm, PluginConfig, PluginSource};
pub use effective_tools::{
    compute_tool_availability, EffectivePluginToolView, PluginTool, ToolAvailabilityResult,
    UnavailableReason,
};
pub use lifecycle::{InstallResult, NullPluginLoader, PluginLoader};
pub use manifest::{ExportedTool, PluginIdentity, PluginManifest, PluginPublisher};
pub use network::NetworkPolicy;
pub use registry::{
    InstallMetadata, PluginAvailabilitySummary, PluginId, PluginRegistry, PluginState,
};
pub use status::{
    PerToolAvailability, PluginHealth, PluginInfo, PluginRuntimeStatus, PluginStatus,
};
