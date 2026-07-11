//! Durable automation-task identity and validation.
//!
//! An `AutomationTask` is a reference to a `StoredPrompt` with its own
//! expected-outcome text. It is designed for GUI management and eventual
//! scheduling. The task does not duplicate instructions, skills, profile,
//! provider, model, tools, or approval policy — those live on the stored
//! prompt and its optional profile.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Schema version for automation-task records.
pub const AUTOMATION_TASK_SCHEMA_VERSION: i64 = 1;

/// A durable automation-task identity.
///
/// Contains a stable ID, a user-facing name, a reference to a stored prompt,
/// required expected-outcome text, and timestamps. Provider, model, tool,
/// approval, skills, and profile configuration are resolved indirectly
/// through the referenced stored prompt and its optional profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationTask {
    /// Stable, case-sensitive task identifier.
    pub id: String,
    /// User-facing display name.
    pub name: String,
    /// ID of the referenced stored prompt.
    pub stored_prompt_id: String,
    /// Required prose describing the desired outcome (not independently verified).
    pub expected_outcome: String,
    /// When the task was first created.
    pub created_at: DateTime<Utc>,
    /// When the task was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or replacing an automation task.
///
/// On replacement, the original creation timestamp is preserved and the
/// update timestamp advances.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationTaskInput {
    /// Stable, case-sensitive task identifier.
    pub id: String,
    /// User-facing display name.
    pub name: String,
    /// ID of the referenced stored prompt (must exist at write time).
    pub stored_prompt_id: String,
    /// Required prose describing the desired outcome.
    pub expected_outcome: String,
}

/// Validate and normalize an automation-task input.
///
/// Trims all fields. Returns `Ok` with normalized values, or `Err` with a
/// human-readable message when a field is empty after trimming or when a
/// textual identifier contains control characters.
pub fn validate_task_input(input: &AutomationTaskInput) -> Result<AutomationTaskInput, String> {
    let id = validate_text_id(&input.id, "Task ID")?;
    let name = validate_text_content(&input.name, "Task name")?;
    let stored_prompt_id = validate_text_id(&input.stored_prompt_id, "Stored prompt ID")?;
    let expected_outcome = validate_text_content(&input.expected_outcome, "Expected outcome")?;
    Ok(AutomationTaskInput {
        id,
        name,
        stored_prompt_id,
        expected_outcome,
    })
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
            name: "Daily Report".to_string(),
            stored_prompt_id: "report-prompt".to_string(),
            expected_outcome: "A summary of today's activity".to_string(),
        }
    }

    #[test]
    fn valid_input_passes_validation() {
        let input = valid_input();
        let result = validate_task_input(&input).unwrap();
        assert_eq!(result.id, "daily-report");
        assert_eq!(result.name, "Daily Report");
        assert_eq!(result.stored_prompt_id, "report-prompt");
        assert_eq!(result.expected_outcome, "A summary of today's activity");
    }

    #[test]
    fn input_is_trimmed() {
        let input = AutomationTaskInput {
            id: "  daily-report  ".to_string(),
            name: "  Daily Report  ".to_string(),
            stored_prompt_id: "  report-prompt  ".to_string(),
            expected_outcome: "  Summary  ".to_string(),
        };
        let result = validate_task_input(&input).unwrap();
        assert_eq!(result.id, "daily-report");
        assert_eq!(result.name, "Daily Report");
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
    fn empty_name_rejected() {
        let mut input = valid_input();
        input.name = "".to_string();
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
            name: "Test".to_string(),
            stored_prompt_id: "p1".to_string(),
            expected_outcome: "Done".to_string(),
            created_at: now,
            updated_at: now,
        };
        let json = serde_json::to_string(&task).unwrap();
        let restored: AutomationTask = serde_json::from_str(&json).unwrap();
        assert_eq!(task, restored);
    }
}
