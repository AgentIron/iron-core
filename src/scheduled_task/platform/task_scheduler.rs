//! Windows Task Scheduler host scheduler adapter.
//!
//! Manages tasks under `\AgentIron\Tasks\<id>` using generated Task Scheduler
//! XML. Uses `schtasks.exe` for lifecycle operations. Cron expressions are
//! compiled into Task Scheduler triggers.
//!
//! The XML rendering and cron expansion logic is platform-independent and
//! tested here. The actual `schtasks.exe` execution requires Windows.

use async_trait::async_trait;
use std::path::PathBuf;

use crate::scheduled_task::cron::CronExpression;
use crate::scheduled_task::host::{
    CommandRunner, HostInstallRequest, HostScheduler, HostSchedulerError, ObservedHostEntry,
};

/// Task folder path for AgentIron tasks.
pub const TASK_FOLDER: &str = r"\AgentIron\Tasks\";

/// Maximum number of triggers before rejecting.
pub const MAX_TRIGGERS: usize = 64;

// ============================================================================
// Cron to Task Scheduler trigger expansion
// ============================================================================

/// A Task Scheduler calendar trigger specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskTrigger {
    pub minutes: Vec<u32>,
    pub hours: Vec<u32>,
    pub days_of_month: Vec<u32>,
    pub months: Vec<u32>,
    pub days_of_week: Vec<u32>,
}

/// Expand a cron expression into Task Scheduler triggers.
///
/// Returns `Err` if the expansion would exceed `MAX_TRIGGERS`.
pub fn expand_cron(cron: &CronExpression) -> Result<TaskTrigger, String> {
    let minutes = cron.minutes().to_vec();
    let hours = cron.hours().to_vec();
    let doms = cron.days_of_month().to_vec();
    let months = cron.months().to_vec();
    let dows = cron.days_of_week().to_vec();

    // Only count non-wildcard field values toward complexity. Wildcard
    // fields are omitted from the XML rendering entirely.
    let dom_in_xml = if doms.len() < 31 { doms.len() } else { 0 };
    let month_in_xml = if months.len() < 12 { months.len() } else { 0 };
    let total_xml_values =
        minutes.len() + hours.len() + dom_in_xml + month_in_xml + dows.len();
    if total_xml_values > MAX_TRIGGERS {
        return Err(format!(
            "cron expression has too many field values ({}, max {}); \
             use a simpler expression",
            total_xml_values, MAX_TRIGGERS
        ));
    }

    Ok(TaskTrigger {
        minutes,
        hours,
        days_of_month: doms,
        months,
        days_of_week: dows,
    })
}

// ============================================================================
// XML rendering
// ============================================================================

/// Render a complete Task Scheduler XML definition.
pub fn render_task_xml(
    schedule_id: &str,
    trigger: &TaskTrigger,
    executable: &str,
    arguments: &str,
    enabled: bool,
) -> String {
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n");
    xml.push_str("<Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n");

    // Triggers
    xml.push_str("  <Triggers>\n");
    xml.push_str("    <CalendarTrigger>\n");

    // Schedule by day of month
    xml.push_str("      <ScheduleByMonth>\n");

    // Days of month
    if trigger.days_of_month.len() < 31 {
        xml.push_str("        <DaysOfMonth>\n");
        for &d in &trigger.days_of_month {
            xml.push_str(&format!("          <Day>{}</Day>\n", d));
        }
        xml.push_str("        </DaysOfMonth>\n");
    }

    // Months
    if trigger.months.len() < 12 {
        xml.push_str("        <Months>\n");
        for &m in &trigger.months {
            let month_name = month_name(m);
            xml.push_str(&format!("          <{}/>\n", month_name));
        }
        xml.push_str("        </Months>\n");
    }

    xml.push_str("      </ScheduleByMonth>\n");
    xml.push_str("    </CalendarTrigger>\n");
    xml.push_str("  </Triggers>\n");

    // Settings
    xml.push_str("  <Settings>\n");
    if !enabled {
        xml.push_str("    <Enabled>false</Enabled>\n");
    } else {
        xml.push_str("    <Enabled>true</Enabled>\n");
    }
    xml.push_str("    <AllowStartIfOnBatteries>true</AllowStartIfOnBatteries>\n");
    xml.push_str("    <DontStopIfGoingOnBatteries>true</DontStopIfGoingOnBatteries>\n");
    xml.push_str("    <ExecutionTimeLimit>PT24H</ExecutionTimeLimit>\n");
    xml.push_str("  </Settings>\n");

    // Actions
    xml.push_str("  <Actions Context=\"Author\">\n");
    xml.push_str("    <Exec>\n");
    xml.push_str(&format!("      <Command>{}</Command>\n", escape_xml(executable)));
    xml.push_str(&format!("      <Arguments>{}</Arguments>\n", escape_xml(arguments)));
    xml.push_str("    </Exec>\n");
    xml.push_str("  </Actions>\n");

    xml.push_str("</Task>\n");
    xml
}

fn month_name(m: u32) -> &'static str {
    match m {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "January",
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn task_path(schedule_id: &str) -> String {
    format!("{}{}", TASK_FOLDER, schedule_id)
}

// ============================================================================
// Adapter
// ============================================================================

/// Windows Task Scheduler host scheduler.
pub struct TaskSchedulerHostScheduler {
    runner: Box<dyn CommandRunner>,
}

impl TaskSchedulerHostScheduler {
    pub fn new(runner: Box<dyn CommandRunner>) -> Self {
        Self { runner }
    }

    fn split_command(command: &str) -> (&str, String) {
        let mut parts = command.splitn(2, ' ');
        let exe = parts.next().unwrap_or(command);
        let args = parts.next().unwrap_or("");
        (exe, args.to_string())
    }
}

#[async_trait]
impl HostScheduler for TaskSchedulerHostScheduler {
    fn platform(&self) -> &'static str {
        "task-scheduler"
    }

    async fn install(&self, request: &HostInstallRequest) -> Result<(), HostSchedulerError> {
        let trigger = expand_cron(&request.cron).map_err(|reason| {
            HostSchedulerError::UnsupportedSchedule {
                platform: "task-scheduler",
                reason,
            }
        })?;

        let (exe, args) = Self::split_command(&request.command);
        let xml = render_task_xml(
            &request.schedule_id,
            &trigger,
            exe,
            &args,
            request.enabled,
        );

        let path = task_path(&request.schedule_id);
        let temp_file = format!(
            "{}\\agentiron_task_{}.xml",
            std::env::temp_dir().display(),
            request.schedule_id
        );

        // Write XML to temp file.
        let write_output = self
            .runner
            .run_with_stdin("cmd", &["/c", "tee", &temp_file], &xml)
            .await
            .map_err(|e| HostSchedulerError::Io(e.to_string()))?;

        if write_output.exit_code != 0 {
            return Err(HostSchedulerError::Io(format!(
                "failed to write XML: {}",
                write_output.stderr
            )));
        }

        // Register task from XML.
        let output = self
            .runner
            .run(
                "schtasks.exe",
                &["/Create", "/TN", &path, "/XML", &temp_file, "/F"],
            )
            .await
            .map_err(|e| HostSchedulerError::Io(e.to_string()))?;

        if output.exit_code != 0 {
            return Err(HostSchedulerError::Io(format!(
                "schtasks /Create failed: {}",
                output.stderr
            )));
        }

        Ok(())
    }

    async fn remove(&self, schedule_id: &str) -> Result<(), HostSchedulerError> {
        let path = task_path(schedule_id);
        let _ = self
            .runner
            .run("schtasks.exe", &["/Delete", "/TN", &path, "/F"])
            .await;
        Ok(())
    }

    async fn list_owned(&self) -> Result<Vec<ObservedHostEntry>, HostSchedulerError> {
        let output = self
            .runner
            .run("schtasks.exe", &["/Query", "/TN", TASK_FOLDER, "/FO", "CSV"])
            .await
            .map_err(|e| HostSchedulerError::Io(e.to_string()))?;

        let mut entries = Vec::new();
        for line in output.stdout.lines() {
            if line.contains(TASK_FOLDER) {
                if let Some(id) = line.strip_prefix(TASK_FOLDER) {
                    let id = id.split(',').next().unwrap_or(id).trim_matches('"');
                    entries.push(ObservedHostEntry {
                        schedule_id: id.to_string(),
                        enabled: true,
                        corrupt: false,
                        raw_schedule: None,
                        observed_command: None,
                        metadata: None,
                    });
                }
            }
        }
        Ok(entries)
    }

    async fn inspect(
        &self,
        schedule_id: &str,
    ) -> Result<Option<ObservedHostEntry>, HostSchedulerError> {
        let path = task_path(schedule_id);
        let output = self
            .runner
            .run("schtasks.exe", &["/Query", "/TN", &path, "/FO", "CSV"])
            .await
            .map_err(|e| HostSchedulerError::Io(e.to_string()))?;

        if output.exit_code != 0 {
            return Ok(None);
        }

        Ok(Some(ObservedHostEntry {
            schedule_id: schedule_id.to_string(),
            enabled: output.stdout.contains("Ready"),
            corrupt: false,
            raw_schedule: None,
            observed_command: None,
            metadata: None,
        }))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_daily() {
        let cron = CronExpression::parse("0 9 * * *").unwrap();
        let trigger = expand_cron(&cron).unwrap();
        assert_eq!(trigger.minutes, vec![0]);
        assert_eq!(trigger.hours, vec![9]);
    }

    #[test]
    fn expand_every_15_min() {
        let cron = CronExpression::parse("*/15 * * * *").unwrap();
        let trigger = expand_cron(&cron).unwrap();
        assert_eq!(trigger.minutes, vec![0, 15, 30, 45]);
    }

    #[test]
    fn expand_rejects_excessive() {
        let cron = CronExpression::parse("* * * * *").unwrap();
        let result = expand_cron(&cron);
        assert!(result.is_err());
    }

    #[test]
    fn render_xml_basic() {
        let trigger = TaskTrigger {
            minutes: vec![0],
            hours: vec![9],
            days_of_month: vec![1],
            months: vec![1],
            days_of_week: vec![],
        };
        let xml = render_task_xml(
            "s1",
            &trigger,
            r"C:\agent-iron.exe",
            "run task-1 --config C:\\config.db",
            true,
        );
        assert!(xml.contains("<Task"));
        assert!(xml.contains("<CalendarTrigger>"));
        assert!(xml.contains("<Day>1</Day>"));
        assert!(xml.contains("<January/>"));
        assert!(xml.contains("<Enabled>true</Enabled>"));
        assert!(xml.contains(r"C:\agent-iron.exe"));
    }

    #[test]
    fn render_xml_disabled() {
        let trigger = TaskTrigger {
            minutes: vec![30],
            hours: vec![5],
            days_of_month: vec![],
            months: vec![],
            days_of_week: vec![],
        };
        let xml = render_task_xml("s1", &trigger, "agent-iron.exe", "run t1", false);
        assert!(xml.contains("<Enabled>false</Enabled>"));
    }

    #[test]
    fn render_xml_escapes_special_chars() {
        let trigger = TaskTrigger {
            minutes: vec![0],
            hours: vec![0],
            days_of_month: vec![],
            months: vec![],
            days_of_week: vec![],
        };
        let xml = render_task_xml("s1", &trigger, "normal", "arg <test> & stuff", true);
        assert!(xml.contains("&lt;test&gt;"));
        assert!(xml.contains("&amp; stuff"));
    }

    #[test]
    fn task_path_format() {
        assert_eq!(task_path("s1"), r"\AgentIron\Tasks\s1");
    }
}
