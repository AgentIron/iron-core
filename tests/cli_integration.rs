//! Integration tests for the `agent-iron` CLI binary.
//!
//! These tests exercise [`iron_core::cli::execute_run_with_streams`] — the
//! core CLI implementation that accepts generic writers — covering:
//!
//! - Command-level usage failures and env/CLI precedence (task 6.2)
//! - JSON output contract: exactly one stdout object, required fields,
//!   structured failure results (task 6.3)
//! - End-to-end bootstrap and preflight paths: saved default provider/model,
//!   AutoApprove preflight enforcement, plugin exclusion, task resolution
//!   (task 6.4)

use base64::Engine;
use iron_core::automation_task::AutomationTaskInput;
use iron_core::cli;
use iron_core::config::crypto::XChaCha20Poly1305Cipher;
use iron_core::config::{ConfigStore, DefaultModelInput, OpenOptions, ProfileInput, PromptInput};
use iron_core::profile::{
    AgentApproval, AgentProfile, AgentProfileId, AgentProfileProvider, SkillFilter, ToolFilter,
    PROFILE_SCHEMA_VERSION,
};
use iron_core::provider_credential::{
    DurableCredentialStore, ProviderCredentialStore, ProviderSlug, StoredCredential,
};
use iron_core::stored_prompt::{StoredPrompt, STORED_PROMPT_SCHEMA_VERSION};
use std::path::Path;
use std::sync::Arc;

// ============================================================================
// Constants
// ============================================================================

const EXIT_COMPLETED: i32 = 0;
const EXIT_USAGE: i32 = 2;
const EXIT_CONFIG: i32 = 3;
const EXIT_UNSAFE_POLICY: i32 = 4;
const EXIT_PROVIDER_INIT: i32 = 5;

/// Deterministic test encryption key (32 bytes of 0x42).
fn test_key() -> [u8; 32] {
    [0x42u8; 32]
}

/// Base64-encode the test key for the env var.
fn test_key_b64() -> String {
    base64::engine::general_purpose::STANDARD.encode(test_key())
}

// ============================================================================
// Store fixture helpers
// ============================================================================

/// Serializes tests that mutate the process-global encryption-key env var so
/// that parallel `cargo test` execution cannot race on `AGENTIRON_CONFIG_ENCRYPTION_KEY`.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Set the process-level encryption key so `ConfigStore::open_at` can decrypt
/// credentials stored by the same key.
///
/// Returns a guard that must be held for the duration of any code path that
/// reads the env var. This serializes the env-dependent tests under parallel
/// `cargo test` execution to avoid races on the process-global env var.
async fn set_encryption_key() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = ENV_LOCK.lock().await;
    std::env::set_var("AGENTIRON_CONFIG_ENCRYPTION_KEY", test_key_b64());
    guard
}

/// Build a cipher matching the env-var key.
fn test_cipher() -> Arc<dyn iron_core::config::crypto::CredentialCipher> {
    Arc::new(XChaCha20Poly1305Cipher::new(&test_key()))
}

/// Open a file-based ConfigStore at `path` using the test cipher.
async fn open_store(path: impl AsRef<Path>) -> ConfigStore {
    let options = OpenOptions {
        cipher: Some(test_cipher()),
        busy_timeout: None,
    };
    ConfigStore::open_at_with_options(path, options)
        .await
        .expect("failed to open test store")
}

/// Seed a store with credential + default model + AutoApprove profile + prompt + task.
async fn seed_complete_store(store: &ConfigStore) {
    seed_credential_and_model(store, "openai", "gpt-4o", "sk-test-key").await;
    seed_auto_approve_profile(store).await;
    seed_prompt_and_task(store, "automation").await;
}

/// Store a provider API-key credential and set the default model.
async fn seed_credential_and_model(
    store: &ConfigStore,
    provider: &str,
    model: &str,
    api_key: &str,
) {
    let durable = Arc::new(DurableCredentialStore::new(store.clone()));
    ProviderCredentialStore::set(
        &*durable,
        &ProviderSlug::new(provider),
        StoredCredential::ApiKey(api_key.to_string()),
    )
    .await;

    store
        .set_default_model(&DefaultModelInput {
            provider_slug: provider.to_string(),
            model_id: model.to_string(),
        })
        .await
        .unwrap();
}

/// Create an AutoApprove profile named "automation".
async fn seed_auto_approve_profile(store: &ConfigStore) {
    let profile = AgentProfile {
        name: "automation".to_string(),
        provider: AgentProfileProvider::RuntimeDefault,
        tools: ToolFilter::Inherit,
        skills: SkillFilter::Inherit,
        approval: AgentApproval::AutoApprove,
        identity_prompt: None,
    };
    store
        .set_profile(&ProfileInput {
            id: "automation".to_string(),
            schema_version: PROFILE_SCHEMA_VERSION,
            payload: serde_json::to_value(&profile).unwrap(),
        })
        .await
        .unwrap();
}

/// Create a PerTool profile named "automation".
async fn seed_pertool_profile(store: &ConfigStore) {
    let profile = AgentProfile {
        name: "automation".to_string(),
        provider: AgentProfileProvider::RuntimeDefault,
        tools: ToolFilter::Inherit,
        skills: SkillFilter::Inherit,
        approval: AgentApproval::PerTool,
        identity_prompt: None,
    };
    store
        .set_profile(&ProfileInput {
            id: "automation".to_string(),
            schema_version: PROFILE_SCHEMA_VERSION,
            payload: serde_json::to_value(&profile).unwrap(),
        })
        .await
        .unwrap();
}

/// Create a stored prompt (referencing the given profile) and an automation task.
async fn seed_prompt_and_task(store: &ConfigStore, profile_id: &str) {
    let prompt = StoredPrompt {
        instructions: "Generate a daily report".to_string(),
        skills: Vec::new(),
        profile: Some(AgentProfileId::from(profile_id)),
    };
    store
        .set_prompt(&PromptInput {
            id: "report-prompt".to_string(),
            schema_version: STORED_PROMPT_SCHEMA_VERSION,
            payload: serde_json::to_value(&prompt).unwrap(),
        })
        .await
        .unwrap();

    store
        .set_automation_task(&AutomationTaskInput {
            id: "daily-report".to_string(),
            name: "Daily Report".to_string(),
            stored_prompt_id: "report-prompt".to_string(),
            expected_outcome: "A summary of today's activity".to_string(),
        })
        .await
        .unwrap();
}

/// Run the CLI with the given args, env, and return (exit_code, stdout, stderr).
async fn run_cli(args: &[String], env: &mut [(String, String)]) -> (i32, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = cli::execute_run_with_streams(args, env, &mut stdout, &mut stderr).await;
    let stdout_str = String::from_utf8_lossy(&stdout).to_string();
    let stderr_str = String::from_utf8_lossy(&stderr).to_string();
    (code, stdout_str, stderr_str)
}

/// Build args for `run <task>` with workspace, config, and timeout.
/// NOTE: args do NOT include the program name (the binary does skip(1)).
fn run_args(task_id: &str, workspace: &str, config: &str, timeout: &str) -> Vec<String> {
    vec![
        "run".to_string(),
        task_id.to_string(),
        "--workspace".to_string(),
        workspace.to_string(),
        "--config".to_string(),
        config.to_string(),
        "--timeout".to_string(),
        timeout.to_string(),
    ]
}

// ============================================================================
// 6.2 Command-level tests
// ============================================================================

#[tokio::test]
async fn usage_no_args() {
    let env = vec![];
    let (code, _out, err) = run_cli(&[], &mut env.clone()).await;
    assert_eq!(code, EXIT_USAGE);
    assert!(!err.is_empty());
}

#[tokio::test]
async fn usage_no_subcommand() {
    let env = vec![];
    let args = vec!["bogus".to_string()];
    let (code, _out, err) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_USAGE);
    assert!(!err.is_empty());
}

#[tokio::test]
async fn usage_missing_task_id() {
    let env = vec![];
    let args = vec!["run".to_string()];
    let (code, _out, err) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_USAGE);
    assert!(!err.is_empty());
}

#[tokio::test]
async fn usage_missing_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let store = open_store(&db_path).await;
    seed_complete_store(&store).await;

    let args = vec![
        "run".to_string(),
        "daily-report".to_string(),
        "--workspace".to_string(),
        dir.path().to_str().unwrap().to_string(),
        "--config".to_string(),
        db_path.to_str().unwrap().to_string(),
    ];
    let env = vec![];
    let (code, _out, _err) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_USAGE);
}

#[tokio::test]
async fn usage_invalid_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let args = vec![
        "run".to_string(),
        "task".to_string(),
        "--workspace".to_string(),
        dir.path().to_str().unwrap().to_string(),
        "--timeout".to_string(),
        "notaduration".to_string(),
    ];
    let env = vec![];
    let (code, _out, _err) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_USAGE);
}

#[tokio::test]
async fn config_invalid_workspace() {
    let args = vec![
        "run".to_string(),
        "task".to_string(),
        "--workspace".to_string(),
        "/nonexistent/path/that/should/not/exist".to_string(),
        "--timeout".to_string(),
        "30s".to_string(),
    ];
    let env = vec![];
    let (code, _out, _err) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_CONFIG);
}

#[tokio::test]
async fn precedence_timeout_cli_over_env() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let _store = open_store(&db_path).await;

    let args = vec![
        "run".to_string(),
        "task".to_string(),
        "--workspace".to_string(),
        dir.path().to_str().unwrap().to_string(),
        "--config".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--timeout".to_string(),
        "1m".to_string(),
    ];
    let mut env = vec![("AGENTIRON_TIMEOUT".to_string(), "30s".to_string())];
    let (code, _out, _err) = run_cli(&args, &mut env).await;
    assert_eq!(code, EXIT_PROVIDER_INIT);
}

#[tokio::test]
async fn precedence_workspace_cli_over_env() {
    let dir = tempfile::tempdir().unwrap();
    let env_workspace = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let _store = open_store(&db_path).await;

    let args = vec![
        "run".to_string(),
        "task".to_string(),
        "--workspace".to_string(),
        dir.path().to_str().unwrap().to_string(),
        "--config".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--timeout".to_string(),
        "30s".to_string(),
    ];
    let mut env = vec![(
        "AGENTIRON_WORKSPACE".to_string(),
        env_workspace.path().to_str().unwrap().to_string(),
    )];
    let (code, _out, _stderr) = run_cli(&args, &mut env).await;
    assert_eq!(code, EXIT_PROVIDER_INIT);
}

#[tokio::test]
async fn precedence_format_cli_over_env() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let _store = open_store(&db_path).await;

    let args = vec![
        "run".to_string(),
        "task".to_string(),
        "--workspace".to_string(),
        dir.path().to_str().unwrap().to_string(),
        "--config".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--timeout".to_string(),
        "30s".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    let mut env = vec![("AGENTIRON_FORMAT".to_string(), "text".to_string())];
    let (code, stdout, _stderr) = run_cli(&args, &mut env).await;
    assert_eq!(code, EXIT_PROVIDER_INIT);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(v["status"], "failed");
}

#[tokio::test]
async fn quiet_suppresses_progress() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let _store = open_store(&db_path).await;

    let args = vec![
        "run".to_string(),
        "task".to_string(),
        "--workspace".to_string(),
        dir.path().to_str().unwrap().to_string(),
        "--config".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--timeout".to_string(),
        "30s".to_string(),
        "--quiet".to_string(),
    ];
    let env = vec![];
    let (_code, _out, stderr) = run_cli(&args, &mut env.clone()).await;
    assert!(!stderr.contains("workspace:"));
    assert!(!stderr.contains("timeout:"));
}

// ============================================================================
// 6.3 JSON contract tests
// ============================================================================

#[tokio::test]
async fn json_failure_has_one_stdout_object() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let _store = open_store(&db_path).await;

    let args = vec![
        "run".to_string(),
        "task".to_string(),
        "--workspace".to_string(),
        dir.path().to_str().unwrap().to_string(),
        "--config".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--timeout".to_string(),
        "30s".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    let env = vec![];
    let (code, stdout, _stderr) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_PROVIDER_INIT);

    let trimmed = stdout.trim();
    assert!(
        trimmed.starts_with('{') && trimmed.ends_with('}'),
        "stdout should be a single JSON object"
    );
    let _v: serde_json::Value = serde_json::from_str(trimmed).expect("stdout should be valid JSON");
}

#[tokio::test]
async fn json_failure_has_required_fields() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let _store = open_store(&db_path).await;

    let args = vec![
        "run".to_string(),
        "task".to_string(),
        "--workspace".to_string(),
        dir.path().to_str().unwrap().to_string(),
        "--config".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--timeout".to_string(),
        "30s".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    let env = vec![];
    let (code, stdout, _stderr) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_PROVIDER_INIT);

    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();

    assert!(v.get("schema_version").is_some(), "missing schema_version");
    assert!(v.get("run_id").is_some(), "missing run_id");
    assert!(v.get("task_id").is_some(), "missing task_id");
    assert!(v.get("task_name").is_some(), "missing task_name");
    assert!(v.get("status").is_some(), "missing status");
    assert!(v.get("output").is_some(), "missing output");
    assert!(
        v.get("expected_outcome").is_some(),
        "missing expected_outcome"
    );
    assert!(v.get("profile_id").is_some(), "missing profile_id");
    assert!(v.get("workspace").is_some(), "missing workspace");
    assert!(v.get("started_at").is_some(), "missing started_at");
    assert!(v.get("ended_at").is_some(), "missing ended_at");
    assert!(v.get("duration_ms").is_some(), "missing duration_ms");

    assert!(v.get("error").is_some(), "missing error");
    assert!(
        v["error"].get("category").is_some(),
        "missing error.category"
    );
    assert!(v["error"].get("message").is_some(), "missing error.message");

    assert_eq!(v["status"], "failed");
    assert_eq!(v["error"]["category"], "provider_init");
}

#[tokio::test]
async fn json_failure_for_usage_error() {
    let dir = tempfile::tempdir().unwrap();

    let args = vec![
        "run".to_string(),
        "task".to_string(),
        "--workspace".to_string(),
        dir.path().to_str().unwrap().to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    let env = vec![];
    let (code, stdout, _stderr) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_USAGE);

    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v["status"], "failed");
    assert_eq!(v["error"]["category"], "config");
}

#[tokio::test]
async fn json_failure_for_pre_parse_usage_error() {
    // Missing task-id with --format json: parse_args fails before format
    // resolution, but the JSON output contract must still be honored.
    let args = vec![
        "run".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    let env = vec![];
    let (code, stdout, stderr) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_USAGE);

    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .expect("stdout should be a JSON object even for pre-parse usage errors");
    assert_eq!(v["status"], "failed");
    assert_eq!(v["error"]["category"], "config");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing required <task-id>"),
        "message should describe the usage error: {:?}",
        v["error"]["message"]
    );
    assert!(stderr.is_empty(), "stderr should be empty in JSON mode");
}

#[tokio::test]
async fn json_failure_for_pre_parse_usage_error_via_env() {
    // AGENTIRON_FORMAT=json in env should also trigger JSON for usage errors.
    let args = vec!["run".to_string()];
    let env = vec![("AGENTIRON_FORMAT".to_string(), "json".to_string())];
    let (code, stdout, stderr) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_USAGE);

    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be JSON via env override");
    assert_eq!(v["status"], "failed");
    assert!(stderr.is_empty(), "stderr should be empty in JSON mode");
}

#[tokio::test]
async fn json_failure_for_config_error() {
    let args = vec![
        "run".to_string(),
        "task".to_string(),
        "--workspace".to_string(),
        "/nonexistent/path".to_string(),
        "--timeout".to_string(),
        "30s".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    let env = vec![];
    let (code, stdout, _stderr) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_CONFIG);

    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v["status"], "failed");
    assert_eq!(v["error"]["category"], "config");
}

#[tokio::test]
async fn json_failure_for_unsafe_policy() {
    let _env_guard = set_encryption_key().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let store = open_store(&db_path).await;
    seed_credential_and_model(&store, "openai", "gpt-4o", "sk-test").await;
    seed_pertool_profile(&store).await;
    seed_prompt_and_task(&store, "automation").await;

    let args = vec![
        "run".to_string(),
        "daily-report".to_string(),
        "--workspace".to_string(),
        dir.path().to_str().unwrap().to_string(),
        "--config".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--timeout".to_string(),
        "30s".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    let env = vec![];
    let (code, stdout, _stderr) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_UNSAFE_POLICY);

    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v["status"], "failed");
    assert_eq!(v["error"]["category"], "unsafe_policy");
}

#[tokio::test]
async fn text_failure_for_unsafe_policy() {
    let _env_guard = set_encryption_key().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let store = open_store(&db_path).await;
    seed_credential_and_model(&store, "openai", "gpt-4o", "sk-test").await;
    seed_pertool_profile(&store).await;
    seed_prompt_and_task(&store, "automation").await;

    let args = run_args(
        "daily-report",
        dir.path().to_str().unwrap(),
        db_path.to_str().unwrap(),
        "30s",
    );
    let env = vec![];
    let (code, _stdout, stderr) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_UNSAFE_POLICY);
    assert!(!stderr.is_empty());
}

// ============================================================================
// 6.4 End-to-end bootstrap and preflight tests
// ============================================================================

#[tokio::test]
async fn e2e_missing_default_provider_exit5() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let _store = open_store(&db_path).await;

    let args = run_args(
        "daily-report",
        dir.path().to_str().unwrap(),
        db_path.to_str().unwrap(),
        "30s",
    );
    let env = vec![];
    let (code, _out, _err) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_PROVIDER_INIT);
}

#[tokio::test]
async fn e2e_missing_credential_exit5() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let store = open_store(&db_path).await;
    store
        .set_default_model(&DefaultModelInput {
            provider_slug: "openai".to_string(),
            model_id: "gpt-4o".to_string(),
        })
        .await
        .unwrap();
    seed_auto_approve_profile(&store).await;
    seed_prompt_and_task(&store, "automation").await;

    let args = run_args(
        "daily-report",
        dir.path().to_str().unwrap(),
        db_path.to_str().unwrap(),
        "30s",
    );
    let env = vec![];
    let (code, _out, _err) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_PROVIDER_INIT);
}

#[tokio::test]
async fn e2e_task_not_found_exit3() {
    let _env_guard = set_encryption_key().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let store = open_store(&db_path).await;
    seed_complete_store(&store).await;

    let args = run_args(
        "nonexistent-task",
        dir.path().to_str().unwrap(),
        db_path.to_str().unwrap(),
        "30s",
    );
    let env = vec![];
    let (code, _out, _err) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_CONFIG);
}

#[tokio::test]
async fn e2e_auto_approve_preflight_blocks_pertool() {
    let _env_guard = set_encryption_key().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let store = open_store(&db_path).await;
    seed_credential_and_model(&store, "openai", "gpt-4o", "sk-test").await;
    seed_pertool_profile(&store).await;
    seed_prompt_and_task(&store, "automation").await;

    let args = run_args(
        "daily-report",
        dir.path().to_str().unwrap(),
        db_path.to_str().unwrap(),
        "30s",
    );
    let env = vec![];
    let (code, _out, _err) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_UNSAFE_POLICY);
}

#[tokio::test]
async fn e2e_plugin_tool_excluded_from_allow_list() {
    let _env_guard = set_encryption_key().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let store = open_store(&db_path).await;
    seed_credential_and_model(&store, "openai", "gpt-4o", "sk-test").await;

    let profile = AgentProfile {
        name: "automation".to_string(),
        provider: AgentProfileProvider::RuntimeDefault,
        tools: ToolFilter::Allow(vec!["plugin_my_plugin_some_tool".to_string()]),
        skills: SkillFilter::Inherit,
        approval: AgentApproval::AutoApprove,
        identity_prompt: None,
    };
    store
        .set_profile(&ProfileInput {
            id: "automation".to_string(),
            schema_version: PROFILE_SCHEMA_VERSION,
            payload: serde_json::to_value(&profile).unwrap(),
        })
        .await
        .unwrap();
    seed_prompt_and_task(&store, "automation").await;

    let args = run_args(
        "daily-report",
        dir.path().to_str().unwrap(),
        db_path.to_str().unwrap(),
        "30s",
    );
    let env = vec![];
    let (code, _out, _err) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_UNSAFE_POLICY);
}

#[tokio::test]
async fn e2e_uses_saved_default_provider_and_model() {
    let _env_guard = set_encryption_key().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let store = open_store(&db_path).await;

    seed_credential_and_model(&store, "openai", "gpt-4o-mini", "sk-test-key").await;
    seed_auto_approve_profile(&store).await;
    seed_prompt_and_task(&store, "automation").await;

    let args = vec![
        "run".to_string(),
        "daily-report".to_string(),
        "--workspace".to_string(),
        dir.path().to_str().unwrap().to_string(),
        "--config".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--timeout".to_string(),
        "5s".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    let env = vec![];
    let (code, stdout, _stderr) = run_cli(&args, &mut env.clone()).await;

    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v["status"], "completed");
    assert_eq!(v["provider"], "openai");
    assert_eq!(v["model"], "gpt-4o-mini");
    assert_eq!(code, EXIT_COMPLETED);
}

#[tokio::test]
async fn e2e_json_success_contract() {
    let _env_guard = set_encryption_key().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let store = open_store(&db_path).await;
    seed_credential_and_model(&store, "openai", "gpt-4o", "sk-test").await;
    seed_auto_approve_profile(&store).await;
    seed_prompt_and_task(&store, "automation").await;

    let args = vec![
        "run".to_string(),
        "daily-report".to_string(),
        "--workspace".to_string(),
        dir.path().to_str().unwrap().to_string(),
        "--config".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--timeout".to_string(),
        "5s".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    let env = vec![];
    let (code, stdout, stderr) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_COMPLETED);

    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be valid JSON");

    assert_eq!(v["schema_version"], 1);
    assert!(v["run_id"].is_string());
    assert_eq!(v["task_id"], "daily-report");
    assert_eq!(v["task_name"], "Daily Report");
    assert_eq!(v["status"], "completed");
    assert!(v["output"].is_string());
    assert_eq!(v["expected_outcome"], "A summary of today's activity");
    assert!(v["profile_id"].is_string());
    assert_eq!(v["provider"], "openai");
    assert_eq!(v["model"], "gpt-4o");
    assert!(v["workspace"].is_string());
    assert!(v["effective_tools"].is_array());
    assert!(v["started_at"].is_string());
    assert!(v["ended_at"].is_string());
    assert!(v["duration_ms"].is_number());
    assert!(v["error"].is_null());

    let _single: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("exactly one JSON object");
    let _ = stderr;
}

#[tokio::test]
async fn e2e_text_mode_output_is_final_text_only() {
    let _env_guard = set_encryption_key().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("config.db");
    let store = open_store(&db_path).await;
    seed_credential_and_model(&store, "openai", "gpt-4o", "sk-test").await;
    seed_auto_approve_profile(&store).await;
    seed_prompt_and_task(&store, "automation").await;

    let args = vec![
        "run".to_string(),
        "daily-report".to_string(),
        "--workspace".to_string(),
        dir.path().to_str().unwrap().to_string(),
        "--config".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--timeout".to_string(),
        "5s".to_string(),
        "--quiet".to_string(),
    ];
    let env = vec![];
    let (code, stdout, stderr) = run_cli(&args, &mut env.clone()).await;
    assert_eq!(code, EXIT_COMPLETED);

    assert!(stderr.is_empty(), "stderr should be empty in quiet mode");
    assert!(
        !stdout.trim().starts_with('{'),
        "text mode should not output JSON"
    );
}
