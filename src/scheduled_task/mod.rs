//! Scheduled automation-task definitions, validation, and persistence.
//!
//! A `ScheduledTask` is a reference to an existing `AutomationTask` paired with
//! a five-field cron expression and enabled state. Schedules never contain
//! executable paths, shell commands, prompt text, or environment variables —
//! the installed host entry is always a core-generated `agent-iron run` call.

pub mod cron;
pub mod host;
pub mod manager;
pub mod platform;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Schema version for scheduled-task payloads stored in ConfigStore.
pub const SCHEDULED_TASK_SCHEMA_VERSION: i64 = 1;

// ============================================================================
// Domain types
// ============================================================================

/// A schedule that invokes an automation task on a recurring cron-like basis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledTask {
    /// Stable schedule identifier.
    pub id: String,
    /// The automation task to invoke.
    pub automation_task_id: String,
    /// Validated five-field cron expression.
    pub cron_expression: String,
    /// Whether this schedule is active.
    pub enabled: bool,
    /// When the schedule was first created.
    pub created_at: DateTime<Utc>,
    /// When the schedule was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or replacing a scheduled task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledTaskInput {
    /// Stable schedule identifier.
    pub id: String,
    /// Must reference an existing automation task.
    pub automation_task_id: String,
    /// Five-field numeric cron expression.
    pub cron_expression: String,
    /// Whether the schedule should be active.
    pub enabled: bool,
}

/// Validate a scheduled-task input.
///
/// Trims the ID and automation-task ID, parses and validates the cron
/// expression, and returns `Ok` with normalized values or `Err` with a
/// human-readable message.
pub fn validate_schedule_input(input: &ScheduledTaskInput) -> Result<ScheduledTaskInput, String> {
    let id = trim_non_empty(&input.id, "Schedule ID")?;
    let automation_task_id = trim_non_empty(&input.automation_task_id, "Automation task ID")?;
    let cron_expression = input.cron_expression.trim().to_string();
    if cron_expression.is_empty() {
        return Err("Cron expression must not be empty".to_string());
    }

    // Validate the cron expression.
    cron::CronExpression::parse(&cron_expression)?;

    Ok(ScheduledTaskInput {
        id,
        automation_task_id,
        cron_expression,
        enabled: input.enabled,
    })
}

fn trim_non_empty(raw: &str, label: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{} must not be empty", label));
    }
    if trimmed.as_bytes().iter().any(|b| b.is_ascii_control()) {
        return Err(format!("{} must not contain control characters", label));
    }
    Ok(trimmed.to_string())
}

// ============================================================================
// Status and diagnostics (used by ScheduleManager in Group 5)
// ============================================================================

/// Overall health classification for a scheduled task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleHealth {
    /// Desired state is valid and host entry matches.
    Healthy,
    /// Desired state exists but host entry is missing, drifted, or partially
    /// installed.
    Degraded,
    /// The platform scheduler is not available or the task cannot be
    /// evaluated.
    Unavailable,
}

/// Desired-state status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredState {
    /// Schedule exists in ConfigStore.
    Present,
    /// Schedule does not exist in ConfigStore.
    Missing,
}

/// Automation-task reference status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceState {
    /// Referenced automation task exists.
    Valid,
    /// Referenced automation task does not exist.
    Missing,
    /// Referenced automation task has an unsupported schema version.
    Invalid,
}

/// Headless execution readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    /// The referenced task's profile is headless-safe.
    Ready,
    /// The referenced task's profile requires interactive approval.
    UnsafePolicy,
    /// Cannot determine readiness because the task is missing or invalid.
    Unknown,
}

/// Observed host-entry status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostState {
    /// Owned host entry exists and matches desired state.
    Installed,
    /// Owned host entry exists but is disabled.
    Disabled,
    /// No owned host entry exists.
    Missing,
    /// Owned host entry exists but differs from desired state.
    Drifted,
    /// Owned host entry exists but is malformed or corrupt.
    Corrupt,
    /// Host scheduler state cannot be determined.
    Unknown,
}

/// Optional metadata reported by the host scheduler, when available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRunMetadata {
    /// Last time the task ran, if known.
    pub last_run: Option<DateTime<Utc>>,
    /// Next scheduled run time, if known.
    pub next_run: Option<DateTime<Utc>>,
    /// Last result summary, if known.
    pub last_result: Option<String>,
}

/// A single diagnostic message about a scheduled task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleDiagnostic {
    /// Machine-readable diagnostic kind.
    pub kind: ScheduleDiagnosticKind,
    /// Human-readable message.
    pub message: String,
}

/// Classification of schedule diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleDiagnosticKind {
    /// Host installation failed.
    InstallationFailed,
    /// Host entry is missing.
    NotInstalled,
    /// Host entry schedule differs from desired.
    ScheduleDrift,
    /// Host entry is corrupt.
    CorruptHostEntry,
    /// Orphaned owned host entry with no desired state.
    OrphanedHostEntry,
    /// Referenced automation task is missing.
    MissingTask,
    /// Referenced automation task is invalid.
    InvalidTask,
    /// Profile is not headless-safe.
    UnsafePolicy,
    /// Platform does not support this schedule.
    UnsupportedSchedule,
    /// Platform scheduler is unavailable.
    PlatformUnavailable,
    /// Runner path differs from installation context.
    RunnerPathDrift,
}

/// A compositional status report for a scheduled task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleStatus {
    /// Schedule ID (or host-entry ID for orphans).
    pub schedule_id: String,
    /// Overall health summary.
    pub health: ScheduleHealth,
    /// Desired ConfigStore state.
    pub desired_state: DesiredState,
    /// Automation-task reference status.
    pub reference_state: ReferenceState,
    /// Headless execution readiness.
    pub execution_state: ExecutionState,
    /// Observed host-entry status.
    pub host_state: HostState,
    /// Diagnostics explaining the health classification.
    pub diagnostics: Vec<ScheduleDiagnostic>,
    /// Optional host-reported metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_metadata: Option<HostRunMetadata>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_input() -> ScheduledTaskInput {
        ScheduledTaskInput {
            id: "morning-brief".to_string(),
            automation_task_id: "daily-report".to_string(),
            cron_expression: "0 9 * * *".to_string(),
            enabled: true,
        }
    }

    #[test]
    fn valid_input_passes_validation() {
        let result = validate_schedule_input(&valid_input()).unwrap();
        assert_eq!(result.id, "morning-brief");
        assert_eq!(result.automation_task_id, "daily-report");
        assert_eq!(result.cron_expression, "0 9 * * *");
        assert!(result.enabled);
    }

    #[test]
    fn empty_id_rejected() {
        let mut input = valid_input();
        input.id = "  ".to_string();
        assert!(validate_schedule_input(&input).is_err());
    }

    #[test]
    fn empty_task_id_rejected() {
        let mut input = valid_input();
        input.automation_task_id = "".to_string();
        assert!(validate_schedule_input(&input).is_err());
    }

    #[test]
    fn empty_cron_rejected() {
        let mut input = valid_input();
        input.cron_expression = "   ".to_string();
        assert!(validate_schedule_input(&input).is_err());
    }

    #[test]
    fn invalid_cron_rejected() {
        let mut input = valid_input();
        input.cron_expression = "not-cron".to_string();
        assert!(validate_schedule_input(&input).is_err());
    }

    #[test]
    fn macro_cron_rejected() {
        let mut input = valid_input();
        input.cron_expression = "@daily".to_string();
        assert!(validate_schedule_input(&input).is_err());
    }

    #[test]
    fn stepped_cron_accepted() {
        let mut input = valid_input();
        input.cron_expression = "*/15 9-17/2 * 1,6,12 1-5".to_string();
        assert!(validate_schedule_input(&input).is_ok());
    }

    #[test]
    fn scheduled_task_serializes() {
        let now = Utc::now();
        let task = ScheduledTask {
            id: "s1".to_string(),
            automation_task_id: "t1".to_string(),
            cron_expression: "0 9 * * *".to_string(),
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        let json = serde_json::to_string(&task).unwrap();
        let restored: ScheduledTask = serde_json::from_str(&json).unwrap();
        assert_eq!(task, restored);
    }
}
