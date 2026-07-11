//! Command-line interface for the `agent-iron` headless binary.
//!
//! The CLI is run-only: `agent-iron run <task-id>` executes an existing
//! automation task non-interactively. All task/prompt/profile management is
//! handled by the GUI and typed core APIs.
//!
//! ## Precedence
//!
//! Command-line values take precedence over `AGENTIRON_*` environment
//! variables, which take precedence over documented defaults. The task ID is
//! always a positional argument and is never sourced from an environment
//! variable.
//!
//! ## Exit codes
//!
//! | Code | Meaning                       |
//! |------|-------------------------------|
//! | 0    | completed                     |
//! | 2    | usage error                   |
//! | 3    | configuration or reference    |
//! | 4    | unsafe policy                 |
//! | 5    | provider/credential init      |
//! | 6    | execution failure             |
//! | 7    | cancelled                     |
//! | 8    | timed out                     |

use crate::config::{default_config_path, ConfigStore};
use crate::execution::{AutomationRunErrorCategory, AutomationRunResult, AutomationRunStatus};
use crate::headless::{bootstrap_headless, run_automation, HeadlessBootstrapError};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

// ============================================================================
// Exit codes
// ============================================================================

pub const EXIT_COMPLETED: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_CONFIG: i32 = 3;
pub const EXIT_UNSAFE_POLICY: i32 = 4;
pub const EXIT_PROVIDER_INIT: i32 = 5;
pub const EXIT_EXECUTION: i32 = 6;
pub const EXIT_CANCELLED: i32 = 7;
pub const EXIT_TIMED_OUT: i32 = 8;

// ============================================================================
// Output format
// ============================================================================

/// Output format selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
}

// ============================================================================
// Parsed arguments
// ============================================================================

/// Parsed CLI arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliArgs {
    pub task_id: String,
    pub config: Option<String>,
    pub workspace: Option<String>,
    pub timeout: Option<String>,
    pub format: Option<String>,
    pub quiet: bool,
}

/// Usage error from argument parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError {
    pub message: String,
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for UsageError {}

const USAGE: &str = "\
Usage: agent-iron run <task-id> [OPTIONS]

Options:
  --config <path>        Path to ConfigStore database
  --workspace <dir>      Workspace directory (default: process cwd)
  --timeout <duration>   Execution timeout (e.g. 30s, 5m, 1h) [required]
  --format <text|json>   Output format (default: text)
  --quiet                Suppress progress output on stderr
  -h, --help             Show this help message";

/// Parse command-line arguments (everything after the program name).
pub fn parse_args(args: &[String]) -> Result<CliArgs, UsageError> {
    if args.is_empty() {
        return Err(UsageError {
            message: format!("missing 'run' subcommand\n\n{}", USAGE),
        });
    }

    let subcommand = args[0].as_str();
    if subcommand == "-h" || subcommand == "--help" {
        return Err(UsageError {
            message: USAGE.to_string(),
        });
    }
    if subcommand != "run" {
        return Err(UsageError {
            message: format!(
                "unknown subcommand '{}': only 'run' is supported\n\n{}",
                subcommand, USAGE
            ),
        });
    }

    let mut task_id: Option<String> = None;
    let mut config: Option<String> = None;
    let mut workspace: Option<String> = None;
    let mut timeout: Option<String> = None;
    let mut format: Option<String> = None;
    let mut quiet = false;

    let rest = &args[1..];
    let mut i = 0;
    while i < rest.len() {
        let arg = &rest[i];
        match arg.as_str() {
            "--config" => {
                i += 1;
                config = Some(expect_value(rest, i, "--config")?);
            }
            "--workspace" => {
                i += 1;
                workspace = Some(expect_value(rest, i, "--workspace")?);
            }
            "--timeout" => {
                i += 1;
                timeout = Some(expect_value(rest, i, "--timeout")?);
            }
            "--format" => {
                i += 1;
                format = Some(expect_value(rest, i, "--format")?);
            }
            "--quiet" => {
                quiet = true;
            }
            "-h" | "--help" => {
                return Err(UsageError {
                    message: USAGE.to_string(),
                });
            }
            s if s.starts_with('-') => {
                return Err(UsageError {
                    message: format!("unknown option '{}'\n\n{}", s, USAGE),
                });
            }
            s => {
                if task_id.is_none() {
                    task_id = Some(s.to_string());
                } else {
                    return Err(UsageError {
                        message: format!("unexpected positional argument '{}'\n\n{}", s, USAGE),
                    });
                }
            }
        }
        i += 1;
    }

    let task_id = task_id.ok_or_else(|| UsageError {
        message: format!(
            "missing required <task-id> positional argument\n\n{}",
            USAGE
        ),
    })?;

    Ok(CliArgs {
        task_id,
        config,
        workspace,
        timeout,
        format,
        quiet,
    })
}

fn expect_value(args: &[String], idx: usize, flag: &str) -> Result<String, UsageError> {
    args.get(idx).cloned().ok_or_else(|| UsageError {
        message: format!("{} requires a value\n\n{}", flag, USAGE),
    })
}

// ============================================================================
// Duration parsing
// ============================================================================

/// Parse a positive duration string like `30s`, `5m`, `1h`.
///
/// Returns an error for zero, negative, or malformed values.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("timeout must not be empty".to_string());
    }

    let last = trimmed.chars().last().unwrap();
    let (num_str, multiplier) = match last {
        's' => (&trimmed[..trimmed.len() - 1], 1u64),
        'm' => (&trimmed[..trimmed.len() - 1], 60),
        'h' => (&trimmed[..trimmed.len() - 1], 3600),
        c if c.is_ascii_digit() => (trimmed, 1u64),
        _ => {
            return Err(format!(
                "invalid timeout unit '{}': use 30s, 5m, or 1h",
                last
            ))
        }
    };

    let seconds: u64 = num_str.parse().map_err(|_| {
        format!(
            "invalid timeout value '{}': expected a positive number",
            num_str
        )
    })?;

    if seconds == 0 {
        return Err("timeout must be greater than zero".to_string());
    }

    let total = seconds
        .checked_mul(multiplier)
        .ok_or_else(|| "timeout value is too large".to_string())?;

    Ok(Duration::from_secs(total))
}

// ============================================================================
// Resolution functions (CLI > env > defaults)
// ============================================================================

/// Resolve workspace from CLI, environment, or process cwd.
///
/// Canonicalizes the path and requires it to be an existing directory.
pub fn resolve_workspace(cli: Option<&str>, env: Option<&str>) -> Result<PathBuf, String> {
    let raw = cli.or(env).unwrap_or(".");

    let path = PathBuf::from(raw);
    if !path.exists() {
        return Err(format!("workspace does not exist: {}", raw));
    }
    if !path.is_dir() {
        return Err(format!("workspace is not a directory: {}", raw));
    }

    path.canonicalize()
        .map_err(|e| format!("failed to canonicalize workspace '{}': {}", raw, e))
}

/// Resolve the ConfigStore database path from CLI, environment, or default.
pub fn resolve_config_path(cli: Option<&str>, env: Option<&str>) -> Result<PathBuf, String> {
    if let Some(p) = cli {
        return Ok(PathBuf::from(p));
    }
    if let Some(p) = env {
        return Ok(PathBuf::from(p));
    }
    default_config_path().map_err(|e| format!("failed to determine default config path: {}", e))
}

/// Resolve timeout duration from CLI or environment (required).
pub fn resolve_timeout(cli: Option<&str>, env: Option<&str>) -> Result<Duration, String> {
    let raw = cli.or(env).ok_or_else(|| {
        "timeout is required: use --timeout or AGENTIRON_TIMEOUT (e.g. 30s, 5m, 1h)".to_string()
    })?;
    parse_duration(raw)
}

/// Resolve output format from CLI or environment (default: text).
pub fn resolve_format(cli: Option<&str>, env: Option<&str>) -> Result<OutputFormat, String> {
    let raw = cli.or(env).unwrap_or("text");
    match raw.trim().to_lowercase().as_str() {
        "text" => Ok(OutputFormat::Text),
        "json" => Ok(OutputFormat::Json),
        other => Err(format!(
            "invalid format '{}': expected 'text' or 'json'",
            other
        )),
    }
}

/// Resolve quiet flag from CLI flag or environment.
pub fn resolve_quiet(quiet_flag: bool, env: Option<&str>) -> bool {
    if quiet_flag {
        return true;
    }
    matches!(
        env.map(|s| s.trim().to_lowercase()),
        Some(ref s) if s == "1" || s == "true" || s == "yes"
    )
}

// ============================================================================
// Exit-code mapping
// ============================================================================

/// Map a terminal run status to the stable exit code.
pub fn exit_code_for_status(status: AutomationRunStatus) -> i32 {
    match status {
        AutomationRunStatus::Completed => EXIT_COMPLETED,
        AutomationRunStatus::Failed => EXIT_EXECUTION,
        AutomationRunStatus::Cancelled => EXIT_CANCELLED,
        AutomationRunStatus::TimedOut => EXIT_TIMED_OUT,
    }
}

/// Map a run result to the stable exit code, considering the error category
/// for failed runs.
pub fn exit_code_for_result(result: &AutomationRunResult) -> i32 {
    match result.status {
        AutomationRunStatus::Completed => EXIT_COMPLETED,
        AutomationRunStatus::Cancelled => EXIT_CANCELLED,
        AutomationRunStatus::TimedOut => EXIT_TIMED_OUT,
        AutomationRunStatus::Failed => match result.error.as_ref().map(|e| &e.category) {
            Some(AutomationRunErrorCategory::Config)
            | Some(AutomationRunErrorCategory::Reference) => EXIT_CONFIG,
            Some(AutomationRunErrorCategory::UnsafePolicy) => EXIT_UNSAFE_POLICY,
            Some(AutomationRunErrorCategory::ProviderInit) => EXIT_PROVIDER_INIT,
            _ => EXIT_EXECUTION,
        },
    }
}

/// Map a bootstrap error to the stable exit code.
pub fn exit_code_for_bootstrap_error(err: &HeadlessBootstrapError) -> i32 {
    match err {
        HeadlessBootstrapError::MissingDefaultProvider
        | HeadlessBootstrapError::ProviderInit { .. }
        | HeadlessBootstrapError::CredentialFailure { .. }
        | HeadlessBootstrapError::InteractiveAuthRequired { .. } => EXIT_PROVIDER_INIT,
        HeadlessBootstrapError::UnsafePolicy(_) | HeadlessBootstrapError::UnavailableTool(_) => {
            EXIT_UNSAFE_POLICY
        }
        HeadlessBootstrapError::Config(_) | HeadlessBootstrapError::Resolution(_) => EXIT_CONFIG,
    }
}

// ============================================================================
// Bootstrap-error to result mapping (for JSON failure output)
// =============================================================================

/// Convert a bootstrap error into an `AutomationRunErrorCategory`.
fn bootstrap_error_category(err: &HeadlessBootstrapError) -> AutomationRunErrorCategory {
    match err {
        HeadlessBootstrapError::MissingDefaultProvider
        | HeadlessBootstrapError::ProviderInit { .. }
        | HeadlessBootstrapError::CredentialFailure { .. }
        | HeadlessBootstrapError::InteractiveAuthRequired { .. } => {
            AutomationRunErrorCategory::ProviderInit
        }
        HeadlessBootstrapError::UnsafePolicy(_) | HeadlessBootstrapError::UnavailableTool(_) => {
            AutomationRunErrorCategory::UnsafePolicy
        }
        HeadlessBootstrapError::Config(_) | HeadlessBootstrapError::Resolution(_) => {
            AutomationRunErrorCategory::Config
        }
    }
}

// ============================================================================
// Output formatting
// ============================================================================

/// Format a run result for text-mode stdout (final assistant text only).
pub fn format_text_output(result: &AutomationRunResult) -> String {
    result.output.clone()
}

/// Format a run result as a single versioned JSON object.
pub fn format_json_output(result: &AutomationRunResult) -> String {
    serde_json::to_string_pretty(result).unwrap_or_else(|e| {
        format!(
            "{{\"schema_version\":1,\"status\":\"failed\",\"error\":{{\"category\":\"execution\",\"message\":\"failed to serialize result: {}\"}}}}",
            e
        )
    })
}

// ============================================================================
// Signal handling
// ============================================================================

/// Wait for an interrupt (SIGINT) or termination (SIGTERM) signal.
async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let sigterm = signal(SignalKind::terminate());
        let mut sigterm = match sigterm {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

// ============================================================================
// Main execution entry point
// ============================================================================

/// Execute a headless automation run end-to-end.
///
/// Returns the process exit code. This function is intended to be called
/// from the binary's `main` on a `current_thread` Tokio runtime inside a
/// `LocalSet`.
pub async fn execute_run(args: &[String]) -> i32 {
    let env: Vec<(String, String)> = std::env::vars().collect();
    execute_run_with_streams(
        args,
        &mut env.clone(),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
    .await
}

/// Execute a headless run using the provided environment variables.
///
/// Separated from [`execute_run`] for testability.
pub async fn execute_run_with_env(args: &[String], env: &mut [(String, String)]) -> i32 {
    execute_run_with_streams(args, env, &mut std::io::stdout(), &mut std::io::stderr()).await
}

/// Execute a headless run writing output to the provided writers.
///
/// This is the core implementation. Tests pass `Vec<u8>` buffers to capture
/// stdout and stderr.
pub async fn execute_run_with_streams(
    args: &[String],
    env: &mut [(String, String)],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> i32 {
    // 1. Parse arguments.
    let parsed = match parse_args(args) {
        Ok(p) => p,
        Err(e) => {
            let _ = writeln!(stderr, "{}", e.message);
            return EXIT_USAGE;
        }
    };

    // 2. Resolve output format early to know if JSON mode is active.
    let format = match resolve_format(parsed.format.as_deref(), env_get(env, "AGENTIRON_FORMAT")) {
        Ok(f) => f,
        Err(e) => {
            let _ = writeln!(stderr, "{}", e);
            return EXIT_USAGE;
        }
    };

    let quiet = resolve_quiet(parsed.quiet, env_get(env, "AGENTIRON_QUIET"));

    // Helper: emit a failure and return an exit code.
    // In JSON mode, constructs a terminal JSON object on stdout.
    // In text mode, writes the error to stderr.
    macro_rules! emit_failure {
        ($category:expr, $message:expr, $exit_code:expr, $workspace:expr) => {{
            match format {
                OutputFormat::Json => {
                    let result = AutomationRunResult::cli_failure(
                        &parsed.task_id,
                        $workspace,
                        $category,
                        $message,
                    );
                    let _ = writeln!(stdout, "{}", format_json_output(&result));
                }
                OutputFormat::Text => {
                    let _ = writeln!(stderr, "{}", &$message);
                }
            }
            return $exit_code;
        }};
    }

    // 3. Resolve workspace.
    let workspace = match resolve_workspace(
        parsed.workspace.as_deref(),
        env_get(env, "AGENTIRON_WORKSPACE"),
    ) {
        Ok(w) => w,
        Err(e) => {
            emit_failure!(
                AutomationRunErrorCategory::Config,
                e,
                EXIT_CONFIG,
                PathBuf::from(parsed.workspace.as_deref().unwrap_or("."))
            );
        }
    };

    if !quiet {
        let _ = writeln!(stderr, "workspace: {}", workspace.display());
    }

    // 4. Resolve timeout (required).
    let timeout =
        match resolve_timeout(parsed.timeout.as_deref(), env_get(env, "AGENTIRON_TIMEOUT")) {
            Ok(t) => t,
            Err(e) => {
                emit_failure!(AutomationRunErrorCategory::Config, e, EXIT_USAGE, workspace);
            }
        };

    if !quiet {
        let _ = writeln!(stderr, "timeout: {:?}", timeout);
    }

    // 5. Resolve config path and open ConfigStore.
    let config_path =
        match resolve_config_path(parsed.config.as_deref(), env_get(env, "AGENTIRON_CONFIG")) {
            Ok(p) => p,
            Err(e) => {
                emit_failure!(
                    AutomationRunErrorCategory::Config,
                    e,
                    EXIT_CONFIG,
                    workspace
                );
            }
        };

    if !quiet {
        let _ = writeln!(stderr, "config: {}", config_path.display());
    }

    let store = match ConfigStore::open_at(&config_path).await {
        Ok(s) => s,
        Err(e) => {
            emit_failure!(
                AutomationRunErrorCategory::Config,
                format!("failed to open config store: {}", e),
                EXIT_CONFIG,
                workspace
            );
        }
    };

    // 6. Bootstrap headless runtime.
    if !quiet {
        let _ = writeln!(stderr, "bootstrapping headless runtime...");
    }

    let headless = match bootstrap_headless(store, &parsed.task_id, workspace.clone()).await {
        Ok(h) => h,
        Err(e) => {
            let code = exit_code_for_bootstrap_error(&e);
            emit_failure!(bootstrap_error_category(&e), e.to_string(), code, workspace);
        }
    };

    if !quiet {
        let _ = writeln!(
            stderr,
            "running task '{}' with provider '{}' model '{}'",
            parsed.task_id, headless.provider_slug, headless.model
        );
    }

    // 7. Set up signal handler.
    let cancel = CancellationToken::new();
    let signal_cancel = cancel.clone();
    tokio::spawn(async move {
        wait_for_signal().await;
        signal_cancel.cancel();
    });

    // 8. Execute the automation run.
    let result = run_automation(headless, timeout, cancel).await;

    // 9. Format and emit output.
    match format {
        OutputFormat::Text => {
            let text = format_text_output(&result);
            let _ = writeln!(stdout, "{}", text);
        }
        OutputFormat::Json => {
            let json = format_json_output(&result);
            let _ = writeln!(stdout, "{}", json);
        }
    }

    exit_code_for_result(&result)
}

/// Look up an environment variable from a collected vector.
fn env_get<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
    env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests;
