//! Typed configuration management service for UI callers.
//!
//! [`ConfigManagementService`] provides application-facing typed CRUD,
//! diagnostics, dependency impact analysis, credential management, and
//! optional scheduler composition over existing domain stores. It is the
//! single boundary between the desktop application and core-owned
//! configuration.
//!
//! # Important constraints
//!
//! - Stored prompts are never direct scheduler targets. Scheduling is
//!   strictly an automation-task concern: `StoredPrompt <- AutomationTask <-
//!   ScheduledTask`.
//! - This service does not create a second non-interactive stored-prompt
//!   runner. Unattended execution continues through `agent-iron run
//!   <automation-task-id>`.
//! - Active in-memory child-session APIs (`hidden_sessions`,
//!   `child_sessions`) are not durable scheduled-run history. Historical
//!   run browsing requires the follow-up capability tracked in #98.
//! - Interactive stored-prompt preview is deferred to #97.

use crate::automation_task::{AutomationTask, AutomationTaskInput};
use crate::config::{ConfigError, ConfigStore};
use crate::profile::{AgentProfile, AgentProfileId, AgentProfileProvider, PROFILE_SCHEMA_VERSION};
use crate::provider_credential::domain::{CredentialMode, ProviderAuthStatus};
use crate::scheduled_task::host::{HostScheduler, SchedulerInstallContext};
use crate::scheduled_task::manager::ScheduleManager;
use crate::scheduled_task::{ScheduleStatus, ScheduledTask, ScheduledTaskInput};
use crate::stored_prompt::{
    normalize_prompt_name, IdentityState, StoredPrompt, StoredPromptRegistry,
};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Error hierarchy
// ============================================================================

/// Fatal errors from management operations.
#[derive(Debug, Error)]
pub enum ManagementError {
    #[error("Storage error: {0}")]
    Storage(ConfigError),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Reference error: {0}")]
    Reference(String),

    #[error("Conflict: {target} is referenced by {referrers:?}")]
    Conflict {
        target: String,
        referrers: Vec<String>,
    },

    #[error("Cannot verify referential integrity: {details}")]
    IntegrityUnknown { details: String },

    #[error("Scheduler is not attached")]
    SchedulerUnavailable,

    #[error("Scheduler error: {0}")]
    Scheduler(String),

    #[error("Partial operation: durable_succeeded={durable_succeeded}, error={error}")]
    Partial {
        durable_succeeded: bool,
        error: String,
    },
}

impl From<ConfigError> for ManagementError {
    fn from(e: ConfigError) -> Self {
        match e {
            ConfigError::Validation(msg) => ManagementError::Validation(msg),
            ConfigError::PromptReferencedByTasks {
                prompt_id,
                task_ids,
            } => ManagementError::Conflict {
                target: prompt_id,
                referrers: task_ids,
            },
            ConfigError::TaskReferencedBySchedules {
                task_id,
                schedule_ids,
            } => ManagementError::Conflict {
                target: task_id,
                referrers: schedule_ids,
            },
            ConfigError::ProfileReferencedByPrompts {
                profile_id,
                prompt_ids,
            } => ManagementError::Conflict {
                target: profile_id,
                referrers: prompt_ids,
            },
            ConfigError::PromptNameConflict {
                normalized_name,
                existing_id,
            } => ManagementError::Conflict {
                target: normalized_name,
                referrers: vec![existing_id],
            },
            ConfigError::IntegrityUnknown { details } => {
                ManagementError::IntegrityUnknown { details }
            }
            other => ManagementError::Storage(other),
        }
    }
}

// ============================================================================
// Managed record outcome
// ============================================================================

/// Diagnostic category for a single record issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticCategory {
    UnsupportedSchemaVersion,
    InvalidPayload,
    MissingRecord,
    UnavailableProfile,
    UnavailableSkill,
    NeedsRename,
    ReadOnlyRejected,
}

/// A diagnostic for a single managed record.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordDiagnostic {
    pub category: DiagnosticCategory,
    pub message: String,
}

/// Outcome of reading a single managed record.
///
/// Both single and bulk reads use this shape so malformed records remain
/// discoverable without being returned as valid values.
#[derive(Debug, Clone, PartialEq)]
pub enum ManagedRecord<T> {
    /// The record is valid and ready for use.
    Ready(T),
    /// The record exists but has one or more issues requiring attention.
    NeedsAttention {
        id: String,
        decoded: Option<T>,
        diagnostics: Vec<RecordDiagnostic>,
    },
}

// ============================================================================
// Dependency impact types
// ============================================================================

/// A typed entity in the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DependencyEntity {
    ProviderCredential { slug: String },
    Profile { id: String },
    Prompt { id: String },
    AutomationTask { id: String },
    ScheduledTask { id: String },
}

/// Whether a link points in the dependency or dependent direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyDirection {
    /// The target depends on this entity.
    Depends,
    /// This entity depends on the target.
    Dependent,
}

/// Whether a link is a direct reference or a transitive chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyProximity {
    Direct,
    Transitive,
}

/// A single link in the dependency graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DependencyLink {
    pub entity: DependencyEntity,
    pub direction: DependencyDirection,
    pub proximity: DependencyProximity,
    /// Ordered path from the target to this entity.
    pub path: Vec<DependencyEntity>,
}

/// Result of querying the dependency impact of a target entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DependencyImpactReport {
    pub target: DependencyEntity,
    pub links: Vec<DependencyLink>,
}

// ============================================================================
// Credential summary
// ============================================================================

/// Secret-safe summary of a configured provider credential.
///
/// Contains only non-secret metadata and persisted-state auth status.
/// Serialization and debug output of this type cannot reveal stored
/// credential material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CredentialSummary {
    pub provider_slug: String,
    pub credential_mode: CredentialMode,
    /// Persisted-state auth status derived from the credential mode.
    pub auth_status: ProviderAuthStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Schedule deletion outcome
// ============================================================================

/// Result of a combined schedule deletion (host + desired state).
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleDeletionOutcome {
    pub schedule_id: String,
    pub host_removed: bool,
    pub desired_deleted: bool,
    pub error: Option<String>,
}

// ============================================================================
// Config Management Service
// ============================================================================

/// Application-facing typed configuration management service.
///
/// Constructed from a [`ConfigStore`] with optional attached registries and
/// scheduler dependencies. Profile, prompt, automation-task, and credential
/// management remain available when host scheduling is not attached.
pub struct ConfigManagementService {
    store: ConfigStore,
    profile_registry: Option<Arc<RwLock<HashMap<AgentProfileId, AgentProfile>>>>,
    prompt_registry: Option<Arc<RwLock<StoredPromptRegistry>>>,
    host_scheduler: Option<Arc<dyn HostScheduler>>,
    install_context: Option<SchedulerInstallContext>,
}

impl ConfigManagementService {
    /// Create a new management service over a ConfigStore.
    pub fn new(store: ConfigStore) -> Self {
        Self {
            store,
            profile_registry: None,
            prompt_registry: None,
            host_scheduler: None,
            install_context: None,
        }
    }

    /// Attach a profile registry for post-persistence synchronization.
    pub fn with_profile_registry(
        mut self,
        registry: Arc<RwLock<HashMap<AgentProfileId, AgentProfile>>>,
    ) -> Self {
        self.profile_registry = Some(registry);
        self
    }

    /// Attach a prompt registry for post-persistence synchronization.
    pub fn with_prompt_registry(mut self, registry: Arc<RwLock<StoredPromptRegistry>>) -> Self {
        self.prompt_registry = Some(registry);
        self
    }

    /// Attach host scheduler support.
    pub fn with_scheduler(
        mut self,
        host: Arc<dyn HostScheduler>,
        context: SchedulerInstallContext,
    ) -> Self {
        self.host_scheduler = Some(host);
        self.install_context = Some(context);
        self
    }

    // ========================================================================
    // Profile management (Tasks 2.1, 2.3, 2.4)
    // ========================================================================

    /// Save a typed agent profile.
    ///
    /// Validates the profile using existing rules including rejection of
    /// `ReadOnly` and `RequireApproval` approval values, then persists it
    /// using `PROFILE_SCHEMA_VERSION`.
    pub async fn save_profile(
        &self,
        id: &str,
        profile: &AgentProfile,
    ) -> Result<(), ManagementError> {
        use crate::config::ProfileInput;

        let trimmed_id = id.trim();
        if !crate::profile::is_valid_profile_id(trimmed_id) {
            return Err(ManagementError::Validation(format!(
                "Invalid profile ID: '{}'",
                trimmed_id
            )));
        }

        let name = profile.name.trim();
        let normalized_name = crate::profile::normalize_profile_name(name).ok_or_else(|| {
            ManagementError::Validation(
                "Profile name must not be empty, reserved, or contain control characters"
                    .to_string(),
            )
        })?;

        let mut durable = profile.clone();
        durable.name = normalized_name;

        let payload = serde_json::to_value(&durable)
            .map_err(|e| ManagementError::Storage(ConfigError::Serialization(e.to_string())))?;

        let input = ProfileInput {
            id: trimmed_id.to_string(),
            schema_version: PROFILE_SCHEMA_VERSION,
            payload,
        };

        self.store.set_profile(&input).await?;

        // Sync attached registry after durable success. The same normalized
        // name is used for both the durable payload and the registry entry.
        if let Some(registry) = &self.profile_registry {
            let mut reg = registry.write();
            reg.insert(AgentProfileId::from(trimmed_id), durable);
        }

        Ok(())
    }

    /// Get a single profile by stable ID.
    pub async fn get_profile(
        &self,
        id: &str,
    ) -> Result<Option<ManagedProfileRecord>, ManagementError> {
        let record = self.store.get_profile(id).await?;
        match record {
            None => Ok(None),
            Some(record) => {
                if record.schema_version != PROFILE_SCHEMA_VERSION {
                    return Ok(Some(ManagedProfileRecord::NeedsAttention {
                        id: record.id,
                        decoded: None,
                        diagnostics: vec![RecordDiagnostic {
                            category: DiagnosticCategory::UnsupportedSchemaVersion,
                            message: format!(
                                "Profile schema version {} is not supported",
                                record.schema_version
                            ),
                        }],
                    }));
                }
                match serde_json::from_value::<AgentProfile>(record.payload.clone()) {
                    Ok(profile) => Ok(Some(ManagedProfileRecord::Ready(ManagedProfileEntry {
                        id: AgentProfileId::from(record.id),
                        profile,
                        created_at: record.created_at,
                        updated_at: record.updated_at,
                    }))),
                    Err(_) => Ok(Some(ManagedProfileRecord::NeedsAttention {
                        id: record.id,
                        decoded: None,
                        diagnostics: vec![RecordDiagnostic {
                            category: DiagnosticCategory::InvalidPayload,
                            message: "Profile payload could not be decoded".to_string(),
                        }],
                    })),
                }
            }
        }
    }

    /// List all profiles with per-record diagnostics.
    pub async fn list_profiles(&self) -> Result<Vec<ManagedProfileRecord>, ManagementError> {
        let ids = self.store.list_profile_ids().await?;
        let mut results = Vec::new();
        for id in ids {
            if let Some(record) = self.store.get_profile(&id).await? {
                if record.schema_version != PROFILE_SCHEMA_VERSION {
                    results.push(ManagedProfileRecord::NeedsAttention {
                        id: record.id,
                        decoded: None,
                        diagnostics: vec![RecordDiagnostic {
                            category: DiagnosticCategory::UnsupportedSchemaVersion,
                            message: format!(
                                "Profile schema version {} is not supported",
                                record.schema_version
                            ),
                        }],
                    });
                    continue;
                }
                match serde_json::from_value::<AgentProfile>(record.payload.clone()) {
                    Ok(profile) => {
                        results.push(ManagedProfileRecord::Ready(ManagedProfileEntry {
                            id: AgentProfileId::from(record.id),
                            profile,
                            created_at: record.created_at,
                            updated_at: record.updated_at,
                        }));
                    }
                    Err(_) => {
                        results.push(ManagedProfileRecord::NeedsAttention {
                            id: record.id,
                            decoded: None,
                            diagnostics: vec![RecordDiagnostic {
                                category: DiagnosticCategory::InvalidPayload,
                                message: "Profile payload could not be decoded".to_string(),
                            }],
                        });
                    }
                }
            }
        }
        Ok(results)
    }

    /// Delete a profile, blocking if stored prompts reference it.
    ///
    /// Returns `ManagementError::Conflict` if prompts reference this profile.
    /// Returns `ManagementError::IntegrityUnknown` if malformed records
    /// prevent safe reference checking.
    pub async fn delete_profile(&self, id: &str) -> Result<(), ManagementError> {
        // Check for malformed records that could obscure references.
        let prompt_ids = self.store.list_prompt_ids_sorted().await?;
        for pid in &prompt_ids {
            match self.store.get_prompt(pid).await {
                Ok(Some(record)) => {
                    // Only v1 (legacy) and v2 (current) prompt records can be
                    // reliably inspected for profile references.
                    if record.schema_version
                        != crate::stored_prompt::LEGACY_STORED_PROMPT_SCHEMA_VERSION
                        && record.schema_version
                            != crate::stored_prompt::STORED_PROMPT_SCHEMA_VERSION
                    {
                        return Err(ManagementError::IntegrityUnknown {
                            details: format!(
                                "Prompt '{}' has unsupported schema version {}",
                                pid, record.schema_version
                            ),
                        });
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    return Err(ManagementError::IntegrityUnknown {
                        details: format!(
                            "Prompt '{}' could not be read to verify referential integrity",
                            pid
                        ),
                    });
                }
            }
        }

        self.store.delete_profile_checked(id).await?;

        // Sync attached registry.
        if let Some(registry) = &self.profile_registry {
            let mut reg = registry.write();
            reg.remove(&AgentProfileId::from(id));
        }

        Ok(())
    }

    // ========================================================================
    // Stored-prompt management (Tasks 2.2, 2.3)
    // ========================================================================

    /// Create a new stored prompt with a core-generated immutable ID.
    ///
    /// Returns the generated ID and the persisted entry.
    pub async fn create_prompt(
        &self,
        display_name: &str,
        instructions: &str,
        skills: Vec<String>,
        profile: Option<AgentProfileId>,
    ) -> Result<String, ManagementError> {
        let id = format!("prompt-{}", Uuid::new_v4());
        let normalized = normalize_prompt_name(display_name);
        if normalized.is_empty() {
            return Err(ManagementError::Validation(
                "Display name must produce a non-empty normalized handle".to_string(),
            ));
        }

        let prompt = StoredPrompt {
            display_name: display_name.trim().to_string(),
            normalized_name: normalized,
            instructions: instructions.trim().to_string(),
            skills,
            profile,
        };
        prompt.validate().map_err(ManagementError::Validation)?;
        self.validate_prompt_references(&prompt).await?;

        self.store.set_typed_prompt(&id, &prompt).await?;
        self.sync_prompt_registry(&id, &prompt)?;
        Ok(id)
    }

    /// Save (create or replace) a stored prompt by stable ID.
    pub async fn save_prompt(
        &self,
        id: &str,
        prompt: &StoredPrompt,
    ) -> Result<(), ManagementError> {
        prompt.validate().map_err(ManagementError::Validation)?;
        self.validate_prompt_references(prompt).await?;
        self.store.set_typed_prompt(id, prompt).await?;
        self.sync_prompt_registry(id, prompt)?;
        Ok(())
    }

    /// Get a single stored prompt by immutable ID.
    pub async fn get_prompt(
        &self,
        id: &str,
    ) -> Result<Option<ManagedPromptRecord>, ManagementError> {
        let record = self.store.get_prompt(id).await?;
        match record {
            None => Ok(None),
            Some(record) => Ok(Some(self.decode_prompt_record(record))),
        }
    }

    /// Get a single stored prompt by normalized handle.
    pub async fn get_prompt_by_handle(
        &self,
        handle: &str,
    ) -> Result<Option<ManagedPromptRecord>, ManagementError> {
        let record = self.store.get_prompt_by_normalized_name(handle).await?;
        match record {
            None => Ok(None),
            Some(record) => Ok(Some(self.decode_prompt_record(record))),
        }
    }

    /// List all stored prompts with per-record diagnostics.
    pub async fn list_prompts(&self) -> Result<Vec<ManagedPromptRecord>, ManagementError> {
        let ids = self.store.list_prompt_ids_sorted().await?;
        let mut results = Vec::new();
        for id in ids {
            if let Some(record) = self.store.get_prompt(&id).await? {
                results.push(self.decode_prompt_record(record));
            }
        }
        Ok(results)
    }

    /// Delete a stored prompt, blocking if automation tasks reference it.
    pub async fn delete_prompt(&self, id: &str) -> Result<(), ManagementError> {
        // Ensure no malformed automation-task records could obscure references.
        // The underlying delete_prompt already checks task references; this
        // guard only refuses to proceed when integrity cannot be verified.
        if self.store.list_automation_tasks().await.is_err() {
            return Err(ManagementError::IntegrityUnknown {
                details: "Malformed automation-task records prevent reference verification"
                    .to_string(),
            });
        }

        self.store.delete_prompt(id).await?;

        if let Some(registry) = &self.prompt_registry {
            let mut reg = registry.write();
            reg.unregister(id);
        }

        Ok(())
    }

    /// Rename a stored prompt's display name while preserving its immutable ID.
    ///
    /// All automation-task references remain valid because they use the
    /// immutable ID, not the normalized handle.
    pub async fn rename_prompt(
        &self,
        id: &str,
        new_display_name: &str,
    ) -> Result<(), ManagementError> {
        let normalized = normalize_prompt_name(new_display_name);
        if normalized.is_empty() {
            return Err(ManagementError::Validation(
                "Display name must produce a non-empty normalized handle".to_string(),
            ));
        }

        let existing = self
            .store
            .get_prompt(id)
            .await?
            .ok_or_else(|| ManagementError::Reference(format!("Prompt '{}' not found", id)))?;

        let prompt: StoredPrompt = match serde_json::from_value(existing.payload) {
            Ok(p) => p,
            Err(_) => {
                return Err(ManagementError::Reference(format!(
                    "Prompt '{}' has undecodable payload",
                    id
                )))
            }
        };

        let updated = StoredPrompt {
            display_name: new_display_name.trim().to_string(),
            normalized_name: normalized,
            instructions: prompt.instructions,
            skills: prompt.skills,
            profile: prompt.profile,
        };
        self.store.set_typed_prompt(id, &updated).await?;
        self.sync_prompt_registry(id, &updated)?;
        Ok(())
    }

    // ========================================================================
    // Automation-task management (delegating to ConfigStore)
    // ========================================================================

    /// Save (create or replace) an automation task.
    pub async fn save_automation_task(
        &self,
        input: &AutomationTaskInput,
    ) -> Result<AutomationTask, ManagementError> {
        self.store
            .set_automation_task(input)
            .await
            .map_err(Into::into)
    }

    /// Get an automation task by ID.
    ///
    /// Returns a [`ManagedAutomationTaskRecord`] so records with an
    /// unsupported schema version are surfaced as `NeedsAttention` rather
    /// than failing the whole call.
    pub async fn get_automation_task(
        &self,
        id: &str,
    ) -> Result<Option<ManagedAutomationTaskRecord>, ManagementError> {
        match self.store.get_automation_task(id).await {
            Ok(Some(task)) => Ok(Some(ManagedAutomationTaskRecord::Ready(task))),
            Ok(None) => Ok(None),
            Err(ConfigError::Deserialization(msg)) => {
                Ok(Some(ManagedAutomationTaskRecord::NeedsAttention {
                    id: id.to_string(),
                    decoded: None,
                    diagnostics: vec![RecordDiagnostic {
                        category: DiagnosticCategory::UnsupportedSchemaVersion,
                        message: msg,
                    }],
                }))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// List all automation tasks in deterministic order.
    ///
    /// Records with an unsupported schema version are surfaced through the
    /// store as errors; valid records are wrapped as `Ready`.
    pub async fn list_automation_tasks(
        &self,
    ) -> Result<Vec<ManagedAutomationTaskRecord>, ManagementError> {
        let tasks = self.store.list_automation_tasks().await?;
        Ok(tasks
            .into_iter()
            .map(ManagedAutomationTaskRecord::Ready)
            .collect())
    }

    /// Delete an automation task, blocking if schedules reference it.
    ///
    /// Refuses to proceed when scheduled-task records are unreadable, since
    /// malformed schedule records could obscure task references. The
    /// underlying store delete already checks schedule references.
    pub async fn delete_automation_task(&self, id: &str) -> Result<(), ManagementError> {
        if self.store.list_scheduled_tasks().await.is_err() {
            return Err(ManagementError::IntegrityUnknown {
                details: "Malformed scheduled-task records prevent reference verification"
                    .to_string(),
            });
        }

        self.store
            .delete_automation_task(id)
            .await
            .map_err(Into::into)
    }

    // ========================================================================
    // Dependency impact (Tasks 3.1-3.4)
    // ========================================================================

    /// Query the dependency impact of a profile.
    ///
    /// Returns all prompts, automation tasks, and schedules that depend on
    /// the given profile, directly or transitively.
    pub async fn profile_impact(
        &self,
        profile_id: &str,
    ) -> Result<DependencyImpactReport, ManagementError> {
        let target = DependencyEntity::Profile {
            id: profile_id.to_string(),
        };
        let mut report = DependencyImpactReport {
            target,
            links: Vec::new(),
        };

        let prompt_ids = self.store.prompts_referencing_profile(profile_id).await?;
        for pid in &prompt_ids {
            let prompt_entity = DependencyEntity::Prompt { id: pid.clone() };
            report.links.push(DependencyLink {
                entity: prompt_entity.clone(),
                direction: DependencyDirection::Dependent,
                proximity: DependencyProximity::Direct,
                path: vec![report.target.clone(), prompt_entity.clone()],
            });

            let task_ids = self.store.tasks_referencing_prompt(pid).await?;
            for tid in &task_ids {
                let task_entity = DependencyEntity::AutomationTask { id: tid.clone() };
                report.links.push(DependencyLink {
                    entity: task_entity.clone(),
                    direction: DependencyDirection::Dependent,
                    proximity: DependencyProximity::Transitive,
                    path: vec![
                        report.target.clone(),
                        prompt_entity.clone(),
                        task_entity.clone(),
                    ],
                });

                let schedule_ids = self.store.schedules_referencing_task(tid).await?;
                for sid in &schedule_ids {
                    let sched_entity = DependencyEntity::ScheduledTask { id: sid.clone() };
                    report.links.push(DependencyLink {
                        entity: sched_entity.clone(),
                        direction: DependencyDirection::Dependent,
                        proximity: DependencyProximity::Transitive,
                        path: vec![
                            report.target.clone(),
                            prompt_entity.clone(),
                            task_entity.clone(),
                            sched_entity,
                        ],
                    });
                }
            }
        }

        Ok(report)
    }

    /// Query the dependency impact of a provider credential.
    ///
    /// Returns all profiles, prompts, automation tasks, and schedules that
    /// depend on the given credential slug.
    pub async fn credential_impact(
        &self,
        slug: &str,
    ) -> Result<DependencyImpactReport, ManagementError> {
        let target = DependencyEntity::ProviderCredential {
            slug: slug.to_string(),
        };
        let mut report = DependencyImpactReport {
            target,
            links: Vec::new(),
        };

        let profiles = self.list_profiles().await?;
        for entry in profiles {
            let (pid, profile) = match entry {
                ManagedProfileRecord::Ready(ManagedProfileEntry { id, profile, .. }) => {
                    (id, profile)
                }
                _ => continue,
            };
            if let AgentProfileProvider::Managed { provider_slug, .. } = &profile.provider {
                if provider_slug.as_str() == slug {
                    let profile_entity = DependencyEntity::Profile {
                        id: pid.as_str().to_string(),
                    };
                    report.links.push(DependencyLink {
                        entity: profile_entity.clone(),
                        direction: DependencyDirection::Dependent,
                        proximity: DependencyProximity::Direct,
                        path: vec![report.target.clone(), profile_entity.clone()],
                    });

                    let prompts = self.store.prompts_referencing_profile(pid.as_str()).await?;
                    for prompt_id in &prompts {
                        let prompt_entity = DependencyEntity::Prompt {
                            id: prompt_id.clone(),
                        };
                        report.links.push(DependencyLink {
                            entity: prompt_entity.clone(),
                            direction: DependencyDirection::Dependent,
                            proximity: DependencyProximity::Transitive,
                            path: vec![
                                report.target.clone(),
                                profile_entity.clone(),
                                prompt_entity.clone(),
                            ],
                        });

                        let tasks = self.store.tasks_referencing_prompt(prompt_id).await?;
                        for tid in &tasks {
                            let task_entity = DependencyEntity::AutomationTask { id: tid.clone() };
                            report.links.push(DependencyLink {
                                entity: task_entity.clone(),
                                direction: DependencyDirection::Dependent,
                                proximity: DependencyProximity::Transitive,
                                path: vec![
                                    report.target.clone(),
                                    profile_entity.clone(),
                                    prompt_entity.clone(),
                                    task_entity.clone(),
                                ],
                            });

                            let schedules = self.store.schedules_referencing_task(tid).await?;
                            for sid in &schedules {
                                let sched_entity =
                                    DependencyEntity::ScheduledTask { id: sid.clone() };
                                report.links.push(DependencyLink {
                                    entity: sched_entity.clone(),
                                    direction: DependencyDirection::Dependent,
                                    proximity: DependencyProximity::Transitive,
                                    path: vec![
                                        report.target.clone(),
                                        profile_entity.clone(),
                                        prompt_entity.clone(),
                                        task_entity.clone(),
                                        sched_entity,
                                    ],
                                });
                            }
                        }
                    }
                }
            }
        }

        Ok(report)
    }

    /// Query the dependency impact of a stored prompt.
    pub async fn prompt_impact(
        &self,
        prompt_id: &str,
    ) -> Result<DependencyImpactReport, ManagementError> {
        let target = DependencyEntity::Prompt {
            id: prompt_id.to_string(),
        };
        let mut report = DependencyImpactReport {
            target,
            links: Vec::new(),
        };

        // Direct dependency: the profile this prompt references (if any).
        if let Some(prompt_record) = self.store.get_prompt(prompt_id).await? {
            if let Ok(prompt) = serde_json::from_value::<StoredPrompt>(prompt_record.payload) {
                if let Some(profile_id) = prompt.profile {
                    let profile_entity = DependencyEntity::Profile {
                        id: profile_id.as_str().to_string(),
                    };
                    report.links.push(DependencyLink {
                        entity: profile_entity.clone(),
                        direction: DependencyDirection::Depends,
                        proximity: DependencyProximity::Direct,
                        path: vec![report.target.clone(), profile_entity.clone()],
                    });
                }
            }
        }

        let task_ids = self.store.tasks_referencing_prompt(prompt_id).await?;
        for tid in &task_ids {
            let task_entity = DependencyEntity::AutomationTask { id: tid.clone() };
            report.links.push(DependencyLink {
                entity: task_entity.clone(),
                direction: DependencyDirection::Dependent,
                proximity: DependencyProximity::Direct,
                path: vec![report.target.clone(), task_entity.clone()],
            });

            let schedule_ids = self.store.schedules_referencing_task(tid).await?;
            for sid in &schedule_ids {
                report.links.push(DependencyLink {
                    entity: DependencyEntity::ScheduledTask { id: sid.clone() },
                    direction: DependencyDirection::Dependent,
                    proximity: DependencyProximity::Transitive,
                    path: vec![
                        report.target.clone(),
                        task_entity.clone(),
                        DependencyEntity::ScheduledTask { id: sid.clone() },
                    ],
                });
            }
        }

        Ok(report)
    }

    /// Query the dependency impact of an automation task.
    pub async fn task_impact(
        &self,
        task_id: &str,
    ) -> Result<DependencyImpactReport, ManagementError> {
        let target = DependencyEntity::AutomationTask {
            id: task_id.to_string(),
        };
        let mut report = DependencyImpactReport {
            target,
            links: Vec::new(),
        };

        let schedule_ids = self.store.schedules_referencing_task(task_id).await?;
        for sid in &schedule_ids {
            report.links.push(DependencyLink {
                entity: DependencyEntity::ScheduledTask { id: sid.clone() },
                direction: DependencyDirection::Dependent,
                proximity: DependencyProximity::Direct,
                path: vec![
                    report.target.clone(),
                    DependencyEntity::ScheduledTask { id: sid.clone() },
                ],
            });
        }

        // Direct dependency: the stored prompt this task references.
        if let Some(task) = self.store.get_automation_task(task_id).await? {
            let prompt_entity = DependencyEntity::Prompt {
                id: task.stored_prompt_id.clone(),
            };
            report.links.push(DependencyLink {
                entity: prompt_entity.clone(),
                direction: DependencyDirection::Depends,
                proximity: DependencyProximity::Direct,
                path: vec![report.target.clone(), prompt_entity.clone()],
            });

            // Transitive dependency: the prompt's profile (if any).
            if let Some(prompt_record) = self.store.get_prompt(&task.stored_prompt_id).await? {
                if let Ok(prompt) = serde_json::from_value::<StoredPrompt>(prompt_record.payload) {
                    if let Some(profile_id) = prompt.profile {
                        let profile_entity = DependencyEntity::Profile {
                            id: profile_id.as_str().to_string(),
                        };
                        report.links.push(DependencyLink {
                            entity: profile_entity.clone(),
                            direction: DependencyDirection::Depends,
                            proximity: DependencyProximity::Transitive,
                            path: vec![
                                report.target.clone(),
                                prompt_entity.clone(),
                                profile_entity.clone(),
                            ],
                        });
                    }
                }
            }
        }

        Ok(report)
    }

    // ========================================================================
    // Credential management (Tasks 4.1-4.4)
    // ========================================================================

    /// List configured credential summaries without secret material.
    pub async fn list_credentials(&self) -> Result<Vec<CredentialSummary>, ManagementError> {
        let slugs = self.store.list_credential_slugs().await?;
        let mut results = Vec::new();
        for slug in &slugs {
            if let Some(record) = self.store.get_credential_metadata(slug).await? {
                let (mode, auth_status) = match record.credential_mode.as_str() {
                    "api_key" => (CredentialMode::ApiKey, ProviderAuthStatus::ConfiguredApiKey),
                    "oauth_bearer" => (
                        CredentialMode::OAuthBearer,
                        ProviderAuthStatus::ConnectedOAuth { expires_at: None },
                    ),
                    // Unknown modes are skipped rather than failing the list.
                    _ => continue,
                };
                results.push(CredentialSummary {
                    provider_slug: record.provider_slug,
                    credential_mode: mode,
                    auth_status,
                    created_at: record.created_at,
                    updated_at: record.updated_at,
                });
            }
        }
        Ok(results)
    }

    /// Add or replace an API-key credential.
    ///
    /// Replaces whichever credential mode is currently configured, including
    /// OAuth. Returns a redacted summary.
    pub async fn set_api_key(
        &self,
        provider_slug: &str,
        api_key: &str,
    ) -> Result<CredentialSummary, ManagementError> {
        if api_key.trim().is_empty() {
            return Err(ManagementError::Validation(
                "API key must not be empty".to_string(),
            ));
        }

        let credential =
            crate::provider_credential::domain::StoredCredential::ApiKey(api_key.to_string());
        let payload = serde_json::to_vec(&credential)
            .map_err(|e| ManagementError::Storage(ConfigError::Serialization(e.to_string())))?;

        self.store
            .set_credential(provider_slug, "api_key", &payload)
            .await?;

        // Reread persisted metadata so created_at/updated_at reflect durable
        // state rather than a transient clock value.
        let metadata = self
            .store
            .get_credential_metadata(provider_slug)
            .await?
            .ok_or_else(|| {
                ManagementError::Storage(ConfigError::NotFound(format!(
                    "credential metadata for '{}'",
                    provider_slug
                )))
            })?;

        Ok(CredentialSummary {
            provider_slug: metadata.provider_slug,
            credential_mode: CredentialMode::ApiKey,
            auth_status: ProviderAuthStatus::ConfiguredApiKey,
            created_at: metadata.created_at,
            updated_at: metadata.updated_at,
        })
    }

    /// Delete whichever credential mode is configured for a provider.
    pub async fn delete_credential(&self, provider_slug: &str) -> Result<(), ManagementError> {
        self.store.remove_credential(provider_slug).await?;
        Ok(())
    }

    // ========================================================================
    // Scheduled-task management (Tasks 5.3, 5.5)
    // ========================================================================

    /// Save (create or replace) a scheduled task.
    pub async fn save_scheduled_task(
        &self,
        input: &ScheduledTaskInput,
    ) -> Result<ScheduledTask, ManagementError> {
        self.store
            .set_scheduled_task(input)
            .await
            .map_err(Into::into)
    }

    /// Get a scheduled task by ID.
    pub async fn get_scheduled_task(
        &self,
        id: &str,
    ) -> Result<Option<ScheduledTask>, ManagementError> {
        self.store.get_scheduled_task(id).await.map_err(Into::into)
    }

    /// List all scheduled tasks in deterministic order.
    pub async fn list_scheduled_tasks(&self) -> Result<Vec<ScheduledTask>, ManagementError> {
        self.store.list_scheduled_tasks().await.map_err(Into::into)
    }

    /// Delete a scheduled task by ID (desired state only).
    pub async fn delete_scheduled_task(&self, id: &str) -> Result<(), ManagementError> {
        self.store
            .delete_scheduled_task(id)
            .await
            .map_err(Into::into)
    }

    /// Combined schedule deletion: removes host entry first, then desired state.
    ///
    /// Host-first ordering avoids leaving an orphan entry that can continue
    /// executing. Returns `Ok(outcome)` only when both host removal and
    /// desired-state deletion succeed (or desired state is already absent).
    /// Host removal failure yields [`ManagementError::Scheduler`]; a failure
    /// after successful host removal yields [`ManagementError::Partial`].
    pub async fn delete_schedule_combined(
        &self,
        schedule_id: &str,
    ) -> Result<ScheduleDeletionOutcome, ManagementError> {
        let host = self
            .host_scheduler
            .as_ref()
            .ok_or(ManagementError::SchedulerUnavailable)?;

        let store = &self.store;
        let mut outcome = ScheduleDeletionOutcome {
            schedule_id: schedule_id.to_string(),
            host_removed: false,
            desired_deleted: false,
            error: None,
        };

        if let Err(e) = host.remove(schedule_id).await {
            return Err(ManagementError::Scheduler(format!(
                "Host removal error: {}",
                e
            )));
        }

        // Check if desired state still exists.
        match store.get_scheduled_task(schedule_id).await {
            Ok(None) => {
                outcome.host_removed = true;
                outcome.desired_deleted = true;
                return Ok(outcome);
            }
            Ok(Some(_)) => {}
            Err(e) => {
                return Err(ManagementError::Partial {
                    durable_succeeded: false,
                    error: format!("Failed to read desired state after host removal: {}", e),
                });
            }
        }

        match store.delete_scheduled_task(schedule_id).await {
            Ok(_) => {
                outcome.host_removed = true;
                outcome.desired_deleted = true;
                Ok(outcome)
            }
            Err(e) => Err(ManagementError::Partial {
                durable_succeeded: false,
                error: e.to_string(),
            }),
        }
    }

    /// Inspect a schedule's status (read-only).
    pub async fn inspect_schedule(
        &self,
        schedule_id: &str,
    ) -> Result<ScheduleStatus, ManagementError> {
        let host = self
            .host_scheduler
            .as_ref()
            .ok_or(ManagementError::SchedulerUnavailable)?;
        let context = self
            .install_context
            .as_ref()
            .ok_or(ManagementError::SchedulerUnavailable)?;
        let manager = ScheduleManager::new(&self.store, host.as_ref(), context.clone());
        Ok(manager.inspect(schedule_id).await)
    }

    /// Inspect all schedules' status (read-only).
    pub async fn inspect_all_schedules(&self) -> Result<Vec<ScheduleStatus>, ManagementError> {
        let host = self
            .host_scheduler
            .as_ref()
            .ok_or(ManagementError::SchedulerUnavailable)?;
        let context = self
            .install_context
            .as_ref()
            .ok_or(ManagementError::SchedulerUnavailable)?;
        let manager = ScheduleManager::new(&self.store, host.as_ref(), context.clone());
        Ok(manager.inspect_all().await)
    }

    /// Explicitly reconcile a schedule.
    pub async fn reconcile_schedule(
        &self,
        schedule_id: &str,
    ) -> Result<ScheduleStatus, ManagementError> {
        let host = self
            .host_scheduler
            .as_ref()
            .ok_or(ManagementError::SchedulerUnavailable)?;
        let context = self
            .install_context
            .as_ref()
            .ok_or(ManagementError::SchedulerUnavailable)?;
        let manager = ScheduleManager::new(&self.store, host.as_ref(), context.clone());
        Ok(manager.reconcile(schedule_id).await)
    }

    // ========================================================================
    // Internal helpers
    // ========================================================================

    fn decode_prompt_record(&self, record: crate::config::PromptRecord) -> ManagedPromptRecord {
        let identity_state = if record.identity_state == "needs_rename" {
            IdentityState::NeedsRename
        } else {
            IdentityState::Ready
        };

        if record.schema_version == crate::stored_prompt::LEGACY_STORED_PROMPT_SCHEMA_VERSION {
            match serde_json::from_value::<StoredPrompt>(record.payload.clone()) {
                Ok(p) if !p.instructions.trim().is_empty() => {
                    let display_name = crate::stored_prompt::kebab_to_title_case(&record.id);
                    let normalized_name = normalize_prompt_name(&display_name);
                    return ManagedPromptRecord::Ready((
                        record.id.clone(),
                        ManagedPromptEntry {
                            prompt: StoredPrompt {
                                display_name,
                                normalized_name,
                                instructions: p.instructions,
                                skills: p.skills,
                                profile: p.profile,
                            },
                            identity_state,
                            created_at: record.created_at,
                            updated_at: record.updated_at,
                        },
                    ));
                }
                _ => {}
            }
        }

        if record.schema_version != crate::stored_prompt::STORED_PROMPT_SCHEMA_VERSION {
            return ManagedPromptRecord::NeedsAttention {
                id: record.id,
                decoded: None,
                diagnostics: vec![RecordDiagnostic {
                    category: DiagnosticCategory::UnsupportedSchemaVersion,
                    message: format!("Schema version {} not supported", record.schema_version),
                }],
            };
        }

        match serde_json::from_value::<StoredPrompt>(record.payload.clone()) {
            Ok(prompt) => {
                let mut diagnostics = Vec::new();
                if identity_state == IdentityState::NeedsRename {
                    diagnostics.push(RecordDiagnostic {
                        category: DiagnosticCategory::NeedsRename,
                        message: "Normalized name collided during migration; rename required"
                            .to_string(),
                    });
                }
                if !diagnostics.is_empty() {
                    ManagedPromptRecord::NeedsAttention {
                        id: record.id.clone(),
                        decoded: Some((
                            record.id,
                            ManagedPromptEntry {
                                prompt,
                                identity_state,
                                created_at: record.created_at,
                                updated_at: record.updated_at,
                            },
                        )),
                        diagnostics,
                    }
                } else {
                    ManagedPromptRecord::Ready((
                        record.id,
                        ManagedPromptEntry {
                            prompt,
                            identity_state,
                            created_at: record.created_at,
                            updated_at: record.updated_at,
                        },
                    ))
                }
            }
            Err(_) => ManagedPromptRecord::NeedsAttention {
                id: record.id,
                decoded: None,
                diagnostics: vec![RecordDiagnostic {
                    category: DiagnosticCategory::InvalidPayload,
                    message: "Prompt payload could not be decoded".to_string(),
                }],
            },
        }
    }

    fn sync_prompt_registry(&self, id: &str, prompt: &StoredPrompt) -> Result<(), ManagementError> {
        if let Some(registry) = &self.prompt_registry {
            let mut reg = registry.write();
            reg.register(id.to_string(), prompt.clone())
                .map_err(|e| ManagementError::Partial {
                    durable_succeeded: true,
                    error: e,
                })?;
        }
        Ok(())
    }

    /// Validate that a stored prompt's profile and skill references are sound.
    ///
    /// Ensures any non-`default` profile reference resolves to an existing
    /// profile record, rejects empty/whitespace skill identifiers, and rejects
    /// duplicate skills (case-insensitive).
    async fn validate_prompt_references(
        &self,
        prompt: &StoredPrompt,
    ) -> Result<(), ManagementError> {
        if let Some(ref pid) = prompt.profile {
            if pid.as_str() != "default" && self.store.get_profile(pid.as_str()).await?.is_none() {
                return Err(ManagementError::Reference(format!(
                    "Profile '{}' not found",
                    pid.as_str()
                )));
            }
        }

        let mut seen: HashSet<String> = HashSet::new();
        for skill in &prompt.skills {
            let trimmed = skill.trim();
            if trimmed.is_empty() {
                return Err(ManagementError::Validation(
                    "Skill identifiers must not be empty or whitespace".to_string(),
                ));
            }
            let key = trimmed.to_ascii_lowercase();
            if !seen.insert(key) {
                return Err(ManagementError::Validation(format!(
                    "Duplicate skill identifier: '{}'",
                    trimmed
                )));
            }
        }

        Ok(())
    }
}

// ============================================================================
// Managed entry types
// ============================================================================

/// A profile entry with timestamps.
#[derive(Debug, Clone, PartialEq)]
pub struct ManagedProfileEntry {
    pub id: AgentProfileId,
    pub profile: AgentProfile,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Outcome of reading a profile record.
pub type ManagedProfileRecord = ManagedRecord<ManagedProfileEntry>;

/// Outcome of reading an automation-task record.
pub type ManagedAutomationTaskRecord = ManagedRecord<AutomationTask>;

/// A prompt entry with timestamps and identity state.
#[derive(Debug, Clone, PartialEq)]
pub struct ManagedPromptEntry {
    pub prompt: StoredPrompt,
    pub identity_state: IdentityState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Outcome of reading a prompt record. The ID is always present.
pub type ManagedPromptRecord = ManagedRecord<(String, ManagedPromptEntry)>;

impl ManagedPromptRecord {
    /// Get the ID from either variant.
    pub fn id(&self) -> &str {
        match self {
            ManagedRecord::Ready((id, _)) => id,
            ManagedRecord::NeedsAttention { id, .. } => id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_summary_debug_does_not_contain_secrets() {
        let summary = CredentialSummary {
            provider_slug: "test".to_string(),
            credential_mode: CredentialMode::ApiKey,
            auth_status: ProviderAuthStatus::ConfiguredApiKey,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let debug_str = format!("{:?}", summary);
        assert!(!debug_str.contains("sk-"));
        assert!(!debug_str.contains("secret"));
        assert!(!debug_str.contains("token"));
    }

    #[test]
    fn credential_summary_serializes_without_secrets() {
        let summary = CredentialSummary {
            provider_slug: "test".to_string(),
            credential_mode: CredentialMode::ApiKey,
            auth_status: ProviderAuthStatus::ConfiguredApiKey,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&summary).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("api_key").is_none());
        assert!(parsed.get("access_token").is_none());
        assert!(parsed.get("refresh_token").is_none());
    }

    #[test]
    fn normalize_prompt_name_in_service() {
        assert_eq!(normalize_prompt_name("Check Email"), "check-email");
        assert_eq!(normalize_prompt_name("Check_Email"), "check-email");
        assert_eq!(normalize_prompt_name("CHECK EMAIL"), "check-email");
    }

    #[test]
    fn kebab_to_title_works() {
        assert_eq!(
            crate::stored_prompt::kebab_to_title_case("check-email"),
            "Check Email"
        );
    }

    #[test]
    fn dependency_entity_serializes() {
        let entity = DependencyEntity::Profile {
            id: "test".to_string(),
        };
        let json = serde_json::to_string(&entity).unwrap();
        let restored: DependencyEntity = serde_json::from_str(&json).unwrap();
        assert_eq!(entity, restored);
    }

    #[tokio::test]
    async fn management_service_constructs_without_scheduler() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);
        // Should be SchedulerUnavailable when scheduler ops are used
        let result = svc.inspect_schedule("test").await;
        assert!(matches!(result, Err(ManagementError::SchedulerUnavailable)));
    }

    #[tokio::test]
    async fn create_prompt_generates_unique_id() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let id1 = svc
            .create_prompt("Check Email", "Check inbox", vec![], None)
            .await
            .unwrap();
        let id2 = svc
            .create_prompt("Daily Report", "Generate report", vec![], None)
            .await
            .unwrap();
        assert_ne!(id1, id2);
        assert!(id1.starts_with("prompt-"));
        assert!(id2.starts_with("prompt-"));
    }

    #[tokio::test]
    async fn prompt_handle_lookup_is_case_insensitive() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let id = svc
            .create_prompt("Check Email", "Check inbox", vec![], None)
            .await
            .unwrap();

        let result = svc.get_prompt_by_handle("CHECK-EMAIL").await.unwrap();
        assert!(result.is_some());
        if let Some(ManagedPromptRecord::Ready((found_id, _))) = result {
            assert_eq!(found_id, id);
        } else {
            panic!("Expected Ready record");
        }
    }

    #[tokio::test]
    async fn prompt_collision_rejected() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        svc.create_prompt("Check Email", "First", vec![], None)
            .await
            .unwrap();

        let result = svc
            .create_prompt("check-email", "Second", vec![], None)
            .await;
        assert!(matches!(result, Err(ManagementError::Conflict { .. })));
    }

    #[tokio::test]
    async fn rename_preserves_id() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let id = svc
            .create_prompt("Check Email", "Check inbox", vec![], None)
            .await
            .unwrap();

        svc.rename_prompt(&id, "Process Inbox").await.unwrap();

        let result = svc.get_prompt(&id).await.unwrap();
        if let Some(ManagedPromptRecord::Ready((found_id, entry))) = result {
            assert_eq!(found_id, id);
            assert_eq!(entry.prompt.display_name, "Process Inbox");
            assert_eq!(entry.prompt.normalized_name, "process-inbox");
        } else {
            panic!("Expected Ready record");
        }
    }

    #[tokio::test]
    async fn credential_listing_omits_unconfigured() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let list = svc.list_credentials().await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn api_key_replace_and_delete() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let summary = svc.set_api_key("test-provider", "sk-secret").await.unwrap();
        assert_eq!(summary.provider_slug, "test-provider");
        assert_eq!(summary.credential_mode, CredentialMode::ApiKey);

        let list = svc.list_credentials().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].provider_slug, "test-provider");

        svc.delete_credential("test-provider").await.unwrap();

        let list = svc.list_credentials().await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn empty_api_key_rejected() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let result = svc.set_api_key("test", "  ").await;
        assert!(matches!(result, Err(ManagementError::Validation(_))));
    }

    #[tokio::test]
    async fn profile_impact_empty_when_no_references() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let report = svc.profile_impact("nonexistent").await.unwrap();
        assert!(report.links.is_empty());
    }

    #[tokio::test]
    async fn profile_save_and_get() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let profile = AgentProfile::with_name("Test Profile");
        svc.save_profile("test-prof", &profile).await.unwrap();

        let result = svc.get_profile("test-prof").await.unwrap();
        assert!(result.is_some());
        if let Some(ManagedProfileRecord::Ready(entry)) = result {
            assert_eq!(entry.id.as_str(), "test-prof");
            assert_eq!(entry.profile.name, "Test Profile");
        } else {
            panic!("Expected Ready record");
        }
    }

    #[tokio::test]
    async fn profile_delete_blocked_by_prompt() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        // Save profile
        let profile = AgentProfile::with_name("Test Profile");
        svc.save_profile("test-prof", &profile).await.unwrap();

        // Save prompt referencing the profile
        svc.create_prompt(
            "Test",
            "instructions",
            vec![],
            Some(AgentProfileId::from("test-prof")),
        )
        .await
        .unwrap();

        // Attempt to delete profile should fail
        let result = svc.delete_profile("test-prof").await;
        assert!(matches!(result, Err(ManagementError::Conflict { .. })));
    }

    #[tokio::test]
    async fn prompt_with_missing_profile_rejected() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let result = svc
            .create_prompt(
                "Test",
                "instructions",
                vec![],
                Some(AgentProfileId::from("nonexistent")),
            )
            .await;
        assert!(matches!(result, Err(ManagementError::Reference(_))));
    }

    #[tokio::test]
    async fn prompt_with_duplicate_skills_rejected() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let result = svc
            .create_prompt(
                "Test",
                "instructions",
                vec!["skill1".to_string(), "skill1".to_string()],
                None,
            )
            .await;
        assert!(matches!(result, Err(ManagementError::Validation(_))));
    }

    #[tokio::test]
    async fn prompt_with_empty_skill_rejected() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let result = svc
            .create_prompt("Test", "instructions", vec!["  ".to_string()], None)
            .await;
        assert!(matches!(result, Err(ManagementError::Validation(_))));
    }

    #[tokio::test]
    async fn handle_lookup_normalizes_spaces() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let id = svc
            .create_prompt("Check Email", "Check inbox", vec![], None)
            .await
            .unwrap();

        let result = svc.get_prompt_by_handle("CHECK EMAIL").await.unwrap();
        assert!(result.is_some());
        if let Some(ManagedPromptRecord::Ready((found_id, _))) = result {
            assert_eq!(found_id, id);
        } else {
            panic!("Expected Ready record for CHECK EMAIL");
        }

        let result = svc.get_prompt_by_handle("Check_Email").await.unwrap();
        assert!(result.is_some());
        if let Some(ManagedPromptRecord::Ready((found_id, _))) = result {
            assert_eq!(found_id, id);
        } else {
            panic!("Expected Ready record for Check_Email");
        }
    }

    #[tokio::test]
    async fn handle_lookup_excludes_repair_handles() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        let id = svc
            .create_prompt("Check Email", "Check inbox", vec![], None)
            .await
            .unwrap();

        let repair_handle = format!("~legacy-{}", id);
        sqlx::query(
            "UPDATE prompts SET identity_state = 'needs_rename', normalized_name = ? WHERE id = ?",
        )
        .bind(&repair_handle)
        .bind(&id)
        .execute(store.pool())
        .await
        .unwrap();

        let result = svc.get_prompt_by_handle(&repair_handle).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn credential_summary_includes_auth_status() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let summary = svc.set_api_key("test", "sk-key").await.unwrap();
        assert_eq!(summary.auth_status, ProviderAuthStatus::ConfiguredApiKey);
    }

    #[tokio::test]
    async fn credential_list_skips_unknown_mode() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        svc.set_api_key("test", "sk-key").await.unwrap();

        sqlx::query(
            "INSERT INTO credentials (provider_slug, credential_mode, encrypted_payload, nonce, encryption_metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("unknown-provider")
        .bind("unknown_mode")
        .bind(vec![0u8])
        .bind(vec![0u8])
        .bind("{}")
        .bind("2024-01-01T00:00:00Z")
        .bind("2024-01-01T00:00:00Z")
        .execute(store.pool())
        .await
        .unwrap();

        let list = svc.list_credentials().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].provider_slug, "test");
    }

    #[tokio::test]
    async fn set_api_key_preserves_created_at() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let summary1 = svc.set_api_key("test", "sk-key1").await.unwrap();
        let summary2 = svc.set_api_key("test", "sk-key2").await.unwrap();

        assert_eq!(summary1.created_at, summary2.created_at);
    }

    #[tokio::test]
    async fn profile_save_validates_control_chars_in_id() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let profile = AgentProfile::with_name("Test Profile");
        let result = svc.save_profile("test\0id", &profile).await;
        assert!(matches!(result, Err(ManagementError::Validation(_))));
    }

    #[tokio::test]
    async fn profile_save_normalizes_name_in_payload() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let profile = AgentProfile::with_name("  Test Profile  ");
        svc.save_profile("test-prof", &profile).await.unwrap();

        let result = svc.get_profile("test-prof").await.unwrap();
        if let Some(ManagedProfileRecord::Ready(entry)) = result {
            assert_eq!(entry.profile.name, "Test Profile");
        } else {
            panic!("Expected Ready record");
        }
    }

    #[tokio::test]
    async fn delete_prompt_integrity_check() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        let prompt_id = svc
            .create_prompt("Test Prompt", "Instructions", vec![], None)
            .await
            .unwrap();

        svc.save_automation_task(&AutomationTaskInput {
            id: "test-task".to_string(),
            display_name: "Test Task".to_string(),
            stored_prompt_id: prompt_id.clone(),
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

        sqlx::query("UPDATE automation_tasks SET schema_version = 99 WHERE id = ?")
            .bind("test-task")
            .execute(store.pool())
            .await
            .unwrap();

        let result = svc.delete_prompt(&prompt_id).await;
        assert!(matches!(
            result,
            Err(ManagementError::IntegrityUnknown { .. })
        ));
    }

    #[tokio::test]
    async fn delete_automation_task_integrity_check() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        let prompt_id = svc
            .create_prompt("Test Prompt", "Instructions", vec![], None)
            .await
            .unwrap();

        svc.save_automation_task(&AutomationTaskInput {
            id: "test-task".to_string(),
            display_name: "Test Task".to_string(),
            stored_prompt_id: prompt_id,
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

        svc.save_scheduled_task(&ScheduledTaskInput {
            id: "test-sched".to_string(),
            automation_task_id: "test-task".to_string(),
            cron_expression: "0 9 * * *".to_string(),
            enabled: true,
        })
        .await
        .unwrap();

        // Corrupt the schedule table so list_scheduled_tasks cannot read it.
        sqlx::query("ALTER TABLE schedule RENAME TO schedule_corrupt")
            .execute(store.pool())
            .await
            .unwrap();

        let result = svc.delete_automation_task("test-task").await;
        assert!(matches!(
            result,
            Err(ManagementError::IntegrityUnknown { .. })
        ));
    }

    #[tokio::test]
    async fn task_impact_includes_prompt_dependency() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let prompt_id = svc
            .create_prompt("Test", "Instructions", vec![], None)
            .await
            .unwrap();

        svc.save_automation_task(&AutomationTaskInput {
            id: "task-1".to_string(),
            display_name: "Task 1".to_string(),
            stored_prompt_id: prompt_id.clone(),
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

        let report = svc.task_impact("task-1").await.unwrap();
        let has_prompt_dep = report.links.iter().any(|link| {
            link.entity
                == DependencyEntity::Prompt {
                    id: prompt_id.clone(),
                }
                && link.direction == DependencyDirection::Depends
        });
        assert!(
            has_prompt_dep,
            "task_impact should include a Depends link to the prompt"
        );
    }

    #[tokio::test]
    async fn prompt_impact_includes_profile_dependency() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let profile = AgentProfile::with_name("Test Profile");
        svc.save_profile("test-prof", &profile).await.unwrap();

        let prompt_id = svc
            .create_prompt(
                "Test",
                "Instructions",
                vec![],
                Some(AgentProfileId::from("test-prof")),
            )
            .await
            .unwrap();

        let report = svc.prompt_impact(&prompt_id).await.unwrap();
        let has_profile_dep = report.links.iter().any(|link| {
            link.entity
                == DependencyEntity::Profile {
                    id: "test-prof".to_string(),
                }
                && link.direction == DependencyDirection::Depends
        });
        assert!(
            has_profile_dep,
            "prompt_impact should include a Depends link to the profile"
        );
    }

    #[tokio::test]
    async fn combined_schedule_deletion_host_failure_returns_error() {
        use crate::scheduled_task::host::{FakeHostScheduler, HostSchedulerError};

        let store = ConfigStore::open_in_memory().await.unwrap();

        let fake = FakeHostScheduler::new();
        *fake.force_error.lock() =
            Some(HostSchedulerError::PlatformUnavailable("test".to_string()));

        let svc = ConfigManagementService::new(store).with_scheduler(
            Arc::new(fake),
            SchedulerInstallContext {
                runner_executable: std::path::PathBuf::from("/usr/local/bin/agent-iron"),
                config_store_path: std::path::PathBuf::from("/tmp/config.db"),
            },
        );

        let prompt_id = svc
            .create_prompt("Test", "Instructions", vec![], None)
            .await
            .unwrap();

        svc.save_automation_task(&AutomationTaskInput {
            id: "test-task".to_string(),
            display_name: "Test Task".to_string(),
            stored_prompt_id: prompt_id,
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

        svc.save_scheduled_task(&ScheduledTaskInput {
            id: "test-sched".to_string(),
            automation_task_id: "test-task".to_string(),
            cron_expression: "0 9 * * *".to_string(),
            enabled: true,
        })
        .await
        .unwrap();

        let result = svc.delete_schedule_combined("test-sched").await;
        assert!(matches!(result, Err(ManagementError::Scheduler(_))));
    }

    #[tokio::test]
    async fn automation_task_list_returns_managed_records() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let prompt_id = svc
            .create_prompt("Test", "Instructions", vec![], None)
            .await
            .unwrap();

        svc.save_automation_task(&AutomationTaskInput {
            id: "task-1".to_string(),
            display_name: "Task 1".to_string(),
            stored_prompt_id: prompt_id,
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

        let list = svc.list_automation_tasks().await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(matches!(list[0], ManagedAutomationTaskRecord::Ready(_)));
    }

    #[tokio::test]
    async fn automation_task_get_returns_managed_record() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let prompt_id = svc
            .create_prompt("Test", "Instructions", vec![], None)
            .await
            .unwrap();

        svc.save_automation_task(&AutomationTaskInput {
            id: "task-1".to_string(),
            display_name: "Task 1".to_string(),
            stored_prompt_id: prompt_id,
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

        let result = svc.get_automation_task("task-1").await.unwrap();
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            ManagedAutomationTaskRecord::Ready(_)
        ));
    }
}
