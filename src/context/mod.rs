//! Context management: accounting, compression, model switching, and handoff.
//!
//! This module implements context management:
//!
//! - **`active_context`**: the provider-visible footprint of the next request
//! - **`compressed_blocks`**: freeform compressed context summaries
//! - **`handoff_bundle`**: portable continuity payload for cross-session transfer
//!
//! ```
//! use iron_core::{Config, ContextManagementConfig, TailRetentionRule};
//!
//! let context = ContextManagementConfig::new()
//!     .enabled()
//!     .with_maintenance_threshold(32_000)
//!     .with_context_window_hint(128_000)
//!     .with_tail_retention(TailRetentionRule::Messages(16));
//! context.validate()?;
//!
//! let config = Config::default().with_context_management(context);
//! assert!(config.context_management.enabled);
//! # Ok::<(), String>(())
//! ```

pub mod accounting;
pub mod compaction;
pub mod config;
pub mod handoff;
pub mod model_switch;
pub mod models;
pub mod telemetry;
pub mod token_tracker;

pub use accounting::{
    ActiveContextAccountant, ActiveContextSnapshot, ContextCategory, ContextCategoryUsage,
    ContextPressure, ContextQuality, SessionModelInfo,
};
pub use compaction::{CompressRange, CompressResult, CompressTool};
pub use config::{
    ContextManagementConfig, HandoffExportConfig, TailRetentionPolicy, TailRetentionRule,
};
pub use handoff::{HandoffBundle, HandoffBundleMetadata, HandoffExporter, HandoffImporter};
pub use model_switch::{
    CapabilityDiff, ContextAdaptationPlan, ModelCapabilityMetadata, ModelCapabilityRegistry,
    ModelSwitchPlan, ModelSwitchRecord, ModelSwitchRequest, PendingModelSwitch,
};
pub use models::{CompressedBlock, HANDOFF_DEFAULT_TARGET_TOKENS};
pub use telemetry::ContextTelemetry;
pub use token_tracker::{SessionTokenTracker, TokenUsageTotals};
