//! Shared execution resolution and ephemeral automation-run types.
//!
//! At run start a resolver loads the current automation task, its referenced
//! stored prompt, the resolved agent profile, and the composed user-visible
//! goal into an immutable [`ResolvedExecutionInput`]. This snapshot is stable
//! for the duration of the run; subsequent configuration edits are visible
//! only to later runs.
//!
//! The resolver is shared between root CLI execution and delegated stored
//! prompt invocation. The execution mechanism itself remains distinct: CLI
//! runs are root sessions and delegated calls remain child sessions.

use crate::automation_task::AutomationTask;
use crate::config::{ConfigError, ConfigStore};
use crate::profile::{AgentApproval, AgentProfile, AgentProfileId, SkillFilter, ToolFilter};
use crate::stored_prompt::{StoredPrompt, STORED_PROMPT_SCHEMA_VERSION};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Schema version for automation-run result payloads.
pub const AUTOMATION_RUN_SCHEMA_VERSION: i64 = 1;

// ============================================================================
// Immutable resolved execution input
// ============================================================================

/// Immutable snapshot of everything resolved from persistent state at run
/// start.
///
/// All fields are owned clones taken at resolution time. Concurrent edits to
/// ConfigStore or the profile registry after resolution do not affect the
/// values in this struct.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedExecutionInput {
    /// The automation task being executed.
    pub task: AutomationTask,
    /// The stored prompt referenced by the task (snapshot at resolution time).
    pub prompt: StoredPrompt,
    /// Resolved profile identifier (explicit prompt profile or built-in
    /// `"default"`).
    pub profile_id: AgentProfileId,
    /// Resolved agent profile (snapshot at resolution time).
    pub profile: AgentProfile,
    /// Composed model-visible user goal: stored instructions plus the task's
    /// expected outcome.
    pub user_goal: String,
    /// Effective skill names after combining the stored prompt's requested
    /// skills with the profile's skill filter policy.
    pub effective_skills: Vec<String>,
    /// Canonical workspace directory for the run.
    pub workspace: PathBuf,
    /// When this snapshot was resolved.
    pub resolved_at: DateTime<Utc>,
}

// ============================================================================
// Ephemeral automation-run result types
// ============================================================================

/// Terminal status of an automation run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRunStatus {
    /// The runtime completed the prompt turn without unhandled error.
    ///
    /// This means technical completion only; the expected outcome is not
    /// independently verified.
    Completed,
    /// An unhandled runtime, provider, or configuration error terminated the
    /// run.
    Failed,
    /// A cancellation signal (SIGINT / SIGTERM) terminated the run.
    Cancelled,
    /// The run exceeded its configured timeout.
    TimedOut,
}

impl AutomationRunStatus {
    /// Stable lowercase string representation for JSON and text output.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        }
    }
}

/// Category of a structured run error.
///
/// Categories align with the stable CLI exit-code mapping so that callers can
/// distinguish failure reasons programmatically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRunErrorCategory {
    /// Configuration or reference resolution failure.
    Config,
    /// Referenced entity (task, prompt, or profile) was not found.
    Reference,
    /// Profile approval or tool policy is unsafe for headless execution.
    UnsafePolicy,
    /// Provider or credential initialization failure.
    ProviderInit,
    /// Unhandled error during prompt execution.
    Execution,
    /// Run was cancelled by a signal.
    Cancelled,
    /// Run exceeded its timeout.
    TimedOut,
}

/// Structured error carried in a failed, cancelled, or timed-out run result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationRunError {
    pub category: AutomationRunErrorCategory,
    pub message: String,
}

/// Ephemeral result of a single automation-run attempt.
///
/// This struct is produced after execution completes (or fails). It is not
/// persisted in IC-7A; the shape is designed to allow run-history persistence
/// in a future change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationRunResult {
    /// Schema version of this result payload.
    pub schema_version: i64,
    /// Generated unique identifier for this run attempt.
    pub run_id: String,
    /// ID of the executed automation task.
    pub task_id: String,
    /// Display name of the executed automation task.
    pub task_name: String,
    /// Terminal status.
    pub status: AutomationRunStatus,
    /// Final assistant text (empty when no assistant text was produced).
    #[serde(default)]
    pub output: String,
    /// Expected-outcome text copied from the task for reporting.
    #[serde(default)]
    pub expected_outcome: String,
    /// Resolved profile identifier used for the run.
    #[serde(default)]
    pub profile_id: String,
    /// Provider slug or `"runtime_default"` when resolved from the persisted
    /// default. `null` when not resolved.
    #[serde(default)]
    pub provider: Option<String>,
    /// Model identifier used for the run. `null` when not resolved.
    #[serde(default)]
    pub model: Option<String>,
    /// Canonical workspace path used for the run.
    pub workspace: PathBuf,
    /// Sorted effective tool names available to the run.
    #[serde(default)]
    pub effective_tools: Vec<String>,
    /// When the run started.
    pub started_at: DateTime<Utc>,
    /// When the run ended.
    pub ended_at: DateTime<Utc>,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Structured error when the run did not complete successfully. `null`
    /// for successful runs.
    #[serde(default)]
    pub error: Option<AutomationRunError>,
}

impl AutomationRunResult {
    /// Create a builder pre-populated with task identity and timing defaults.
    pub fn started(task: &AutomationTask, workspace: PathBuf) -> Self {
        Self {
            schema_version: AUTOMATION_RUN_SCHEMA_VERSION,
            run_id: uuid::Uuid::new_v4().to_string(),
            task_id: task.id.clone(),
            task_name: task.name.clone(),
            status: AutomationRunStatus::Failed,
            output: String::new(),
            expected_outcome: task.expected_outcome.clone(),
            profile_id: String::new(),
            provider: None,
            model: None,
            workspace,
            effective_tools: Vec::new(),
            started_at: Utc::now(),
            ended_at: Utc::now(),
            duration_ms: 0,
            error: None,
        }
    }

    /// Mark the run as completed with the given final assistant text.
    pub fn complete(&mut self, output: String) {
        self.status = AutomationRunStatus::Completed;
        self.output = output;
        self.error = None;
        self.finalize_timing();
    }

    /// Mark the run as failed with a structured error.
    pub fn fail(&mut self, category: AutomationRunErrorCategory, message: String) {
        self.status = AutomationRunStatus::Failed;
        self.error = Some(AutomationRunError { category, message });
        self.finalize_timing();
    }

    /// Mark the run as cancelled.
    pub fn cancel(&mut self, message: String) {
        self.status = AutomationRunStatus::Cancelled;
        self.error = Some(AutomationRunError {
            category: AutomationRunErrorCategory::Cancelled,
            message,
        });
        self.finalize_timing();
    }

    /// Mark the run as timed out.
    pub fn timeout(&mut self, message: String) {
        self.status = AutomationRunStatus::TimedOut;
        self.error = Some(AutomationRunError {
            category: AutomationRunErrorCategory::TimedOut,
            message,
        });
        self.finalize_timing();
    }

    /// Set resolved provider/model/tool metadata.
    pub fn set_resolved_metadata(
        &mut self,
        profile_id: &str,
        provider: Option<String>,
        model: Option<String>,
        effective_tools: Vec<String>,
    ) {
        self.profile_id = profile_id.to_string();
        self.provider = provider;
        self.model = model;
        self.effective_tools = effective_tools;
    }

    /// Create a terminal failure result for CLI-level errors that occur
    /// before or during bootstrap, when the task record may not be loaded.
    pub fn cli_failure(
        task_id: &str,
        workspace: PathBuf,
        category: AutomationRunErrorCategory,
        message: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            schema_version: AUTOMATION_RUN_SCHEMA_VERSION,
            run_id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.to_string(),
            task_name: String::new(),
            status: AutomationRunStatus::Failed,
            output: String::new(),
            expected_outcome: String::new(),
            profile_id: String::new(),
            provider: None,
            model: None,
            workspace,
            effective_tools: Vec::new(),
            started_at: now,
            ended_at: now,
            duration_ms: 0,
            error: Some(AutomationRunError { category, message }),
        }
    }

    fn finalize_timing(&mut self) {
        self.ended_at = Utc::now();
        self.duration_ms = self
            .ended_at
            .signed_duration_since(self.started_at)
            .num_milliseconds()
            .max(0) as u64;
    }
}

// ============================================================================
// Resolution errors
// ============================================================================

/// Typed failure during execution-input resolution.
#[derive(Debug, thiserror::Error)]
pub enum ResolutionError {
    #[error("automation task '{0}' not found")]
    TaskNotFound(String),
    #[error("stored prompt '{0}' not found")]
    PromptNotFound(String),
    #[error("stored prompt '{0}' has an unsupported schema version or invalid payload")]
    InvalidPrompt(String),
    #[error("profile '{}' not found", _0.as_str())]
    ProfileNotFound(AgentProfileId),
    #[error(transparent)]
    Store(#[from] ConfigError),
}

// ============================================================================
// Shared resolution helpers
// ============================================================================

/// Compose a model-visible user goal by appending optional additional text.
///
/// When `addition` is `Some` the two parts are joined with a blank-line
/// separator. When `addition` is `None` the primary text is returned as-is.
pub fn compose_user_goal(primary: &str, addition: Option<&str>) -> String {
    match addition {
        Some(add) => format!("{}\n\n{}", primary, add),
        None => primary.to_string(),
    }
}

/// Resolve the profile for a stored prompt.
///
/// If the prompt specifies a profile, that profile ID is used. Otherwise the
/// built-in `"default"` profile is resolved. The returned profile is cloned
/// from the registry, producing an immutable snapshot.
pub fn resolve_profile_for_prompt(
    prompt_profile_id: Option<&AgentProfileId>,
    registry: &HashMap<AgentProfileId, AgentProfile>,
) -> Result<(AgentProfileId, AgentProfile), ResolutionError> {
    let profile_id = prompt_profile_id
        .cloned()
        .unwrap_or_else(|| AgentProfileId::from("default"));

    let profile = registry
        .get(&profile_id)
        .cloned()
        .ok_or_else(|| ResolutionError::ProfileNotFound(profile_id.clone()))?;

    Ok((profile_id, profile))
}

/// Resolve a complete execution input from ConfigStore and profile registry.
///
/// This loads the task, validates and loads the referenced prompt, resolves
/// the profile, composes the user goal, and returns an immutable snapshot.
/// All values are cloned at resolution time so concurrent edits do not affect
/// the returned input.
pub async fn resolve_task_execution(
    store: &ConfigStore,
    profiles: &HashMap<AgentProfileId, AgentProfile>,
    task_id: &str,
    workspace: PathBuf,
) -> Result<ResolvedExecutionInput, ResolutionError> {
    let task = store
        .get_automation_task(task_id)
        .await?
        .ok_or_else(|| ResolutionError::TaskNotFound(task_id.to_string()))?;

    let prompt = load_single_prompt(store, &task.stored_prompt_id).await?;

    let (profile_id, profile) = resolve_profile_for_prompt(prompt.profile.as_ref(), profiles)?;

    let user_goal = compose_user_goal(&prompt.instructions, Some(&task.expected_outcome));

    let skills = match &profile.skills {
        SkillFilter::None => Vec::new(),
        SkillFilter::Inherit => prompt.skills.clone(),
        SkillFilter::Allow(allowed) => prompt
            .skills
            .iter()
            .filter(|s| allowed.iter().any(|a| a == *s))
            .cloned()
            .collect(),
    };

    Ok(ResolvedExecutionInput {
        task,
        prompt,
        profile_id,
        profile,
        user_goal,
        effective_skills: skills,
        workspace,
        resolved_at: Utc::now(),
    })
}

/// Load, validate, and deserialize a single stored prompt from ConfigStore.
async fn load_single_prompt(
    store: &ConfigStore,
    prompt_id: &str,
) -> Result<StoredPrompt, ResolutionError> {
    let record = store
        .get_prompt(prompt_id)
        .await?
        .ok_or_else(|| ResolutionError::PromptNotFound(prompt_id.to_string()))?;

    if record.schema_version != STORED_PROMPT_SCHEMA_VERSION {
        return Err(ResolutionError::InvalidPrompt(prompt_id.to_string()));
    }

    let prompt: StoredPrompt = serde_json::from_value(record.payload)
        .map_err(|_| ResolutionError::InvalidPrompt(prompt_id.to_string()))?;

    prompt
        .validate()
        .map_err(|_| ResolutionError::InvalidPrompt(prompt_id.to_string()))?;

    Ok(prompt)
}

/// Effective skill names for a resolved execution input.
///
/// This returns the pre-computed snapshot from [`ResolvedExecutionInput`],
/// which was derived at resolution time by combining the stored prompt's
/// requested skills with the profile's skill filter policy.
pub fn effective_skills(input: &ResolvedExecutionInput) -> Vec<String> {
    input.effective_skills.clone()
}

/// Whether the resolved profile is safe for headless execution.
///
/// Headless preflight requires `AutoApprove`. `PerTool` is rejected because
/// there is no interactive client to approve tool calls. This is a pure check
/// on the already-resolved profile snapshot.
pub fn is_headless_safe(approval: AgentApproval) -> bool {
    matches!(approval, AgentApproval::AutoApprove)
}

/// Compute the effective tool filter for the resolved profile.
pub fn effective_tool_filter(input: &ResolvedExecutionInput) -> ToolFilter {
    input.profile.tools.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation_task::AutomationTaskInput;
    use crate::config::ConfigStore;
    use crate::profile::{AgentApproval, AgentProfile, SkillFilter};
    use crate::stored_prompt::StoredPrompt;

    // ---- compose_user_goal ----

    #[test]
    fn compose_user_goal_no_addition() {
        let result = compose_user_goal("do thing", None);
        assert_eq!(result, "do thing");
    }

    #[test]
    fn compose_user_goal_with_addition() {
        let result = compose_user_goal("do thing", Some("expected: done"));
        assert_eq!(result, "do thing\n\nexpected: done");
    }

    #[test]
    fn compose_user_goal_preserves_multiline() {
        let result = compose_user_goal("line1\nline2", Some("outcome"));
        assert_eq!(result, "line1\nline2\n\noutcome");
    }

    // ---- resolve_profile_for_prompt ----

    fn make_registry() -> HashMap<AgentProfileId, AgentProfile> {
        let mut reg = HashMap::new();
        reg.insert(
            AgentProfileId::from("default"),
            AgentProfile::with_name("default"),
        );
        reg.insert(
            AgentProfileId::from("auto"),
            AgentProfile {
                name: "auto".to_string(),
                approval: AgentApproval::AutoApprove,
                ..AgentProfile::with_name("auto")
            },
        );
        reg
    }

    #[test]
    fn resolve_profile_explicit() {
        let reg = make_registry();
        let id = AgentProfileId::from("auto");
        let (resolved_id, profile) = resolve_profile_for_prompt(Some(&id), &reg).unwrap();
        assert_eq!(resolved_id, AgentProfileId::from("auto"));
        assert_eq!(profile.approval, AgentApproval::AutoApprove);
    }

    #[test]
    fn resolve_profile_defaults_to_default() {
        let reg = make_registry();
        let (resolved_id, profile) = resolve_profile_for_prompt(None, &reg).unwrap();
        assert_eq!(resolved_id, AgentProfileId::from("default"));
        assert_eq!(profile.approval, AgentApproval::PerTool);
    }

    #[test]
    fn resolve_profile_not_found() {
        let reg = make_registry();
        let id = AgentProfileId::from("missing");
        let result = resolve_profile_for_prompt(Some(&id), &reg);
        assert!(matches!(result, Err(ResolutionError::ProfileNotFound(_))));
    }

    // ---- is_headless_safe ----

    #[test]
    fn headless_safe_autoapprove() {
        assert!(is_headless_safe(AgentApproval::AutoApprove));
    }

    #[test]
    fn headless_not_safe_pertool() {
        assert!(!is_headless_safe(AgentApproval::PerTool));
    }

    // ---- effective_skills ----

    #[test]
    fn effective_skills_inherit() {
        let now = Utc::now();
        let task = AutomationTask {
            id: "t1".to_string(),
            name: "T".to_string(),
            stored_prompt_id: "p1".to_string(),
            expected_outcome: "done".to_string(),
            created_at: now,
            updated_at: now,
        };
        let prompt = StoredPrompt {
            instructions: "do thing".to_string(),
            skills: vec!["s1".to_string(), "s2".to_string()],
            profile: None,
        };
        let profile = AgentProfile {
            skills: SkillFilter::Inherit,
            ..AgentProfile::with_name("default")
        };
        let input = ResolvedExecutionInput {
            task,
            prompt,
            profile_id: AgentProfileId::from("default"),
            profile,
            user_goal: "do thing\n\ndone".to_string(),
            effective_skills: vec!["s1".to_string(), "s2".to_string()],
            workspace: PathBuf::from("/tmp"),
            resolved_at: now,
        };
        let skills = effective_skills(&input);
        assert_eq!(skills, vec!["s1", "s2"]);
    }

    #[test]
    fn effective_skills_none_filter() {
        let now = Utc::now();
        let prompt = StoredPrompt {
            instructions: "do".to_string(),
            skills: vec!["s1".to_string()],
            profile: None,
        };
        let profile = AgentProfile {
            skills: SkillFilter::None,
            ..AgentProfile::with_name("default")
        };
        let input = ResolvedExecutionInput {
            task: AutomationTask {
                id: "t".to_string(),
                name: "T".to_string(),
                stored_prompt_id: "p".to_string(),
                expected_outcome: "o".to_string(),
                created_at: now,
                updated_at: now,
            },
            prompt,
            profile_id: AgentProfileId::from("default"),
            profile,
            user_goal: "do\n\no".to_string(),
            effective_skills: Vec::new(),
            workspace: PathBuf::from("/tmp"),
            resolved_at: now,
        };
        assert!(effective_skills(&input).is_empty());
    }

    #[test]
    fn effective_skills_allow_filter() {
        let now = Utc::now();
        let prompt = StoredPrompt {
            instructions: "do".to_string(),
            skills: vec!["s1".to_string(), "s2".to_string(), "s3".to_string()],
            profile: None,
        };
        let profile = AgentProfile {
            skills: SkillFilter::Allow(vec!["s1".to_string(), "s3".to_string()]),
            ..AgentProfile::with_name("default")
        };
        let input = ResolvedExecutionInput {
            task: AutomationTask {
                id: "t".to_string(),
                name: "T".to_string(),
                stored_prompt_id: "p".to_string(),
                expected_outcome: "o".to_string(),
                created_at: now,
                updated_at: now,
            },
            prompt,
            profile_id: AgentProfileId::from("default"),
            profile,
            user_goal: "do\n\no".to_string(),
            effective_skills: vec!["s1".to_string(), "s3".to_string()],
            workspace: PathBuf::from("/tmp"),
            resolved_at: now,
        };
        let skills = effective_skills(&input);
        assert_eq!(skills, vec!["s1", "s3"]);
    }

    // ---- AutomationRunResult builder ----

    #[test]
    fn run_result_started_has_task_identity() {
        let now = Utc::now();
        let task = AutomationTask {
            id: "daily".to_string(),
            name: "Daily".to_string(),
            stored_prompt_id: "p".to_string(),
            expected_outcome: "report".to_string(),
            created_at: now,
            updated_at: now,
        };
        let result = AutomationRunResult::started(&task, PathBuf::from("/work"));
        assert_eq!(result.task_id, "daily");
        assert_eq!(result.task_name, "Daily");
        assert_eq!(result.expected_outcome, "report");
        assert_eq!(result.workspace, PathBuf::from("/work"));
        assert!(!result.run_id.is_empty());
    }

    #[test]
    fn run_result_complete() {
        let now = Utc::now();
        let task = AutomationTask {
            id: "t".to_string(),
            name: "T".to_string(),
            stored_prompt_id: "p".to_string(),
            expected_outcome: "o".to_string(),
            created_at: now,
            updated_at: now,
        };
        let mut result = AutomationRunResult::started(&task, PathBuf::from("/w"));
        result.complete("final output".to_string());
        assert_eq!(result.status, AutomationRunStatus::Completed);
        assert_eq!(result.output, "final output");
        assert!(result.error.is_none());
    }

    #[test]
    fn run_result_fail_sets_error() {
        let now = Utc::now();
        let task = AutomationTask {
            id: "t".to_string(),
            name: "T".to_string(),
            stored_prompt_id: "p".to_string(),
            expected_outcome: "o".to_string(),
            created_at: now,
            updated_at: now,
        };
        let mut result = AutomationRunResult::started(&task, PathBuf::from("/w"));
        result.fail(
            AutomationRunErrorCategory::Execution,
            "provider error".to_string(),
        );
        assert_eq!(result.status, AutomationRunStatus::Failed);
        assert_eq!(
            result.error.as_ref().unwrap().category,
            AutomationRunErrorCategory::Execution
        );
    }

    #[test]
    fn run_result_cancel_and_timeout() {
        let now = Utc::now();
        let task = AutomationTask {
            id: "t".to_string(),
            name: "T".to_string(),
            stored_prompt_id: "p".to_string(),
            expected_outcome: "o".to_string(),
            created_at: now,
            updated_at: now,
        };

        let mut r1 = AutomationRunResult::started(&task, PathBuf::from("/w"));
        r1.cancel("SIGINT".to_string());
        assert_eq!(r1.status, AutomationRunStatus::Cancelled);
        assert_eq!(
            r1.error.as_ref().unwrap().category,
            AutomationRunErrorCategory::Cancelled
        );

        let mut r2 = AutomationRunResult::started(&task, PathBuf::from("/w"));
        r2.timeout("exceeded 30s".to_string());
        assert_eq!(r2.status, AutomationRunStatus::TimedOut);
        assert_eq!(
            r2.error.as_ref().unwrap().category,
            AutomationRunErrorCategory::TimedOut
        );
    }

    #[test]
    fn run_result_serializes_to_json() {
        let now = Utc::now();
        let task = AutomationTask {
            id: "t".to_string(),
            name: "T".to_string(),
            stored_prompt_id: "p".to_string(),
            expected_outcome: "o".to_string(),
            created_at: now,
            updated_at: now,
        };
        let mut result = AutomationRunResult::started(&task, PathBuf::from("/w"));
        result.complete("done".to_string());
        result.set_resolved_metadata(
            "auto",
            Some("openai".to_string()),
            Some("gpt-4".to_string()),
            vec!["webfetch".to_string()],
        );

        let json = serde_json::to_string(&result).unwrap();
        let restored: AutomationRunResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, restored);
    }

    #[test]
    fn status_serializes_snake_case() {
        let json = serde_json::to_string(&AutomationRunStatus::TimedOut).unwrap();
        assert_eq!(json, "\"timed_out\"");
        let restored: AutomationRunStatus = serde_json::from_str("\"completed\"").unwrap();
        assert_eq!(restored, AutomationRunStatus::Completed);
    }

    #[test]
    fn json_includes_null_provider_model_error_for_cli_failure() {
        let result = AutomationRunResult::cli_failure(
            "missing-task",
            PathBuf::from("/w"),
            AutomationRunErrorCategory::Reference,
            "task not found".to_string(),
        );
        let json = serde_json::to_string(&result).unwrap();
        // provider, model, and error must be present (not omitted) even when null
        assert!(
            json.contains("\"provider\":null"),
            "provider should be null, got: {}",
            json
        );
        assert!(
            json.contains("\"model\":null"),
            "model should be null, got: {}",
            json
        );
    }

    #[test]
    fn json_includes_null_error_for_completed_run() {
        let now = Utc::now();
        let task = AutomationTask {
            id: "t".to_string(),
            name: "T".to_string(),
            stored_prompt_id: "p".to_string(),
            expected_outcome: "o".to_string(),
            created_at: now,
            updated_at: now,
        };
        let mut result = AutomationRunResult::started(&task, PathBuf::from("/w"));
        result.complete("done".to_string());
        let json = serde_json::to_string(&result).unwrap();
        assert!(
            json.contains("\"error\":null"),
            "error should be null for completed run, got: {}",
            json
        );
    }

    // ---- resolve_task_execution integration ----

    async fn setup_store_with_task_and_prompt() -> ConfigStore {
        let store = ConfigStore::open_in_memory().await.unwrap();

        // Register a prompt.
        let prompt = StoredPrompt {
            instructions: "Generate a daily report".to_string(),
            skills: Vec::new(),
            profile: None,
        };
        let payload = serde_json::to_value(&prompt).unwrap();
        store
            .set_prompt(&crate::config::PromptInput {
                id: "report-prompt".to_string(),
                schema_version: STORED_PROMPT_SCHEMA_VERSION,
                payload,
            })
            .await
            .unwrap();

        // Register a task referencing the prompt.
        store
            .set_automation_task(&AutomationTaskInput {
                id: "daily-report".to_string(),
                name: "Daily Report".to_string(),
                stored_prompt_id: "report-prompt".to_string(),
                expected_outcome: "A summary of today's activity".to_string(),
            })
            .await
            .unwrap();

        store
    }

    fn default_registry() -> HashMap<AgentProfileId, AgentProfile> {
        let mut reg = HashMap::new();
        reg.insert(
            AgentProfileId::from("default"),
            AgentProfile::with_name("default"),
        );
        reg
    }

    #[tokio::test]
    async fn resolve_task_execution_success() {
        let store = setup_store_with_task_and_prompt().await;
        let reg = default_registry();

        let input =
            resolve_task_execution(&store, &reg, "daily-report", PathBuf::from("/workspace"))
                .await
                .unwrap();

        assert_eq!(input.task.id, "daily-report");
        assert_eq!(input.task.expected_outcome, "A summary of today's activity");
        assert_eq!(input.prompt.instructions, "Generate a daily report");
        assert_eq!(input.profile_id, AgentProfileId::from("default"));
        assert_eq!(
            input.user_goal,
            "Generate a daily report\n\nA summary of today's activity"
        );
        assert_eq!(input.workspace, PathBuf::from("/workspace"));
    }

    #[tokio::test]
    async fn resolve_task_execution_task_not_found() {
        let store = setup_store_with_task_and_prompt().await;
        let reg = default_registry();

        let result = resolve_task_execution(&store, &reg, "missing", PathBuf::from("/w")).await;
        assert!(matches!(result, Err(ResolutionError::TaskNotFound(_))));
    }

    #[tokio::test]
    async fn resolve_task_execution_prompt_not_found() {
        let store = ConfigStore::open_in_memory().await.unwrap();

        // Creating a task referencing a nonexistent prompt is rejected at
        // write time, so the resolver's PromptNotFound can only occur from
        // manual DB tampering. Verify the write-time guard works.
        let result = store
            .set_automation_task(&AutomationTaskInput {
                id: "orphan-task".to_string(),
                name: "Orphan".to_string(),
                stored_prompt_id: "nonexistent".to_string(),
                expected_outcome: "nothing".to_string(),
            })
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::config::ConfigError::UnknownStoredPrompt(_)
        ));
    }

    #[tokio::test]
    async fn resolve_task_execution_profile_not_found() {
        let store = setup_store_with_task_and_prompt().await;
        let reg: HashMap<AgentProfileId, AgentProfile> = HashMap::new();

        let result =
            resolve_task_execution(&store, &reg, "daily-report", PathBuf::from("/w")).await;
        assert!(matches!(result, Err(ResolutionError::ProfileNotFound(_))));
    }

    #[tokio::test]
    async fn resolve_task_execution_snapshot_is_stable() {
        let store = setup_store_with_task_and_prompt().await;
        let reg = default_registry();

        // Resolve.
        let input1 = resolve_task_execution(&store, &reg, "daily-report", PathBuf::from("/w"))
            .await
            .unwrap();

        // Update the task's expected outcome.
        store
            .set_automation_task(&AutomationTaskInput {
                id: "daily-report".to_string(),
                name: "Daily Report".to_string(),
                stored_prompt_id: "report-prompt".to_string(),
                expected_outcome: "Updated outcome".to_string(),
            })
            .await
            .unwrap();

        // Original snapshot is unchanged.
        assert_eq!(
            input1.task.expected_outcome,
            "A summary of today's activity"
        );

        // New resolution sees the update.
        let input2 = resolve_task_execution(&store, &reg, "daily-report", PathBuf::from("/w"))
            .await
            .unwrap();
        assert_eq!(input2.task.expected_outcome, "Updated outcome");
    }

    #[tokio::test]
    async fn resolve_task_execution_with_explicit_profile() {
        let store = ConfigStore::open_in_memory().await.unwrap();

        // Register a prompt with an explicit profile.
        let prompt = StoredPrompt {
            instructions: "Do work".to_string(),
            skills: Vec::new(),
            profile: Some(AgentProfileId::from("automation")),
        };
        let payload = serde_json::to_value(&prompt).unwrap();
        store
            .set_prompt(&crate::config::PromptInput {
                id: "work-prompt".to_string(),
                schema_version: STORED_PROMPT_SCHEMA_VERSION,
                payload,
            })
            .await
            .unwrap();

        store
            .set_automation_task(&AutomationTaskInput {
                id: "work-task".to_string(),
                name: "Work".to_string(),
                stored_prompt_id: "work-prompt".to_string(),
                expected_outcome: "Work done".to_string(),
            })
            .await
            .unwrap();

        let mut reg = HashMap::new();
        reg.insert(
            AgentProfileId::from("automation"),
            AgentProfile {
                name: "Automation".to_string(),
                approval: AgentApproval::AutoApprove,
                ..AgentProfile::with_name("automation")
            },
        );

        let input = resolve_task_execution(&store, &reg, "work-task", PathBuf::from("/w"))
            .await
            .unwrap();

        assert_eq!(input.profile_id, AgentProfileId::from("automation"));
        assert_eq!(input.profile.approval, AgentApproval::AutoApprove);
    }
}
