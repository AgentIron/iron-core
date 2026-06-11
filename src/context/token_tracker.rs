use crate::context::ContextQuality;
use iron_providers::TokenUsage;

/// Accumulated provider-reported token usage totals.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenUsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub reasoning_output_tokens: u64,
}

/// Session-owned token tracker that uses provider-reported usage as a
/// baseline and accumulates locally estimated delta for additions after
/// that baseline.
///
/// Provider `input_tokens` represent the full prompt sent in the request
/// that produced the usage event.  Any assistant output, tool calls, tool
/// results, or user messages added after the usage event are counted as
/// delta for the next request.
///
/// The baseline is invalidated when provider-visible context is rewritten
/// (e.g. compaction) so that estimates fall back to full heuristic until
/// the next usage-bearing response resynchronises the tracker.
#[derive(Debug, Clone, Default)]
pub struct SessionTokenTracker {
    baseline_input_tokens: Option<usize>,
    delta_tokens_after_baseline: usize,
    accumulated_input_tokens: u64,
    accumulated_output_tokens: u64,
    accumulated_cached_input_tokens: u64,
    accumulated_cache_creation_input_tokens: u64,
    accumulated_cache_read_input_tokens: u64,
    accumulated_reasoning_output_tokens: u64,
}

impl SessionTokenTracker {
    /// Record provider-reported usage.
    ///
    /// When `input_tokens` is present it becomes the new authoritative
    /// baseline and the local delta is reset.  Output and cache fields are
    /// added to accumulated totals for telemetry / cost accounting.
    pub fn record_provider_usage(&mut self, usage: &TokenUsage) {
        if let Some(input) = usage.input_tokens {
            self.baseline_input_tokens = Some(input as usize);
            self.delta_tokens_after_baseline = 0;
            self.accumulated_input_tokens += input;
        }
        if let Some(output) = usage.output_tokens {
            self.accumulated_output_tokens += output;
        }
        if let Some(cached) = usage.cached_input_tokens {
            self.accumulated_cached_input_tokens += cached;
        }
        if let Some(creation) = usage.cache_creation_input_tokens {
            self.accumulated_cache_creation_input_tokens += creation;
        }
        if let Some(read) = usage.cache_read_input_tokens {
            self.accumulated_cache_read_input_tokens += read;
        }
        if let Some(reasoning) = usage.reasoning_output_tokens {
            self.accumulated_reasoning_output_tokens += reasoning;
        }
    }

    /// Add a locally estimated token delta.
    ///
    /// Called when the session appends provider-visible content (user
    /// messages, assistant text, tool calls, tool results).
    pub fn add_delta(&mut self, tokens: usize) {
        self.delta_tokens_after_baseline += tokens;
    }

    /// Clear the provider-reported baseline and local delta.
    ///
    /// Called after compaction or other provider-visible rewrites where
    /// the prior baseline no longer describes the current context.
    pub fn invalidate_baseline(&mut self) {
        self.baseline_input_tokens = None;
        self.delta_tokens_after_baseline = 0;
    }

    /// Return the best available context estimate.
    ///
    /// `Some(baseline + delta)` when a provider-reported baseline exists,
    /// `None` when the caller should fall back to full heuristic estimation.
    pub fn estimate_current_context(&self) -> Option<usize> {
        self.baseline_input_tokens
            .map(|baseline| baseline.saturating_add(self.delta_tokens_after_baseline))
    }

    /// Whether a provider-reported baseline is currently available.
    pub fn has_baseline(&self) -> bool {
        self.baseline_input_tokens.is_some()
    }

    /// Current accounting quality.
    ///
    /// Returns `Exact` only when a baseline exists and no local delta has
    /// been accumulated since that baseline.  In practice this is usually
    /// `Estimated` because assistant output and tool results are added as
    /// delta immediately after the usage event.
    pub fn quality(&self) -> ContextQuality {
        if self.baseline_input_tokens.is_some() && self.delta_tokens_after_baseline == 0 {
            ContextQuality::Exact
        } else {
            ContextQuality::Estimated
        }
    }

    // ── accumulated usage telemetry ──

    pub fn accumulated_input_tokens(&self) -> u64 {
        self.accumulated_input_tokens
    }

    pub fn accumulated_output_tokens(&self) -> u64 {
        self.accumulated_output_tokens
    }

    pub fn accumulated_cached_input_tokens(&self) -> u64 {
        self.accumulated_cached_input_tokens
    }

    pub fn accumulated_cache_creation_input_tokens(&self) -> u64 {
        self.accumulated_cache_creation_input_tokens
    }

    pub fn accumulated_cache_read_input_tokens(&self) -> u64 {
        self.accumulated_cache_read_input_tokens
    }

    pub fn accumulated_reasoning_output_tokens(&self) -> u64 {
        self.accumulated_reasoning_output_tokens
    }

    pub fn accumulated_totals(&self) -> TokenUsageTotals {
        TokenUsageTotals {
            input_tokens: self.accumulated_input_tokens,
            output_tokens: self.accumulated_output_tokens,
            cached_input_tokens: self.accumulated_cached_input_tokens,
            cache_creation_input_tokens: self.accumulated_cache_creation_input_tokens,
            cache_read_input_tokens: self.accumulated_cache_read_input_tokens,
            reasoning_output_tokens: self.accumulated_reasoning_output_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_without_baseline_returns_none() {
        let tracker = SessionTokenTracker::default();
        assert!(!tracker.has_baseline());
        assert_eq!(tracker.estimate_current_context(), None);
        assert_eq!(tracker.quality(), ContextQuality::Estimated);
    }

    #[test]
    fn tracker_records_baseline_and_accumulates_totals() {
        let mut tracker = SessionTokenTracker::default();
        let usage = TokenUsage {
            input_tokens: Some(100),
            output_tokens: Some(50),
            total_tokens: None,
            cached_input_tokens: Some(10),
            cache_creation_input_tokens: Some(5),
            cache_read_input_tokens: Some(15),
            reasoning_output_tokens: Some(20),
        };
        tracker.record_provider_usage(&usage);

        assert!(tracker.has_baseline());
        assert_eq!(tracker.estimate_current_context(), Some(100));
        assert_eq!(tracker.quality(), ContextQuality::Exact);
        assert_eq!(tracker.accumulated_input_tokens(), 100);
        assert_eq!(tracker.accumulated_output_tokens(), 50);
        assert_eq!(tracker.accumulated_cached_input_tokens(), 10);
        assert_eq!(tracker.accumulated_cache_creation_input_tokens(), 5);
        assert_eq!(tracker.accumulated_cache_read_input_tokens(), 15);
        assert_eq!(tracker.accumulated_reasoning_output_tokens(), 20);
    }

    #[test]
    fn tracker_adds_delta_after_baseline() {
        let mut tracker = SessionTokenTracker::default();
        tracker.record_provider_usage(&TokenUsage {
            input_tokens: Some(100),
            ..TokenUsage::default()
        });
        tracker.add_delta(25);

        assert_eq!(tracker.estimate_current_context(), Some(125));
        assert_eq!(tracker.quality(), ContextQuality::Estimated);
    }

    #[test]
    fn tracker_resets_on_new_usage() {
        let mut tracker = SessionTokenTracker::default();
        tracker.record_provider_usage(&TokenUsage {
            input_tokens: Some(100),
            ..TokenUsage::default()
        });
        tracker.add_delta(25);
        tracker.record_provider_usage(&TokenUsage {
            input_tokens: Some(200),
            ..TokenUsage::default()
        });

        assert_eq!(tracker.estimate_current_context(), Some(200));
        assert_eq!(tracker.quality(), ContextQuality::Exact);
    }

    #[test]
    fn tracker_invalidates_baseline() {
        let mut tracker = SessionTokenTracker::default();
        tracker.record_provider_usage(&TokenUsage {
            input_tokens: Some(100),
            ..TokenUsage::default()
        });
        tracker.add_delta(25);
        tracker.invalidate_baseline();

        assert!(!tracker.has_baseline());
        assert_eq!(tracker.estimate_current_context(), None);
        assert_eq!(tracker.quality(), ContextQuality::Estimated);
    }

    #[test]
    fn tracker_does_not_establish_baseline_without_input_tokens() {
        let mut tracker = SessionTokenTracker::default();
        tracker.record_provider_usage(&TokenUsage {
            input_tokens: None,
            output_tokens: Some(50),
            ..TokenUsage::default()
        });

        assert!(!tracker.has_baseline());
        assert_eq!(tracker.estimate_current_context(), None);
        assert_eq!(tracker.accumulated_output_tokens(), 50);
    }
}
