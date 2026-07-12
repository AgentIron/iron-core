//! Schedule manager: desired-state inspection and host reconciliation.
//!
//! `ScheduleManager` is the application-facing API that reads desired
//! ConfigStore schedules, validates automation-task references, asks the
//! host scheduler for observed state, and returns compositional status
//! reports. Inspection is read-only; reconciliation mutates host entries.

use crate::config::ConfigStore;
use crate::scheduled_task::cron::CronExpression;
use crate::scheduled_task::host::{
    HostInstallRequest, HostScheduler, ObservedHostEntry, SchedulerInstallContext,
};
use crate::scheduled_task::{
    DesiredState, ExecutionState, HostState, ReferenceState, ScheduleDiagnostic,
    ScheduleDiagnosticKind, ScheduleHealth, ScheduleStatus, ScheduledTask,
};

/// Application-facing schedule manager.
///
/// Combines desired ConfigStore state with observed host state to provide
/// read-only inspection and explicit reconciliation.
pub struct ScheduleManager<'a> {
    store: &'a ConfigStore,
    host: &'a dyn HostScheduler,
    context: SchedulerInstallContext,
}

/// Policy controlling orphan removal during reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrphanPolicy {
    /// Report orphans but do not remove them.
    ReportOnly,
    /// Remove orphaned owned entries during reconcile_all.
    RemoveOrphans,
}

impl<'a> ScheduleManager<'a> {
    /// Create a new schedule manager.
    pub fn new(
        store: &'a ConfigStore,
        host: &'a dyn HostScheduler,
        context: SchedulerInstallContext,
    ) -> Self {
        Self {
            store,
            host,
            context,
        }
    }

    /// Read-only inspection of a single schedule's status.
    ///
    /// Compares desired ConfigStore state with observed host state without
    /// mutating either. Reports orphans (host entry without desired state)
    /// and drift (mismatched schedule or enabled state).
    pub async fn inspect(&self, schedule_id: &str) -> ScheduleStatus {
        let (desired, store_error) = match self.store.get_scheduled_task(schedule_id).await {
            Ok(opt) => (opt, None),
            Err(e) => (None, Some(format!("config store error: {}", e))),
        };

        let (observed, host_error) = match self.host.inspect(schedule_id).await {
            Ok(opt) => (opt, None),
            Err(e) => (None, Some(format!("host error: {}", e))),
        };

        // Check automation-task reference.
        let reference_state = match &desired {
            Some(schedule) => {
                match self
                    .store
                    .get_automation_task(&schedule.automation_task_id)
                    .await
                {
                    Ok(Some(_)) => ReferenceState::Valid,
                    Ok(None) => ReferenceState::Missing,
                    Err(_) => ReferenceState::Invalid,
                }
            }
            None => ReferenceState::Valid,
        };

        self.synthesize_status(
            schedule_id,
            desired,
            observed,
            store_error,
            host_error,
            reference_state,
        )
    }

    /// Read-only inspection of all known schedules and owned host entries.
    pub async fn inspect_all(&self) -> Vec<ScheduleStatus> {
        let desired_list = match self.store.list_scheduled_tasks().await {
            Ok(list) => list,
            Err(_) => return Vec::new(),
        };
        let observed_list = self.host.list_owned().await.unwrap_or_default();

        let mut ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for d in &desired_list {
            ids.insert(d.id.clone());
        }
        for o in &observed_list {
            ids.insert(o.schedule_id.clone());
        }

        let mut results = Vec::new();
        for id in ids {
            let desired = desired_list.iter().find(|d| d.id == id).cloned();
            let observed = observed_list.iter().find(|o| o.schedule_id == id).cloned();

            // Check reference for desired schedules.
            let reference_state = match &desired {
                Some(schedule) => {
                    match self
                        .store
                        .get_automation_task(&schedule.automation_task_id)
                        .await
                    {
                        Ok(Some(_)) => ReferenceState::Valid,
                        Ok(None) => ReferenceState::Missing,
                        Err(_) => ReferenceState::Invalid,
                    }
                }
                None => ReferenceState::Valid,
            };

            results.push(self.synthesize_status(
                &id,
                desired,
                observed,
                None,
                None,
                reference_state,
            ));
        }
        results
    }

    /// Mutating reconciliation of a single schedule.
    ///
    /// Installs, replaces, enables, disables, or removes the host entry to
    /// match desired ConfigStore state. Returns the resulting status.
    pub async fn reconcile(&self, schedule_id: &str) -> ScheduleStatus {
        let desired = match self.store.get_scheduled_task(schedule_id).await {
            Ok(opt) => opt,
            Err(e) => {
                return self.error_status(
                    schedule_id,
                    DesiredState::Present,
                    HostState::Unknown,
                    ScheduleDiagnosticKind::InstallationFailed,
                    format!("config store error: {}", e),
                );
            }
        };

        match desired {
            Some(schedule) => {
                let request = self.build_install_request(&schedule);
                match request {
                    Ok(req) => {
                        if let Err(e) = self.host.install(&req).await {
                            return self.error_status(
                                schedule_id,
                                DesiredState::Present,
                                HostState::Unknown,
                                ScheduleDiagnosticKind::InstallationFailed,
                                format!("host install failed: {}", e),
                            );
                        }
                    }
                    Err(diagnostic) => {
                        return self.error_status(
                            schedule_id,
                            DesiredState::Present,
                            HostState::Missing,
                            ScheduleDiagnosticKind::UnsupportedSchedule,
                            diagnostic,
                        );
                    }
                }
            }
            None => {
                if let Err(e) = self.host.remove(schedule_id).await {
                    return self.error_status(
                        schedule_id,
                        DesiredState::Missing,
                        HostState::Unknown,
                        ScheduleDiagnosticKind::InstallationFailed,
                        format!("host remove failed: {}", e),
                    );
                }
            }
        }

        self.inspect(schedule_id).await
    }

    /// Reconcile all desired schedules and optionally remove orphans.
    ///
    /// Returns status for every schedule and orphan processed.
    pub async fn reconcile_all(&self, orphan_policy: OrphanPolicy) -> Vec<ScheduleStatus> {
        let desired_list = self.store.list_scheduled_tasks().await.unwrap_or_default();
        let observed_list = self.host.list_owned().await.unwrap_or_default();

        // Reconcile all desired schedules.
        for schedule in &desired_list {
            let request = self.build_install_request(schedule);
            if let Ok(req) = request {
                let _ = self.host.install(&req).await;
            }
        }

        // Handle orphans.
        let desired_ids: std::collections::HashSet<&str> =
            desired_list.iter().map(|s| s.id.as_str()).collect();
        for observed in &observed_list {
            if !desired_ids.contains(observed.schedule_id.as_str())
                && orphan_policy == OrphanPolicy::RemoveOrphans
            {
                let _ = self.host.remove(&observed.schedule_id).await;
            }
        }

        self.inspect_all().await
    }

    /// Build a host install request from a desired schedule.
    fn build_install_request(
        &self,
        schedule: &ScheduledTask,
    ) -> Result<HostInstallRequest, String> {
        let cron = CronExpression::parse(&schedule.cron_expression)
            .map_err(|e| format!("schedule cron invalid: {}", e))?;

        let (program, args) = self
            .context
            .generate_invocation(&schedule.automation_task_id);

        Ok(HostInstallRequest {
            schedule_id: schedule.id.clone(),
            automation_task_id: schedule.automation_task_id.clone(),
            cron,
            enabled: schedule.enabled,
            program,
            args,
        })
    }

    /// Synthesize a compositional status from desired and observed state.
    #[allow(clippy::too_many_arguments)]
    fn synthesize_status(
        &self,
        schedule_id: &str,
        desired: Option<ScheduledTask>,
        observed: Option<ObservedHostEntry>,
        store_error: Option<String>,
        host_error: Option<String>,
        reference_state: ReferenceState,
    ) -> ScheduleStatus {
        let mut diagnostics = Vec::new();
        let execution_state = ExecutionState::Unknown; // Cannot verify headless safety without profile registry.
        let mut host_state = HostState::Missing;

        // Surface store errors.
        if let Some(ref msg) = store_error {
            diagnostics.push(ScheduleDiagnostic {
                kind: ScheduleDiagnosticKind::PlatformUnavailable,
                message: msg.clone(),
            });
        }

        // Surface host errors.
        if let Some(ref msg) = host_error {
            diagnostics.push(ScheduleDiagnostic {
                kind: ScheduleDiagnosticKind::PlatformUnavailable,
                message: msg.clone(),
            });
            host_state = HostState::Unknown;
        }

        let desired_state = if let Some(ref schedule) = desired {
            // Add reference diagnostics from pre-checked state.
            match reference_state {
                ReferenceState::Missing => {
                    diagnostics.push(ScheduleDiagnostic {
                        kind: ScheduleDiagnosticKind::MissingTask,
                        message: format!(
                            "automation task '{}' does not exist",
                            schedule.automation_task_id
                        ),
                    });
                }
                ReferenceState::Invalid => {
                    diagnostics.push(ScheduleDiagnostic {
                        kind: ScheduleDiagnosticKind::InvalidTask,
                        message: format!(
                            "automation task '{}' has an unsupported schema",
                            schedule.automation_task_id
                        ),
                    });
                }
                ReferenceState::Valid => {}
            }

            DesiredState::Present
        } else {
            DesiredState::Missing
        };

        // Analyze host state (only if no host error).
        if host_error.is_none() {
            if let Some(ref entry) = observed {
                if entry.corrupt {
                    host_state = HostState::Corrupt;
                    diagnostics.push(ScheduleDiagnostic {
                        kind: ScheduleDiagnosticKind::CorruptHostEntry,
                        message: format!("host entry '{}' is corrupt", schedule_id),
                    });
                } else if desired.is_none() {
                    host_state = HostState::Installed;
                    diagnostics.push(ScheduleDiagnostic {
                        kind: ScheduleDiagnosticKind::OrphanedHostEntry,
                        message: format!(
                            "host entry '{}' exists but no desired schedule",
                            schedule_id
                        ),
                    });
                } else if !entry.enabled && desired.as_ref().map(|d| d.enabled).unwrap_or(true) {
                    host_state = HostState::Disabled;
                    diagnostics.push(ScheduleDiagnostic {
                        kind: ScheduleDiagnosticKind::ScheduleDrift,
                        message: format!(
                            "host entry '{}' is disabled but desired is enabled",
                            schedule_id
                        ),
                    });
                } else if entry.enabled && !desired.as_ref().map(|d| d.enabled).unwrap_or(true) {
                    host_state = HostState::Drifted;
                    diagnostics.push(ScheduleDiagnostic {
                        kind: ScheduleDiagnosticKind::ScheduleDrift,
                        message: format!(
                            "host entry '{}' is enabled but desired is disabled",
                            schedule_id
                        ),
                    });
                } else if self.check_drift(&desired, entry) {
                    host_state = HostState::Drifted;
                    diagnostics.push(ScheduleDiagnostic {
                        kind: ScheduleDiagnosticKind::ScheduleDrift,
                        message: format!(
                            "host entry '{}' schedule or command differs from desired",
                            schedule_id
                        ),
                    });
                } else {
                    host_state = if entry.enabled {
                        HostState::Installed
                    } else {
                        HostState::Disabled
                    };
                }
            } else if desired.is_some() {
                host_state = HostState::Missing;
                diagnostics.push(ScheduleDiagnostic {
                    kind: ScheduleDiagnosticKind::NotInstalled,
                    message: format!("host entry '{}' is not installed", schedule_id),
                });
            }
        }

        // Check runner-path drift if we have both desired and observed.
        if let (Some(ref schedule), Some(ref entry)) = (&desired, &observed) {
            let (program, args) = self
                .context
                .generate_invocation(&schedule.automation_task_id);
            let expected_cmd = crate::scheduled_task::host::render_command(&program, &args);
            if let Some(ref observed_cmd) = entry.observed_command {
                if *observed_cmd != expected_cmd && host_state != HostState::Corrupt {
                    host_state = HostState::Drifted;
                    diagnostics.push(ScheduleDiagnostic {
                        kind: ScheduleDiagnosticKind::RunnerPathDrift,
                        message: "host entry command differs from installation context".to_string(),
                    });
                }
            }
        }

        let health = if store_error.is_some() || host_error.is_some() {
            ScheduleHealth::Unavailable
        } else {
            self.compute_health(&desired_state, &reference_state, &host_state)
        };

        ScheduleStatus {
            schedule_id: schedule_id.to_string(),
            health,
            desired_state,
            reference_state,
            execution_state,
            host_state,
            diagnostics,
            host_metadata: observed.and_then(|o| o.metadata),
        }
    }

    fn compute_health(
        &self,
        desired: &DesiredState,
        reference: &ReferenceState,
        host: &HostState,
    ) -> ScheduleHealth {
        if *reference == ReferenceState::Missing || *reference == ReferenceState::Invalid {
            return ScheduleHealth::Degraded;
        }

        match (*desired, *host) {
            (DesiredState::Missing, HostState::Missing) => ScheduleHealth::Healthy,
            (DesiredState::Missing, _) => ScheduleHealth::Degraded,
            (DesiredState::Present, HostState::Installed)
            | (DesiredState::Present, HostState::Disabled) => ScheduleHealth::Healthy,
            (DesiredState::Present, _) => ScheduleHealth::Degraded,
        }
    }

    /// Check whether the desired schedule matches the observed host entry.
    fn check_drift(&self, desired: &Option<ScheduledTask>, observed: &ObservedHostEntry) -> bool {
        let Some(schedule) = desired else {
            return false;
        };

        if let Some(ref raw) = observed.raw_schedule {
            if *raw != schedule.cron_expression {
                return true;
            }
        }

        false
    }

    /// Build an error status for failed operations.
    fn error_status(
        &self,
        schedule_id: &str,
        desired: DesiredState,
        host: HostState,
        kind: ScheduleDiagnosticKind,
        message: String,
    ) -> ScheduleStatus {
        ScheduleStatus {
            schedule_id: schedule_id.to_string(),
            health: ScheduleHealth::Degraded,
            desired_state: desired,
            reference_state: ReferenceState::Valid,
            execution_state: ExecutionState::Unknown,
            host_state: host,
            diagnostics: vec![ScheduleDiagnostic { kind, message }],
            host_metadata: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation_task::AutomationTaskInput;
    use crate::config::records::PromptInput;
    use crate::scheduled_task::host::{FakeHostScheduler, HostSchedulerError};
    use crate::scheduled_task::ScheduledTaskInput;
    use serde_json::json;
    use std::path::PathBuf;

    async fn setup_store() -> ConfigStore {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let temp = tempfile::tempdir().unwrap();

        store
            .set_prompt(&PromptInput {
                id: "prompt-1".to_string(),
                schema_version: crate::stored_prompt::STORED_PROMPT_SCHEMA_VERSION,
                payload: json!({"instructions": "Do thing", "skills": []}),
            })
            .await
            .unwrap();

        store
            .set_automation_task(&AutomationTaskInput {
                id: "task-1".to_string(),
                display_name: "Task One".to_string(),
                stored_prompt_id: "prompt-1".to_string(),
                expected_outcome: "Done".to_string(),
                project_root: temp.path().to_path_buf(),
                timeout_seconds: 300,
            })
            .await
            .unwrap();

        store
            .set_scheduled_task(&ScheduledTaskInput {
                id: "sched-1".to_string(),
                automation_task_id: "task-1".to_string(),
                cron_expression: "0 9 * * *".to_string(),
                enabled: true,
            })
            .await
            .unwrap();

        store
    }

    fn test_context() -> SchedulerInstallContext {
        SchedulerInstallContext {
            runner_executable: PathBuf::from("/usr/local/bin/agent-iron"),
            config_store_path: PathBuf::from("/home/user/.config/agentiron/config.db"),
        }
    }

    #[tokio::test]
    async fn inspect_healthy_schedule() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        // No host entry yet → degraded (not installed).
        let status = mgr.inspect("sched-1").await;
        assert_eq!(status.health, ScheduleHealth::Degraded);
        assert_eq!(status.desired_state, DesiredState::Present);
        assert_eq!(status.host_state, HostState::Missing);
    }

    #[tokio::test]
    async fn reconcile_installs_host_entry() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        let status = mgr.reconcile("sched-1").await;
        assert_eq!(status.health, ScheduleHealth::Healthy);
        assert_eq!(status.host_state, HostState::Installed);

        let entry = host.inspect("sched-1").await.unwrap().unwrap();
        assert!(entry.enabled);
        assert!(entry
            .observed_command
            .as_deref()
            .unwrap()
            .contains("run task-1"));
    }

    #[tokio::test]
    async fn reconcile_disabled_schedule() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        // Update to disabled.
        store
            .set_scheduled_task(&ScheduledTaskInput {
                id: "sched-1".to_string(),
                automation_task_id: "task-1".to_string(),
                cron_expression: "0 9 * * *".to_string(),
                enabled: false,
            })
            .await
            .unwrap();

        let status = mgr.reconcile("sched-1").await;
        assert_eq!(status.health, ScheduleHealth::Healthy);
        assert_eq!(status.host_state, HostState::Disabled);
    }

    #[tokio::test]
    async fn inspect_reports_drift() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        // Install with original schedule.
        mgr.reconcile("sched-1").await;

        // Change desired schedule.
        store
            .set_scheduled_task(&ScheduledTaskInput {
                id: "sched-1".to_string(),
                automation_task_id: "task-1".to_string(),
                cron_expression: "0 10 * * *".to_string(),
                enabled: true,
            })
            .await
            .unwrap();

        let status = mgr.inspect("sched-1").await;
        assert_eq!(status.health, ScheduleHealth::Degraded);
        assert_eq!(status.host_state, HostState::Drifted);
        assert!(status
            .diagnostics
            .iter()
            .any(|d| d.kind == ScheduleDiagnosticKind::ScheduleDrift));
    }

    #[tokio::test]
    async fn inspect_reports_orphan() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx.clone());

        let cron = CronExpression::parse("0 6 * * *").unwrap();
        let (program, args) = ctx.generate_invocation("task-1");
        host.install(&HostInstallRequest {
            schedule_id: "orphan".to_string(),
            automation_task_id: "task-1".to_string(),
            cron,
            enabled: true,
            program,
            args,
        })
        .await
        .unwrap();

        let status = mgr.inspect("orphan").await;
        assert_eq!(status.health, ScheduleHealth::Degraded);
        assert_eq!(status.desired_state, DesiredState::Missing);
        assert!(status
            .diagnostics
            .iter()
            .any(|d| d.kind == ScheduleDiagnosticKind::OrphanedHostEntry));
    }

    #[tokio::test]
    async fn reconcile_removes_orphan_with_policy() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx.clone());

        let cron = CronExpression::parse("0 6 * * *").unwrap();
        let (program, args) = ctx.generate_invocation("task-1");
        host.install(&HostInstallRequest {
            schedule_id: "orphan".to_string(),
            automation_task_id: "task-1".to_string(),
            cron,
            enabled: true,
            program,
            args,
        })
        .await
        .unwrap();

        mgr.reconcile_all(OrphanPolicy::RemoveOrphans).await;
        assert!(host.inspect("orphan").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reconcile_all_keeps_orphan_with_report_only() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx.clone());

        let cron = CronExpression::parse("0 6 * * *").unwrap();
        let (program, args) = ctx.generate_invocation("task-1");
        host.install(&HostInstallRequest {
            schedule_id: "orphan".to_string(),
            automation_task_id: "task-1".to_string(),
            cron,
            enabled: true,
            program,
            args,
        })
        .await
        .unwrap();

        mgr.reconcile_all(OrphanPolicy::ReportOnly).await;
        assert!(host.inspect("orphan").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn reconcile_idempotent() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        let status1 = mgr.reconcile("sched-1").await;
        let status2 = mgr.reconcile("sched-1").await;

        assert_eq!(status1.health, ScheduleHealth::Healthy);
        assert_eq!(status2.health, ScheduleHealth::Healthy);
    }

    #[tokio::test]
    async fn inspect_runner_path_drift() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        // Install with correct context.
        mgr.reconcile("sched-1").await;

        // Now create a manager with a different context.
        let ctx2 = SchedulerInstallContext {
            runner_executable: PathBuf::from("/different/path/agent-iron"),
            config_store_path: PathBuf::from("/different/config.db"),
        };
        let mgr2 = ScheduleManager::new(&store, &host, ctx2);

        let status = mgr2.inspect("sched-1").await;
        assert!(status
            .diagnostics
            .iter()
            .any(|d| d.kind == ScheduleDiagnosticKind::RunnerPathDrift));
    }

    #[tokio::test]
    async fn inspect_missing_schedule_returns_healthy_missing() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        let status = mgr.inspect("nonexistent").await;
        assert_eq!(status.health, ScheduleHealth::Healthy);
        assert_eq!(status.desired_state, DesiredState::Missing);
        assert_eq!(status.host_state, HostState::Missing);
    }

    #[tokio::test]
    async fn reconcile_removes_deleted_schedule() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        // Install.
        mgr.reconcile("sched-1").await;
        assert!(host.inspect("sched-1").await.unwrap().is_some());

        // Delete desired state.
        store.delete_scheduled_task("sched-1").await.unwrap();

        // Reconcile should remove host entry.
        mgr.reconcile("sched-1").await;
        assert!(host.inspect("sched-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn host_error_during_reconcile_reports_degraded() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        *host.force_error.lock() =
            Some(HostSchedulerError::PlatformUnavailable("test".to_string()));
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        let status = mgr.reconcile("sched-1").await;
        assert_eq!(status.health, ScheduleHealth::Degraded);
        assert!(status
            .diagnostics
            .iter()
            .any(|d| d.kind == ScheduleDiagnosticKind::InstallationFailed));
    }
}
