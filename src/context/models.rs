use serde::{Deserialize, Serialize};

pub const HANDOFF_DEFAULT_TARGET_TOKENS: usize = 15_000;

/// A compressed block stores a freeform summary produced by model-driven
/// compression via the `compress` tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompressedBlock {
    /// Stable block identifier, e.g. `c0003`.
    pub id: String,
    /// Topic or heading for the block.
    pub topic: String,
    /// Source range this block replaces, e.g. `m0010-m0040`.
    pub source_range: String,
    /// Durable summary produced by the model.
    pub summary: String,
    /// When the block was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Estimated token count before compression (optional telemetry).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_estimate_before: Option<u32>,
    /// Estimated token count after compression (optional telemetry).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_estimate_after: Option<u32>,
}

impl CompressedBlock {
    pub fn new(
        id: impl Into<String>,
        topic: impl Into<String>,
        source_range: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            topic: topic.into(),
            source_range: source_range.into(),
            summary: summary.into(),
            created_at: chrono::Utc::now(),
            token_estimate_before: None,
            token_estimate_after: None,
        }
    }

    pub fn render_to_text(&self) -> String {
        format!(
            "[Compressed context: {}]\nID: {}\nSource range: {}\n{}\n",
            self.topic, self.id, self.source_range, self.summary
        )
    }
}
