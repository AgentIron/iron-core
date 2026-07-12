use super::*;
use crate::execution::{AutomationRunErrorCategory, AutomationRunStatus};
use std::time::Duration;

// ============================================================================
// Argument parsing
// ============================================================================

#[test]
fn parse_minimal_valid() {
    let args = vec!["run".to_string(), "my-task".to_string()];
    let parsed = parse_args(&args).unwrap();
    assert_eq!(parsed.task_id, "my-task");
    assert_eq!(parsed.config, None);
    assert_eq!(parsed.workspace, None);
    assert_eq!(parsed.timeout, None);
    assert_eq!(parsed.format, None);
    assert!(!parsed.quiet);
}

#[test]
fn parse_all_options() {
    let args = vec![
        "run".to_string(),
        "daily-report".to_string(),
        "--config".to_string(),
        "/etc/iron.db".to_string(),
        "--workspace".to_string(),
        "/tmp/work".to_string(),
        "--timeout".to_string(),
        "30s".to_string(),
        "--format".to_string(),
        "json".to_string(),
        "--quiet".to_string(),
    ];
    let parsed = parse_args(&args).unwrap();
    assert_eq!(parsed.task_id, "daily-report");
    assert_eq!(parsed.config.as_deref(), Some("/etc/iron.db"));
    assert_eq!(parsed.workspace.as_deref(), Some("/tmp/work"));
    assert_eq!(parsed.timeout.as_deref(), Some("30s"));
    assert_eq!(parsed.format.as_deref(), Some("json"));
    assert!(parsed.quiet);
}

#[test]
fn parse_options_before_task_id() {
    let args = vec![
        "run".to_string(),
        "--timeout".to_string(),
        "5m".to_string(),
        "report".to_string(),
    ];
    let parsed = parse_args(&args).unwrap();
    assert_eq!(parsed.task_id, "report");
    assert_eq!(parsed.timeout.as_deref(), Some("5m"));
}

#[test]
fn parse_missing_subcommand() {
    let args: Vec<String> = vec![];
    let err = parse_args(&args).unwrap_err();
    assert!(err.message.contains("missing 'run'"));
}

#[test]
fn parse_unknown_subcommand() {
    let args = vec!["create".to_string(), "task".to_string()];
    let err = parse_args(&args).unwrap_err();
    assert!(err.message.contains("unknown subcommand"));
}

#[test]
fn parse_missing_task_id() {
    let args = vec!["run".to_string()];
    let err = parse_args(&args).unwrap_err();
    assert!(err.message.contains("missing required <task-id>"));
}

#[test]
fn parse_unknown_option() {
    let args = vec!["run".to_string(), "task".to_string(), "--bogus".to_string()];
    let err = parse_args(&args).unwrap_err();
    assert!(err.message.contains("unknown option"));
}

#[test]
fn parse_option_without_value() {
    let args = vec![
        "run".to_string(),
        "task".to_string(),
        "--timeout".to_string(),
    ];
    let err = parse_args(&args).unwrap_err();
    assert!(err.message.contains("--timeout requires a value"));
}

#[test]
fn parse_extra_positional() {
    let args = vec!["run".to_string(), "task1".to_string(), "task2".to_string()];
    let err = parse_args(&args).unwrap_err();
    assert!(err.message.contains("unexpected positional"));
}

#[test]
fn parse_help_flag() {
    let args = vec!["--help".to_string()];
    let err = parse_args(&args).unwrap_err();
    assert!(err.message.contains("Usage:"));
}

#[test]
fn parse_help_shorthand() {
    let args = vec!["run".to_string(), "task".to_string(), "-h".to_string()];
    let err = parse_args(&args).unwrap_err();
    assert!(err.message.contains("Usage:"));
}

#[test]
fn parse_short_flag_aliases() {
    let args = vec![
        "run".to_string(),
        "daily-report".to_string(),
        "-c".to_string(),
        "/etc/iron.db".to_string(),
        "-o".to_string(),
        "json".to_string(),
        "-q".to_string(),
    ];
    let parsed = parse_args(&args).unwrap();
    assert_eq!(parsed.task_id, "daily-report");
    assert_eq!(parsed.config.as_deref(), Some("/etc/iron.db"));
    assert_eq!(parsed.format.as_deref(), Some("json"));
    assert!(parsed.quiet);
}

#[test]
fn parse_short_config_without_value_errors() {
    let args = vec!["run".to_string(), "task".to_string(), "-c".to_string()];
    let err = parse_args(&args).unwrap_err();
    assert!(err.message.contains("--config requires a value"));
}

// ============================================================================
// Raw JSON detection (pre-parse usage-error contract)
// ============================================================================

#[test]
fn raw_json_detects_long_flag() {
    let args = vec![
        "run".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    assert!(raw_args_request_json(&args));
}

#[test]
fn raw_json_detects_short_flag() {
    let args = vec!["run".to_string(), "-o".to_string(), "json".to_string()];
    assert!(raw_args_request_json(&args));
}

#[test]
fn raw_json_detects_equals_form() {
    let args = vec!["--format=json".to_string()];
    assert!(raw_args_request_json(&args));
}

#[test]
fn raw_json_case_insensitive() {
    let args = vec!["--format".to_string(), "JSON".to_string()];
    assert!(raw_args_request_json(&args));
}

#[test]
fn raw_json_false_for_text_format() {
    let args = vec![
        "run".to_string(),
        "--format".to_string(),
        "text".to_string(),
    ];
    assert!(!raw_args_request_json(&args));
}

#[test]
fn raw_json_false_when_absent() {
    let args = vec!["run".to_string(), "task".to_string()];
    assert!(!raw_args_request_json(&args));
}

// ============================================================================
// Duration parsing
// ============================================================================

#[test]
fn duration_seconds() {
    assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
    assert_eq!(parse_duration("1s").unwrap(), Duration::from_secs(1));
    assert_eq!(parse_duration("600s").unwrap(), Duration::from_secs(600));
}

#[test]
fn duration_minutes() {
    assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
    assert_eq!(parse_duration("1m").unwrap(), Duration::from_secs(60));
}

#[test]
fn duration_hours() {
    assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
}

#[test]
fn duration_bare_seconds() {
    assert_eq!(parse_duration("45").unwrap(), Duration::from_secs(45));
}

#[test]
fn duration_rejects_zero() {
    assert!(parse_duration("0s").is_err());
    assert!(parse_duration("0").is_err());
    assert!(parse_duration("0m").is_err());
}

#[test]
fn duration_rejects_empty() {
    assert!(parse_duration("").is_err());
    assert!(parse_duration("   ").is_err());
}

#[test]
fn duration_rejects_invalid_unit() {
    assert!(parse_duration("30x").is_err());
    assert!(parse_duration("30d").is_err());
}

#[test]
fn duration_rejects_non_numeric() {
    assert!(parse_duration("abcs").is_err());
    assert!(parse_duration("--5s").is_err());
}

#[test]
fn duration_trims_whitespace() {
    assert_eq!(parse_duration("  30s  ").unwrap(), Duration::from_secs(30));
}

// ============================================================================
// Workspace resolution
// ============================================================================

#[test]
fn resolve_workspace_uses_cli() {
    let env = vec![(
        "AGENTIRON_WORKSPACE".to_string(),
        "/nonexistent-env".to_string(),
    )];
    let env_ref: Vec<(String, String)> = env;
    let w = resolve_workspace(
        Some("."),
        env_get(&env_ref, "AGENTIRON_WORKSPACE"),
        None,
    );
    assert!(w.is_ok());
    assert!(w.unwrap().is_absolute());
}

#[test]
fn resolve_workspace_uses_env() {
    let env_val = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let env = vec![("AGENTIRON_WORKSPACE".to_string(), env_val.clone())];
    let w = resolve_workspace(None, env_get(&env, "AGENTIRON_WORKSPACE"), None);
    assert!(w.is_ok());
    assert_eq!(w.unwrap(), PathBuf::from(env_val).canonicalize().unwrap());
}

#[test]
fn resolve_workspace_uses_fallback() {
    let fallback = std::env::current_dir().unwrap();
    let w = resolve_workspace(None, None, Some(&fallback)).unwrap();
    assert_eq!(w, fallback.canonicalize().unwrap());
}

#[test]
fn resolve_workspace_missing_without_fallback() {
    let err = resolve_workspace(None, None, None).unwrap_err();
    assert!(err.contains("required"));
}

#[test]
fn resolve_workspace_rejects_nonexistent() {
    let err =
        resolve_workspace(Some("/nonexistent/path/xyz"), None, None).unwrap_err();
    assert!(err.contains("does not exist"));
}

#[test]
fn resolve_workspace_rejects_file() {
    let err = resolve_workspace(Some("/etc/hostname"), None, None);
    match err {
        Ok(path) => {
            if path.exists() && !path.is_dir() {
                panic!("expected error for file as workspace");
            }
        }
        Err(e) => {
            assert!(e.contains("not a directory") || e.contains("does not exist"));
        }
    }
}

// ============================================================================
// Timeout resolution
// ============================================================================

#[test]
fn resolve_timeout_from_cli() {
    let t = resolve_timeout(Some("5m"), Some("1h"), None).unwrap();
    assert_eq!(t, Duration::from_secs(300));
}

#[test]
fn resolve_timeout_from_env() {
    let t = resolve_timeout(None, Some("1h"), None).unwrap();
    assert_eq!(t, Duration::from_secs(3600));
}

#[test]
fn resolve_timeout_from_fallback() {
    let t = resolve_timeout(None, None, Some(Duration::from_secs(120))).unwrap();
    assert_eq!(t, Duration::from_secs(120));
}

#[test]
fn resolve_timeout_missing() {
    let err = resolve_timeout(None, None, None).unwrap_err();
    assert!(err.contains("required"));
}

#[test]
fn resolve_timeout_invalid_value() {
    let err = resolve_timeout(Some("0s"), None, None).unwrap_err();
    assert!(err.contains("greater than zero"));
}

// ============================================================================
// Format resolution
// ============================================================================

#[test]
fn resolve_format_default_text() {
    assert_eq!(resolve_format(None, None).unwrap(), OutputFormat::Text);
}

#[test]
fn resolve_format_from_cli() {
    assert_eq!(
        resolve_format(Some("json"), Some("text")).unwrap(),
        OutputFormat::Json
    );
}

#[test]
fn resolve_format_from_env() {
    assert_eq!(
        resolve_format(None, Some("json")).unwrap(),
        OutputFormat::Json
    );
}

#[test]
fn resolve_format_case_insensitive() {
    assert_eq!(
        resolve_format(Some("JSON"), None).unwrap(),
        OutputFormat::Json
    );
    assert_eq!(
        resolve_format(Some("Text"), None).unwrap(),
        OutputFormat::Text
    );
}

#[test]
fn resolve_format_invalid() {
    assert!(resolve_format(Some("xml"), None).is_err());
}

// ============================================================================
// Quiet resolution
// ============================================================================

#[test]
fn resolve_quiet_from_flag() {
    assert!(resolve_quiet(true, Some("false")));
}

#[test]
fn resolve_quiet_from_env() {
    assert!(resolve_quiet(false, Some("true")));
    assert!(resolve_quiet(false, Some("1")));
    assert!(resolve_quiet(false, Some("yes")));
}

#[test]
fn resolve_quiet_default_false() {
    assert!(!resolve_quiet(false, None));
    assert!(!resolve_quiet(false, Some("no")));
    assert!(!resolve_quiet(false, Some("0")));
}

// ============================================================================
// Exit-code mapping
// ============================================================================

#[test]
fn exit_code_completed() {
    assert_eq!(exit_code_for_status(AutomationRunStatus::Completed), 0);
}

#[test]
fn exit_code_failed() {
    assert_eq!(exit_code_for_status(AutomationRunStatus::Failed), 6);
}

#[test]
fn exit_code_cancelled() {
    assert_eq!(exit_code_for_status(AutomationRunStatus::Cancelled), 7);
}

#[test]
fn exit_code_timed_out() {
    assert_eq!(exit_code_for_status(AutomationRunStatus::TimedOut), 8);
}

#[test]
fn exit_code_for_result_failed_unsafe_policy() {
    let now = chrono::Utc::now();
    let mut result = AutomationRunResult::started(
        &crate::automation_task::AutomationTask {
            id: "t".to_string(),
            display_name: "T".to_string(),
            normalized_name: "t".to_string(),
            stored_prompt_id: "p".to_string(),
            expected_outcome: "o".to_string(),
            project_root: std::path::PathBuf::from("/tmp"),
            timeout_seconds: 60,
            created_at: now,
            updated_at: now,
        },
        std::path::PathBuf::from("/w"),
    );
    result.fail(
        AutomationRunErrorCategory::UnsafePolicy,
        "tool not available".to_string(),
    );
    assert_eq!(exit_code_for_result(&result), EXIT_UNSAFE_POLICY);
}

#[test]
fn exit_code_for_result_failed_execution() {
    let now = chrono::Utc::now();
    let mut result = AutomationRunResult::started(
        &crate::automation_task::AutomationTask {
            id: "t".to_string(),
            display_name: "T".to_string(),
            normalized_name: "t".to_string(),
            stored_prompt_id: "p".to_string(),
            expected_outcome: "o".to_string(),
            project_root: std::path::PathBuf::from("/tmp"),
            timeout_seconds: 60,
            created_at: now,
            updated_at: now,
        },
        std::path::PathBuf::from("/w"),
    );
    result.fail(
        AutomationRunErrorCategory::Execution,
        "provider error".to_string(),
    );
    assert_eq!(exit_code_for_result(&result), EXIT_EXECUTION);
}

#[test]
fn exit_code_for_result_completed() {
    let now = chrono::Utc::now();
    let mut result = AutomationRunResult::started(
        &crate::automation_task::AutomationTask {
            id: "t".to_string(),
            display_name: "T".to_string(),
            normalized_name: "t".to_string(),
            stored_prompt_id: "p".to_string(),
            expected_outcome: "o".to_string(),
            project_root: std::path::PathBuf::from("/tmp"),
            timeout_seconds: 60,
            created_at: now,
            updated_at: now,
        },
        std::path::PathBuf::from("/w"),
    );
    result.complete("done".to_string());
    assert_eq!(exit_code_for_result(&result), EXIT_COMPLETED);
}

#[test]
fn exit_code_for_result_failed_config() {
    let now = chrono::Utc::now();
    let mut result = AutomationRunResult::started(
        &crate::automation_task::AutomationTask {
            id: "t".to_string(),
            display_name: "T".to_string(),
            normalized_name: "t".to_string(),
            stored_prompt_id: "p".to_string(),
            expected_outcome: "o".to_string(),
            project_root: std::path::PathBuf::from("/tmp"),
            timeout_seconds: 60,
            created_at: now,
            updated_at: now,
        },
        std::path::PathBuf::from("/w"),
    );
    result.fail(
        AutomationRunErrorCategory::Config,
        "config error".to_string(),
    );
    assert_eq!(exit_code_for_result(&result), EXIT_CONFIG);
}

#[test]
fn exit_code_for_missing_default() {
    assert_eq!(
        exit_code_for_bootstrap_error(&HeadlessBootstrapError::MissingDefaultProvider),
        EXIT_PROVIDER_INIT
    );
}

#[test]
fn exit_code_for_provider_init() {
    assert_eq!(
        exit_code_for_bootstrap_error(&HeadlessBootstrapError::ProviderInit {
            provider: "openai".to_string(),
            reason: "timeout".to_string()
        }),
        EXIT_PROVIDER_INIT
    );
}

#[test]
fn exit_code_for_unsafe_policy() {
    assert_eq!(
        exit_code_for_bootstrap_error(&HeadlessBootstrapError::UnsafePolicy("PerTool".to_string())),
        EXIT_UNSAFE_POLICY
    );
}

#[test]
fn exit_code_for_unavailable_tool() {
    assert_eq!(
        exit_code_for_bootstrap_error(&HeadlessBootstrapError::UnavailableTool(
            "plugin_x".to_string()
        )),
        EXIT_UNSAFE_POLICY
    );
}

#[test]
fn exit_code_for_config_error() {
    assert_eq!(
        exit_code_for_bootstrap_error(&HeadlessBootstrapError::Config(
            crate::config::ConfigError::Path("bad".to_string())
        )),
        EXIT_CONFIG
    );
}

#[test]
fn exit_code_for_resolution_error() {
    assert_eq!(
        exit_code_for_bootstrap_error(&HeadlessBootstrapError::Resolution(
            crate::execution::ResolutionError::TaskNotFound("x".to_string())
        )),
        EXIT_CONFIG
    );
}

// ============================================================================
// Output formatting
// ============================================================================

#[test]
fn text_output_returns_final_text() {
    let result = AutomationRunResult::cli_failure(
        "task1",
        PathBuf::from("/tmp"),
        AutomationRunErrorCategory::Execution,
        "test error".to_string(),
    );
    // cli_failure has empty output
    assert_eq!(format_text_output(&result), "");
}

#[test]
fn json_output_is_valid_json_with_schema_version() {
    let result = AutomationRunResult::cli_failure(
        "task1",
        PathBuf::from("/tmp"),
        AutomationRunErrorCategory::Config,
        "config error".to_string(),
    );
    let json = format_json_output(&result);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["status"], "failed");
    assert_eq!(parsed["task_id"], "task1");
    assert_eq!(parsed["error"]["category"], "config");
    assert_eq!(parsed["error"]["message"], "config error");
}

// ============================================================================
// Config path resolution
// ============================================================================

#[test]
fn resolve_config_from_cli() {
    let p = resolve_config_path(Some("/custom/path.db"), Some("/env/path.db")).unwrap();
    assert_eq!(p, PathBuf::from("/custom/path.db"));
}

#[test]
fn resolve_config_from_env() {
    let p = resolve_config_path(None, Some("/env/path.db")).unwrap();
    assert_eq!(p, PathBuf::from("/env/path.db"));
}

// ============================================================================
// Env helper
// ============================================================================

#[test]
fn env_get_finds_key() {
    let env = vec![
        ("FOO".to_string(), "bar".to_string()),
        ("BAZ".to_string(), "qux".to_string()),
    ];
    assert_eq!(env_get(&env, "FOO"), Some("bar"));
    assert_eq!(env_get(&env, "BAZ"), Some("qux"));
}

#[test]
fn env_get_returns_none_for_missing() {
    let env = vec![("FOO".to_string(), "bar".to_string())];
    assert_eq!(env_get(&env, "MISSING"), None);
}
