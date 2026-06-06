//! Context management: compression, telemetry, and handoff
//!
//! This module implements context management:
//!
//! - **`active_context`**: the provider-visible footprint of the next request
//! - **`compressed_blocks`**: freeform compressed context summaries
//! - **`handoff_bundle`**: portable continuity payload for cross-session transfer

pub mod accounting;
pub mod compaction;
pub mod config;
pub mod handoff;
pub mod model_switch;
pub mod models;
pub mod telemetry;

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
