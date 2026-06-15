//! Stored prompt definitions and registry.
//!
//! Stored prompts are named reusable task definitions persisted in the core
//! config store. They are invoked through delegated child-session execution.

use crate::config::{ConfigError, ConfigStore};
use crate::profile::AgentProfileId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Schema version for typed `StoredPrompt` payloads stored in ConfigStore.
pub const STORED_PROMPT_SCHEMA_VERSION: i64 = 1;

/// A reusable prompt task definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredPrompt {
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
    /// Validate invariants.
    pub fn validate(&self) -> Result<(), String> {
        if self.instructions.trim().is_empty() {
            return Err("stored prompt instructions must not be empty".to_string());
        }
        Ok(())
    }
}

/// A stored prompt paired with its stable ID.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPromptEntry {
    pub id: String,
    pub prompt: StoredPrompt,
}

/// Issue category reported for a skipped prompt during `load_prompts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptLoadIssue {
    UnsupportedSchemaVersion { version: i64 },
    InvalidPayload,
    MissingRecord,
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

/// Load stored prompts from a ConfigStore.
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

        if record.schema_version != STORED_PROMPT_SCHEMA_VERSION {
            report.diagnostics.push(PromptLoadDiagnostic {
                prompt_id: record.id,
                issue: PromptLoadIssue::UnsupportedSchemaVersion {
                    version: record.schema_version,
                },
            });
            continue;
        }

        let prompt: StoredPrompt = match serde_json::from_value(record.payload) {
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

        report.loaded.push(StoredPromptEntry {
            id: record.id,
            prompt,
        });
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_prompt_rejects_empty_instructions() {
        let prompt = StoredPrompt {
            instructions: "   ".to_string(),
            skills: Vec::new(),
            profile: None,
        };
        assert!(prompt.validate().is_err());
    }

    #[test]
    fn registry_replaces_existing() {
        let mut reg = StoredPromptRegistry::new();
        let prompt = StoredPrompt {
            instructions: "do thing".to_string(),
            skills: Vec::new(),
            profile: None,
        };
        reg.register("task".to_string(), prompt.clone()).unwrap();
        reg.register("task".to_string(), prompt).unwrap();
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn registry_list_is_sorted() {
        let mut reg = StoredPromptRegistry::new();
        let prompt = StoredPrompt {
            instructions: "do thing".to_string(),
            skills: Vec::new(),
            profile: None,
        };
        reg.register("b".to_string(), prompt.clone()).unwrap();
        reg.register("a".to_string(), prompt).unwrap();
        let list = reg.list();
        assert_eq!(list[0].id, "a");
        assert_eq!(list[1].id, "b");
    }
}
