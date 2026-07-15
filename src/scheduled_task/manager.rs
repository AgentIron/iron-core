//! Schedule manager: desired-state inspection and host reconciliation.
//!
//! `ScheduleManager` is the application-facing API that reads desired
//! ConfigStore schedules, validates automation-task references, asks the
//! host scheduler for observed state, and returns compositional status
//! reports. Inspection is read-only; reconciliation mutates host entries.

use crate::config::ConfigStore;
use crate::profile::{AgentApproval, AgentProfile};
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
    /// and drift (mismatched schedule or enabled state). A schedule row that
    /// exists but has an unsupported schema is reported as desired-state
    /// `Unsupported` rather than `Missing` so the host entry is not treated
    /// as orphaned.
    pub async fn inspect(&self, schedule_id: &str) -> ScheduleStatus {
        let desired = match self.store.get_scheduled_task(schedule_id).await {
            Ok(opt) => opt,
            Err(e) => {
                return self.error_status(
                    schedule_id,
                    DesiredState::Unsupported,
                    HostState::Unknown,
                    ScheduleDiagnosticKind::UnsupportedSchedule,
                    format!("config store error: {}", e),
                );
            }
        };

        let (desired_state, observed, host_error) = match self.host.inspect(schedule_id).await {
            Ok(opt) => (DesiredState::Present, opt, None),
            Err(e) => (
                DesiredState::Present,
                None,
                Some(format!("host error: {}", e)),
            ),
        };

        let (desired_state, store_error) = if desired.is_none() {
            match self.store.schedule_exists(schedule_id).await {
                Ok(true) => (
                    DesiredState::Unsupported,
                    Some("Schedule has an unsupported schema version".to_string()),
                ),
                Ok(false) => (DesiredState::Missing, None),
                Err(e) => (
                    DesiredState::Unsupported,
                    Some(format!("config store error: {}", e)),
                ),
            }
        } else {
            (desired_state, None)
        };

        // Check automation-task reference and headless-execution safety.
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

        let execution_state = match &desired {
            Some(schedule) if reference_state == ReferenceState::Valid => {
                self.resolve_execution_state(schedule).await
            }
            _ => ExecutionState::Unknown,
        };

        self.synthesize_status(
            schedule_id,
            desired_state,
            desired,
            observed,
            store_error,
            host_error,
            reference_state,
            execution_state,
        )
    }
    /// Read-only inspection of all known schedules and owned host entries.
    ///
    /// Returns a vector with a single error status if the ConfigStore ID read
    /// fails, so callers are never misled into thinking there are zero
    /// schedules. Host-list failures are surfaced as per-schedule diagnostics.
    /// Schedule rows with unsupported schemas are surfaced as
    /// `DesiredState::Unsupported` rather than skipped.
    pub async fn inspect_all(&self) -> Vec<ScheduleStatus> {
        let ids = match self.store.list_schedule_ids().await {
            Ok(list) => list,
            Err(e) => {
                return vec![self.error_status(
                    "",
                    DesiredState::Missing,
                    HostState::Unknown,
                    ScheduleDiagnosticKind::PlatformUnavailable,
                    format!("config store error: {}", e),
                )];
            }
        };
        let (observed_list, host_error) = match self.host.list_owned().await {
            Ok(list) => (list, None),
            Err(e) => (Vec::new(), Some(format!("host error: {}", e))),
        };

        let mut id_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for id in &ids {
            id_set.insert(id.clone());
        }
        for o in &observed_list {
            id_set.insert(o.schedule_id.clone());
        }

        let mut results = Vec::new();
        for id in id_set {
            let (desired, desired_state) = match self.store.get_scheduled_task(&id).await {
                Ok(Some(t)) => (Some(t), DesiredState::Present),
                Ok(None) => match self.store.schedule_exists(&id).await {
                    Ok(true) => (None, DesiredState::Unsupported),
                    Ok(false) => (None, DesiredState::Missing),
                    Err(_) => (None, DesiredState::Unsupported),
                },
                Err(e) => {
                    results.push(self.error_status(
                        &id,
                        DesiredState::Unsupported,
                        HostState::Unknown,
                        ScheduleDiagnosticKind::UnsupportedSchedule,
                        format!("config store error: {}", e),
                    ));
                    continue;
                }
            };

            let (observed, per_host_error) = if let Some(host_error) = host_error.clone() {
                (None, Some(host_error))
            } else {
                match self.host.inspect(&id).await {
                    Ok(opt) => (opt, None),
                    Err(e) => (None, Some(format!("host error: {}", e))),
                }
            };

            // Check reference for desired schedules and headless safety.
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

            let execution_state = match &desired {
                Some(schedule) if reference_state == ReferenceState::Valid => {
                    self.resolve_execution_state(schedule).await
                }
                _ => ExecutionState::Unknown,
            };

            let store_error = if desired_state == DesiredState::Unsupported {
                Some("Schedule has an unsupported schema version".to_string())
            } else {
                None
            };

            results.push(self.synthesize_status(
                &id,
                desired_state,
                desired,
                observed,
                store_error,
                per_host_error,
                reference_state,
                execution_state,
            ));
        }
        results
    }

    /// Mutating reconciliation of a single schedule.
    ///
    /// Installs, replaces, enables, disables, or removes the host entry to
    /// match desired ConfigStore state. Returns the resulting status. A
    /// desired row with an unsupported schema is not reconciled and does not
    /// cause host removal.
    pub async fn reconcile(&self, schedule_id: &str) -> ScheduleStatus {
        let desired = match self.store.get_scheduled_task(schedule_id).await {
            Ok(opt) => opt,
            Err(e) => {
                return self.error_status(
                    schedule_id,
                    DesiredState::Unsupported,
                    HostState::Unknown,
                    ScheduleDiagnosticKind::UnsupportedSchedule,
                    format!("config store error: {}", e),
                );
            }
        };

        if desired.is_none() {
            match self.store.schedule_exists(schedule_id).await {
                Ok(true) => {
                    return self.error_status(
                        schedule_id,
                        DesiredState::Unsupported,
                        HostState::Unknown,
                        ScheduleDiagnosticKind::UnsupportedSchedule,
                        "Schedule has an unsupported schema version".to_string(),
                    );
                }
                Ok(false) => {}
                Err(e) => {
                    return self.error_status(
                        schedule_id,
                        DesiredState::Unsupported,
                        HostState::Unknown,
                        ScheduleDiagnosticKind::UnsupportedSchedule,
                        format!("config store error: {}", e),
                    );
                }
            }
        }

        match desired {
            Some(schedule) => {
                let request = self.build_install_request(&schedule);
                match request {
                    Ok(req) => {
                        // Inspect current host state before mutating.
                        // Skip install if the entry already matches.
                        let needs_install = match self.host.inspect(schedule_id).await {
                            Ok(Some(entry)) => {
                                entry.corrupt
                                    || entry.enabled != req.enabled
                                    || entry
                                        .raw_schedule
                                        .as_deref()
                                        .map(|s| s != req.cron.as_str())
                                        .unwrap_or(true)
                                    || entry
                                        .observed_command
                                        .as_deref()
                                        .map(|c| {
                                            let expected =
                                                crate::scheduled_task::host::render_command(
                                                    &req.program,
                                                    &req.args,
                                                );
                                            c != expected
                                        })
                                        .unwrap_or(true)
                            }
                            _ => true, // Missing, corrupt, or error → needs install
                        };

                        if needs_install {
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
    /// Returns status for every schedule and orphan processed. Aborts
    /// before any host mutation if the desired-state ID read fails, so a
    /// transient ConfigStore error can never cause host-entry removal.
    pub async fn reconcile_all(&self, orphan_policy: OrphanPolicy) -> Vec<ScheduleStatus> {
        let desired_ids = match self.store.list_schedule_ids().await {
            Ok(list) => list,
            Err(e) => {
                return vec![self.error_status(
                    "",
                    DesiredState::Missing,
                    HostState::Unknown,
                    ScheduleDiagnosticKind::PlatformUnavailable,
                    format!("config store error: {}", e),
                )];
            }
        };
        let observed_list = match self.host.list_owned().await {
            Ok(list) => list,
            Err(e) => {
                return vec![self.error_status(
                    "",
                    DesiredState::Missing,
                    HostState::Unknown,
                    ScheduleDiagnosticKind::PlatformUnavailable,
                    format!("host error: {}", e),
                )];
            }
        };

        // Reconcile all desired schedules via per-schedule reconcile so each
        // entry goes through the inspect-before-mutate path. Unsupported rows
        // are skipped by the per-ID reconcile without host mutation.
        for id in &desired_ids {
            let _ = self.reconcile(id).await;
        }

        // Handle orphans.
        let desired_id_set: std::collections::HashSet<&str> =
            desired_ids.iter().map(|s| s.as_str()).collect();
        for observed in &observed_list {
            if !desired_id_set.contains(observed.schedule_id.as_str())
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
        desired_state: DesiredState,
        desired: Option<ScheduledTask>,
        observed: Option<ObservedHostEntry>,
        store_error: Option<String>,
        host_error: Option<String>,
        reference_state: ReferenceState,
        execution_state: ExecutionState,
    ) -> ScheduleStatus {
        let mut diagnostics = Vec::new();
        let execution_state = if desired_state == DesiredState::Present && store_error.is_none() {
            execution_state
        } else {
            ExecutionState::Unknown
        };
        let mut host_state = HostState::Missing;

        // Surface store errors with the appropriate diagnostic kind.
        if let Some(ref msg) = store_error {
            let kind = if desired_state == DesiredState::Unsupported {
                ScheduleDiagnosticKind::UnsupportedSchedule
            } else {
                ScheduleDiagnosticKind::PlatformUnavailable
            };
            diagnostics.push(ScheduleDiagnostic {
                kind,
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

        // Surface headless-execution safety.
        if execution_state == ExecutionState::UnsafePolicy {
            diagnostics.push(ScheduleDiagnostic {
                kind: ScheduleDiagnosticKind::UnsafePolicy,
                message: format!(
                    "automation task '{}' profile does not allow unattended execution",
                    desired
                        .as_ref()
                        .map(|s| s.automation_task_id.as_str())
                        .unwrap_or(schedule_id)
                ),
            });
        }

        // Add reference diagnostics for readable desired schedules.
        if desired_state == DesiredState::Present {
            if let Some(ref schedule) = desired {
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
            }
        }

        // Analyze host state (only if no host error).
        if host_error.is_none() {
            if let Some(ref entry) = observed {
                if entry.corrupt {
                    host_state = HostState::Corrupt;
                    diagnostics.push(ScheduleDiagnostic {
                        kind: ScheduleDiagnosticKind::CorruptHostEntry,
                        message: format!("host entry '{}' is corrupt", schedule_id),
                    });
                } else if desired_state == DesiredState::Missing {
                    host_state = HostState::Installed;
                    diagnostics.push(ScheduleDiagnostic {
                        kind: ScheduleDiagnosticKind::OrphanedHostEntry,
                        message: format!(
                            "host entry '{}' exists but no desired schedule",
                            schedule_id
                        ),
                    });
                } else if desired_state == DesiredState::Unsupported {
                    host_state = HostState::Unknown;
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
            } else if desired_state == DesiredState::Present {
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
            self.compute_health(
                &desired_state,
                &reference_state,
                &execution_state,
                &host_state,
            )
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
        execution: &ExecutionState,
        host: &HostState,
    ) -> ScheduleHealth {
        if *reference == ReferenceState::Missing || *reference == ReferenceState::Invalid {
            return ScheduleHealth::Degraded;
        }

        match (*desired, *execution, *host) {
            (DesiredState::Missing, _, HostState::Missing) => ScheduleHealth::Healthy,
            (DesiredState::Missing, _, _) => ScheduleHealth::Degraded,
            (DesiredState::Unsupported, _, _) => ScheduleHealth::Degraded,
            (DesiredState::Present, ExecutionState::Ready, HostState::Installed)
            | (DesiredState::Present, ExecutionState::Ready, HostState::Disabled) => {
                ScheduleHealth::Healthy
            }
            (DesiredState::Present, _, _) => ScheduleHealth::Degraded,
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

    /// Resolve headless execution readiness for a desired schedule.
    ///
    /// Traverses the referenced automation task to its stored prompt and
    /// profile, returning `Ready` only when the profile's approval posture
    /// is `AutoApprove`. Any missing or unreadable intermediate yields
    /// `Unknown`.
    async fn resolve_execution_state(&self, schedule: &ScheduledTask) -> ExecutionState {
        let task = match self
            .store
            .get_automation_task(&schedule.automation_task_id)
            .await
        {
            Ok(Some(t)) => t,
            _ => return ExecutionState::Unknown,
        };

        let profile_id = match self.store.get_prompt(&task.stored_prompt_id).await {
            Ok(Some(record)) => {
                if record.schema_version
                    != crate::stored_prompt::STORED_PROMPT_SCHEMA_VERSION
                {
                    return ExecutionState::Unknown;
                }
                match serde_json::from_value::<crate::stored_prompt::StoredPrompt>(record.payload) {
                    Ok(prompt) => prompt
                        .profile
                        .as_ref()
                        .map(|id| id.as_str().to_string())
                        .unwrap_or_else(|| "default".to_string()),
                    Err(_) => return ExecutionState::Unknown,
                }
            }
            _ => return ExecutionState::Unknown,
        };

        if profile_id == "default" {
            return ExecutionState::Ready;
        }

        let record = match self.store.get_profile(&profile_id).await {
            Ok(Some(r)) => r,
            _ => return ExecutionState::Unknown,
        };

        if record.schema_version != crate::profile::PROFILE_SCHEMA_VERSION {
            return ExecutionState::Unknown;
        }

        match serde_json::from_value::<AgentProfile>(record.payload) {
            Ok(profile) => {
                if profile.approval == AgentApproval::AutoApprove {
                    ExecutionState::Ready
                } else {
                    ExecutionState::UnsafePolicy
                }
            }
            Err(_) => ExecutionState::Unknown,
        }
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
                payload: json!({"display_name": "Test Prompt", "normalized_name": "test-prompt", "instructions": "Do thing", "skills": []}),
                display_name: "Test Prompt".to_string(),
                normalized_name: "test-prompt".to_string(),
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

    #[tokio::test]
    async fn reconcile_unsupported_schedule_does_not_remove_host() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        // Install a host entry for a schedule ID.
        mgr.reconcile("sched-1").await;
        assert!(host.inspect("sched-1").await.unwrap().is_some());

        // Change the desired schedule to an unsupported schema version.
        sqlx::query("UPDATE schedule SET schema_version = 99 WHERE id = ?")
            .bind("sched-1")
            .execute(store.pool())
            .await
            .unwrap();

        // Reconcile should not remove the host entry; it should report Unsupported.
        let status = mgr.reconcile("sched-1").await;
        assert_eq!(status.desired_state, DesiredState::Unsupported);
        assert!(host.inspect("sched-1").await.unwrap().is_some());
        assert!(status
            .diagnostics
            .iter()
            .any(|d| d.kind == ScheduleDiagnosticKind::UnsupportedSchedule));
    }

    #[tokio::test]
    async fn inspect_unsupported_schedule_is_not_orphan() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        mgr.reconcile("sched-1").await;

        sqlx::query("UPDATE schedule SET schema_version = 99 WHERE id = ?")
            .bind("sched-1")
            .execute(store.pool())
            .await
            .unwrap();

        let status = mgr.inspect("sched-1").await;
        assert_eq!(status.desired_state, DesiredState::Unsupported);
        assert_ne!(status.host_state, HostState::Missing);
        assert!(!status
            .diagnostics
            .iter()
            .any(|d| d.kind == ScheduleDiagnosticKind::OrphanedHostEntry));
    }

    #[tokio::test]
    async fn execution_state_is_unsafe_for_non_autoapprove_profile() {
        use crate::profile::{AgentApproval, AgentProfile, AgentProfileProvider};

        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        // Save a non-default profile with PerTool approval.
        store
            .set_profile(&crate::config::records::ProfileInput {
                id: "prof-1".to_string(),
                schema_version: crate::profile::PROFILE_SCHEMA_VERSION,
                payload: serde_json::to_value(&AgentProfile {
                    name: "Prof 1".to_string(),
                    provider: AgentProfileProvider::Managed {
                        provider_slug: "test".into(),
                        model: "m".to_string(),
                    },
                    tools: crate::profile::ToolFilter::default(),
                    skills: crate::profile::SkillFilter::default(),
                    approval: AgentApproval::PerTool,
                    identity_prompt: None,
                })
                .unwrap(),
            })
            .await
            .unwrap();

        // Create a prompt referencing the profile and a task referencing the prompt.
        store
            .set_prompt(&PromptInput {
                id: "prompt-2".to_string(),
                schema_version: crate::stored_prompt::STORED_PROMPT_SCHEMA_VERSION,
                payload: json!({"display_name": "P2", "normalized_name": "p2", "instructions": "Do", "skills": [], "profile": "prof-1"}),
                display_name: "P2".to_string(),
                normalized_name: "p2".to_string(),
            })
            .await
            .unwrap();

        store
            .set_automation_task(&AutomationTaskInput {
                id: "task-2".to_string(),
                display_name: "Task Two".to_string(),
                stored_prompt_id: "prompt-2".to_string(),
                expected_outcome: "Done".to_string(),
                project_root: tempfile::tempdir().unwrap().path().to_path_buf(),
                timeout_seconds: 300,
            })
            .await
            .unwrap();

        store
            .set_scheduled_task(&ScheduledTaskInput {
                id: "sched-2".to_string(),
                automation_task_id: "task-2".to_string(),
                cron_expression: "0 9 * * *".to_string(),
                enabled: true,
            })
            .await
            .unwrap();

        let status = mgr.inspect("sched-2").await;
        assert_eq!(status.execution_state, ExecutionState::UnsafePolicy);
        assert!(status
            .diagnostics
            .iter()
            .any(|d| d.kind == ScheduleDiagnosticKind::UnsafePolicy));
    }

    #[tokio::test]
    async fn execution_state_is_ready_for_autoapprove_profile() {
        use crate::profile::{AgentApproval, AgentProfile, AgentProfileProvider};

        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        store
            .set_profile(&crate::config::records::ProfileInput {
                id: "prof-1".to_string(),
                schema_version: crate::profile::PROFILE_SCHEMA_VERSION,
                payload: serde_json::to_value(&AgentProfile {
                    name: "Prof 1".to_string(),
                    provider: AgentProfileProvider::Managed {
                        provider_slug: "test".into(),
                        model: "m".to_string(),
                    },
                    tools: crate::profile::ToolFilter::default(),
                    skills: crate::profile::SkillFilter::default(),
                    approval: AgentApproval::AutoApprove,
                    identity_prompt: None,
                })
                .unwrap(),
            })
            .await
            .unwrap();

        store
            .set_prompt(&PromptInput {
                id: "prompt-2".to_string(),
                schema_version: crate::stored_prompt::STORED_PROMPT_SCHEMA_VERSION,
                payload: json!({"display_name": "P2", "normalized_name": "p2", "instructions": "Do", "skills": [], "profile": "prof-1"}),
                display_name: "P2".to_string(),
                normalized_name: "p2".to_string(),
            })
            .await
            .unwrap();

        store
            .set_automation_task(&AutomationTaskInput {
                id: "task-2".to_string(),
                display_name: "Task Two".to_string(),
                stored_prompt_id: "prompt-2".to_string(),
                expected_outcome: "Done".to_string(),
                project_root: tempfile::tempdir().unwrap().path().to_path_buf(),
                timeout_seconds: 300,
            })
            .await
            .unwrap();

        store
            .set_scheduled_task(&ScheduledTaskInput {
                id: "sched-2".to_string(),
                automation_task_id: "task-2".to_string(),
                cron_expression: "0 9 * * *".to_string(),
                enabled: true,
            })
            .await
            .unwrap();

        let status = mgr.inspect("sched-2").await;
        assert_eq!(status.execution_state, ExecutionState::Ready);
    }

    #[tokio::test]
    async fn reconcile_repairs_drifted_schedule() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        // Install with original schedule (enabled=true, cron "0 9 * * *").
        mgr.reconcile("sched-1").await;
        assert_eq!(mgr.inspect("sched-1").await.health, ScheduleHealth::Healthy);

        // Change desired schedule to a different cron expression.
        store
            .set_scheduled_task(&ScheduledTaskInput {
                id: "sched-1".to_string(),
                automation_task_id: "task-1".to_string(),
                cron_expression: "0 10 * * *".to_string(),
                enabled: true,
            })
            .await
            .unwrap();

        // Inspect should show drift.
        let pre = mgr.inspect("sched-1").await;
        assert_eq!(pre.host_state, HostState::Drifted);

        // Reconcile should repair the drift.
        let post = mgr.reconcile("sched-1").await;
        assert_eq!(post.health, ScheduleHealth::Healthy);
        assert_eq!(post.host_state, HostState::Installed);
    }

    #[tokio::test]
    async fn reconcile_skips_matching_entry() {
        let store = setup_store().await;
        let host = FakeHostScheduler::new();
        let ctx = test_context();
        let mgr = ScheduleManager::new(&store, &host, ctx);

        // Install.
        mgr.reconcile("sched-1").await;
        let install_count_before = host.install_count();

        // Reconcile again — should not reinstall because the entry matches.
        mgr.reconcile("sched-1").await;

        assert_eq!(
            host.install_count(),
            install_count_before,
            "matching entry should not be reinstalled"
        );
    }
}
