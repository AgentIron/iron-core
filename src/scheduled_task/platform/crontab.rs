//! Crontab content parsing, rendering, and ownership management.
//!
//! AgentIron owns crontab entries via marker-delimited blocks:
//!
//! ```text
//! # iron-core-task:<id>:begin
//! 0 9 * * * /path/to/agent-iron run task-1 --config /path/to/config.db
//! # iron-core-task:<id>:end
//! ```
//!
//! Disabled tasks retain the block but comment out the schedule line:
//!
//! ```text
//! # iron-core-task:<id>:begin
//! # iron-core-task:<id>:disabled
//! # 0 9 * * * /path/to/agent-iron run task-1 --config /path/to/config.db
//! # iron-core-task:<id>:end
//! ```
//!
//! Non-owned crontab content is preserved byte-for-byte.

use std::collections::HashSet;

// ============================================================================
// Constants
// ============================================================================

/// Prefix for ownership marker comments.
pub const MARKER_PREFIX: &str = "# iron-core-task:";

// ============================================================================
// Owned block
// ============================================================================

/// A parsed owned crontab block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronOwnedBlock {
    pub schedule_id: String,
    pub enabled: bool,
    /// The cron schedule fields (e.g. `"0 9 * * *"`).
    pub cron_schedule: String,
    /// The full command line.
    pub command: String,
}

impl CronOwnedBlock {
    /// Render the block as crontab text.
    pub fn render(&self) -> String {
        let begin = format!("{}{}:begin", MARKER_PREFIX, self.schedule_id);
        let end = format!("{}{}:end", MARKER_PREFIX, self.schedule_id);
        let cron_line = format!("{} {}", self.cron_schedule, self.command);

        if self.enabled {
            format!("{}\n{}\n{}\n", begin, cron_line, end)
        } else {
            format!(
                "{}\n{}{}:disabled\n# {}\n{}\n",
                begin,
                MARKER_PREFIX,
                self.schedule_id,
                cron_line,
                end
            )
        }
    }
}

/// Build an owned block from a schedule ID, cron text, command, and enabled
/// state.
pub fn make_block(
    schedule_id: &str,
    cron_text: &str,
    command: &str,
    enabled: bool,
) -> CronOwnedBlock {
    CronOwnedBlock {
        schedule_id: schedule_id.to_string(),
        enabled,
        cron_schedule: cron_text.to_string(),
        command: command.to_string(),
    }
}

// ============================================================================
// Crontab content
// ============================================================================

/// A parsed crontab with owned blocks separated from non-owned content.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCrontab {
    /// All segments in order.
    pub segments: Vec<CrontabSegment>,
}

/// A segment of the crontab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrontabSegment {
    /// A valid owned block.
    Owned(CronOwnedBlock),
    /// Non-owned lines, preserved as-is.
    Other(String),
    /// A malformed owned block (unbalanced or duplicate markers).
    Malformed {
        schedule_id: String,
        raw: String,
    },
}

impl ParsedCrontab {
    /// Parse raw crontab text into segments.
    pub fn parse(raw: &str) -> Self {
        let lines: Vec<&str> = raw.lines().collect();
        let mut segments = Vec::new();
        let mut other_buf = String::new();
        let mut seen_ids: HashSet<String> = HashSet::new();

        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];

            if let Some(sid) = parse_begin_marker(line) {
                // Flush pending other content.
                if !other_buf.is_empty() {
                    segments.push(CrontabSegment::Other(std::mem::take(&mut other_buf)));
                }

                // Find the matching end marker.
                let end_idx = find_end_marker(&lines, i + 1, &sid);

                match end_idx {
                    Some(ei) => {
                        let is_duplicate = !seen_ids.insert(sid.clone());
                        let block_lines = &lines[i + 1..ei];
                        let raw_block: String = lines[i..=ei]
                            .iter()
                            .copied()
                            .collect::<Vec<_>>()
                            .join("\n")
                            + "\n";

                        if is_duplicate {
                            segments.push(CrontabSegment::Malformed {
                                schedule_id: sid,
                                raw: raw_block,
                            });
                        } else if let Some(block) = parse_block_body(&sid, block_lines) {
                            segments.push(CrontabSegment::Owned(block));
                        } else {
                            segments.push(CrontabSegment::Malformed {
                                schedule_id: sid,
                                raw: raw_block,
                            });
                        }
                        i = ei + 1;
                    }
                    None => {
                        // No end marker — everything from here to EOF is malformed.
                        let raw_block: String = lines[i..]
                            .iter()
                            .copied()
                            .collect::<Vec<_>>()
                            .join("\n")
                            + "\n";
                        segments.push(CrontabSegment::Malformed {
                            schedule_id: sid,
                            raw: raw_block,
                        });
                        i = lines.len();
                    }
                }
            } else {
                // Non-owned line.
                other_buf.push_str(line);
                other_buf.push('\n');
                i += 1;
            }
        }

        if !other_buf.is_empty() {
            segments.push(CrontabSegment::Other(other_buf));
        }

        ParsedCrontab { segments }
    }

    /// Render the crontab back to text.
    pub fn render(&self) -> String {
        let mut result = String::new();
        for seg in &self.segments {
            match seg {
                CrontabSegment::Owned(block) => result.push_str(&block.render()),
                CrontabSegment::Other(text) => result.push_str(text),
                CrontabSegment::Malformed { raw, .. } => result.push_str(raw),
            }
        }
        result
    }

    /// Find an owned block by schedule ID.
    pub fn find_owned(&self, schedule_id: &str) -> Option<&CronOwnedBlock> {
        self.segments.iter().find_map(|s| match s {
            CrontabSegment::Owned(b) if b.schedule_id == schedule_id => Some(b),
            _ => None,
        })
    }

    /// Check if any segment (owned or malformed) references the given ID.
    pub fn contains_id(&self, schedule_id: &str) -> bool {
        self.segments.iter().any(|s| match s {
            CrontabSegment::Owned(b) => b.schedule_id == schedule_id,
            CrontabSegment::Malformed { schedule_id: sid, .. } => sid == schedule_id,
            _ => false,
        })
    }

    /// Insert or replace an owned block, preserving non-owned content.
    pub fn upsert(&mut self, block: CronOwnedBlock) {
        let mut new_segments = Vec::new();
        let mut inserted = false;

        for seg in self.segments.drain(..) {
            match &seg {
                CrontabSegment::Owned(existing) if existing.schedule_id == block.schedule_id => {
                    // Replace in place.
                    new_segments.push(CrontabSegment::Owned(block.clone()));
                    inserted = true;
                }
                CrontabSegment::Malformed {
                    schedule_id: sid, ..
                } if sid == &block.schedule_id => {
                    // Replace malformed block too.
                    new_segments.push(CrontabSegment::Owned(block.clone()));
                    inserted = true;
                }
                _ => {
                    new_segments.push(seg);
                }
            }
        }

        if !inserted {
            new_segments.push(CrontabSegment::Owned(block));
        }

        self.segments = new_segments;
    }

    /// Remove an owned block by schedule ID. Returns true if something was
    /// removed.
    pub fn remove(&mut self, schedule_id: &str) -> bool {
        let before = self.segments.len();
        self.segments.retain(|s| match s {
            CrontabSegment::Owned(b) => b.schedule_id != schedule_id,
            CrontabSegment::Malformed {
                schedule_id: sid, ..
            } => sid != schedule_id,
            _ => true,
        });
        self.segments.len() < before
    }

    /// List all valid owned blocks.
    pub fn owned_blocks(&self) -> Vec<&CronOwnedBlock> {
        self.segments
            .iter()
            .filter_map(|s| match s {
                CrontabSegment::Owned(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    /// List all malformed blocks.
    pub fn malformed_blocks(&self) -> Vec<(&String, &String)> {
        self.segments
            .iter()
            .filter_map(|s| match s {
                CrontabSegment::Malformed { schedule_id, raw } => {
                    Some((schedule_id, raw))
                }
                _ => None,
            })
            .collect()
    }
}

// ============================================================================
// Marker parsing helpers
// ============================================================================

/// Extract the schedule ID from a `# iron-core-task:<id>:begin` line.
fn parse_begin_marker(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with(MARKER_PREFIX) {
        return None;
    }
    let rest = &trimmed[MARKER_PREFIX.len()..];
    if let Some(id) = rest.strip_suffix(":begin") {
        let id = id.trim();
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }
    None
}

/// Find the line index of `# iron-core-task:<id>:end` starting from `from`.
fn find_end_marker(lines: &[&str], from: usize, id: &str) -> Option<usize> {
    let end_marker = format!("{}{}:end", MARKER_PREFIX, id);
    for (idx, line) in lines.iter().enumerate().skip(from) {
        if line.trim() == end_marker {
            return Some(idx);
        }
        // Detect nested begin of the same ID (malformed).
        if let Some(nested_id) = parse_begin_marker(line) {
            if nested_id == id {
                return None;
            }
        }
    }
    None
}

/// Parse the body of an owned block (lines between begin and end markers).
fn parse_block_body(schedule_id: &str, body_lines: &[&str]) -> Option<CronOwnedBlock> {
    if body_lines.is_empty() {
        return None;
    }

    let mut enabled = true;
    let mut cron_line: Option<String> = None;

    for line in body_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Check for disabled marker.
        let disabled_marker = format!("{}{}:disabled", MARKER_PREFIX, schedule_id);
        if trimmed == disabled_marker {
            enabled = false;
            continue;
        }

        // Check for commented cron line (disabled).
        if let Some(rest) = trimmed.strip_prefix("# ") {
            if looks_like_cron_line(rest) {
                if cron_line.is_some() {
                    return None; // Multiple cron lines — malformed.
                }
                cron_line = Some(rest.to_string());
                continue;
            }
        }

        // Active cron line.
        if looks_like_cron_line(trimmed) {
            if cron_line.is_some() {
                return None;
            }
            cron_line = Some(trimmed.to_string());
        }
    }

    let cron_line = cron_line?;

    // Split into schedule fields and command.
    let parts: Vec<&str> = cron_line.splitn(6, ' ').collect();
    if parts.len() < 6 {
        return None;
    }
    let cron_schedule = parts[..5].join(" ");
    let command = parts[5].to_string();

    Some(CronOwnedBlock {
        schedule_id: schedule_id.to_string(),
        enabled,
        cron_schedule,
        command,
    })
}

/// Heuristic: does a line look like a cron schedule entry?
fn looks_like_cron_line(line: &str) -> bool {
    let parts: Vec<&str> = line.split_whitespace().collect();
    parts.len() >= 6
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Block rendering ----

    #[test]
    fn render_enabled_block() {
        let block = make_block("morning", "0 9 * * *", "/usr/bin/agent-iron run t1", true);
        let text = block.render();
        assert!(text.contains("# iron-core-task:morning:begin"));
        assert!(text.contains("0 9 * * * /usr/bin/agent-iron run t1"));
        assert!(text.contains("# iron-core-task:morning:end"));
        assert!(!text.contains("disabled"));
    }

    #[test]
    fn render_disabled_block() {
        let block = make_block("morning", "0 9 * * *", "/usr/bin/agent-iron run t1", false);
        let text = block.render();
        assert!(text.contains("# iron-core-task:morning:begin"));
        assert!(text.contains("# iron-core-task:morning:disabled"));
        assert!(text.contains("# 0 9 * * * /usr/bin/agent-iron run t1"));
        assert!(text.contains("# iron-core-task:morning:end"));
    }

    // ---- Round-trip parse/render ----

    #[test]
    fn round_trip_enabled_block() {
        let block = make_block("s1", "0 9 * * *", "/agent-iron run t1 --config /c.db", true);
        let rendered = block.render();
        let parsed = ParsedCrontab::parse(&rendered);
        let found = parsed.find_owned("s1").unwrap();
        assert_eq!(found, &block);
    }

    #[test]
    fn round_trip_disabled_block() {
        let block = make_block("s1", "*/15 * * * *", "/agent-iron run t1", false);
        let rendered = block.render();
        let parsed = ParsedCrontab::parse(&rendered);
        let found = parsed.find_owned("s1").unwrap();
        assert_eq!(found, &block);
    }

    // ---- Mixed content ----

    #[test]
    fn parse_preserves_non_owned_content() {
        let raw = "# My crontab\n\
                   0 0 * * * /usr/bin/cleanup\n\
                   \n\
                   # iron-core-task:morning:begin\n\
                   0 9 * * * /agent-iron run t1\n\
                   # iron-core-task:morning:end\n\
                   \n\
                   30 5 * * 1 /usr/bin/backup\n";

        let parsed = ParsedCrontab::parse(raw);
        assert_eq!(parsed.owned_blocks().len(), 1);
        assert_eq!(parsed.malformed_blocks().len(), 0);

        // Non-owned content is preserved.
        let rendered = parsed.render();
        assert!(rendered.contains("0 0 * * * /usr/bin/cleanup"));
        assert!(rendered.contains("30 5 * * 1 /usr/bin/backup"));
        assert!(rendered.contains("# My crontab"));
    }

    #[test]
    fn parse_multiple_owned_blocks() {
        let raw = "# iron-core-task:a:begin\n\
                   0 9 * * * /agent-iron run a\n\
                   # iron-core-task:a:end\n\
                   # iron-core-task:b:begin\n\
                   0 10 * * * /agent-iron run b\n\
                   # iron-core-task:b:end\n";

        let parsed = ParsedCrontab::parse(raw);
        assert_eq!(parsed.owned_blocks().len(), 2);
    }

    #[test]
    fn parse_malformed_missing_end() {
        let raw = "# iron-core-task:bad:begin\n\
                   0 9 * * * /agent-iron run t1\n";

        let parsed = ParsedCrontab::parse(raw);
        assert_eq!(parsed.malformed_blocks().len(), 1);
        assert_eq!(parsed.owned_blocks().len(), 0);
    }

    #[test]
    fn parse_duplicate_id_marks_second_malformed() {
        let raw = "# iron-core-task:dup:begin\n\
                   0 9 * * * /agent-iron run t1\n\
                   # iron-core-task:dup:end\n\
                   # iron-core-task:dup:begin\n\
                   0 10 * * * /agent-iron run t2\n\
                   # iron-core-task:dup:end\n";

        let parsed = ParsedCrontab::parse(raw);
        assert_eq!(parsed.owned_blocks().len(), 1);
        assert_eq!(parsed.malformed_blocks().len(), 1);
    }

    #[test]
    fn parse_nested_begin_malformed() {
        let raw = "# iron-core-task:a:begin\n\
                   # iron-core-task:a:begin\n\
                   0 9 * * * /agent-iron run t1\n\
                   # iron-core-task:a:end\n";

        let parsed = ParsedCrontab::parse(raw);
        // The nested begin makes the first block malformed.
        assert_eq!(parsed.malformed_blocks().len(), 1);
    }

    // ---- Upsert / remove ----

    #[test]
    fn upsert_replaces_in_place() {
        let raw = "# iron-core-task:s1:begin\n\
                   0 9 * * * /agent-iron run old\n\
                   # iron-core-task:s1:end\n";

        let mut parsed = ParsedCrontab::parse(raw);
        let new_block = make_block("s1", "0 10 * * *", "/agent-iron run new", true);
        parsed.upsert(new_block);

        let found = parsed.find_owned("s1").unwrap();
        assert_eq!(found.cron_schedule, "0 10 * * *");
        assert_eq!(found.command, "/agent-iron run new");
    }

    #[test]
    fn upsert_appends_new_block() {
        let raw = "0 0 * * * /usr/bin/cleanup\n";
        let mut parsed = ParsedCrontab::parse(raw);
        let block = make_block("s1", "0 9 * * *", "/agent-iron run t1", true);
        parsed.upsert(block);

        assert_eq!(parsed.owned_blocks().len(), 1);
        assert!(parsed.render().contains("0 0 * * * /usr/bin/cleanup"));
    }

    #[test]
    fn remove_owned_block_preserves_others() {
        let raw = "# iron-core-task:a:begin\n\
                   0 9 * * * /agent-iron run a\n\
                   # iron-core-task:a:end\n\
                   # iron-core-task:b:begin\n\
                   0 10 * * * /agent-iron run b\n\
                   # iron-core-task:b:end\n";

        let mut parsed = ParsedCrontab::parse(raw);
        assert!(parsed.remove("a"));
        assert_eq!(parsed.owned_blocks().len(), 1);
        assert!(parsed.find_owned("a").is_none());
        assert!(parsed.find_owned("b").is_some());
    }

    #[test]
    fn remove_nonexistent_returns_false() {
        let mut parsed = ParsedCrontab::parse("# unrelated\n");
        assert!(!parsed.remove("nope"));
    }

    // ---- Edge cases ----

    #[test]
    fn empty_crontab() {
        let parsed = ParsedCrontab::parse("");
        assert!(parsed.owned_blocks().is_empty());
        assert!(parsed.malformed_blocks().is_empty());
    }

    #[test]
    fn no_owned_blocks() {
        let raw = "0 0 * * * /usr/bin/cleanup\n30 5 * * 1 /usr/bin/backup\n";
        let parsed = ParsedCrontab::parse(raw);
        assert!(parsed.owned_blocks().is_empty());
        assert_eq!(parsed.render(), raw);
    }

    #[test]
    fn render_preserves_trailing_newlines() {
        let raw = "0 0 * * * /cleanup\n\n";
        let parsed = ParsedCrontab::parse(raw);
        let rendered = parsed.render();
        assert!(rendered.ends_with("\n\n"));
    }
}
