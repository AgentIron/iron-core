//! macOS launchd host scheduler adapter.
//!
//! Manages user-level LaunchAgent plists with `com.agentiron.task.` labels.
//! Cron expressions are expanded into launchd `StartCalendarInterval`
//! entries. plist files live in `~/Library/LaunchAgents/`.
//!
//! The plist rendering and cron expansion logic is platform-independent
//! and tested here. The actual `launchctl` command execution is Linux
//! unavailable but the adapter is designed for correctness on macOS.

use async_trait::async_trait;
use std::path::PathBuf;

use crate::scheduled_task::cron::CronExpression;
use crate::scheduled_task::host::{
    CommandRunner, HostInstallRequest, HostScheduler, HostSchedulerError, ObservedHostEntry,
};

/// Escape XML special characters.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Label prefix for AgentIron launchd entries.
pub const LABEL_PREFIX: &str = "com.agentiron.task.";

/// Maximum number of StartCalendarInterval entries before rejecting.
/// Prevents combinatorial explosion from complex cron expressions.
pub const MAX_INTERVALS: usize = 64;

// ============================================================================
// Cron to StartCalendarInterval expansion
// ============================================================================

/// A single launchd calendar interval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarInterval {
    pub minute: Option<u32>,
    pub hour: Option<u32>,
    pub day_of_month: Option<u32>,
    pub month: Option<u32>,
    pub day_of_week: Option<u32>,
}

/// Expand a parsed cron expression into launchd calendar intervals.
///
/// Returns `Err` if the expansion would exceed `MAX_INTERVALS` entries or
/// if the expression cannot be faithfully represented.
pub fn expand_cron(cron: &CronExpression) -> Result<Vec<CalendarInterval>, String> {
    let minutes = cron.minutes();
    let hours = cron.hours();
    let doms = cron.days_of_month();
    let months = cron.months();
    let dows = cron.days_of_week();

    let dom_restricted = doms.len() < 31;
    let dow_restricted = dows.len() < 7;
    let month_restricted = months.len() < 12;

    // Wildcard minute/hour fields produce None (match any).
    let minute_vals: Vec<Option<u32>> = if minutes.len() < 60 {
        minutes.iter().copied().map(Some).collect()
    } else {
        vec![None]
    };
    let hour_vals: Vec<Option<u32>> = if hours.len() < 24 {
        hours.iter().copied().map(Some).collect()
    } else {
        vec![None]
    };
    let month_vals: Vec<Option<u32>> = if month_restricted {
        months.iter().copied().map(Some).collect()
    } else {
        vec![None]
    };

    let mut intervals = Vec::new();

    if dom_restricted && dow_restricted {
        // Cron DOM-or-DOW: fire when either matches.
        // Set 1: specific DOM, DOW wildcard.
        for &m_val in &minute_vals {
            for &h_val in &hour_vals {
                for &dom in doms {
                    for &mo in &month_vals {
                        intervals.push(CalendarInterval {
                            minute: m_val,
                            hour: h_val,
                            day_of_month: Some(dom),
                            month: mo,
                            day_of_week: None,
                        });
                    }
                }
            }
        }
        // Set 2: DOM wildcard, specific DOW.
        for &m_val in &minute_vals {
            for &h_val in &hour_vals {
                for &dow in dows {
                    for &mo in &month_vals {
                        intervals.push(CalendarInterval {
                            minute: m_val,
                            hour: h_val,
                            day_of_month: None,
                            month: mo,
                            day_of_week: Some(if dow == 0 { 7 } else { dow }),
                        });
                    }
                }
            }
        }
    } else {
        // Simple case: iterate only over restricted fields.
        let dom_vals: Vec<Option<u32>> = if dom_restricted {
            doms.iter().copied().map(Some).collect()
        } else {
            vec![None]
        };
        let dow_vals: Vec<Option<u32>> = if dow_restricted {
            dows.iter()
                .copied()
                .map(|d| Some(if d == 0 { 7 } else { d }))
                .collect()
        } else {
            vec![None]
        };

        for &m_val in &minute_vals {
            for &h_val in &hour_vals {
                for &dom in &dom_vals {
                    for &mo in &month_vals {
                        for &dow in &dow_vals {
                            intervals.push(CalendarInterval {
                                minute: m_val,
                                hour: h_val,
                                day_of_month: dom,
                                month: mo,
                                day_of_week: dow,
                            });
                        }
                    }
                }
            }
        }
    }

    intervals.sort_by_key(|i| (i.minute, i.hour, i.day_of_month, i.month, i.day_of_week));
    intervals.dedup();

    if intervals.len() > MAX_INTERVALS {
        return Err(format!(
            "cron expression expands to {} launchd intervals (max {}); \
             use a simpler expression",
            intervals.len(),
            MAX_INTERVALS
        ));
    }

    Ok(intervals)
}

// ============================================================================
// Plist rendering
// ============================================================================

/// Render a complete LaunchAgent plist for the given parameters.
pub fn render_plist(
    schedule_id: &str,
    intervals: &[CalendarInterval],
    program_args: &[String],
    disabled: bool,
) -> String {
    let label = format!("{}{}", LABEL_PREFIX, escape_xml(schedule_id));

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
    xml.push_str("<plist version=\"1.0\">\n");
    xml.push_str("<dict>\n");

    xml.push_str(&format!(
        "    <key>Label</key>\n    <string>{}</string>\n",
        label
    ));

    if disabled {
        xml.push_str("    <key>Disabled</key>\n    <true/>\n");
    }

    xml.push_str("    <key>ProgramArguments</key>\n    <array>\n");
    for arg in program_args {
        xml.push_str(&format!("        <string>{}</string>\n", escape_xml(arg)));
    }
    xml.push_str("    </array>\n");

    xml.push_str("    <key>StartCalendarInterval</key>\n");
    if intervals.len() == 1 {
        xml.push_str("    <dict>\n");
        push_interval_keys(&mut xml, &intervals[0], "        ");
        xml.push_str("    </dict>\n");
    } else {
        xml.push_str("    <array>\n");
        for interval in intervals {
            xml.push_str("        <dict>\n");
            push_interval_keys(&mut xml, interval, "            ");
            xml.push_str("        </dict>\n");
        }
        xml.push_str("    </array>\n");
    }

    xml.push_str("</dict>\n");
    xml.push_str("</plist>\n");
    xml
}

fn push_interval_keys(xml: &mut String, interval: &CalendarInterval, indent: &str) {
    if let Some(m) = interval.minute {
        xml.push_str(&format!(
            "{}<key>Minute</key>\n{}<integer>{}</integer>\n",
            indent, indent, m
        ));
    }
    if let Some(h) = interval.hour {
        xml.push_str(&format!(
            "{}<key>Hour</key>\n{}<integer>{}</integer>\n",
            indent, indent, h
        ));
    }
    if let Some(d) = interval.day_of_month {
        xml.push_str(&format!(
            "{}<key>Day</key>\n{}<integer>{}</integer>\n",
            indent, indent, d
        ));
    }
    if let Some(mo) = interval.month {
        xml.push_str(&format!(
            "{}<key>Month</key>\n{}<integer>{}</integer>\n",
            indent, indent, mo
        ));
    }
    if let Some(dow) = interval.day_of_week {
        xml.push_str(&format!(
            "{}<key>Weekday</key>\n{}<integer>{}</integer>\n",
            indent, indent, dow
        ));
    }
}

/// Extract the `<string>` values inside a plist's `ProgramArguments` array.
///
/// Returns an empty vector if the key or array cannot be located.
fn parse_program_arguments(text: &str) -> Vec<String> {
    let key = "<key>ProgramArguments</key>";
    let key_pos = match text.find(key) {
        Some(p) => p + key.len(),
        None => return Vec::new(),
    };
    let after_key = &text[key_pos..];
    let array_open = match after_key.find("<array>") {
        Some(p) => key_pos + p + "<array>".len(),
        None => return Vec::new(),
    };
    let array_close = match text[array_open..].find("</array>") {
        Some(p) => array_open + p,
        None => return Vec::new(),
    };
    let body = &text[array_open..array_close];

    let mut args = Vec::new();
    let mut cursor = body;
    while let Some(open) = cursor.find("<string>") {
        let value_start = open + "<string>".len();
        let rest = &cursor[value_start..];
        let close = match rest.find("</string>") {
            Some(p) => p,
            None => break,
        };
        args.push(rest[..close].to_string());
        cursor = &rest[close + "</string>".len()..];
    }
    args
}

// ============================================================================
// Adapter
// ============================================================================

/// macOS launchd host scheduler.
pub struct LaunchdHostScheduler {
    runner: Box<dyn CommandRunner>,
    launchagents_dir: PathBuf,
    uid: u32,
}

impl LaunchdHostScheduler {
    pub fn new(runner: Box<dyn CommandRunner>, launchagents_dir: PathBuf) -> Self {
        let uid = extern_uid();
        Self {
            runner,
            launchagents_dir,
            uid,
        }
    }

    fn plist_path(&self, schedule_id: &str) -> PathBuf {
        self.launchagents_dir
            .join(format!("{}{}.plist", LABEL_PREFIX, schedule_id))
    }

    fn domain_target(&self) -> String {
        format!("gui/{}", self.uid)
    }

    fn service_target(&self, schedule_id: &str) -> String {
        format!("{}/{}{}", self.domain_target(), LABEL_PREFIX, schedule_id)
    }

    /// Bootout (unload) the agent if currently loaded.
    async fn bootout_if_loaded(&self, schedule_id: &str) {
        let target = self.service_target(schedule_id);
        let _ = self.runner.run("launchctl", &["bootout", &target]).await;
    }

    /// Bootstrap (load) the agent from its plist.
    async fn bootstrap(&self, schedule_id: &str) -> Result<(), HostSchedulerError> {
        let plist_path = self.plist_path(schedule_id);
        let path_str = plist_path.to_string_lossy().to_string();
        let domain = self.domain_target();

        let output = self
            .runner
            .run("launchctl", &["bootstrap", &domain, &path_str])
            .await
            .map_err(|e| HostSchedulerError::Io(e.to_string()))?;

        if output.exit_code != 0 && !output.stderr.contains("already") {
            return Err(HostSchedulerError::Io(format!(
                "launchctl bootstrap failed: {}",
                output.stderr
            )));
        }

        Ok(())
    }

    /// Read and parse a schedule's plist file into an observed entry.
    ///
    /// Returns `None` if the file cannot be read or reports a non-zero exit.
    /// The XML text is parsed to determine enabled/disabled state, extract the
    /// program command, and flag corruption.
    async fn read_plist_entry(&self, schedule_id: &str) -> Option<ObservedHostEntry> {
        let plist_path = self.plist_path(schedule_id);
        let path_str = plist_path.to_string_lossy().to_string();

        let output = self.runner.run("cat", &[&path_str]).await.ok()?;
        if output.exit_code != 0 {
            return None;
        }

        let text = &output.stdout;

        // A valid launchd plist has a <plist> root and a Label key.
        let corrupt = !text.contains("<plist") || !text.contains("<key>Label</key>");

        // Disabled is true when <true/> is the value following the Disabled key,
        // i.e. it appears before the next sibling <key>.
        let disabled = text.find("<key>Disabled</key>").map(|pos| {
            let after = &text[pos + "<key>Disabled</key>".len()..];
            match (after.find("<true/>"), after.find("<key>")) {
                (Some(t), Some(k)) => t < k,
                (Some(_), None) => true,
                _ => false,
            }
        });

        let command = parse_program_arguments(text).join(" ");

        Some(ObservedHostEntry {
            schedule_id: schedule_id.to_string(),
            enabled: !disabled.unwrap_or(false),
            corrupt,
            raw_schedule: None,
            observed_command: Some(command),
            metadata: None,
        })
    }
}

/// Get the current process UID without external crate dependencies.
fn extern_uid() -> u32 {
    extern "C" {
        fn getuid() -> u32;
    }
    unsafe { getuid() }
}

#[async_trait]
impl HostScheduler for LaunchdHostScheduler {
    fn platform(&self) -> &'static str {
        "launchd"
    }

    async fn install(&self, request: &HostInstallRequest) -> Result<(), HostSchedulerError> {
        let intervals = expand_cron(&request.cron).map_err(|reason| {
            HostSchedulerError::UnsupportedSchedule {
                platform: "launchd",
                reason,
            }
        })?;

        let mut program_args = vec![request.program.display().to_string()];
        program_args.extend(request.args.iter().cloned());

        let plist_content = render_plist(
            &request.schedule_id,
            &intervals,
            &program_args,
            !request.enabled,
        );

        // Write plist file.
        let plist_path = self.plist_path(&request.schedule_id);
        let path_str = plist_path.to_string_lossy().to_string();

        let output = self
            .runner
            .run_with_stdin("tee", &[&path_str], &plist_content)
            .await
            .map_err(|e| HostSchedulerError::Io(e.to_string()))?;

        if output.exit_code != 0 {
            return Err(HostSchedulerError::Io(format!(
                "failed to write plist: {}",
                output.stderr
            )));
        }

        // Reload: bootout if loaded, then bootstrap.
        self.bootout_if_loaded(&request.schedule_id).await;
        self.bootstrap(&request.schedule_id).await?;

        // Apply enabled/disabled state via launchctl.
        if !request.enabled {
            let target = self.service_target(&request.schedule_id);
            let _ = self.runner.run("launchctl", &["disable", &target]).await;
        }

        Ok(())
    }

    async fn remove(&self, schedule_id: &str) -> Result<(), HostSchedulerError> {
        // Unload the agent first.
        self.bootout_if_loaded(schedule_id).await;

        // Then delete the plist file.
        let plist_path = self.plist_path(schedule_id);
        let path_str = plist_path.to_string_lossy().to_string();

        let output = self
            .runner
            .run("rm", &["-f", &path_str])
            .await
            .map_err(|e| HostSchedulerError::Io(e.to_string()))?;

        if output.exit_code != 0 {
            return Err(HostSchedulerError::Io(format!(
                "failed to remove plist: {}",
                output.stderr
            )));
        }

        Ok(())
    }

    async fn list_owned(&self) -> Result<Vec<ObservedHostEntry>, HostSchedulerError> {
        let dir_str = self.launchagents_dir.to_string_lossy().to_string();
        let output = self
            .runner
            .run("ls", &["-1", &dir_str])
            .await
            .map_err(|e| HostSchedulerError::Io(e.to_string()))?;

        let mut entries = Vec::new();
        for line in output.stdout.lines() {
            if let Some(schedule_id) = line
                .strip_prefix(&format!("{}{}", LABEL_PREFIX, ""))
                .and_then(|s| s.strip_suffix(".plist"))
            {
                if let Some(entry) = self.read_plist_entry(schedule_id).await {
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
        Ok(self.read_plist_entry(schedule_id).await)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_simple_daily() {
        let cron = CronExpression::parse("0 9 * * *").unwrap();
        let intervals = expand_cron(&cron).unwrap();
        assert_eq!(intervals.len(), 1);
        assert_eq!(intervals[0].minute, Some(0));
        assert_eq!(intervals[0].hour, Some(9));
    }

    #[test]
    fn expand_every_15_minutes() {
        let cron = CronExpression::parse("*/15 * * * *").unwrap();
        let intervals = expand_cron(&cron).unwrap();
        assert_eq!(intervals.len(), 4);
        assert_eq!(intervals[0].minute, Some(0));
        assert_eq!(intervals[1].minute, Some(15));
    }

    #[test]
    fn expand_weekdays() {
        let cron = CronExpression::parse("0 9 * * 1-5").unwrap();
        let intervals = expand_cron(&cron).unwrap();
        assert!(intervals.len() >= 5);
    }

    #[test]
    fn expand_dom_and_dow_creates_union() {
        let cron = CronExpression::parse("0 9 1 * 1").unwrap();
        let intervals = expand_cron(&cron).unwrap();
        // DOM=1 OR DOW=1 → at least 2 intervals
        assert!(intervals.len() >= 2);
    }

    #[test]
    fn expand_rejects_excessive() {
        // Complex expression that exceeds MAX_INTERVALS (64).
        // 4 minutes × 5 hours × 2 DOM × 2 DOW (DOM-and-DOW union doubles) = ~80+
        let cron = CronExpression::parse("0,15,30,45 9,10,11,12,13 1,15 * 1,2,3,4,5").unwrap();
        let result = expand_cron(&cron);
        assert!(result.is_err());
    }

    #[test]
    fn render_plist_basic() {
        let intervals = vec![CalendarInterval {
            minute: Some(0),
            hour: Some(9),
            day_of_month: None,
            month: None,
            day_of_week: None,
        }];
        let args = vec![
            "/usr/local/bin/agent-iron".to_string(),
            "run".to_string(),
            "task-1".to_string(),
        ];
        let plist = render_plist("s1", &intervals, &args, false);
        assert!(plist.contains("<string>com.agentiron.task.s1</string>"));
        assert!(plist.contains("<key>Minute</key>"));
        assert!(plist.contains("<key>Hour</key>"));
        assert!(plist.contains("<integer>0</integer>"));
        assert!(plist.contains("<integer>9</integer>"));
        assert!(!plist.contains("<key>Disabled</key>"));
    }

    #[test]
    fn render_plist_disabled() {
        let intervals = vec![CalendarInterval {
            minute: Some(30),
            hour: Some(5),
            day_of_month: None,
            month: None,
            day_of_week: None,
        }];
        let args = vec!["/agent-iron".to_string(), "run".to_string()];
        let plist = render_plist("s1", &intervals, &args, true);
        assert!(plist.contains("<key>Disabled</key>"));
        assert!(plist.contains("<true/>"));
    }

    #[test]
    fn render_plist_multiple_intervals() {
        let intervals = vec![
            CalendarInterval {
                minute: Some(0),
                hour: Some(9),
                day_of_month: None,
                month: None,
                day_of_week: None,
            },
            CalendarInterval {
                minute: Some(0),
                hour: Some(17),
                day_of_month: None,
                month: None,
                day_of_week: None,
            },
        ];
        let args = vec!["/agent-iron".to_string()];
        let plist = render_plist("s1", &intervals, &args, false);
        assert!(plist.contains("<array>"));
    }
}
