//! Stored prompt definitions and registry.
//!
//! Stored prompts are named reusable task definitions persisted in the core
//! config store. They are invoked through delegated child-session execution.

use crate::config::{ConfigError, ConfigStore};
use crate::profile::AgentProfileId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current schema version for typed `StoredPrompt` payloads stored in ConfigStore.
pub const STORED_PROMPT_SCHEMA_VERSION: i64 = 2;

/// Legacy schema version still recognized during best-effort loading.
pub const LEGACY_STORED_PROMPT_SCHEMA_VERSION: i64 = 1;

/// Identity state of a stored prompt record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum IdentityState {
    /// The prompt has a valid display name and normalized handle.
    #[default]
    Ready,
    /// The prompt's handle collided during migration and must be renamed.
    NeedsRename,
}

/// A reusable prompt task definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredPrompt {
    /// User-facing display name. Mutable; renaming preserves the immutable ID.
    #[serde(default)]
    pub display_name: String,
    /// Canonical ASCII kebab-case lookup handle derived from `display_name`.
    #[serde(default)]
    pub normalized_name: String,
    /// Instructions passed as the child's goal or system prompt layer.
    pub instructions: String,
    /// Requested skills to activate for the child run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    /// Optional profile ID to use for the child run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<AgentProfileId>,
}

impl StoredPrompt {
    /// Validate invariants for v2 records.
    pub fn validate(&self) -> Result<(), String> {
        if self.instructions.trim().is_empty() {
            return Err("stored prompt instructions must not be empty".to_string());
        }
        if self.display_name.trim().is_empty() {
            return Err("stored prompt display name must not be empty".to_string());
        }
        let normalized = normalize_prompt_name(&self.display_name);
        if normalized.is_empty() {
            return Err(
                "stored prompt display name must produce a non-empty normalized handle".to_string(),
            );
        }
        if self.normalized_name != normalized {
            return Err(format!(
                "stored prompt normalized_name '{}' does not match derived '{}'",
                self.normalized_name, normalized
            ));
        }
        Ok(())
    }
}

/// A stored prompt paired with its stable ID and identity state.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPromptEntry {
    pub id: String,
    pub prompt: StoredPrompt,
    pub identity_state: IdentityState,
}

/// Issue category reported for a skipped prompt during `load_prompts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptLoadIssue {
    UnsupportedSchemaVersion { version: i64 },
    InvalidPayload,
    MissingRecord,
    UnavailableProfile { profile_id: String },
    UnavailableSkill { skill: String },
    NeedsRename,
}

/// Per-prompt diagnostic returned by best-effort prompt loading.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptLoadDiagnostic {
    pub prompt_id: String,
    pub issue: PromptLoadIssue,
}

/// Result of loading typed stored prompts from ConfigStore.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PromptLoadReport {
    pub loaded: Vec<StoredPromptEntry>,
    pub diagnostics: Vec<PromptLoadDiagnostic>,
}

/// In-memory registry of stored prompts.
#[derive(Debug, Default)]
pub struct StoredPromptRegistry {
    prompts: HashMap<String, StoredPrompt>,
}

impl StoredPromptRegistry {
    pub fn new() -> Self {
        Self {
            prompts: HashMap::new(),
        }
    }

    pub fn register(&mut self, id: String, prompt: StoredPrompt) -> Result<(), String> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err("prompt ID must not be empty".to_string());
        }
        if trimmed.as_bytes().iter().any(|b| b.is_ascii_control()) {
            return Err("prompt ID must not contain control characters".to_string());
        }
        prompt.validate()?;
        self.prompts.insert(trimmed.to_string(), prompt);
        Ok(())
    }

    pub fn unregister(&mut self, id: &str) -> bool {
        self.prompts.remove(id.trim()).is_some()
    }

    pub fn get(&self, id: &str) -> Option<&StoredPrompt> {
        self.prompts.get(id.trim())
    }

    pub fn list(&self) -> Vec<StoredPromptEntry> {
        let mut ids: Vec<&String> = self.prompts.keys().collect();
        ids.sort();
        ids.into_iter()
            .map(|id| StoredPromptEntry {
                id: id.clone(),
                prompt: self.prompts[id].clone(),
                identity_state: IdentityState::Ready,
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.prompts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.prompts.len()
    }
}

/// Normalize a user-facing prompt name into a canonical ASCII kebab-case handle.
///
/// Lowercases the input, converts whitespace, underscores, and existing hyphens
/// to single hyphens, removes all other non-alphanumeric characters, collapses
/// consecutive hyphens, and strips leading/trailing hyphens.
///
/// Examples:
/// - `Check Email` → `check-email`
/// - `Check_Email` → `check-email`
/// - `CHECK-EMAIL` → `check-email`
pub fn normalize_prompt_name(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    let mut result = String::with_capacity(lower.len());
    let mut prev_was_hyphen = false;

    for c in lower.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c);
            prev_was_hyphen = false;
        } else if (c == '-' || c == '_' || c.is_whitespace())
            && !prev_was_hyphen
            && !result.is_empty()
        {
            result.push('-');
            prev_was_hyphen = true;
        }
    }

    if result.ends_with('-') {
        result.pop();
    }

    result
}

/// Convert a kebab-case identifier to title case for display purposes.
///
/// Used during v1 migration to derive a human-readable display name from
/// a legacy record ID.
///
/// Examples:
/// - `check-email` → `Check Email`
/// - `daily_report` → `Daily Report`
pub fn kebab_to_title_case(id: &str) -> String {
    let normalized = id.replace('_', "-");
    normalized
        .split('-')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut chars = s.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Load stored prompts from a ConfigStore.
///
/// Accepts both legacy v1 and current v2 schema versions. For v1 records,
/// display name and normalized handle are derived from the stable record ID.
pub async fn load_prompts(store: &ConfigStore) -> Result<PromptLoadReport, ConfigError> {
    let mut report = PromptLoadReport::default();
    let mut ids = store.list_prompt_ids().await?;
    ids.sort();

    for id in ids {
        let record = match store.get_prompt(&id).await? {
            Some(record) => record,
            None => {
                report.diagnostics.push(PromptLoadDiagnostic {
                    prompt_id: id,
                    issue: PromptLoadIssue::MissingRecord,
                });
                continue;
            }
        };

        if record.schema_version == LEGACY_STORED_PROMPT_SCHEMA_VERSION {
            let prompt: StoredPrompt = match serde_json::from_value(record.payload.clone()) {
                Ok(p) => p,
                Err(_) => {
                    report.diagnostics.push(PromptLoadDiagnostic {
                        prompt_id: record.id,
                        issue: PromptLoadIssue::InvalidPayload,
                    });
                    continue;
                }
            };
            if prompt.instructions.trim().is_empty() {
                report.diagnostics.push(PromptLoadDiagnostic {
                    prompt_id: record.id,
                    issue: PromptLoadIssue::InvalidPayload,
                });
                continue;
            }
            let display_name = kebab_to_title_case(&record.id);
            let normalized_name = normalize_prompt_name(&display_name);
            let identity_state = if record.identity_state == "needs_rename" {
                IdentityState::NeedsRename
            } else {
                IdentityState::Ready
            };
            report.loaded.push(StoredPromptEntry {
                id: record.id,
                prompt: StoredPrompt {
                    display_name,
                    normalized_name,
                    instructions: prompt.instructions,
                    skills: prompt.skills,
                    profile: prompt.profile,
                },
                identity_state,
            });
            continue;
        }

        if record.schema_version != STORED_PROMPT_SCHEMA_VERSION {
            report.diagnostics.push(PromptLoadDiagnostic {
                prompt_id: record.id,
                issue: PromptLoadIssue::UnsupportedSchemaVersion {
                    version: record.schema_version,
                },
            });
            continue;
        }

        let prompt: StoredPrompt = match serde_json::from_value(record.payload.clone()) {
            Ok(prompt) => prompt,
            Err(_) => {
                report.diagnostics.push(PromptLoadDiagnostic {
                    prompt_id: record.id,
                    issue: PromptLoadIssue::InvalidPayload,
                });
                continue;
            }
        };

        if prompt.validate().is_err() {
            report.diagnostics.push(PromptLoadDiagnostic {
                prompt_id: record.id,
                issue: PromptLoadIssue::InvalidPayload,
            });
            continue;
        }

        let identity_state = if record.identity_state == "needs_rename" {
            IdentityState::NeedsRename
        } else {
            IdentityState::Ready
        };

        report.loaded.push(StoredPromptEntry {
            id: record.id,
            prompt,
            identity_state,
        });
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_prompt() -> StoredPrompt {
        StoredPrompt {
            display_name: "Check Email".to_string(),
            normalized_name: "check-email".to_string(),
            instructions: "do thing".to_string(),
            skills: Vec::new(),
            profile: None,
        }
    }

    #[test]
    fn stored_prompt_rejects_empty_instructions() {
        let prompt = StoredPrompt {
            display_name: "Test".to_string(),
            normalized_name: "test".to_string(),
            instructions: "   ".to_string(),
            skills: Vec::new(),
            profile: None,
        };
        assert!(prompt.validate().is_err());
    }

    #[test]
    fn stored_prompt_rejects_empty_display_name() {
        let prompt = StoredPrompt {
            display_name: "  ".to_string(),
            normalized_name: "".to_string(),
            instructions: "do thing".to_string(),
            skills: Vec::new(),
            profile: None,
        };
        assert!(prompt.validate().is_err());
    }

    #[test]
    fn stored_prompt_rejects_mismatched_normalized() {
        let prompt = StoredPrompt {
            display_name: "Check Email".to_string(),
            normalized_name: "wrong-handle".to_string(),
            instructions: "do thing".to_string(),
            skills: Vec::new(),
            profile: None,
        };
        assert!(prompt.validate().is_err());
    }

    #[test]
    fn registry_replaces_existing() {
        let mut reg = StoredPromptRegistry::new();
        let prompt = valid_prompt();
        reg.register("task".to_string(), prompt.clone()).unwrap();
        reg.register("task".to_string(), prompt).unwrap();
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn registry_list_is_sorted() {
        let mut reg = StoredPromptRegistry::new();
        let prompt = valid_prompt();
        reg.register("b".to_string(), prompt.clone()).unwrap();
        reg.register("a".to_string(), prompt).unwrap();
        let list = reg.list();
        assert_eq!(list[0].id, "a");
        assert_eq!(list[1].id, "b");
    }

    // ---- normalize_prompt_name tests ----

    #[test]
    fn normalize_basic() {
        assert_eq!(normalize_prompt_name("Check Email"), "check-email");
    }

    #[test]
    fn normalize_underscore_as_separator() {
        assert_eq!(normalize_prompt_name("Check_Email"), "check-email");
    }

    #[test]
    fn normalize_equivalent_separators() {
        assert_eq!(normalize_prompt_name("Check Email"), "check-email");
        assert_eq!(normalize_prompt_name("Check_Email"), "check-email");
        assert_eq!(normalize_prompt_name("CHECK-EMAIL"), "check-email");
    }

    #[test]
    fn normalize_collapses_multiple_separators() {
        assert_eq!(normalize_prompt_name("My___Task"), "my-task");
        assert_eq!(normalize_prompt_name("My   Task"), "my-task");
    }

    #[test]
    fn normalize_strips_leading_trailing() {
        assert_eq!(normalize_prompt_name("  __Check__  "), "check");
    }

    #[test]
    fn normalize_removes_special_chars() {
        assert_eq!(normalize_prompt_name("Paul's Brief!"), "pauls-brief");
    }

    #[test]
    fn normalize_empty_result() {
        assert_eq!(normalize_prompt_name("'!!!'"), "");
    }

    #[test]
    fn normalize_ascii_only() {
        assert_eq!(normalize_prompt_name("Café Report"), "caf-report");
    }

    // ---- kebab_to_title_case tests ----

    #[test]
    fn title_case_basic() {
        assert_eq!(kebab_to_title_case("check-email"), "Check Email");
    }

    #[test]
    fn title_case_underscore() {
        assert_eq!(kebab_to_title_case("daily_report"), "Daily Report");
    }

    #[test]
    fn title_case_single_word() {
        assert_eq!(kebab_to_title_case("report"), "Report");
    }
}
