//! Context management: compaction, telemetry, and handoff
//!
//! This module implements the three-concept context model described in the
//! context-management design:
//!
//! - **`active_context`**: the provider-visible footprint of the next request
//! - **`compacted_context`**: canonical structured semantic summary
//! - **`handoff_bundle`**: portable continuity payload for cross-session transfer

pub mod accounting;
pub mod compaction;
pub mod config;
pub mod handoff;
pub mod models;
pub mod telemetry;

pub use accounting::{
    ActiveContextAccountant, ActiveContextSnapshot, ContextCategory, ContextCategoryUsage,
    ContextQuality,
};
pub use compaction::{CompactionCheckpoint, CompactionEngine, CompactionInput, CompactionReason};
pub use config::{
    ContextManagementConfig, HandoffExportConfig, TailRetentionPolicy, TailRetentionRule,
};
pub use handoff::{HandoffBundle, HandoffBundleMetadata, HandoffExporter, HandoffImporter};
pub use models::{
    CompactedContext, Decision, PortabilityNote, UnresolvedQuestion, HANDOFF_DEFAULT_TARGET_TOKENS,
};
pub use telemetry::ContextTelemetry;
