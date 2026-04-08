//! Tests for iron-core configuration

use iron_core::config::{ApprovalStrategy, Config, ContextWindowPolicy};
use iron_providers::{GenerationConfig, ToolPolicy};

#[test]
fn test_config_defaults() {
    let config = Config::default();
    assert_eq!(config.max_iterations, 10);
    assert_eq!(config.default_approval_strategy, ApprovalStrategy::PerTool);
    assert_eq!(config.context_window_policy, ContextWindowPolicy::KeepAll);
    assert_eq!(config.model, "gpt-4o");
    assert_eq!(config.default_generation, GenerationConfig::default());
    assert_eq!(config.default_tool_policy, ToolPolicy::Auto);
}

#[test]
fn test_config_builder() {
    let config = Config::new()
        .with_max_iterations(5)
        .with_approval_strategy(ApprovalStrategy::Always)
        .with_model("gpt-3.5-turbo")
        .with_default_generation(GenerationConfig::new().with_temperature(0.7))
        .with_default_tool_policy(ToolPolicy::Required);

    assert_eq!(config.max_iterations, 5);
    assert_eq!(config.default_approval_strategy, ApprovalStrategy::Always);
    assert_eq!(config.model, "gpt-3.5-turbo");
    assert_eq!(config.default_generation.temperature, Some(0.7));
    assert_eq!(config.default_tool_policy, ToolPolicy::Required);
}

#[test]
fn test_config_default_generation_is_none() {
    let config = Config::default();
    assert!(config.default_generation.temperature.is_none());
    assert!(config.default_generation.max_tokens.is_none());
    assert!(config.default_generation.top_p.is_none());
    assert!(config.default_generation.stop.is_none());
}

#[test]
fn test_config_default_tool_policy_is_auto() {
    let config = Config::default();
    assert_eq!(config.default_tool_policy, ToolPolicy::Auto);
}

#[test]
fn test_approval_strategy_always() {
    let strategy = ApprovalStrategy::Always;
    assert!(strategy.is_approval_required(false));
    assert!(strategy.is_approval_required(true));
}

#[test]
fn test_approval_strategy_never() {
    let strategy = ApprovalStrategy::Never;
    assert!(!strategy.is_approval_required(false));
    assert!(!strategy.is_approval_required(true));
}

#[test]
fn test_approval_strategy_per_tool() {
    let strategy = ApprovalStrategy::PerTool;
    assert!(!strategy.is_approval_required(false));
    assert!(strategy.is_approval_required(true));
}

#[test]
fn test_context_window_keep_recent() {
    let policy = ContextWindowPolicy::KeepRecent(5);
    let mut messages: Vec<i32> = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    policy.apply(&mut messages, |_| 0);
    assert_eq!(messages, vec![6, 7, 8, 9, 10]);
}

#[test]
fn test_context_window_keep_all() {
    let policy = ContextWindowPolicy::KeepAll;
    let mut messages: Vec<i32> = vec![1, 2, 3, 4, 5];
    policy.apply(&mut messages, |_| 0);
    assert_eq!(messages, vec![1, 2, 3, 4, 5]);
}
