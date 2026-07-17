//! Windows Task Scheduler host scheduler adapter.
//!
//! Manages tasks under `\AgentIron\Tasks\<id>` using generated Task Scheduler
//! XML. Uses `schtasks.exe` for lifecycle operations. Cron expressions are
//! compiled into calendar triggers, with separate monthly and weekly triggers
//! preserving cron's day-of-month OR weekday semantics. Expansion rejects
//! definitions whose represented field values exceed [`MAX_TRIGGERS`].
//! Inspection recovers enabled and command state from XML but does not
//! reconstruct the original cron expression.
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
///
/// Values are expanded numeric cron matches. Rendering emits one Windows
/// calendar trigger per minute/hour pair and preserves cron's day-of-month OR
/// day-of-week behavior by emitting separate monthly and weekly triggers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskTrigger {
    /// Matching minutes in `0..=59`.
    pub minutes: Vec<u32>,
    /// Matching hours in `0..=23`.
    pub hours: Vec<u32>,
    /// Matching days of month in `1..=31`.
    pub days_of_month: Vec<u32>,
    /// Matching months in `1..=12`.
    pub months: Vec<u32>,
    /// Matching cron weekdays, where `0` is Sunday and `6` is Saturday.
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
/// semantics), two independent triggers are emitted. When neither day field
/// is restricted, an unrestricted month field renders as `<ScheduleByDay>`
/// and a month restriction renders as `<ScheduleByMonth>` covering every day
/// of the selected months.
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
            // No day-of-week or day-of-month restriction: a daily trigger,
            // optionally constrained to a set of months.
            if !dom_restricted && !dow_restricted {
                let body = if months_restricted {
                    let all_days: Vec<u32> = (1..=31).collect();
                    render_schedule_by_month(&all_days, true, &trigger.months)
                } else {
                    render_schedule_by_day()
                };
                xml.push_str(&render_calendar_trigger(&boundary, &body));
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
/// schedule body.
fn render_calendar_trigger(boundary: &str, schedule_body: &str) -> String {
    let mut s = String::new();
    s.push_str("    <CalendarTrigger>\n");
    s.push_str(&format!(
        "      <StartBoundary>{}</StartBoundary>\n",
        boundary
    ));
    debug_assert!(!schedule_body.is_empty());
    if !schedule_body.is_empty() {
        s.push_str(schedule_body);
    }
    s.push_str("    </CalendarTrigger>\n");
    s
}

/// Render a `<ScheduleByDay>` body with a one-day interval.
fn render_schedule_by_day() -> String {
    let mut s = String::new();
    s.push_str("      <ScheduleByDay>\n");
    s.push_str("        <DaysInterval>1</DaysInterval>\n");
    s.push_str("      </ScheduleByDay>\n");
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

/// Extract the text between the first occurrence of `open` and its matching
/// `close` tag in `text`.
fn extract_xml_block<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(&text[start..end])
}

/// Reverse the entity escaping applied by [`escape_xml`].
fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

/// Parse a Task Scheduler XML document into `(enabled, corrupt, observed_command)`.
///
/// Returns `None` if the document does not resemble a task definition
/// (missing `<Task` or `<Actions>`), indicating corruption. When `Some` is
/// returned, `corrupt` is always `false`.
fn parse_task_xml(xml_text: &str) -> Option<(bool, bool, Option<String>)> {
    if !xml_text.contains("<Task") || !xml_text.contains("<Actions") {
        return None;
    }

    let enabled = extract_xml_block(xml_text, "<Settings>", "</Settings>")
        .map(|block| block.contains("<Enabled>true</Enabled>"))
        .unwrap_or(true);

    let observed_command = extract_xml_block(xml_text, "<Exec>", "</Exec>").and_then(|exec| {
        let command = extract_xml_block(exec, "<Command>", "</Command>").map(unescape_xml);
        let arguments = extract_xml_block(exec, "<Arguments>", "</Arguments>").map(unescape_xml);
        match (command, arguments) {
            (Some(c), Some(a)) if !a.is_empty() => Some(format!("{c} {a}")),
            (Some(c), _) => Some(c),
            (None, Some(a)) => Some(a),
            (None, None) => None,
        }
    });

    Some((enabled, false, observed_command))
}

/// Extract a schedule id from a CSV task-list line containing the AgentIron
/// task folder.
fn schedule_id_from_csv_line(line: &str) -> Option<&str> {
    let idx = line.find(TASK_FOLDER)?;
    let after = &line[idx + TASK_FOLDER.len()..];
    let end = after.find([',', '"']).unwrap_or(after.len());
    Some(&after[..end])
}

// ============================================================================
// Adapter
// ============================================================================

/// Windows Task Scheduler host scheduler.
pub struct TaskSchedulerHostScheduler {
    runner: Box<dyn CommandRunner>,
}

impl TaskSchedulerHostScheduler {
    /// Create a Windows Task Scheduler adapter using the supplied command runner.
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

        // Write XML as UTF-16LE with BOM (required by schtasks.exe).
        let mut xml_bytes = Vec::with_capacity(xml.len() * 2 + 2);
        xml_bytes.extend_from_slice(&[0xFF, 0xFE]); // UTF-16LE BOM
        for unit in xml.encode_utf16() {
            xml_bytes.extend_from_slice(&unit.to_le_bytes());
        }
        tokio::fs::write(&temp_file, &xml_bytes)
            .await
            .map_err(|e| HostSchedulerError::Io(e.to_string()))?;

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
        let output = self
            .runner
            .run("schtasks.exe", &["/Delete", "/TN", &path, "/F"])
            .await
            .map_err(|e| HostSchedulerError::Io(e.to_string()))?;

        if output.exit_code != 0 {
            return Err(HostSchedulerError::Io(format!(
                "schtasks /Delete failed: {}",
                output.stderr
            )));
        }

        Ok(())
    }

    async fn list_owned(&self) -> Result<Vec<ObservedHostEntry>, HostSchedulerError> {
        let output = match self
            .runner
            .run("schtasks.exe", &["/Query", "/FO", "CSV", "/NH"])
            .await
        {
            Ok(o) => o,
            Err(_) => return Ok(Vec::new()),
        };

        if output.exit_code != 0 {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        for line in output.stdout.lines() {
            if !line.contains(TASK_FOLDER) {
                continue;
            }
            if let Some(id) = schedule_id_from_csv_line(line) {
                if let Some(entry) = self.inspect(id).await? {
                    entries.push(entry);
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
            .run("schtasks.exe", &["/Query", "/TN", &path, "/XML"])
            .await
            .map_err(|e| HostSchedulerError::Io(e.to_string()))?;

        if output.exit_code != 0 {
            return Ok(None);
        }

        let (enabled, corrupt, observed_command) =
            parse_task_xml(&output.stdout).unwrap_or((true, true, None));

        Ok(Some(ObservedHostEntry {
            schedule_id: schedule_id.to_string(),
            enabled,
            corrupt,
            raw_schedule: None,
            observed_command,
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
        // No day restriction: daily schedule body, not monthly or weekly.
        assert!(xml.contains("<ScheduleByDay>"));
        assert!(!xml.contains("<ScheduleByMonth>"));
        assert!(!xml.contains("<ScheduleByWeek>"));
        assert!(xml.contains("<Enabled>true</Enabled>"));
        assert!(xml.contains(r"C:\agent-iron.exe"));
        assert!(xml.contains("run task-1"));
    }

    #[test]
    fn render_xml_daily_includes_schedule_by_day() {
        let cron = CronExpression::parse("0 3 * * *").unwrap();
        let trigger = expand_cron(&cron).unwrap();
        let xml = render_task_xml(&trigger, "agent-iron.exe", "run t1", true);
        assert!(xml.contains("<StartBoundary>2024-01-01T03:00:00</StartBoundary>"));
        assert!(xml.contains("<ScheduleByDay>"));
        assert!(xml.contains("<DaysInterval>1</DaysInterval>"));
        assert!(!xml.contains("<ScheduleByMonth>"));
        assert!(!xml.contains("<ScheduleByWeek>"));
    }

    #[test]
    fn render_xml_month_restricted_daily_uses_schedule_by_month() {
        let cron = CronExpression::parse("0 3 * 6 *").unwrap();
        let trigger = expand_cron(&cron).unwrap();
        let xml = render_task_xml(&trigger, "agent-iron.exe", "run t1", true);
        assert!(xml.contains("<ScheduleByMonth>"));
        for day in 1..=31 {
            assert!(
                xml.contains(&format!("<Day>{day}</Day>")),
                "missing day {day}"
            );
        }
        assert!(xml.contains("<June/>"));
        assert!(!xml.contains("<July/>"));
        assert!(!xml.contains("<ScheduleByDay>"));
        assert!(!xml.contains("<ScheduleByWeek>"));
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

    /// Deletes a disposable root task and its temporary XML file. Cleanup
    /// failures are fatal unless the test is already unwinding, in which case
    /// they are reported on stderr.
    struct NativeProbeCleanup {
        task_name: String,
        xml_path: std::path::PathBuf,
        registered: bool,
    }

    impl Drop for NativeProbeCleanup {
        fn drop(&mut self) {
            let mut failures = Vec::new();
            if self.registered {
                match std::process::Command::new("schtasks.exe")
                    .args(["/Delete", "/TN", &self.task_name, "/F"])
                    .output()
                {
                    Ok(out) if out.status.success() => {}
                    Ok(out) => failures.push(format!(
                        "schtasks /Delete exited with {}: {}",
                        out.status,
                        String::from_utf8_lossy(&out.stderr)
                    )),
                    Err(e) => failures.push(format!("failed to run schtasks /Delete: {e}")),
                }
            }
            if let Err(e) = std::fs::remove_file(&self.xml_path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    failures.push(format!("failed to remove {}: {e}", self.xml_path.display()));
                }
            }
            if !failures.is_empty() {
                let msg = format!("native probe cleanup failed: {}", failures.join("; "));
                if std::thread::panicking() {
                    eprintln!("{msg}");
                } else {
                    panic!("{msg}");
                }
            }
        }
    }

    #[test]
    fn native_schtasks_accepts_generated_daily_xml() {
        if std::env::var("AGENTIRON_RUN_NATIVE_SCHEDULER_TESTS").as_deref() != Ok("1") {
            eprintln!(
                "skipping native schtasks probe; set AGENTIRON_RUN_NATIVE_SCHEDULER_TESTS=1 to enable"
            );
            return;
        }

        for cron_text in ["0 3 * * *", "0 3 * 6 *"] {
            let cron = CronExpression::parse(cron_text).unwrap();
            let trigger = expand_cron(&cron).unwrap();
            let xml = render_task_xml(&trigger, "cmd.exe", "/c exit 0", false);

            let unique = format!(
                "AgentIronXmlProbe-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let task_name = format!("\\{unique}");
            let xml_path = std::env::temp_dir().join(format!("{unique}.xml"));

            // Same UTF-16LE-with-BOM encoding the adapter uses for schtasks.
            let mut bytes = Vec::with_capacity(xml.len() * 2 + 2);
            bytes.extend_from_slice(&[0xFF, 0xFE]);
            for unit in xml.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            std::fs::write(&xml_path, &bytes).expect("write native probe XML");

            let mut cleanup = NativeProbeCleanup {
                task_name: task_name.clone(),
                xml_path: xml_path.clone(),
                registered: false,
            };

            let output = std::process::Command::new("schtasks.exe")
                .args(["/Create", "/TN", &task_name, "/XML"])
                .arg(&xml_path)
                .arg("/F")
                .output()
                .expect("run schtasks /Create");
            assert!(
                output.status.success(),
                "schtasks /Create rejected generated XML for `{cron_text}`: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            cleanup.registered = true;
        }
    }
}
