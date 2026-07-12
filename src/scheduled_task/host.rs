//! Host scheduler abstraction, installation context, and platform factory.
//!
//! The `HostScheduler` trait defines the operations each platform adapter
//! implements: install, remove, list, and inspect owned entries. The
//! `SchedulerInstallContext` supplies the trusted runner executable and
//! ConfigStore path used to generate fixed `agent-iron run` invocations.
//!
//! Callers never provide executables, arguments, shell text, or environment
//! variables — the installed command is always core-derived.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::path::PathBuf;

use super::cron::CronExpression;
use super::{HostRunMetadata, ScheduleHealth};

// ============================================================================
// Installation context
// ============================================================================

/// Trusted installation context for generating host scheduler entries.
///
/// Supplied by the embedding application (e.g. the desktop app). Core does
/// not assume `current_exe()` is the runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerInstallContext {
    /// Absolute path to the `agent-iron` binary.
    pub runner_executable: PathBuf,
    /// Absolute path to the ConfigStore database.
    pub config_store_path: PathBuf,
}

impl SchedulerInstallContext {
    /// Generate the fixed command line for a scheduled task.
    ///
    /// The command invokes `agent-iron run <task-id> --config <path>` with
    /// absolute paths and no reliance on `PATH`, process working directory,
    /// or environment variables.
    pub fn generate_command(&self, automation_task_id: &str) -> String {
        format!(
            "{} run {} --config {}",
            self.runner_executable.display(),
            automation_task_id,
            self.config_store_path.display()
        )
    }
}

// ============================================================================
// Request and observed types
// ============================================================================

/// A core-generated request to install or replace a host scheduler entry.
///
/// All fields are derived by core from the desired ConfigStore state and the
/// trusted installation context. The host adapter never receives arbitrary
/// caller-provided command content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInstallRequest {
    /// Stable schedule ID used for ownership.
    pub schedule_id: String,
    /// The automation task being scheduled.
    pub automation_task_id: String,
    /// Parsed cron expression to compile into native triggers.
    pub cron: CronExpression,
    /// Whether the entry should be enabled or disabled.
    pub enabled: bool,
    /// Core-generated command line (from `SchedulerInstallContext`).
    pub command: String,
}

/// An observed host scheduler entry belonging to AgentIron.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedHostEntry {
    /// Schedule ID extracted from ownership markers.
    pub schedule_id: String,
    /// Whether the entry is currently enabled.
    pub enabled: bool,
    /// Whether the entry appears corrupt or malformed.
    pub corrupt: bool,
    /// The observed raw schedule text, if parseable.
    pub raw_schedule: Option<String>,
    /// The observed command, if extractable.
    pub observed_command: Option<String>,
    /// Optional host-reported run metadata.
    pub metadata: Option<HostRunMetadata>,
}

// ============================================================================
// Errors
// ============================================================================

/// Errors from host scheduler operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum HostSchedulerError {
    /// The platform does not support the requested schedule.
    #[error("unsupported schedule for {platform}: {reason}")]
    UnsupportedSchedule {
        platform: &'static str,
        reason: String,
    },

    /// The host scheduler service is unavailable.
    #[error("platform unavailable: {0}")]
    PlatformUnavailable(String),

    /// A command or filesystem operation failed.
    #[error("io error: {0}")]
    Io(String),

    /// The cron expression cannot be faithfully compiled.
    #[error("compilation failed: {0}")]
    CompilationFailed(String),
}

// ============================================================================
// HostScheduler trait
// ============================================================================

/// Platform-specific host scheduler operations.
///
/// Each adapter manages only AgentIron-owned entries identified by
/// platform-specific ownership markers. Non-owned entries are never
/// mutated.
#[async_trait]
pub trait HostScheduler: Send + Sync {
    /// The platform name (e.g. "cron", "launchd", "task-scheduler").
    fn platform(&self) -> &'static str;

    /// Install or replace an owned host entry.
    ///
    /// If an owned entry for the same schedule ID already exists, it is
    /// replaced atomically. Disabled requests install a disabled entry
    /// rather than skipping installation.
    async fn install(&self, request: &HostInstallRequest) -> Result<(), HostSchedulerError>;

    /// Remove an owned host entry by schedule ID.
    ///
    /// Returns `Ok(())` if the entry did not exist.
    async fn remove(&self, schedule_id: &str) -> Result<(), HostSchedulerError>;

    /// List all AgentIron-owned host entries.
    async fn list_owned(&self) -> Result<Vec<ObservedHostEntry>, HostSchedulerError>;

    /// Inspect a single owned host entry by schedule ID.
    ///
    /// Returns `Ok(None)` if no owned entry exists for the given ID.
    async fn inspect(&self, schedule_id: &str) -> Result<Option<ObservedHostEntry>, HostSchedulerError>;
}

// ============================================================================
// Factory
// ============================================================================

/// Create the platform-appropriate host scheduler.
///
/// Returns `Err(PlatformUnavailable)` on unsupported targets.
pub fn create_host_scheduler(
    _context: SchedulerInstallContext,
) -> Result<Box<dyn HostScheduler>, HostSchedulerError> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "linux")] {
            use super::platform::cron_adapter::CronHostScheduler;
            Ok(Box::new(CronHostScheduler::new(Box::new(ProductionCommandRunner))))
        } else if #[cfg(target_os = "macos")] {
            use super::platform::launchd::LaunchdHostScheduler;
            use std::path::PathBuf;
            let home = std::env::var("HOME").unwrap_or_default();
            let dir = PathBuf::from(home).join("Library/LaunchAgents");
            Ok(Box::new(LaunchdHostScheduler::new(Box::new(ProductionCommandRunner), dir)))
        } else if #[cfg(target_os = "windows")] {
            use super::platform::task_scheduler::TaskSchedulerHostScheduler;
            Ok(Box::new(TaskSchedulerHostScheduler::new(Box::new(ProductionCommandRunner))))
        } else {
            Err(HostSchedulerError::PlatformUnavailable(
                "unsupported platform".to_string(),
            ))
        }
    }
}

/// Production command runner using `tokio::process::Command`.
pub struct ProductionCommandRunner;

#[async_trait]
impl CommandRunner for ProductionCommandRunner {
    async fn run(
        &self,
        program: &str,
        args: &[&str],
    ) -> Result<CommandOutput, std::io::Error> {
        let output = tokio::process::Command::new(program)
            .args(args)
            .output()
            .await?;
        Ok(CommandOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }

    async fn run_with_stdin(
        &self,
        program: &str,
        args: &[&str],
        stdin: &str,
    ) -> Result<CommandOutput, std::io::Error> {
        use tokio::io::AsyncWriteExt;
        let mut child = tokio::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        if let Some(ref mut stdin_handle) = child.stdin {
            stdin_handle.write_all(stdin.as_bytes()).await?;
        }
        let output = child.wait_with_output().await?;
        Ok(CommandOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

// ============================================================================
// Injectable boundaries for testing
// ============================================================================

/// Injectable command runner for executing host scheduler commands.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Run a program with arguments and return the output.
    async fn run(
        &self,
        program: &str,
        args: &[&str],
    ) -> Result<CommandOutput, std::io::Error>;

    /// Run a program with stdin input and return the output.
    async fn run_with_stdin(
        &self,
        program: &str,
        args: &[&str],
        stdin: &str,
    ) -> Result<CommandOutput, std::io::Error>;
}

/// Output from a command runner invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Injectable filesystem boundary.
#[async_trait]
pub trait SchedulerFilesystem: Send + Sync {
    async fn read_to_string(&self, path: &std::path::Path) -> Result<String, std::io::Error>;
    async fn write(&self, path: &std::path::Path, content: &str) -> Result<(), std::io::Error>;
    async fn exists(&self, path: &std::path::Path) -> bool;
    async fn remove_file(&self, path: &std::path::Path) -> Result<(), std::io::Error>;
}

// ============================================================================
// Fake host scheduler for testing
// ============================================================================

/// In-memory fake host scheduler for testing schedule management without
/// touching the real host scheduler.
#[derive(Debug, Default)]
pub struct FakeHostScheduler {
    entries: parking_lot::RwLock<std::collections::HashMap<String, ObservedHostEntry>>,
    /// If set, all operations return this error.
    pub force_error: parking_lot::Mutex<Option<HostSchedulerError>>,
}

impl FakeHostScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    fn check_error(&self) -> Result<(), HostSchedulerError> {
        if let Some(ref e) = *self.force_error.lock() {
            return Err(e.clone());
        }
        Ok(())
    }
}

#[async_trait]
impl HostScheduler for FakeHostScheduler {
    fn platform(&self) -> &'static str {
        "fake"
    }

    async fn install(&self, request: &HostInstallRequest) -> Result<(), HostSchedulerError> {
        self.check_error()?;
        let mut entries = self.entries.write();
        entries.insert(
            request.schedule_id.clone(),
            ObservedHostEntry {
                schedule_id: request.schedule_id.clone(),
                enabled: request.enabled,
                corrupt: false,
                raw_schedule: Some(request.cron.as_str().to_string()),
                observed_command: Some(request.command.clone()),
                metadata: None,
            },
        );
        Ok(())
    }

    async fn remove(&self, schedule_id: &str) -> Result<(), HostSchedulerError> {
        self.check_error()?;
        let mut entries = self.entries.write();
        entries.remove(schedule_id);
        Ok(())
    }

    async fn list_owned(&self) -> Result<Vec<ObservedHostEntry>, HostSchedulerError> {
        self.check_error()?;
        let entries = self.entries.read();
        let mut list: Vec<_> = entries.values().cloned().collect();
        list.sort_by(|a, b| a.schedule_id.cmp(&b.schedule_id));
        Ok(list)
    }

    async fn inspect(
        &self,
        schedule_id: &str,
    ) -> Result<Option<ObservedHostEntry>, HostSchedulerError> {
        self.check_error()?;
        let entries = self.entries.read();
        Ok(entries.get(schedule_id).cloned())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> SchedulerInstallContext {
        SchedulerInstallContext {
            runner_executable: PathBuf::from("/usr/local/bin/agent-iron"),
            config_store_path: PathBuf::from("/home/user/.config/agentiron/config.db"),
        }
    }

    #[test]
    fn generate_command_uses_absolute_paths() {
        let ctx = test_context();
        let cmd = ctx.generate_command("daily-report");
        assert!(cmd.contains("/usr/local/bin/agent-iron"));
        assert!(cmd.contains("run daily-report"));
        assert!(cmd.contains("--config /home/user/.config/agentiron/config.db"));
    }

    #[test]
    fn generate_command_no_path_reliance() {
        let ctx = test_context();
        let cmd = ctx.generate_command("task-1");
        assert!(!cmd.starts_with("agent-iron"));
        assert!(cmd.starts_with("/"));
    }

    #[test]
    fn factory_returns_scheduler_on_supported_platform() {
        let ctx = test_context();
        let result = create_host_scheduler(ctx);
        cfg_if::cfg_if! {
            if #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))] {
                assert!(result.is_ok());
            } else {
                assert!(matches!(result, Err(HostSchedulerError::PlatformUnavailable(_))));
            }
        }
    }

    #[tokio::test]
    async fn fake_install_and_inspect() {
        let scheduler = FakeHostScheduler::new();
        let ctx = test_context();
        let cron = CronExpression::parse("0 9 * * *").unwrap();
        let request = HostInstallRequest {
            schedule_id: "s1".to_string(),
            automation_task_id: "daily-report".to_string(),
            cron,
            enabled: true,
            command: ctx.generate_command("daily-report"),
        };

        scheduler.install(&request).await.unwrap();

        let entry = scheduler.inspect("s1").await.unwrap().unwrap();
        assert_eq!(entry.schedule_id, "s1");
        assert!(entry.enabled);
        assert!(!entry.corrupt);
        assert_eq!(entry.raw_schedule.as_deref(), Some("0 9 * * *"));
    }

    #[tokio::test]
    async fn fake_remove() {
        let scheduler = FakeHostScheduler::new();
        let ctx = test_context();
        let cron = CronExpression::parse("0 9 * * *").unwrap();
        let request = HostInstallRequest {
            schedule_id: "s1".to_string(),
            automation_task_id: "t1".to_string(),
            cron,
            enabled: true,
            command: ctx.generate_command("t1"),
        };

        scheduler.install(&request).await.unwrap();
        scheduler.remove("s1").await.unwrap();
        assert!(scheduler.inspect("s1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn fake_list_owned_sorted() {
        let scheduler = FakeHostScheduler::new();
        let ctx = test_context();
        let cron = CronExpression::parse("0 9 * * *").unwrap();

        for id in &["charlie", "alpha", "bravo"] {
            scheduler
                .install(&HostInstallRequest {
                    schedule_id: id.to_string(),
                    automation_task_id: "t1".to_string(),
                    cron: cron.clone(),
                    enabled: true,
                    command: ctx.generate_command("t1"),
                })
                .await
                .unwrap();
        }

        let list = scheduler.list_owned().await.unwrap();
        let ids: Vec<&str> = list.iter().map(|e| e.schedule_id.as_str()).collect();
        assert_eq!(ids, vec!["alpha", "bravo", "charlie"]);
    }

    #[tokio::test]
    async fn fake_replace_on_reinstall() {
        let scheduler = FakeHostScheduler::new();
        let ctx = test_context();
        let cron = CronExpression::parse("0 9 * * *").unwrap();

        scheduler
            .install(&HostInstallRequest {
                schedule_id: "s1".to_string(),
                automation_task_id: "t1".to_string(),
                cron: cron.clone(),
                enabled: true,
                command: ctx.generate_command("t1"),
            })
            .await
            .unwrap();

        // Reinstall with disabled.
        scheduler
            .install(&HostInstallRequest {
                schedule_id: "s1".to_string(),
                automation_task_id: "t1".to_string(),
                cron: cron.clone(),
                enabled: false,
                command: ctx.generate_command("t1"),
            })
            .await
            .unwrap();

        let entry = scheduler.inspect("s1").await.unwrap().unwrap();
        assert!(!entry.enabled);
    }

    #[tokio::test]
    async fn fake_install_request_has_no_arbitrary_command() {
        let scheduler = FakeHostScheduler::new();
        let ctx = test_context();
        let cron = CronExpression::parse("0 9 * * *").unwrap();

        let request = HostInstallRequest {
            schedule_id: "s1".to_string(),
            automation_task_id: "t1".to_string(),
            cron,
            enabled: true,
            command: ctx.generate_command("t1"),
        };

        // The command must come from SchedulerInstallContext, not from user input.
        assert!(request.command.contains(ctx.runner_executable.display().to_string().as_str()));
        assert!(request.command.contains("run t1"));
    }

    #[tokio::test]
    async fn fake_force_error() {
        let scheduler = FakeHostScheduler::new();
        *scheduler.force_error.lock() = Some(HostSchedulerError::PlatformUnavailable(
            "test".to_string(),
        ));

        let result = scheduler.list_owned().await;
        assert!(matches!(result, Err(HostSchedulerError::PlatformUnavailable(_))));
    }
}
