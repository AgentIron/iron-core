//! Windows Task Scheduler host scheduler adapter.
//!
//! Manages tasks under `\AgentIron\Tasks\<id>` using generated Task Scheduler
//! XML. Uses `schtasks.exe` for lifecycle operations. Cron expressions are
//! compiled into Task Scheduler triggers.
//!
//! The XML rendering and cron expansion logic is platform-independent and
//! tested here. The actual `schtasks.exe` execution requires Windows.

use async_trait::async_trait;

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
    let total_xml_values = minutes.len() + hours.len() + dom_in_xml + month_in_xml + dows.len();
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
///
/// Each unique `(minute, hour)` pair becomes its own `<CalendarTrigger>` with
/// a `<StartBoundary>` encoding the time of day. Day-of-week restrictions are
/// emitted as `<ScheduleByWeek>`, day-of-month restrictions as
/// `<ScheduleByMonth>`. When cron restricts **both** DOM and DOW (OR
/// semantics), two independent triggers are emitted.
pub fn render_task_xml(
    trigger: &TaskTrigger,
    executable: &str,
    arguments: &str,
    enabled: bool,
) -> String {
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n");
    xml.push_str(
        "<Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n",
    );

    // Triggers
    xml.push_str("  <Triggers>\n");

    let dom_restricted = !trigger.days_of_month.is_empty() && trigger.days_of_month.len() < 31;
    let dow_restricted = !trigger.days_of_week.is_empty() && trigger.days_of_week.len() < 7;
    let months_restricted = !trigger.months.is_empty() && trigger.months.len() < 12;

    for &hour in &trigger.hours {
        for &minute in &trigger.minutes {
            let boundary = format!("2024-01-01T{:02}:{:02}:00", hour, minute);

            if dom_restricted {
                let body = render_schedule_by_month(
                    &trigger.days_of_month,
                    months_restricted,
                    &trigger.months,
                );
                xml.push_str(&render_calendar_trigger(&boundary, &body));
            }
            if dow_restricted {
                let body = render_schedule_by_week(
                    &trigger.days_of_week,
                    months_restricted,
                    &trigger.months,
                );
                xml.push_str(&render_calendar_trigger(&boundary, &body));
            }
            // No day-of-week or day-of-month restriction: a plain daily
            // trigger that fires at the given StartBoundary.
            if !dom_restricted && !dow_restricted {
                xml.push_str(&render_calendar_trigger(&boundary, ""));
            }
        }
    }

    xml.push_str("  </Triggers>\n");

    // Settings
    xml.push_str("  <Settings>\n");
    if enabled {
        xml.push_str("    <Enabled>true</Enabled>\n");
    } else {
        xml.push_str("    <Enabled>false</Enabled>\n");
    }
    xml.push_str("    <AllowStartIfOnBatteries>true</AllowStartIfOnBatteries>\n");
    xml.push_str("    <DontStopIfGoingOnBatteries>true</DontStopIfGoingOnBatteries>\n");
    xml.push_str("    <ExecutionTimeLimit>PT24H</ExecutionTimeLimit>\n");
    xml.push_str("  </Settings>\n");

    // Actions
    xml.push_str("  <Actions Context=\"Author\">\n");
    xml.push_str("    <Exec>\n");
    xml.push_str(&format!(
        "      <Command>{}</Command>\n",
        escape_xml(executable)
    ));
    xml.push_str(&format!(
        "      <Arguments>{}</Arguments>\n",
        escape_xml(arguments)
    ));
    xml.push_str("    </Exec>\n");
    xml.push_str("  </Actions>\n");

    xml.push_str("</Task>\n");
    xml
}

/// Render a single `<CalendarTrigger>` with the given start boundary and
/// optional schedule body.
fn render_calendar_trigger(boundary: &str, schedule_body: &str) -> String {
    let mut s = String::new();
    s.push_str("    <CalendarTrigger>\n");
    s.push_str(&format!(
        "      <StartBoundary>{}</StartBoundary>\n",
        boundary
    ));
    if !schedule_body.is_empty() {
        s.push_str(schedule_body);
    }
    s.push_str("    </CalendarTrigger>\n");
    s
}

/// Render a `<ScheduleByMonth>` body containing days of month and optional
/// months.
fn render_schedule_by_month(doms: &[u32], include_months: bool, months: &[u32]) -> String {
    let mut s = String::new();
    s.push_str("      <ScheduleByMonth>\n");
    s.push_str("        <DaysOfMonth>\n");
    for &d in doms {
        s.push_str(&format!("          <Day>{}</Day>\n", d));
    }
    s.push_str("        </DaysOfMonth>\n");
    if include_months {
        s.push_str(&render_months(months));
    }
    s.push_str("      </ScheduleByMonth>\n");
    s
}

/// Render a `<ScheduleByWeek>` body containing days of week and optional
/// months. Cron day-of-week values use 0=Sunday through 6=Saturday.
fn render_schedule_by_week(dows: &[u32], include_months: bool, months: &[u32]) -> String {
    let mut s = String::new();
    s.push_str("      <ScheduleByWeek>\n");
    s.push_str("        <DaysOfWeek>\n");
    for &d in dows {
        s.push_str(&format!("          <{}/>\n", weekday_name(d)));
    }
    s.push_str("        </DaysOfWeek>\n");
    if include_months {
        s.push_str(&render_months(months));
    }
    s.push_str("      </ScheduleByWeek>\n");
    s
}

/// Render a `<Months>` block from month numbers (1-12).
fn render_months(months: &[u32]) -> String {
    let mut s = String::new();
    s.push_str("        <Months>\n");
    for &m in months {
        s.push_str(&format!("          <{}/>\n", month_name(m)));
    }
    s.push_str("        </Months>\n");
    s
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

/// Map a cron day-of-week value to the Task Scheduler weekday element name.
/// Cron uses 0=Sunday through 6=Saturday.
fn weekday_name(dow: u32) -> &'static str {
    match dow {
        0 => "Sunday",
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        6 => "Saturday",
        _ => "Sunday",
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

        let arguments = request.args.join(" ");
        let xml = render_task_xml(
            &trigger,
            &request.program.display().to_string(),
            &arguments,
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
            .run(
                "schtasks.exe",
                &["/Query", "/TN", TASK_FOLDER, "/FO", "CSV"],
            )
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
        // Daily at 9:00 with no weekday or day-of-month restriction.
        let trigger = TaskTrigger {
            minutes: vec![0],
            hours: vec![9],
            days_of_month: vec![],
            months: vec![],
            days_of_week: vec![],
        };
        let xml = render_task_xml(
            &trigger,
            r"C:\agent-iron.exe",
            "run task-1 --config C:\\config.db",
            true,
        );
        assert!(xml.contains("<Task"));
        assert!(xml.contains("<CalendarTrigger>"));
        assert!(xml.contains("<StartBoundary>2024-01-01T09:00:00</StartBoundary>"));
        // No day restriction: neither schedule element should be emitted.
        assert!(!xml.contains("<ScheduleByMonth>"));
        assert!(!xml.contains("<ScheduleByWeek>"));
        assert!(xml.contains("<Enabled>true</Enabled>"));
        assert!(xml.contains(r"C:\agent-iron.exe"));
        assert!(xml.contains("run task-1"));
    }

    #[test]
    fn render_xml_weekdays() {
        // `0 9 * * 1-5` — weekdays only at 9:00 AM.
        let trigger = TaskTrigger {
            minutes: vec![0],
            hours: vec![9],
            days_of_month: vec![],
            months: vec![],
            days_of_week: vec![1, 2, 3, 4, 5],
        };
        let xml = render_task_xml(&trigger, "agent-iron.exe", "run t1", true);
        assert!(xml.contains("<StartBoundary>2024-01-01T09:00:00</StartBoundary>"));
        assert!(xml.contains("<ScheduleByWeek>"));
        assert!(xml.contains("<DaysOfWeek>"));
        assert!(xml.contains("<Monday/>"));
        assert!(xml.contains("<Friday/>"));
        assert!(!xml.contains("<Sunday/>"));
        assert!(!xml.contains("<ScheduleByMonth>"));
    }

    #[test]
    fn render_xml_dom_and_months() {
        // Day of month and month restriction, no weekday restriction.
        let trigger = TaskTrigger {
            minutes: vec![30],
            hours: vec![5],
            days_of_month: vec![1, 15],
            months: vec![1, 6],
            days_of_week: vec![],
        };
        let xml = render_task_xml(&trigger, "agent-iron.exe", "run t1", true);
        assert!(xml.contains("<StartBoundary>2024-01-01T05:30:00</StartBoundary>"));
        assert!(xml.contains("<ScheduleByMonth>"));
        assert!(xml.contains("<Day>1</Day>"));
        assert!(xml.contains("<Day>15</Day>"));
        assert!(xml.contains("<January/>"));
        assert!(xml.contains("<June/>"));
        assert!(!xml.contains("<ScheduleByWeek>"));
    }

    #[test]
    fn render_xml_dom_and_dow_emits_two_triggers() {
        // Both DOM and DOW restricted — cron OR semantics produce two triggers.
        let trigger = TaskTrigger {
            minutes: vec![0],
            hours: vec![12],
            days_of_month: vec![1],
            months: vec![],
            days_of_week: vec![1],
        };
        let xml = render_task_xml(&trigger, "agent-iron.exe", "run t1", true);
        // Exactly two CalendarTrigger blocks for the single time pair.
        let trigger_count = xml.matches("<CalendarTrigger>").count();
        assert_eq!(trigger_count, 2);
        assert!(xml.contains("<ScheduleByMonth>"));
        assert!(xml.contains("<ScheduleByWeek>"));
        assert!(xml.contains("<Monday/>"));
        assert!(xml.contains("<Day>1</Day>"));
    }

    #[test]
    fn render_xml_multiple_times() {
        // Two times expand to two triggers.
        let trigger = TaskTrigger {
            minutes: vec![0, 30],
            hours: vec![9],
            days_of_month: vec![],
            months: vec![],
            days_of_week: vec![],
        };
        let xml = render_task_xml(&trigger, "agent-iron.exe", "run t1", true);
        assert!(xml.contains("<StartBoundary>2024-01-01T09:00:00</StartBoundary>"));
        assert!(xml.contains("<StartBoundary>2024-01-01T09:30:00</StartBoundary>"));
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
        let xml = render_task_xml(&trigger, "agent-iron.exe", "run t1", false);
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
        let xml = render_task_xml(&trigger, "normal", "arg <test> & stuff", true);
        assert!(xml.contains("&lt;test&gt;"));
        assert!(xml.contains("&amp; stuff"));
    }

    #[test]
    fn task_path_format() {
        assert_eq!(task_path("s1"), r"\AgentIron\Tasks\s1");
    }
}
