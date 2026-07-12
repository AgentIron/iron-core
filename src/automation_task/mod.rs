//! Durable automation-task identity and validation.
//!
//! An `AutomationTask` is a reference to a `StoredPrompt` with its own
//! expected-outcome text, project root, and timeout. It is designed for GUI
//! management and scheduling. The task does not duplicate instructions,
//! skills, profile, provider, model, tools, or approval policy — those live
//! on the stored prompt and its optional profile.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Schema version for automation-task records.
pub const AUTOMATION_TASK_SCHEMA_VERSION: i64 = 2;

/// Maximum allowed execution timeout in seconds (24 hours).
pub const MAX_TIMEOUT_SECONDS: u64 = 86_400;

/// A durable automation-task identity.
///
/// Contains an immutable internal ID, a unique normalized lookup name,
/// a user-facing display name, a reference to a stored prompt, required
/// expected-outcome text, a canonical project root, a positive execution
/// timeout, and timestamps. Provider, model, tool, approval, skills, and
/// profile configuration are resolved indirectly through the referenced
/// stored prompt and its optional profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationTask {
    /// Immutable, case-sensitive task identifier.
    pub id: String,
    /// User-facing display name.
    pub display_name: String,
    /// Unique case-insensitive normalized name derived from `display_name`.
    pub normalized_name: String,
    /// ID of the referenced stored prompt.
    pub stored_prompt_id: String,
    /// Required prose describing the desired outcome (not independently verified).
    pub expected_outcome: String,
    /// Canonical project root directory used as the execution workspace.
    pub project_root: PathBuf,
    /// Positive bounded execution timeout in seconds.
    pub timeout_seconds: u64,
    /// When the task was first created.
    pub created_at: DateTime<Utc>,
    /// When the task was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or replacing an automation task.
///
/// On replacement, the original creation timestamp is preserved and the
/// update timestamp advances. The `display_name` is normalized into
/// `normalized_name` during validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationTaskInput {
    /// Immutable, case-sensitive task identifier.
    pub id: String,
    /// User-facing display name.
    pub display_name: String,
    /// ID of the referenced stored prompt (must exist at write time).
    pub stored_prompt_id: String,
    /// Required prose describing the desired outcome.
    pub expected_outcome: String,
    /// Project root directory (must be an existing directory at write time).
    pub project_root: PathBuf,
    /// Positive bounded execution timeout in seconds (1..=86_400).
    pub timeout_seconds: u64,
}

/// Validate and normalize an automation-task input.
///
/// Trims all text fields, derives `normalized_name` from `display_name`,
/// validates the timeout, and returns `Ok` with normalized values, or
/// `Err` with a human-readable message when a field is empty after
/// trimming, when a textual identifier contains control characters, or
/// when the timeout is outside the supported range.
pub fn validate_task_input(input: &AutomationTaskInput) -> Result<AutomationTaskInput, String> {
    let id = validate_text_id(&input.id, "Task ID")?;
    let display_name = validate_text_content(&input.display_name, "Display name")?;
    let normalized = normalize_task_name(&display_name);
    if normalized.is_empty() {
        return Err("Display name must contain alphanumeric characters".to_string());
    }
    let stored_prompt_id = validate_text_id(&input.stored_prompt_id, "Stored prompt ID")?;
    let expected_outcome = validate_text_content(&input.expected_outcome, "Expected outcome")?;

    let project_root_str = input.project_root.to_string_lossy();
    let project_root_trimmed = project_root_str.trim();
    if project_root_trimmed.is_empty() {
        return Err("Project root must not be empty".to_string());
    }

    if input.timeout_seconds == 0 {
        return Err("Timeout must be greater than zero".to_string());
    }
    if input.timeout_seconds > MAX_TIMEOUT_SECONDS {
        return Err(format!(
            "Timeout must not exceed {} seconds",
            MAX_TIMEOUT_SECONDS
        ));
    }

    Ok(AutomationTaskInput {
        id,
        display_name,
        stored_prompt_id,
        expected_outcome,
        project_root: PathBuf::from(project_root_trimmed),
        timeout_seconds: input.timeout_seconds,
    })
}

/// Normalize a user-facing task name into a stable lookup handle.
///
/// Lowercases the input, replaces whitespace with hyphens, drops all other
/// non-alphanumeric characters, and collapses consecutive hyphens.
///
/// Examples:
/// - `Check Email` → `check-email`
/// - `Paul's daily brief` → `pauls-daily-brief`
pub fn normalize_task_name(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    let mut result = String::with_capacity(lower.len());
    let mut prev_was_hyphen = false;

    for c in lower.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c);
            prev_was_hyphen = false;
        } else if c == '-' || c.is_whitespace() {
            if !prev_was_hyphen && !result.is_empty() {
                result.push('-');
                prev_was_hyphen = true;
            }
        }
    }

    if result.ends_with('-') {
        result.pop();
    }

    result
}

/// Validate a textual identifier: trim, non-empty, no control characters.
fn validate_text_id(raw: &str, label: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{} must not be empty", label));
    }
    if trimmed.as_bytes().iter().any(|b| b.is_ascii_control()) {
        return Err(format!("{} must not contain control characters", label));
    }
    Ok(trimmed.to_string())
}

/// Validate a general text field: trim, non-empty. Control characters are
/// permitted because these fields carry prose (names, descriptions).
fn validate_text_content(raw: &str, label: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{} must not be empty", label));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_input() -> AutomationTaskInput {
        AutomationTaskInput {
            id: "daily-report".to_string(),
            display_name: "Daily Report".to_string(),
            stored_prompt_id: "report-prompt".to_string(),
            expected_outcome: "A summary of today's activity".to_string(),
            project_root: PathBuf::from("/tmp"),
            timeout_seconds: 300,
        }
    }

    // ---- normalize_task_name tests (Task 1.2) ----

    #[test]
    fn normalize_basic() {
        assert_eq!(normalize_task_name("Check Email"), "check-email");
    }

    #[test]
    fn normalize_strips_apostrophe() {
        assert_eq!(normalize_task_name("Paul's daily brief"), "pauls-daily-brief");
    }

    #[test]
    fn normalize_collapses_multiple_spaces() {
        assert_eq!(normalize_task_name("My   Task"), "my-task");
    }

    #[test]
    fn normalize_strips_leading_trailing_whitespace() {
        assert_eq!(normalize_task_name("  Spaced Task  "), "spaced-task");
    }

    #[test]
    fn normalize_drops_special_chars() {
        assert_eq!(normalize_task_name("Task #1!"), "task-1");
    }

    #[test]
    fn normalize_empty_after_processing() {
        assert_eq!(normalize_task_name("'!!!'"), "");
    }

    #[test]
    fn normalize_already_kebab() {
        assert_eq!(normalize_task_name("check-email"), "check-email");
    }

    #[test]
    fn normalize_case_insensitive() {
        assert_eq!(normalize_task_name("DAILY-REPORT"), "daily-report");
        assert_eq!(normalize_task_name("Daily Report"), "daily-report");
    }

    // ---- validate_task_input tests ----

    #[test]
    fn valid_input_passes_validation() {
        let input = valid_input();
        let result = validate_task_input(&input).unwrap();
        assert_eq!(result.id, "daily-report");
        assert_eq!(result.display_name, "Daily Report");
        assert_eq!(result.stored_prompt_id, "report-prompt");
        assert_eq!(result.expected_outcome, "A summary of today's activity");
        assert_eq!(result.project_root, PathBuf::from("/tmp"));
        assert_eq!(result.timeout_seconds, 300);
    }

    #[test]
    fn input_is_trimmed() {
        let input = AutomationTaskInput {
            id: "  daily-report  ".to_string(),
            display_name: "  Daily Report  ".to_string(),
            stored_prompt_id: "  report-prompt  ".to_string(),
            expected_outcome: "  Summary  ".to_string(),
            project_root: PathBuf::from("  /tmp  "),
            timeout_seconds: 300,
        };
        let result = validate_task_input(&input).unwrap();
        assert_eq!(result.id, "daily-report");
        assert_eq!(result.display_name, "Daily Report");
        assert_eq!(result.stored_prompt_id, "report-prompt");
        assert_eq!(result.expected_outcome, "Summary");
    }

    #[test]
    fn empty_id_rejected() {
        let mut input = valid_input();
        input.id = "   ".to_string();
        assert!(validate_task_input(&input).is_err());
    }

    #[test]
    fn empty_display_name_rejected() {
        let mut input = valid_input();
        input.display_name = "".to_string();
        assert!(validate_task_input(&input).is_err());
    }

    #[test]
    fn display_name_only_special_chars_rejected() {
        let mut input = valid_input();
        input.display_name = "'!!!'".to_string();
        assert!(validate_task_input(&input).is_err());
    }

    #[test]
    fn empty_stored_prompt_id_rejected() {
        let mut input = valid_input();
        input.stored_prompt_id = "  ".to_string();
        assert!(validate_task_input(&input).is_err());
    }

    #[test]
    fn empty_expected_outcome_rejected() {
        let mut input = valid_input();
        input.expected_outcome = "".to_string();
        assert!(validate_task_input(&input).is_err());
    }

    #[test]
    fn empty_project_root_rejected() {
        let mut input = valid_input();
        input.project_root = PathBuf::from("");
        assert!(validate_task_input(&input).is_err());
    }

    #[test]
    fn zero_timeout_rejected() {
        let mut input = valid_input();
        input.timeout_seconds = 0;
        assert!(validate_task_input(&input).is_err());
    }

    #[test]
    fn excessive_timeout_rejected() {
        let mut input = valid_input();
        input.timeout_seconds = MAX_TIMEOUT_SECONDS + 1;
        assert!(validate_task_input(&input).is_err());
    }

    #[test]
    fn max_timeout_accepted() {
        let mut input = valid_input();
        input.timeout_seconds = MAX_TIMEOUT_SECONDS;
        assert!(validate_task_input(&input).is_ok());
    }

    #[test]
    fn control_chars_in_id_rejected() {
        let mut input = valid_input();
        input.id = "task\0id".to_string();
        assert!(validate_task_input(&input).is_err());
    }

    #[test]
    fn control_chars_in_stored_prompt_id_rejected() {
        let mut input = valid_input();
        input.stored_prompt_id = "prompt\rid".to_string();
        assert!(validate_task_input(&input).is_err());
    }

    #[test]
    fn ids_are_case_sensitive() {
        let input_a = AutomationTaskInput {
            id: "TaskA".to_string(),
            ..valid_input()
        };
        let input_b = AutomationTaskInput {
            id: "taska".to_string(),
            ..valid_input()
        };
        let a = validate_task_input(&input_a).unwrap();
        let b = validate_task_input(&input_b).unwrap();
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn multiline_expected_outcome_accepted() {
        let mut input = valid_input();
        input.expected_outcome = "Line one\nLine two".to_string();
        let result = validate_task_input(&input).unwrap();
        assert_eq!(result.expected_outcome, "Line one\nLine two");
    }

    #[test]
    fn automation_task_serializes() {
        let now = Utc::now();
        let task = AutomationTask {
            id: "t1".to_string(),
            display_name: "Test".to_string(),
            normalized_name: "test".to_string(),
            stored_prompt_id: "p1".to_string(),
            expected_outcome: "Done".to_string(),
            project_root: PathBuf::from("/tmp"),
            timeout_seconds: 60,
            created_at: now,
            updated_at: now,
        };
        let json = serde_json::to_string(&task).unwrap();
        let restored: AutomationTask = serde_json::from_str(&json).unwrap();
        assert_eq!(task, restored);
    }
}
