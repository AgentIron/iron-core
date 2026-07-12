//! Linux crontab host scheduler adapter.
//!
//! Uses the `crontab` command to read and write the user's crontab. Owned
//! entries are managed via marker-delimited blocks (see `crontab` module).
//! Non-owned content is preserved.
//!
//! The cron expression maps directly to crontab's native five-field syntax,
//! so no compilation or expansion is needed — the schedule text is used
//! as-is.

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::scheduled_task::host::{
    render_cron_command, CommandRunner, HostInstallRequest, HostScheduler, HostSchedulerError,
    ObservedHostEntry,
};
use crate::scheduled_task::platform::crontab::{make_block, CronOwnedBlock, ParsedCrontab};

/// Linux crontab host scheduler.
pub struct CronHostScheduler {
    runner: Box<dyn CommandRunner>,
    lock: Mutex<()>,
}

impl CronHostScheduler {
    /// Create a new cron adapter with the given command runner.
    pub fn new(runner: Box<dyn CommandRunner>) -> Self {
        Self {
            runner,
            lock: Mutex::new(()),
        }
    }

    /// Read the current crontab text. Returns empty string if no crontab.
    async fn read_crontab(&self) -> Result<String, HostSchedulerError> {
        let output = self
            .runner
            .run("crontab", &["-l"])
            .await
            .map_err(|e| HostSchedulerError::Io(e.to_string()))?;

        if output.exit_code == 0 {
            return Ok(output.stdout);
        }

        // crontab -l exits 1 with "no crontab" on stderr when the user has no
        // crontab. Only that case is treated as an empty crontab. Any other
        // non-zero exit (permission denied, command not found, etc.) is a real
        // failure and must propagate rather than silently erasing the crontab.
        if output.exit_code == 1 && output.stderr.contains("no crontab") {
            return Ok(String::new());
        }

        Err(HostSchedulerError::Io(format!(
            "crontab -l failed (exit {}): {}",
            output.exit_code, output.stderr
        )))
    }

    /// Write the full crontab text via stdin.
    async fn write_crontab(&self, content: &str) -> Result<(), HostSchedulerError> {
        let output = self
            .runner
            .run_with_stdin("crontab", &["-"], content)
            .await
            .map_err(|e| HostSchedulerError::Io(e.to_string()))?;

        if output.exit_code != 0 {
            return Err(HostSchedulerError::Io(format!(
                "crontab install failed (exit {}): {}",
                output.exit_code, output.stderr
            )));
        }

        Ok(())
    }

    /// Parse the current crontab.
    async fn parse_current(&self) -> Result<ParsedCrontab, HostSchedulerError> {
        let raw = self.read_crontab().await?;
        Ok(ParsedCrontab::parse(&raw))
    }

    /// Convert an owned block to an observed host entry.
    fn block_to_observed(block: &CronOwnedBlock) -> ObservedHostEntry {
        ObservedHostEntry {
            schedule_id: block.schedule_id.clone(),
            enabled: block.enabled,
            corrupt: false,
            raw_schedule: Some(block.cron_schedule.clone()),
            observed_command: Some(block.command.clone()),
            metadata: None,
        }
    }
}

#[async_trait]
impl HostScheduler for CronHostScheduler {
    fn platform(&self) -> &'static str {
        "cron"
    }

    async fn install(&self, request: &HostInstallRequest) -> Result<(), HostSchedulerError> {
        let _guard = self.lock.lock().await;
        let mut parsed = self.parse_current().await?;

        // The cron expression maps directly to crontab syntax. Escape `%`
        // for cron's metacharacter handling.
        let command = render_cron_command(&request.program, &request.args);
        let block = make_block(
            &request.schedule_id,
            request.cron.as_str(),
            &command,
            request.enabled,
        );

        parsed.upsert(block);

        let rendered = parsed.render();
        self.write_crontab(&rendered).await
    }

    async fn remove(&self, schedule_id: &str) -> Result<(), HostSchedulerError> {
        let _guard = self.lock.lock().await;
        let mut parsed = self.parse_current().await?;
        parsed.remove(schedule_id);
        let rendered = parsed.render();
        self.write_crontab(&rendered).await
    }

    async fn list_owned(&self) -> Result<Vec<ObservedHostEntry>, HostSchedulerError> {
        let parsed = self.parse_current().await?;
        let mut entries: Vec<ObservedHostEntry> = parsed
            .owned_blocks()
            .iter()
            .map(|b| Self::block_to_observed(b))
            .collect();
        // Expose malformed owned blocks so reconciliation can discover
        // corrupted AgentIron entries.
        for (schedule_id, _raw) in parsed.malformed_blocks() {
            entries.push(ObservedHostEntry {
                schedule_id: schedule_id.clone(),
                enabled: false,
                corrupt: true,
                raw_schedule: None,
                observed_command: None,
                metadata: None,
            });
        }
        Ok(entries)
    }

    async fn inspect(
        &self,
        schedule_id: &str,
    ) -> Result<Option<ObservedHostEntry>, HostSchedulerError> {
        let parsed = self.parse_current().await?;

        if let Some(block) = parsed.find_owned(schedule_id) {
            return Ok(Some(Self::block_to_observed(block)));
        }

        // Check for malformed blocks.
        if parsed.contains_id(schedule_id) {
            return Ok(Some(ObservedHostEntry {
                schedule_id: schedule_id.to_string(),
                enabled: false,
                corrupt: true,
                raw_schedule: None,
                observed_command: None,
                metadata: None,
            }));
        }

        Ok(None)
    }
}

// ============================================================================
// Mock command runner for testing
// ============================================================================

/// Mock command runner that simulates crontab interactions.
#[cfg(test)]
struct MockCrontabRunner {
    state: parking_lot::Mutex<MockCrontabState>,
}

/// Simulated crontab state for the mock runner.
#[cfg(test)]
#[derive(Clone)]
enum MockCrontabState {
    /// Active crontab with content.
    Content(String),
    /// No crontab for the user (exit 1, "no crontab").
    Absent,
    /// `crontab -l` fails with the given exit code and stderr. Used to
    /// simulate permission errors and other non-"no crontab" failures.
    ReadError { exit_code: i32, stderr: String },
}

#[cfg(test)]
use crate::scheduled_task::cron::CronExpression;
#[cfg(test)]
use crate::scheduled_task::host::CommandOutput;

#[cfg(test)]
impl MockCrontabRunner {
    fn new(initial: Option<&str>) -> Self {
        let state = match initial {
            Some(text) => MockCrontabState::Content(text.to_string()),
            None => MockCrontabState::Absent,
        };
        Self {
            state: parking_lot::Mutex::new(state),
        }
    }

    /// Create a runner where `crontab -l` fails with the given exit code
    /// and stderr (e.g. a permission error with exit code 2).
    fn with_read_error(exit_code: i32, stderr: &str) -> Self {
        Self {
            state: parking_lot::Mutex::new(MockCrontabState::ReadError {
                exit_code,
                stderr: stderr.to_string(),
            }),
        }
    }
}

#[cfg(test)]
#[async_trait]
impl CommandRunner for MockCrontabRunner {
    async fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, std::io::Error> {
        assert_eq!(program, "crontab");

        match args {
            ["-l"] => {
                let state = self.state.lock().clone();
                match state {
                    MockCrontabState::Content(text) => Ok(CommandOutput {
                        exit_code: 0,
                        stdout: text,
                        stderr: String::new(),
                    }),
                    MockCrontabState::Absent => Ok(CommandOutput {
                        exit_code: 1,
                        stdout: String::new(),
                        stderr: "no crontab for user".to_string(),
                    }),
                    MockCrontabState::ReadError { exit_code, stderr } => Ok(CommandOutput {
                        exit_code,
                        stdout: String::new(),
                        stderr,
                    }),
                }
            }
            _ => Ok(CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: "unknown args".to_string(),
            }),
        }
    }

    async fn run_with_stdin(
        &self,
        program: &str,
        args: &[&str],
        stdin: &str,
    ) -> Result<CommandOutput, std::io::Error> {
        assert_eq!(program, "crontab");
        assert_eq!(args, &["-"]);

        *self.state.lock() = MockCrontabState::Content(stdin.to_string());

        Ok(CommandOutput {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduled_task::host::SchedulerInstallContext;

    fn make_request(id: &str, cron: &str, enabled: bool) -> HostInstallRequest {
        let ctx = SchedulerInstallContext {
            runner_executable: std::path::PathBuf::from("/usr/local/bin/agent-iron"),
            config_store_path: std::path::PathBuf::from("/home/user/.config/agentiron/config.db"),
        };
        let (program, args) = ctx.generate_invocation("task-1");
        HostInstallRequest {
            schedule_id: id.to_string(),
            automation_task_id: "task-1".to_string(),
            cron: CronExpression::parse(cron).unwrap(),
            enabled,
            program,
            args,
        }
    }

    #[tokio::test]
    async fn install_into_empty_crontab() {
        let runner = MockCrontabRunner::new(None);
        let scheduler = CronHostScheduler::new(Box::new(runner));

        let request = make_request("s1", "0 9 * * *", true);
        scheduler.install(&request).await.unwrap();

        // Read back and verify.
        let entry = scheduler.inspect("s1").await.unwrap().unwrap();
        assert_eq!(entry.schedule_id, "s1");
        assert!(entry.enabled);
        assert_eq!(entry.raw_schedule.as_deref(), Some("0 9 * * *"));
        assert!(entry
            .observed_command
            .as_deref()
            .unwrap()
            .contains("run task-1"));
    }

    #[tokio::test]
    async fn install_beside_existing_entries() {
        let initial = "# My crontab\n0 0 * * * /usr/bin/cleanup\n";
        let runner = MockCrontabRunner::new(Some(initial));
        let scheduler = CronHostScheduler::new(Box::new(runner));

        let request = make_request("s1", "0 9 * * *", true);
        scheduler.install(&request).await.unwrap();

        // List should only show owned entries.
        let owned = scheduler.list_owned().await.unwrap();
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].schedule_id, "s1");
    }

    #[tokio::test]
    async fn install_disabled_task() {
        let runner = MockCrontabRunner::new(None);
        let scheduler = CronHostScheduler::new(Box::new(runner));

        let request = make_request("s1", "0 9 * * *", false);
        scheduler.install(&request).await.unwrap();

        let entry = scheduler.inspect("s1").await.unwrap().unwrap();
        assert!(!entry.enabled);
    }

    #[tokio::test]
    async fn replace_existing_block() {
        let runner = MockCrontabRunner::new(None);
        let scheduler = CronHostScheduler::new(Box::new(runner));

        // Install initial.
        scheduler
            .install(&make_request("s1", "0 9 * * *", true))
            .await
            .unwrap();

        // Replace with different schedule.
        scheduler
            .install(&make_request("s1", "0 10 * * *", true))
            .await
            .unwrap();

        let entry = scheduler.inspect("s1").await.unwrap().unwrap();
        assert_eq!(entry.raw_schedule.as_deref(), Some("0 10 * * *"));
    }

    #[tokio::test]
    async fn remove_block() {
        let runner = MockCrontabRunner::new(None);
        let scheduler = CronHostScheduler::new(Box::new(runner));

        scheduler
            .install(&make_request("s1", "0 9 * * *", true))
            .await
            .unwrap();
        scheduler
            .install(&make_request("s2", "0 10 * * *", true))
            .await
            .unwrap();

        scheduler.remove("s1").await.unwrap();

        assert!(scheduler.inspect("s1").await.unwrap().is_none());
        assert!(scheduler.inspect("s2").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn list_multiple_owned() {
        let runner = MockCrontabRunner::new(None);
        let scheduler = CronHostScheduler::new(Box::new(runner));

        for (id, cron) in &[("alpha", "0 9 * * *"), ("bravo", "0 10 * * *")] {
            scheduler
                .install(&make_request(id, cron, true))
                .await
                .unwrap();
        }

        let owned = scheduler.list_owned().await.unwrap();
        assert_eq!(owned.len(), 2);
    }

    #[tokio::test]
    async fn inspect_nonexistent_returns_none() {
        let runner = MockCrontabRunner::new(None);
        let scheduler = CronHostScheduler::new(Box::new(runner));

        let result = scheduler.inspect("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn faithful_cron_compilation() {
        // On Linux, the cron expression maps directly to crontab syntax.
        // No transformation is needed.
        let runner = MockCrontabRunner::new(None);
        let scheduler = CronHostScheduler::new(Box::new(runner));

        let complex = "*/15 9-17/2 1,15 * 1-5";
        scheduler
            .install(&make_request("s1", complex, true))
            .await
            .unwrap();

        let entry = scheduler.inspect("s1").await.unwrap().unwrap();
        assert_eq!(entry.raw_schedule.as_deref(), Some(complex));
    }

    #[tokio::test]
    async fn enable_after_disabled() {
        let runner = MockCrontabRunner::new(None);
        let scheduler = CronHostScheduler::new(Box::new(runner));

        // Install disabled.
        scheduler
            .install(&make_request("s1", "0 9 * * *", false))
            .await
            .unwrap();

        // Replace with enabled.
        scheduler
            .install(&make_request("s1", "0 9 * * *", true))
            .await
            .unwrap();

        let entry = scheduler.inspect("s1").await.unwrap().unwrap();
        assert!(entry.enabled);
    }

    #[tokio::test]
    async fn read_permission_error_propagates() {
        // Exit code 2 (e.g. permission denied) must not be treated as an empty
        // crontab; it must propagate as an error.
        let runner = MockCrontabRunner::with_read_error(2, "crontab: permission denied");
        let scheduler = CronHostScheduler::new(Box::new(runner));

        let result = scheduler.list_owned().await;
        assert!(result.is_err());
        assert!(matches!(result, Err(HostSchedulerError::Io(_))));
    }
}
