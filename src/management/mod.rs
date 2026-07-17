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
//!
//! ```no_run
//! use iron_core::ConfigManagementService;
//! use iron_core::config::ConfigStore;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let store = ConfigStore::open_in_memory().await?;
//! let service = ConfigManagementService::new(store);
//! let profiles = service.list_profiles().await?;
//! for record in profiles {
//!     println!("{record:?}");
//! }
//! # Ok(())
//! # }
//! ```

use crate::automation_task::{AutomationTask, AutomationTaskInput};
use crate::config::{ConfigError, ConfigStore};
use crate::profile::{
    AgentProfile, AgentProfileId, AgentProfileProvider, SkillFilter, PROFILE_SCHEMA_VERSION,
};
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
    /// A durable configuration operation failed without a more specific mapping.
    #[error("Storage error: {0}")]
    Storage(ConfigError),

    /// Caller input or a decoded record violated a management invariant.
    #[error("Validation error: {0}")]
    Validation(String),

    /// An operation named a related entity that does not exist or is unusable.
    #[error("Reference error: {0}")]
    Reference(String),

    /// Deletion or identity assignment was blocked by existing referrers.
    #[error("Conflict: {target} is referenced by {referrers:?}")]
    Conflict {
        /// ID or normalized identity that could not be changed.
        target: String,
        /// Stable IDs of records causing the conflict.
        referrers: Vec<String>,
    },

    /// Malformed or unsupported rows prevented a safe dependency decision.
    #[error("Cannot verify referential integrity: {details}")]
    IntegrityUnknown {
        /// Records or queries that made the integrity result indeterminate.
        details: String,
    },

    /// Profile deletion would violate the requested valid-profile floor.
    #[error(
        "Cannot delete profile: {remaining} valid profile(s) would remain, below the requested minimum of {minimum}"
    )]
    MinimumValidProfiles {
        /// Minimum number of valid persisted profiles requested by the caller.
        minimum: usize,
        /// Number of valid profiles that the deletion would leave.
        remaining: usize,
    },

    /// A host-scheduler operation was requested without attaching a scheduler.
    #[error("Scheduler is not attached")]
    SchedulerUnavailable,

    /// The attached host scheduler rejected an operation.
    #[error("Scheduler error: {0}")]
    Scheduler(String),

    /// Durable state changed successfully but a secondary synchronization step failed.
    #[error(
        "Partial operation for '{target}': durable_succeeded={durable_succeeded}, error={error}"
    )]
    Partial {
        /// Stable ID whose operation completed only partially.
        target: String,
        /// Whether the durable configuration mutation committed.
        durable_succeeded: bool,
        /// Failure from registry synchronization or another secondary step.
        error: String,
    },
}

impl From<ConfigError> for ManagementError {
    fn from(e: ConfigError) -> Self {
        match e {
            ConfigError::Validation(msg) => ManagementError::Validation(msg),
            ConfigError::UnknownStoredPrompt(id) => {
                ManagementError::Reference(format!("Stored prompt '{}' does not exist", id))
            }
            ConfigError::UnknownAutomationTask(id) => {
                ManagementError::Reference(format!("Automation task '{}' does not exist", id))
            }
            ConfigError::TaskNameConflict {
                normalized_name,
                existing_id,
            } => ManagementError::Conflict {
                target: normalized_name,
                referrers: vec![existing_id],
            },
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
            ConfigError::MinimumValidProfiles { minimum, remaining } => {
                ManagementError::MinimumValidProfiles { minimum, remaining }
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
    /// The record's schema version is not understood by this binary.
    UnsupportedSchemaVersion,
    /// Serialized data could not be decoded or violated structural invariants.
    InvalidPayload,
    /// A record named by an earlier listing disappeared before it was read.
    MissingRecord,
    /// A referenced profile is absent or unusable.
    UnavailableProfile,
    /// A referenced skill is unavailable to the selected profile.
    UnavailableSkill,
    /// A migrated prompt is quarantined until its identity is renamed.
    NeedsRename,
    /// A legacy approval value is not valid for user-facing profiles.
    ReadOnlyRejected,
    /// A persisted prompt identity-state discriminator is not recognized.
    UnknownIdentityState,
}

/// A diagnostic for a single managed record.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordDiagnostic {
    /// Stable category suitable for programmatic handling.
    pub category: DiagnosticCategory,
    /// Record-specific explanation suitable for display or logs.
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
        /// Stable ID of the degraded record.
        id: String,
        /// Partially usable decoded value, when safe decoding was possible.
        decoded: Option<T>,
        /// Issues preventing the record from being considered ready.
        diagnostics: Vec<RecordDiagnostic>,
    },
}

// ============================================================================
// Dependency impact types
// ============================================================================

/// A typed entity in the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DependencyEntity {
    /// Provider credential identified by provider slug.
    ProviderCredential {
        /// Provider slug used for credential resolution.
        slug: String,
    },
    /// Agent profile identified by stable record ID.
    Profile {
        /// Stable profile ID.
        id: String,
    },
    /// Stored prompt identified by immutable record ID.
    Prompt {
        /// Immutable prompt ID.
        id: String,
    },
    /// Automation task identified by stable record ID.
    AutomationTask {
        /// Stable automation-task ID.
        id: String,
    },
    /// Desired scheduled task identified by stable record ID.
    ScheduledTask {
        /// Stable schedule ID.
        id: String,
    },
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
    /// The target or entity names the other without an intermediate record.
    Direct,
    /// The relationship passes through one or more intermediate records.
    Transitive,
}

/// A single link in the dependency graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DependencyLink {
    /// Entity reached from the report target.
    pub entity: DependencyEntity,
    /// Whether the reached entity is required by or depends on the target.
    pub direction: DependencyDirection,
    /// Whether the relationship is immediate or crosses intermediate records.
    pub proximity: DependencyProximity,
    /// Ordered path from the target to this entity.
    pub path: Vec<DependencyEntity>,
}

/// Result of querying the dependency impact of a target entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DependencyImpactReport {
    /// Entity from which dependency traversal began.
    pub target: DependencyEntity,
    /// Discovered dependencies and dependents, each with an explicit path.
    pub links: Vec<DependencyLink>,
    /// Warnings about malformed or unsupported records that may have caused
    /// the report to omit dependency paths.
    pub diagnostics: Vec<String>,
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
    /// Provider that owns the persisted credential.
    pub provider_slug: String,
    /// Recognized persisted mode, or [`CredentialMode::Unsupported`].
    pub credential_mode: CredentialMode,
    /// Persisted-state auth status derived from the credential mode.
    pub auth_status: ProviderAuthStatus,
    /// Time at which a credential was first stored for this provider.
    pub created_at: DateTime<Utc>,
    /// Time at which the credential was last replaced or refreshed.
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Schedule deletion outcome
// ============================================================================

/// Result of a combined schedule deletion (host + desired state).
///
/// When host removal succeeds but deleting the desired ConfigStore row fails,
/// `drift` contains the post-failure status of the schedule so callers can
/// see the remaining desired-state drift and any host/diagnostic state.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleDeletionOutcome {
    /// Stable ID of the schedule targeted for deletion.
    pub schedule_id: String,
    /// Whether removal from the host scheduler succeeded.
    pub host_removed: bool,
    /// Whether deletion of the durable desired-state row succeeded.
    pub desired_deleted: bool,
    /// Post-failure schedule status when host and desired state diverged.
    pub drift: Option<crate::scheduled_task::ScheduleStatus>,
}

/// Fallible registry abstraction for profile synchronization.
///
/// Implementations are typically an `Arc<RwLock<HashMap<AgentProfileId,
/// AgentProfile>>>` shared with a runtime. The trait is object-safe so the
/// management service can hold either implementation without generics.
pub trait ProfileRegistry: Send + Sync {
    /// Insert or replace a profile after its durable write commits.
    ///
    /// Returns a synchronization diagnostic if the registry rejects the value.
    fn insert(&self, id: AgentProfileId, profile: AgentProfile) -> Result<(), String>;
    /// Remove a profile after its durable deletion commits.
    ///
    /// Returns a synchronization diagnostic if the registry cannot remove it.
    fn remove(&self, id: &AgentProfileId) -> Result<(), String>;
    /// Find another profile that owns `name` after normalization.
    ///
    /// `excluding_id` is ignored so an update does not conflict with itself.
    fn find_by_normalized_name(
        &self,
        name: &str,
        excluding_id: &AgentProfileId,
    ) -> Option<AgentProfileId>;
}

impl ProfileRegistry for RwLock<HashMap<AgentProfileId, AgentProfile>> {
    fn insert(&self, id: AgentProfileId, profile: AgentProfile) -> Result<(), String> {
        let mut map = self.write();
        map.insert(id, profile);
        Ok(())
    }

    fn remove(&self, id: &AgentProfileId) -> Result<(), String> {
        let mut map = self.write();
        map.remove(id);
        Ok(())
    }

    fn find_by_normalized_name(
        &self,
        name: &str,
        excluding_id: &AgentProfileId,
    ) -> Option<AgentProfileId> {
        let map = self.read();
        for (id, profile) in map.iter() {
            if id == excluding_id {
                continue;
            }
            if let Some(normalized) = crate::profile::normalize_profile_name(&profile.name) {
                if normalized == name {
                    return Some(id.clone());
                }
            }
        }
        None
    }
}

/// Fallible registry abstraction for stored-prompt synchronization.
pub trait PromptRegistry: Send + Sync {
    /// Validate and insert or replace a prompt after durable persistence.
    fn register(&self, id: String, prompt: StoredPrompt) -> Result<(), String>;
    /// Remove a prompt after durable deletion.
    fn unregister(&self, id: &str) -> Result<(), String>;
}

impl PromptRegistry for RwLock<StoredPromptRegistry> {
    fn register(&self, id: String, prompt: StoredPrompt) -> Result<(), String> {
        let mut reg = self.write();
        reg.register(id, prompt)
    }

    fn unregister(&self, id: &str) -> Result<(), String> {
        let mut reg = self.write();
        reg.unregister(id);
        Ok(())
    }
}

// ============================================================================
// Config Management Service
// ============================================================================

/// Application-facing typed configuration management service.
///
/// Constructed from a [`ConfigStore`] with optional attached registries and
/// scheduler dependencies. Profile, prompt, automation-task, and credential
/// management remain available when host scheduling is not attached.
///
/// Methods map durable failures into [`ManagementError::Storage`] or a more
/// specific validation, reference, conflict, or integrity variant. Registry
/// synchronization occurs after durable commits, so a synchronization failure
/// is reported as [`ManagementError::Partial`] rather than rolled back.
pub struct ConfigManagementService {
    store: ConfigStore,
    profile_registry: Option<Arc<dyn ProfileRegistry>>,
    prompt_registry: Option<Arc<dyn PromptRegistry>>,
    host_scheduler: Option<Arc<dyn HostScheduler>>,
    install_context: Option<SchedulerInstallContext>,
    /// Optional snapshot of available skill names for best-effort validation.
    /// When absent, skill-availability checks are skipped.
    skill_inventory: Option<HashSet<String>>,
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
            skill_inventory: None,
        }
    }

    /// Attach a profile registry for post-persistence synchronization.
    pub fn with_profile_registry(
        mut self,
        registry: Arc<RwLock<HashMap<AgentProfileId, AgentProfile>>>,
    ) -> Self {
        self.profile_registry = Some(registry as Arc<dyn ProfileRegistry>);
        self
    }

    /// Attach a prompt registry for post-persistence synchronization.
    pub fn with_prompt_registry(mut self, registry: Arc<RwLock<StoredPromptRegistry>>) -> Self {
        self.prompt_registry = Some(registry as Arc<dyn PromptRegistry>);
        self
    }

    #[cfg(test)]
    /// Attach a type-erased profile registry for management-service tests.
    pub(crate) fn with_profile_registry_for_test(
        mut self,
        registry: Arc<dyn ProfileRegistry>,
    ) -> Self {
        self.profile_registry = Some(registry);
        self
    }

    #[cfg(test)]
    /// Attach a type-erased prompt registry for management-service tests.
    pub(crate) fn with_prompt_registry_for_test(
        mut self,
        registry: Arc<dyn PromptRegistry>,
    ) -> Self {
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

    /// Attach a snapshot of available skill names for best-effort validation.
    ///
    /// When attached, prompt writes reject skills unavailable to the selected
    /// profile, and reads report diagnostics for persisted skills no longer in
    /// the inventory. When absent, skill-availability checks are skipped.
    pub fn with_skill_inventory(mut self, skills: HashSet<String>) -> Self {
        self.skill_inventory = Some(skills);
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
        durable.name = normalized_name.clone();

        // Validate managed provider context.
        if let AgentProfileProvider::Managed {
            provider_slug,
            model,
        } = &durable.provider
        {
            if provider_slug.as_str().trim().is_empty() {
                return Err(ManagementError::Validation(
                    "Managed profile provider_slug must not be empty".to_string(),
                ));
            }
            if model.trim().is_empty() {
                return Err(ManagementError::Validation(
                    "Managed profile model must not be empty".to_string(),
                ));
            }
        }

        // Check attached registry for a same-named profile with a different ID
        // before durable persistence, so the registry's normal collision rules
        // are not bypassed.
        if let Some(registry) = &self.profile_registry {
            let self_id = AgentProfileId::from(trimmed_id);
            if let Some(existing_id) = registry.find_by_normalized_name(&normalized_name, &self_id)
            {
                return Err(ManagementError::Conflict {
                    target: trimmed_id.to_string(),
                    referrers: vec![existing_id.as_str().to_string()],
                });
            }
        }

        let payload = serde_json::to_value(&durable)
            .map_err(|e| ManagementError::Storage(ConfigError::Serialization(e.to_string())))?;

        let input = ProfileInput {
            id: trimmed_id.to_string(),
            schema_version: PROFILE_SCHEMA_VERSION,
            payload,
        };

        // Transactional name-uniqueness enforcement.
        self.store
            .set_profile_checked(&input, &normalized_name)
            .await?;

        // Sync attached registry after durable success. The same normalized
        // name is used for both the durable payload and the registry entry.
        if let Some(registry) = &self.profile_registry {
            registry
                .insert(AgentProfileId::from(trimmed_id), durable)
                .map_err(|e| ManagementError::Partial {
                    target: trimmed_id.to_string(),
                    durable_succeeded: true,
                    error: e,
                })?;
        }

        Ok(())
    }

    /// Get a single profile by stable ID.
    pub async fn get_profile(
        &self,
        id: &str,
    ) -> Result<Option<ManagedProfileRecord>, ManagementError> {
        let record = match self.store.get_profile(id).await {
            Ok(None) => return Ok(None),
            Ok(Some(r)) => r,
            Err(ConfigError::Deserialization(msg)) => {
                return Ok(Some(ManagedProfileRecord::NeedsAttention {
                    id: id.to_string(),
                    decoded: None,
                    diagnostics: vec![RecordDiagnostic {
                        category: DiagnosticCategory::InvalidPayload,
                        message: msg,
                    }],
                }));
            }
            Err(e) => return Err(e.into()),
        };
        match crate::profile::classify_profile_record(
            &record.id,
            record.schema_version,
            &record.payload,
        ) {
            Ok(profile) => Ok(Some(ManagedProfileRecord::Ready(ManagedProfileEntry {
                id: AgentProfileId::from(record.id),
                profile,
                created_at: record.created_at,
                updated_at: record.updated_at,
            }))),
            Err(error) => Ok(Some(ManagedProfileRecord::NeedsAttention {
                id: record.id,
                decoded: None,
                diagnostics: Self::profile_record_diagnostics(error),
            })),
        }
    }

    /// List all profiles with per-record diagnostics.
    pub async fn list_profiles(&self) -> Result<Vec<ManagedProfileRecord>, ManagementError> {
        let ids = self.store.list_profile_ids().await?;
        let mut results = Vec::new();
        for id in ids {
            let record = match self.store.get_profile(&id).await {
                Ok(None) => continue,
                Ok(Some(r)) => r,
                Err(ConfigError::Deserialization(msg)) => {
                    results.push(ManagedProfileRecord::NeedsAttention {
                        id,
                        decoded: None,
                        diagnostics: vec![RecordDiagnostic {
                            category: DiagnosticCategory::InvalidPayload,
                            message: msg,
                        }],
                    });
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            match crate::profile::classify_profile_record(
                &record.id,
                record.schema_version,
                &record.payload,
            ) {
                Ok(profile) => {
                    results.push(ManagedProfileRecord::Ready(ManagedProfileEntry {
                        id: AgentProfileId::from(record.id),
                        profile,
                        created_at: record.created_at,
                        updated_at: record.updated_at,
                    }));
                }
                Err(error) => {
                    results.push(ManagedProfileRecord::NeedsAttention {
                        id: record.id,
                        decoded: None,
                        diagnostics: Self::profile_record_diagnostics(error),
                    });
                }
            }
        }
        Ok(results)
    }

    /// Delete a profile, blocking if stored prompts reference it.
    ///
    /// Returns `ManagementError::Conflict` if prompts reference this profile.
    /// Returns `ManagementError::IntegrityUnknown` if malformed records
    /// prevent safe reference checking. All checks are transactional in the
    /// store layer. Uses the unrestricted deletion policy, which permits zero
    /// valid persisted profiles to remain.
    pub async fn delete_profile(&self, id: &str) -> Result<(), ManagementError> {
        self.delete_profile_with_policy(id, crate::profile::ProfileDeletePolicy::AllowZero)
            .await
    }

    /// Delete a profile under a caller-selected minimum-valid-profile policy.
    ///
    /// Preserves protected-default validation, prompt-reference and integrity
    /// checks, and durable-first registry synchronization. Returns
    /// [`ManagementError::MinimumValidProfiles`] when the computed remaining
    /// valid persisted profile count is below the requested minimum. Prompt-
    /// reference and integrity failures retain precedence over the minimum
    /// check. A registry synchronization failure after durable commit retains
    /// the existing [`ManagementError::Partial`] behavior.
    pub async fn delete_profile_with_policy(
        &self,
        id: &str,
        policy: crate::profile::ProfileDeletePolicy,
    ) -> Result<(), ManagementError> {
        let trimmed_id = id.trim();
        if trimmed_id.eq_ignore_ascii_case("default") {
            return Err(ManagementError::Validation(
                "The default profile cannot be deleted".to_string(),
            ));
        }

        self.store
            .delete_profile_with_policy(trimmed_id, policy)
            .await?;

        // Sync attached registry only after the durable transaction commits.
        if let Some(registry) = &self.profile_registry {
            registry
                .remove(&AgentProfileId::from(trimmed_id))
                .map_err(|e| ManagementError::Partial {
                    target: trimmed_id.to_string(),
                    durable_succeeded: true,
                    error: e,
                })?;
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
    ) -> Result<(String, StoredPrompt), ManagementError> {
        let id = format!("prompt-{}", Uuid::new_v4());
        let normalized = normalize_prompt_name(display_name);
        if normalized.is_empty() {
            return Err(ManagementError::Validation(
                "Display name must produce a non-empty normalized handle".to_string(),
            ));
        }
        if crate::stored_prompt::is_reserved_handle(&normalized) {
            return Err(ManagementError::Validation(
                "Display name uses reserved 'legacy-' handle prefix".to_string(),
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

        self.store.insert_typed_prompt(&id, &prompt).await?;
        if let Some(registry) = &self.prompt_registry {
            registry.register(id.clone(), prompt.clone()).map_err(|e| {
                ManagementError::Partial {
                    target: id.clone(),
                    durable_succeeded: true,
                    error: e,
                }
            })?;
        }
        Ok((id, prompt))
    }

    /// Save (replace) a stored prompt by stable ID.
    ///
    /// The prompt must already exist; use [`create_prompt`](Self::create_prompt)
    /// to generate a new prompt with a core-generated ID. The normalized handle
    /// is derived from the display name inside core so callers do not need to
    /// reproduce identity logic.
    pub async fn save_prompt(
        &self,
        id: &str,
        prompt: &StoredPrompt,
    ) -> Result<(), ManagementError> {
        let existing = self.store.get_prompt(id).await?.ok_or_else(|| {
            ManagementError::Reference(format!(
                "Prompt '{}' not found; use create_prompt to generate a new prompt",
                id
            ))
        })?;
        if !matches!(
            existing.schema_version,
            crate::stored_prompt::LEGACY_STORED_PROMPT_SCHEMA_VERSION
                | crate::stored_prompt::STORED_PROMPT_SCHEMA_VERSION
        ) {
            return Err(ManagementError::Validation(format!(
                "Prompt '{}' uses unsupported schema version {} and cannot be overwritten",
                id, existing.schema_version
            )));
        }
        let mut prompt = prompt.clone();
        prompt.normalized_name = normalize_prompt_name(&prompt.display_name);
        if prompt.normalized_name.is_empty() {
            return Err(ManagementError::Validation(
                "Display name must produce a non-empty normalized handle".to_string(),
            ));
        }
        if crate::stored_prompt::is_reserved_handle(&prompt.normalized_name) {
            return Err(ManagementError::Validation(
                "Display name uses reserved 'legacy-' handle prefix".to_string(),
            ));
        }
        prompt.validate().map_err(ManagementError::Validation)?;
        self.validate_prompt_references(&prompt).await?;
        self.store.set_typed_prompt(id, &prompt).await?;
        if let Some(registry) = &self.prompt_registry {
            registry
                .register(id.to_string(), prompt.clone())
                .map_err(|e| ManagementError::Partial {
                    target: id.to_string(),
                    durable_succeeded: true,
                    error: e,
                })?;
        }
        Ok(())
    }

    /// Get a single stored prompt by immutable ID.
    pub async fn get_prompt(
        &self,
        id: &str,
    ) -> Result<Option<ManagedPromptRecord>, ManagementError> {
        let record = match self.store.get_prompt(id).await {
            Ok(None) => return Ok(None),
            Ok(Some(r)) => r,
            Err(ConfigError::Deserialization(msg)) => {
                return Ok(Some(ManagedPromptRecord::NeedsAttention {
                    id: id.to_string(),
                    decoded: None,
                    diagnostics: vec![RecordDiagnostic {
                        category: DiagnosticCategory::InvalidPayload,
                        message: msg,
                    }],
                }));
            }
            Err(e) => return Err(e.into()),
        };
        Ok(Some(
            self.with_reference_diagnostics(self.decode_prompt_record(record))
                .await?,
        ))
    }

    /// Get a single stored prompt by normalized handle.
    pub async fn get_prompt_by_handle(
        &self,
        handle: &str,
    ) -> Result<Option<ManagedPromptRecord>, ManagementError> {
        let record = match self.store.get_prompt_by_normalized_name(handle).await {
            Ok(None) => return Ok(None),
            Ok(Some(r)) => r,
            Err(ConfigError::Deserialization(msg)) => {
                let id = self
                    .store
                    .get_prompt_id_by_normalized_name(handle)
                    .await?
                    .unwrap_or_else(|| handle.to_string());
                return Ok(Some(ManagedPromptRecord::NeedsAttention {
                    id,
                    decoded: None,
                    diagnostics: vec![RecordDiagnostic {
                        category: DiagnosticCategory::InvalidPayload,
                        message: msg,
                    }],
                }));
            }
            Err(e) => return Err(e.into()),
        };
        Ok(Some(
            self.with_reference_diagnostics(self.decode_prompt_record(record))
                .await?,
        ))
    }

    /// List all stored prompts with per-record diagnostics.
    pub async fn list_prompts(&self) -> Result<Vec<ManagedPromptRecord>, ManagementError> {
        let ids = self.store.list_prompt_ids_sorted().await?;
        let mut results = Vec::new();
        for id in ids {
            let record = match self.store.get_prompt(&id).await {
                Ok(None) => continue,
                Ok(Some(r)) => r,
                Err(ConfigError::Deserialization(msg)) => {
                    results.push(ManagedPromptRecord::NeedsAttention {
                        id,
                        decoded: None,
                        diagnostics: vec![RecordDiagnostic {
                            category: DiagnosticCategory::InvalidPayload,
                            message: msg,
                        }],
                    });
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            results.push(
                self.with_reference_diagnostics(self.decode_prompt_record(record))
                    .await?,
            );
        }
        Ok(results)
    }

    /// Delete a stored prompt, blocking if automation tasks reference it.
    ///
    /// The store-level reference check uses a direct column comparison
    /// (`stored_prompt_id = ?`) within a transaction, so it is reliable
    /// regardless of schema version or payload validity. Store failures are
    /// returned as typed storage errors.
    pub async fn delete_prompt(&self, id: &str) -> Result<(), ManagementError> {
        match self.store.delete_prompt(id).await {
            Ok(()) => {}
            Err(e) => {
                if matches!(
                    e,
                    ConfigError::PromptReferencedByTasks { .. }
                        | ConfigError::IntegrityUnknown { .. }
                ) {
                    return Err(e.into());
                }
                return Err(ManagementError::Storage(e));
            }
        }

        if let Some(registry) = &self.prompt_registry {
            registry
                .unregister(id)
                .map_err(|e| ManagementError::Partial {
                    target: id.to_string(),
                    durable_succeeded: true,
                    error: e,
                })?;
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
        if crate::stored_prompt::is_reserved_handle(&normalized) {
            return Err(ManagementError::Validation(
                "Display name uses reserved 'legacy-' handle prefix".to_string(),
            ));
        }

        let existing = self
            .store
            .get_prompt(id)
            .await?
            .ok_or_else(|| ManagementError::Reference(format!("Prompt '{}' not found", id)))?;

        if !matches!(
            existing.schema_version,
            crate::stored_prompt::LEGACY_STORED_PROMPT_SCHEMA_VERSION
                | crate::stored_prompt::STORED_PROMPT_SCHEMA_VERSION
        ) {
            return Err(ManagementError::Validation(format!(
                "Prompt '{}' uses unsupported schema version {} and cannot be renamed",
                id, existing.schema_version
            )));
        }

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
        updated.validate().map_err(ManagementError::Validation)?;
        self.validate_prompt_references(&updated).await?;
        self.store.set_typed_prompt(id, &updated).await?;
        if let Some(registry) = &self.prompt_registry {
            registry
                .register(id.to_string(), updated.clone())
                .map_err(|e| ManagementError::Partial {
                    target: id.to_string(),
                    durable_succeeded: true,
                    error: e,
                })?;
        }
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
            Ok(Some(task)) => {
                let diagnostics = self.validate_automation_task_record(&task).await?;
                if diagnostics.is_empty() {
                    Ok(Some(ManagedAutomationTaskRecord::Ready(task)))
                } else {
                    Ok(Some(ManagedAutomationTaskRecord::NeedsAttention {
                        id: task.id.clone(),
                        decoded: Some(task),
                        diagnostics,
                    }))
                }
            }
            Ok(None) => Ok(None),
            Err(ConfigError::Deserialization(msg)) => {
                let category = if msg.contains("unsupported schema version") {
                    DiagnosticCategory::UnsupportedSchemaVersion
                } else {
                    DiagnosticCategory::InvalidPayload
                };
                Ok(Some(ManagedAutomationTaskRecord::NeedsAttention {
                    id: id.to_string(),
                    decoded: None,
                    diagnostics: vec![RecordDiagnostic {
                        category,
                        message: msg,
                    }],
                }))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Get an automation task by its normalized handle (case-insensitive).
    pub async fn get_automation_task_by_handle(
        &self,
        handle: &str,
    ) -> Result<Option<ManagedAutomationTaskRecord>, ManagementError> {
        match self
            .store
            .get_automation_task_by_normalized_name(handle)
            .await
        {
            Ok(Some(task)) => {
                let diagnostics = self.validate_automation_task_record(&task).await?;
                if diagnostics.is_empty() {
                    Ok(Some(ManagedAutomationTaskRecord::Ready(task)))
                } else {
                    Ok(Some(ManagedAutomationTaskRecord::NeedsAttention {
                        id: task.id.clone(),
                        decoded: Some(task),
                        diagnostics,
                    }))
                }
            }
            Ok(None) => Ok(None),
            Err(ConfigError::Deserialization(msg)) => {
                let id = self
                    .store
                    .get_automation_task_id_by_normalized_name(handle)
                    .await?
                    .unwrap_or_else(|| handle.to_string());
                let category = if msg.contains("unsupported schema version") {
                    DiagnosticCategory::UnsupportedSchemaVersion
                } else {
                    DiagnosticCategory::InvalidPayload
                };
                Ok(Some(ManagedAutomationTaskRecord::NeedsAttention {
                    id,
                    decoded: None,
                    diagnostics: vec![RecordDiagnostic {
                        category,
                        message: msg,
                    }],
                }))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// List all automation tasks in deterministic order.
    ///
    /// Records with an unsupported schema version or malformed payload are
    /// surfaced as `NeedsAttention` rather than failing the whole list.
    pub async fn list_automation_tasks(
        &self,
    ) -> Result<Vec<ManagedAutomationTaskRecord>, ManagementError> {
        let ids = self.store.list_automation_task_ids().await?;
        let mut results = Vec::new();
        for id in &ids {
            match self.store.get_automation_task(id).await {
                Ok(Some(task)) => {
                    let diagnostics = self.validate_automation_task_record(&task).await?;
                    if diagnostics.is_empty() {
                        results.push(ManagedAutomationTaskRecord::Ready(task));
                    } else {
                        results.push(ManagedAutomationTaskRecord::NeedsAttention {
                            id: task.id.clone(),
                            decoded: Some(task),
                            diagnostics,
                        });
                    }
                }
                Ok(None) => {} // raced; skip
                Err(ConfigError::Deserialization(msg)) => {
                    let category = if msg.contains("unsupported schema version") {
                        DiagnosticCategory::UnsupportedSchemaVersion
                    } else {
                        DiagnosticCategory::InvalidPayload
                    };
                    results.push(ManagedAutomationTaskRecord::NeedsAttention {
                        id: id.clone(),
                        decoded: None,
                        diagnostics: vec![RecordDiagnostic {
                            category,
                            message: msg,
                        }],
                    });
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(results)
    }

    /// Delete an automation task, blocking if schedules reference it.
    ///
    /// All integrity and reference checks are transactional in the store layer.
    pub async fn delete_automation_task(&self, id: &str) -> Result<(), ManagementError> {
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
            diagnostics: Vec::new(),
        };

        // Direct dependency: the provider credential this profile selects.
        match self.store.get_profile(profile_id).await {
            Ok(Some(record)) => {
                match Self::decode_profile_for_impact(
                    &record.id,
                    record.schema_version,
                    record.payload,
                ) {
                    Ok(profile) => {
                        if let Some(slug) = provider_slug_of(&profile) {
                            let cred_entity = DependencyEntity::ProviderCredential { slug };
                            report.links.push(DependencyLink {
                                entity: cred_entity.clone(),
                                direction: DependencyDirection::Depends,
                                proximity: DependencyProximity::Direct,
                                path: vec![report.target.clone(), cred_entity],
                            });
                        }
                    }
                    Err(reason) => {
                        report.diagnostics.push(format!(
                            "Profile '{}' is not usable for dependency analysis: {}",
                            profile_id, reason
                        ));
                    }
                }
            }
            Ok(None) => {}
            Err(ConfigError::Deserialization(msg)) => {
                report.diagnostics.push(format!(
                    "Profile '{}' could not be decoded: {}",
                    profile_id, msg
                ));
            }
            Err(e) => return Err(e.into()),
        }

        let (prompt_references, prompt_diagnostics) =
            self.prompt_profile_references_for_impact().await?;
        report.diagnostics.extend(prompt_diagnostics);
        let prompt_ids: Vec<String> = prompt_references
            .into_iter()
            .filter_map(|(prompt_id, referenced_profile)| {
                (referenced_profile.as_str() == profile_id).then_some(prompt_id)
            })
            .collect();
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
                match self.store.unsupported_schedules_referencing_task(tid).await {
                    Ok(unsupported) => {
                        for sid in unsupported {
                            report.diagnostics.push(format!(
                                "Schedule '{}' has unsupported schema and may reference task '{}'",
                                sid, tid
                            ));
                        }
                    }
                    Err(e) => {
                        report.diagnostics.push(format!(
                            "Could not query unsupported schedules referencing task '{}': {}",
                            tid, e
                        ));
                    }
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
            diagnostics: Vec::new(),
        };

        // Scan all profile records directly (including malformed ones) so that
        // no dependent of the queried credential is silently omitted.
        let mut profiles = self.store.list_profile_records_raw().await?;
        profiles.sort_by(|a, b| a.0.cmp(&b.0));
        let (prompt_references, prompt_diagnostics) =
            self.prompt_profile_references_for_impact().await?;
        report.diagnostics.extend(prompt_diagnostics);
        for (pid, schema_version, payload) in &profiles {
            let payload_value = match serde_json::from_str::<serde_json::Value>(payload) {
                Ok(v) => v,
                Err(e) => {
                    report.diagnostics.push(format!(
                        "Profile '{}' could not be decoded; dependency path may be incomplete: {}",
                        pid, e
                    ));
                    continue;
                }
            };
            let profile = match Self::decode_profile_for_impact(pid, *schema_version, payload_value)
            {
                Ok(p) => p,
                Err(reason) => {
                    report.diagnostics.push(format!(
                        "Profile '{}' is not usable for dependency analysis; dependency paths may be incomplete: {}",
                        pid, reason
                    ));
                    continue;
                }
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

                    let prompts: Vec<String> = prompt_references
                        .iter()
                        .filter_map(|(prompt_id, profile_id)| {
                            (profile_id.as_str() == pid).then_some(prompt_id.clone())
                        })
                        .collect();
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
                            match self.store.unsupported_schedules_referencing_task(tid).await {
                                Ok(unsupported) => {
                                    for sid in unsupported {
                                        report.diagnostics.push(format!(
                                            "Schedule '{}' has unsupported schema and may reference task '{}'",
                                            sid, tid
                                        ));
                                    }
                                }
                                Err(e) => {
                                    report.diagnostics.push(format!(
                                        "Could not query unsupported schedules referencing task '{}': {}",
                                        tid, e
                                    ));
                                }
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
            diagnostics: Vec::new(),
        };

        // Direct dependency: the profile this prompt references (if any).
        match self.store.get_prompt(prompt_id).await {
            Ok(Some(prompt_record)) => {
                match Self::decode_prompt_for_impact(
                    &prompt_record.id,
                    prompt_record.schema_version,
                    prompt_record.payload,
                ) {
                    Ok(prompt) => {
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

                            // Transitive dependency: the profile's provider credential.
                            match self.store.get_profile(profile_id.as_str()).await {
                                Ok(Some(profile_record)) => {
                                    match Self::decode_profile_for_impact(
                                        &profile_record.id,
                                        profile_record.schema_version,
                                        profile_record.payload,
                                    ) {
                                        Ok(profile) => {
                                            if let Some(slug) = provider_slug_of(&profile) {
                                                let cred_entity =
                                                    DependencyEntity::ProviderCredential { slug };
                                                report.links.push(DependencyLink {
                                                    entity: cred_entity.clone(),
                                                    direction: DependencyDirection::Depends,
                                                    proximity: DependencyProximity::Transitive,
                                                    path: vec![
                                                        report.target.clone(),
                                                        profile_entity.clone(),
                                                        cred_entity,
                                                    ],
                                                });
                                            }
                                        }
                                        Err(reason) => {
                                            report.diagnostics.push(format!(
                                                "Profile '{}' is not usable for dependency analysis; dependency path may be incomplete: {}",
                                                profile_id.as_str(), reason
                                            ));
                                        }
                                    }
                                }
                                Ok(None) => {}
                                Err(ConfigError::Deserialization(msg)) => {
                                    report.diagnostics.push(format!(
                                        "Profile '{}' could not be decoded; dependency path may be incomplete: {}",
                                        profile_id.as_str(), msg
                                    ));
                                }
                                Err(e) => return Err(e.into()),
                            }
                        }
                    }
                    Err(reason) => {
                        report.diagnostics.push(format!(
                            "Prompt '{}' is not usable for dependency analysis; dependency path may be incomplete: {}",
                            prompt_id, reason
                        ));
                    }
                }
            }
            Ok(None) => {}
            Err(ConfigError::Deserialization(msg)) => {
                report.diagnostics.push(format!(
                    "Prompt '{}' could not be decoded; dependency path may be incomplete: {}",
                    prompt_id, msg
                ));
            }
            Err(e) => return Err(e.into()),
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
            match self.store.unsupported_schedules_referencing_task(tid).await {
                Ok(unsupported) => {
                    for sid in unsupported {
                        report.diagnostics.push(format!(
                            "Schedule '{}' has unsupported schema and may reference task '{}'",
                            sid, tid
                        ));
                    }
                }
                Err(e) => {
                    report.diagnostics.push(format!(
                        "Could not query unsupported schedules referencing task '{}': {}",
                        tid, e
                    ));
                }
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
            diagnostics: Vec::new(),
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
        match self
            .store
            .unsupported_schedules_referencing_task(task_id)
            .await
        {
            Ok(unsupported) => {
                for sid in unsupported {
                    report.diagnostics.push(format!(
                        "Schedule '{}' has unsupported schema and may reference task '{}'",
                        sid, task_id
                    ));
                }
            }
            Err(e) => {
                report.diagnostics.push(format!(
                    "Could not query unsupported schedules referencing task '{}': {}",
                    task_id, e
                ));
            }
        }

        // Direct dependency: the stored prompt this task references.
        match self.store.get_automation_task(task_id).await {
            Ok(Some(task)) => {
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
                match self.store.get_prompt(&task.stored_prompt_id).await {
                    Ok(Some(prompt_record)) => {
                        match Self::decode_prompt_for_impact(
                            &prompt_record.id,
                            prompt_record.schema_version,
                            prompt_record.payload,
                        ) {
                            Ok(prompt) => {
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

                                    // Transitive dependency: the profile's credential.
                                    match self.store.get_profile(profile_id.as_str()).await {
                                        Ok(Some(profile_record)) => {
                                            match Self::decode_profile_for_impact(
                                                &profile_record.id,
                                                profile_record.schema_version,
                                                profile_record.payload,
                                            ) {
                                                Ok(profile) => {
                                                    if let Some(slug) = provider_slug_of(&profile) {
                                                        let cred_entity =
                                                            DependencyEntity::ProviderCredential {
                                                                slug,
                                                            };
                                                        report.links.push(DependencyLink {
                                                            entity: cred_entity.clone(),
                                                            direction: DependencyDirection::Depends,
                                                            proximity:
                                                                DependencyProximity::Transitive,
                                                            path: vec![
                                                                report.target.clone(),
                                                                prompt_entity.clone(),
                                                                profile_entity.clone(),
                                                                cred_entity,
                                                            ],
                                                        });
                                                    }
                                                }
                                                Err(reason) => {
                                                    report.diagnostics.push(format!(
                                                        "Profile '{}' is not usable for dependency analysis; dependency path may be incomplete: {}",
                                                        profile_id.as_str(), reason
                                                    ));
                                                }
                                            }
                                        }
                                        Ok(None) => {}
                                        Err(ConfigError::Deserialization(msg)) => {
                                            report.diagnostics.push(format!(
                                                "Profile '{}' could not be decoded; dependency path may be incomplete: {}",
                                                profile_id.as_str(), msg
                                            ));
                                        }
                                        Err(e) => return Err(e.into()),
                                    }
                                }
                            }
                            Err(reason) => {
                                report.diagnostics.push(format!(
                                    "Prompt '{}' is not usable for dependency analysis; dependency path may be incomplete: {}",
                                    task.stored_prompt_id, reason
                                ));
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(ConfigError::Deserialization(msg)) => {
                        report.diagnostics.push(format!(
                            "Prompt '{}' could not be decoded; dependency path may be incomplete: {}",
                            task.stored_prompt_id, msg
                        ));
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Ok(None) => {}
            Err(ConfigError::Deserialization(msg)) => {
                report.diagnostics.push(format!(
                    "Automation task '{}' could not be decoded; dependency path may be incomplete: {}",
                    task_id, msg
                ));
            }
            Err(e) => return Err(e.into()),
        }

        Ok(report)
    }

    // ========================================================================
    // Credential management (Tasks 4.1-4.4)
    // ========================================================================

    /// List configured credential summaries without secret material.
    ///
    /// For OAuth credentials, the auth status (including `expires_at`) is
    /// derived from persisted token material when the encryption key is
    /// available. If the key is unavailable, OAuth rows fall back to
    /// `ConnectedOAuth { expires_at: None }`.
    pub async fn list_credentials(&self) -> Result<Vec<CredentialSummary>, ManagementError> {
        use crate::provider_credential::domain::StoredCredential;
        use std::time::SystemTime;

        let slugs = self.store.list_credential_slugs().await?;
        let mut results = Vec::new();
        for slug in &slugs {
            if let Some(record) = self.store.get_credential_metadata(slug).await? {
                let (mode, auth_status) = match record.credential_mode.as_str() {
                    "api_key" => (CredentialMode::ApiKey, ProviderAuthStatus::ConfiguredApiKey),
                    "oauth_bearer" => {
                        let status = match self.store.get_credential(slug).await {
                            Ok(Some(decrypted)) => {
                                match serde_json::from_slice::<StoredCredential>(&decrypted) {
                                    Ok(StoredCredential::OAuthBearer(tokens)) => {
                                        let now = SystemTime::now();
                                        match tokens.expires_at {
                                            Some(exp) if now >= exp => ProviderAuthStatus::Expired,
                                            _ => ProviderAuthStatus::ConnectedOAuth {
                                                expires_at: tokens.expires_at,
                                            },
                                        }
                                    }
                                    // Credential metadata says oauth_bearer but
                                    // payload is a different type — surface as
                                    // an error rather than hiding it.
                                    Ok(_) => {
                                        return Err(ManagementError::Storage(
                                            ConfigError::Deserialization(format!(
                                                "credential '{}' metadata says oauth_bearer but payload is a different type",
                                                slug
                                            )),
                                        ));
                                    }
                                    Err(e) => {
                                        return Err(ManagementError::Storage(
                                            ConfigError::Deserialization(format!(
                                                "credential '{}' could not be decoded: {}",
                                                slug, e
                                            )),
                                        ));
                                    }
                                }
                            }
                            // No encryption key available — degrade gracefully.
                            Err(ConfigError::KeyUnavailable(_)) => {
                                ProviderAuthStatus::ConnectedOAuth { expires_at: None }
                            }
                            Err(e) => return Err(ManagementError::Storage(e)),
                            Ok(None) => ProviderAuthStatus::NotConfigured,
                        };
                        (CredentialMode::OAuthBearer, status)
                    }
                    // Unknown modes are surfaced with an explicit unsupported
                    // status rather than being silently omitted, so callers can
                    // see that a row is present but not a recognized mode.
                    _ => (
                        CredentialMode::Unsupported,
                        ProviderAuthStatus::UnsupportedCredential,
                    ),
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

    /// Save (create or replace) a scheduled task's desired state.
    ///
    /// This persists only the ConfigStore desired-state definition. It does
    /// not reconcile the host scheduler. Use [`reconcile_schedule`](Self::reconcile_schedule)
    /// separately to install or update the host entry.
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
    ///
    /// Returns a [`ManagedScheduledTaskRecord`] so records with an unsupported
    /// schema version or malformed payload are surfaced as `NeedsAttention`.
    pub async fn get_scheduled_task(
        &self,
        id: &str,
    ) -> Result<Option<ManagedScheduledTaskRecord>, ManagementError> {
        match self.store.get_scheduled_task(id).await {
            Ok(Some(task)) => {
                let diagnostics = self.validate_scheduled_task_record(&task).await?;
                if diagnostics.is_empty() {
                    Ok(Some(ManagedScheduledTaskRecord::Ready(task)))
                } else {
                    Ok(Some(ManagedScheduledTaskRecord::NeedsAttention {
                        id: task.id.clone(),
                        decoded: Some(task),
                        diagnostics,
                    }))
                }
            }
            Ok(None) => {
                // Distinguish genuinely absent from unsupported-schema.
                if self.store.schedule_exists(id).await? {
                    Ok(Some(ManagedScheduledTaskRecord::NeedsAttention {
                        id: id.to_string(),
                        decoded: None,
                        diagnostics: vec![RecordDiagnostic {
                            category: DiagnosticCategory::UnsupportedSchemaVersion,
                            message: "Schedule record has an unsupported schema version"
                                .to_string(),
                        }],
                    }))
                } else {
                    Ok(None)
                }
            }
            Err(ConfigError::Deserialization(msg)) => {
                Ok(Some(ManagedScheduledTaskRecord::NeedsAttention {
                    id: id.to_string(),
                    decoded: None,
                    diagnostics: vec![RecordDiagnostic {
                        category: DiagnosticCategory::InvalidPayload,
                        message: msg,
                    }],
                }))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// List all scheduled tasks in deterministic order with per-record diagnostics.
    pub async fn list_scheduled_tasks(
        &self,
    ) -> Result<Vec<ManagedScheduledTaskRecord>, ManagementError> {
        let ids = self.store.list_schedule_ids().await?;
        let mut results = Vec::new();
        for id in &ids {
            match self.store.get_scheduled_task(id).await {
                Ok(Some(task)) => {
                    let diagnostics = self.validate_scheduled_task_record(&task).await?;
                    if diagnostics.is_empty() {
                        results.push(ManagedScheduledTaskRecord::Ready(task));
                    } else {
                        results.push(ManagedScheduledTaskRecord::NeedsAttention {
                            id: task.id.clone(),
                            decoded: Some(task),
                            diagnostics,
                        });
                    }
                }
                Ok(None) => {
                    results.push(ManagedScheduledTaskRecord::NeedsAttention {
                        id: id.clone(),
                        decoded: None,
                        diagnostics: vec![RecordDiagnostic {
                            category: DiagnosticCategory::UnsupportedSchemaVersion,
                            message: "Schedule record has an unsupported schema version"
                                .to_string(),
                        }],
                    });
                }
                Err(ConfigError::Deserialization(msg)) => {
                    results.push(ManagedScheduledTaskRecord::NeedsAttention {
                        id: id.clone(),
                        decoded: None,
                        diagnostics: vec![RecordDiagnostic {
                            category: DiagnosticCategory::InvalidPayload,
                            message: msg,
                        }],
                    });
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(results)
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
    /// executing. On success both `host_removed` and `desired_deleted` are
    /// true. When host removal succeeds but desired-state deletion fails,
    /// the outcome is returned with `host_removed = true`,
    /// `desired_deleted = false`, and `drift` containing the post-failure
    /// schedule status plus a diagnostic describing the deletion error.
    /// Host removal failure yields [`ManagementError::Scheduler`].
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
            drift: None,
        };

        if let Err(e) = host.remove(schedule_id).await {
            return Err(ManagementError::Scheduler(format!(
                "Host removal error: {}",
                e
            )));
        }

        // Unconditionally delete desired state. This removes the row regardless
        // of schema version, avoiding the case where an unsupported-schema row
        // is reported as already-deleted but left in the database.
        outcome.host_removed = true;
        match store.delete_scheduled_task(schedule_id).await {
            Ok(_) => {
                outcome.desired_deleted = true;
                Ok(outcome)
            }
            Err(e) => {
                outcome.desired_deleted = false;
                let context = self
                    .install_context
                    .as_ref()
                    .ok_or(ManagementError::SchedulerUnavailable)?;
                let manager = ScheduleManager::new(store, host.as_ref(), context.clone());
                let mut status = manager.inspect(schedule_id).await;
                status
                    .diagnostics
                    .push(crate::scheduled_task::ScheduleDiagnostic {
                        kind: crate::scheduled_task::ScheduleDiagnosticKind::DesiredDeletionFailed,
                        message: format!(
                            "Failed to delete desired state after host removal: {}",
                            e
                        ),
                    });
                outcome.drift = Some(status);
                Ok(outcome)
            }
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

    /// Explicitly reconcile all schedules and optionally remove orphaned host
    /// entries.
    pub async fn reconcile_all_schedules(
        &self,
        orphan_policy: crate::scheduled_task::manager::OrphanPolicy,
    ) -> Result<Vec<ScheduleStatus>, ManagementError> {
        let host = self
            .host_scheduler
            .as_ref()
            .ok_or(ManagementError::SchedulerUnavailable)?;
        let context = self
            .install_context
            .as_ref()
            .ok_or(ManagementError::SchedulerUnavailable)?;
        let manager = ScheduleManager::new(&self.store, host.as_ref(), context.clone());
        Ok(manager.reconcile_all(orphan_policy).await)
    }

    // ========================================================================
    // Internal helpers
    // ========================================================================

    fn decode_profile_for_impact(
        id: &str,
        schema_version: i64,
        payload: serde_json::Value,
    ) -> Result<AgentProfile, String> {
        crate::profile::classify_profile_record(id, schema_version, &payload)
            .map_err(profile_record_error_to_impact_reason)
    }

    fn decode_prompt_for_impact(
        id: &str,
        schema_version: i64,
        payload: serde_json::Value,
    ) -> Result<StoredPrompt, String> {
        let prompt = match schema_version {
            crate::stored_prompt::LEGACY_STORED_PROMPT_SCHEMA_VERSION => {
                let legacy = serde_json::from_value::<StoredPrompt>(payload)
                    .map_err(|e| format!("could not be decoded: {}", e))?;
                let display_name = crate::stored_prompt::kebab_to_title_case(id);
                StoredPrompt {
                    normalized_name: normalize_prompt_name(&display_name),
                    display_name,
                    instructions: legacy.instructions,
                    skills: legacy.skills,
                    profile: legacy.profile,
                }
            }
            crate::stored_prompt::STORED_PROMPT_SCHEMA_VERSION => {
                serde_json::from_value::<StoredPrompt>(payload)
                    .map_err(|e| format!("could not be decoded: {}", e))?
            }
            version => return Err(format!("unsupported schema version {}", version)),
        };
        prompt
            .validate()
            .map_err(|e| format!("failed validation: {}", e))?;
        Ok(prompt)
    }

    async fn prompt_profile_references_for_impact(
        &self,
    ) -> Result<(Vec<(String, AgentProfileId)>, Vec<String>), ManagementError> {
        let records = self.store.list_prompt_records_raw().await?;
        let mut references = Vec::new();
        let mut diagnostics = Vec::new();
        for (id, schema_version, payload) in records {
            let payload = match serde_json::from_str::<serde_json::Value>(&payload) {
                Ok(payload) => payload,
                Err(e) => {
                    diagnostics.push(format!(
                        "Prompt '{}' has malformed payload; dependency paths may be incomplete: {}",
                        id, e
                    ));
                    continue;
                }
            };
            match Self::decode_prompt_for_impact(&id, schema_version, payload) {
                Ok(prompt) => {
                    if let Some(profile_id) = prompt.profile {
                        references.push((id, profile_id));
                    }
                }
                Err(reason) => diagnostics.push(format!(
                    "Prompt '{}' is not usable for dependency analysis; dependency paths may be incomplete: {}",
                    id, reason
                )),
            }
        }
        Ok((references, diagnostics))
    }

    /// Map a record-local profile classification failure to the same management
    /// diagnostics that direct reads produced before classification was
    /// centralized.
    fn profile_record_diagnostics(
        error: crate::profile::ProfileRecordError,
    ) -> Vec<RecordDiagnostic> {
        use crate::profile::ProfileRecordError;
        match error {
            ProfileRecordError::UnsupportedSchemaVersion { actual } => {
                vec![RecordDiagnostic {
                    category: DiagnosticCategory::UnsupportedSchemaVersion,
                    message: format!("Profile schema version {} is not supported", actual),
                }]
            }
            ProfileRecordError::ReadOnlyRejected { approval } => {
                vec![RecordDiagnostic {
                    category: DiagnosticCategory::ReadOnlyRejected,
                    message: format!(
                        "{} is not a valid user-facing profile approval value",
                        approval
                    ),
                }]
            }
            ProfileRecordError::DecodeFailure { error } => {
                vec![RecordDiagnostic {
                    category: DiagnosticCategory::InvalidPayload,
                    message: format!("Profile payload could not be decoded: {}", error),
                }]
            }
            ProfileRecordError::InvalidFields { messages } => messages
                .into_iter()
                .map(|message| RecordDiagnostic {
                    category: DiagnosticCategory::InvalidPayload,
                    message,
                })
                .collect(),
        }
    }

    /// Validate a decoded automation-task record and return diagnostics for any
    /// structural issues. An empty vector means the record is valid.
    async fn validate_automation_task_record(
        &self,
        task: &crate::automation_task::AutomationTask,
    ) -> Result<Vec<RecordDiagnostic>, ManagementError> {
        use crate::automation_task::{normalize_task_name, MAX_TIMEOUT_SECONDS};
        let mut diagnostics = Vec::new();

        if task.id.trim().is_empty() {
            diagnostics.push(RecordDiagnostic {
                category: DiagnosticCategory::InvalidPayload,
                message: "Automation task ID is empty".to_string(),
            });
        }

        let display_trimmed = task.display_name.trim();
        if display_trimmed.is_empty() {
            diagnostics.push(RecordDiagnostic {
                category: DiagnosticCategory::InvalidPayload,
                message: "Automation task display name is empty".to_string(),
            });
        } else {
            let expected = normalize_task_name(&task.display_name);
            if task.normalized_name.trim().is_empty() {
                diagnostics.push(RecordDiagnostic {
                    category: DiagnosticCategory::InvalidPayload,
                    message: "Automation task normalized name is empty".to_string(),
                });
            } else if task.normalized_name != expected {
                diagnostics.push(RecordDiagnostic {
                    category: DiagnosticCategory::InvalidPayload,
                    message: format!(
                        "Automation task normalized name '{}' does not match display name",
                        task.normalized_name
                    ),
                });
            }
        }

        if task.stored_prompt_id.trim().is_empty() {
            diagnostics.push(RecordDiagnostic {
                category: DiagnosticCategory::InvalidPayload,
                message: "Automation task references an empty stored prompt ID".to_string(),
            });
        } else {
            match self.store.get_prompt(&task.stored_prompt_id).await {
                Ok(None) => {
                    diagnostics.push(RecordDiagnostic {
                        category: DiagnosticCategory::InvalidPayload,
                        message: format!(
                            "Referenced stored prompt '{}' does not exist",
                            task.stored_prompt_id
                        ),
                    });
                }
                Ok(Some(prompt_record)) => {
                    let supported = matches!(
                        prompt_record.schema_version,
                        crate::stored_prompt::LEGACY_STORED_PROMPT_SCHEMA_VERSION
                            | crate::stored_prompt::STORED_PROMPT_SCHEMA_VERSION
                    );
                    if !supported {
                        diagnostics.push(RecordDiagnostic {
                            category: DiagnosticCategory::UnsupportedSchemaVersion,
                            message: format!(
                                "Referenced prompt '{}' has unsupported schema version {}",
                                task.stored_prompt_id, prompt_record.schema_version
                            ),
                        });
                    } else if serde_json::from_value::<StoredPrompt>(prompt_record.payload).is_err()
                    {
                        diagnostics.push(RecordDiagnostic {
                            category: DiagnosticCategory::InvalidPayload,
                            message: format!(
                                "Referenced prompt '{}' could not be decoded",
                                task.stored_prompt_id
                            ),
                        });
                    }
                }
                Err(ConfigError::Deserialization(msg)) => {
                    diagnostics.push(RecordDiagnostic {
                        category: DiagnosticCategory::InvalidPayload,
                        message: format!(
                            "Referenced stored prompt '{}' could not be decoded: {}",
                            task.stored_prompt_id, msg
                        ),
                    });
                }
                Err(e) => return Err(e.into()),
            }
        }

        if task.expected_outcome.trim().is_empty() {
            diagnostics.push(RecordDiagnostic {
                category: DiagnosticCategory::InvalidPayload,
                message: "Automation task expected outcome is empty".to_string(),
            });
        }

        if task.project_root.as_os_str().is_empty() {
            diagnostics.push(RecordDiagnostic {
                category: DiagnosticCategory::InvalidPayload,
                message: "Automation task project root is empty".to_string(),
            });
        }

        if task.timeout_seconds == 0 {
            diagnostics.push(RecordDiagnostic {
                category: DiagnosticCategory::InvalidPayload,
                message: "Automation task timeout must be greater than zero".to_string(),
            });
        } else if task.timeout_seconds > MAX_TIMEOUT_SECONDS {
            diagnostics.push(RecordDiagnostic {
                category: DiagnosticCategory::InvalidPayload,
                message: format!(
                    "Automation task timeout exceeds {} seconds",
                    MAX_TIMEOUT_SECONDS
                ),
            });
        }

        Ok(diagnostics)
    }

    /// Validate a decoded scheduled-task record and return diagnostics for any
    /// structural issues. An empty vector means the record is valid.
    async fn validate_scheduled_task_record(
        &self,
        task: &crate::scheduled_task::ScheduledTask,
    ) -> Result<Vec<RecordDiagnostic>, ManagementError> {
        use crate::scheduled_task::cron::CronExpression;
        let mut diagnostics = Vec::new();

        if task.id.trim().is_empty() {
            diagnostics.push(RecordDiagnostic {
                category: DiagnosticCategory::InvalidPayload,
                message: "Schedule ID is empty".to_string(),
            });
        }

        if task.automation_task_id.trim().is_empty() {
            diagnostics.push(RecordDiagnostic {
                category: DiagnosticCategory::InvalidPayload,
                message: "Schedule references an empty automation task ID".to_string(),
            });
        } else {
            match self
                .store
                .get_automation_task(&task.automation_task_id)
                .await
            {
                Ok(None) => {
                    diagnostics.push(RecordDiagnostic {
                        category: DiagnosticCategory::InvalidPayload,
                        message: format!(
                            "Referenced automation task '{}' does not exist",
                            task.automation_task_id
                        ),
                    });
                }
                Ok(Some(_)) => {}
                Err(ConfigError::Deserialization(msg)) => {
                    diagnostics.push(RecordDiagnostic {
                        category: DiagnosticCategory::InvalidPayload,
                        message: format!(
                            "Referenced automation task '{}' could not be decoded: {}",
                            task.automation_task_id, msg
                        ),
                    });
                }
                Err(e) => return Err(e.into()),
            }
        }

        if task.cron_expression.trim().is_empty() {
            diagnostics.push(RecordDiagnostic {
                category: DiagnosticCategory::InvalidPayload,
                message: "Schedule cron expression is empty".to_string(),
            });
        } else if let Err(e) = CronExpression::parse(&task.cron_expression) {
            diagnostics.push(RecordDiagnostic {
                category: DiagnosticCategory::InvalidPayload,
                message: format!("Schedule cron expression is invalid: {}", e),
            });
        }

        Ok(diagnostics)
    }

    /// Parse a persisted identity_state column value. Returns `None` for
    /// unknown values so callers can surface a diagnostic instead of treating
    /// them as ready.
    fn parse_identity_state(state: &str) -> (Option<IdentityState>, Vec<RecordDiagnostic>) {
        match state {
            "ready" => (Some(IdentityState::Ready), Vec::new()),
            "needs_rename" => (Some(IdentityState::NeedsRename), Vec::new()),
            other => (
                None,
                vec![RecordDiagnostic {
                    category: DiagnosticCategory::UnknownIdentityState,
                    message: format!("Unknown identity_state value '{}'", other),
                }],
            ),
        }
    }

    fn decode_prompt_record(&self, record: crate::config::PromptRecord) -> ManagedPromptRecord {
        let (identity_state, diagnostics) = Self::parse_identity_state(&record.identity_state);

        let make_entry = |prompt: StoredPrompt, identity_state: IdentityState| ManagedPromptEntry {
            prompt,
            identity_state,
            created_at: record.created_at,
            updated_at: record.updated_at,
        };

        let ready = |prompt: StoredPrompt,
                     identity_state: IdentityState,
                     diagnostics: Vec<RecordDiagnostic>|
         -> ManagedPromptRecord {
            if diagnostics.is_empty() {
                ManagedPromptRecord::Ready((record.id.clone(), make_entry(prompt, identity_state)))
            } else {
                ManagedPromptRecord::NeedsAttention {
                    id: record.id.clone(),
                    decoded: Some((record.id.clone(), make_entry(prompt, identity_state))),
                    diagnostics,
                }
            }
        };

        let needs_attention = |prompt: Option<StoredPrompt>,
                               identity_state: IdentityState,
                               mut diagnostics: Vec<RecordDiagnostic>,
                               mut extra: Vec<RecordDiagnostic>|
         -> ManagedPromptRecord {
            diagnostics.append(&mut extra);
            ManagedPromptRecord::NeedsAttention {
                id: record.id.clone(),
                decoded: prompt.map(|p| (record.id.clone(), make_entry(p, identity_state))),
                diagnostics,
            }
        };

        if record.schema_version == crate::stored_prompt::LEGACY_STORED_PROMPT_SCHEMA_VERSION {
            match serde_json::from_value::<StoredPrompt>(record.payload.clone()) {
                Ok(p) if !p.instructions.trim().is_empty() => {
                    let display_name = crate::stored_prompt::kebab_to_title_case(&record.id);
                    let normalized_name =
                        crate::stored_prompt::normalize_prompt_name(&display_name);
                    let prompt = StoredPrompt {
                        display_name: display_name.clone(),
                        normalized_name: normalized_name.clone(),
                        instructions: p.instructions,
                        skills: p.skills,
                        profile: p.profile,
                    };
                    if let Err(e) = prompt.validate() {
                        return needs_attention(
                            Some(prompt),
                            identity_state.unwrap_or(IdentityState::Ready),
                            diagnostics,
                            vec![RecordDiagnostic {
                                category: DiagnosticCategory::InvalidPayload,
                                message: format!("Legacy prompt validation failed: {}", e),
                            }],
                        );
                    }
                    if normalized_name.is_empty() {
                        return needs_attention(
                            Some(prompt),
                            identity_state.unwrap_or(IdentityState::Ready),
                            diagnostics,
                            vec![RecordDiagnostic {
                                category: DiagnosticCategory::InvalidPayload,
                                message: "Legacy prompt ID produces an empty normalized handle"
                                    .to_string(),
                            }],
                        );
                    }
                    if identity_state == Some(IdentityState::NeedsRename) {
                        return needs_attention(
                            Some(prompt),
                            IdentityState::NeedsRename,
                            diagnostics,
                            vec![RecordDiagnostic {
                                category: DiagnosticCategory::NeedsRename,
                                message:
                                    "Normalized name collided during migration; rename required"
                                        .to_string(),
                            }],
                        );
                    }
                    return ready(
                        prompt,
                        identity_state.unwrap_or(IdentityState::Ready),
                        diagnostics,
                    );
                }
                Ok(_) => {
                    return needs_attention(
                        None,
                        IdentityState::Ready,
                        diagnostics,
                        vec![RecordDiagnostic {
                            category: DiagnosticCategory::InvalidPayload,
                            message: "Legacy prompt has empty instructions".to_string(),
                        }],
                    );
                }
                Err(e) => {
                    return needs_attention(
                        None,
                        IdentityState::Ready,
                        diagnostics,
                        vec![RecordDiagnostic {
                            category: DiagnosticCategory::InvalidPayload,
                            message: format!("Legacy prompt payload could not be decoded: {}", e),
                        }],
                    );
                }
            }
        }

        if record.schema_version != crate::stored_prompt::STORED_PROMPT_SCHEMA_VERSION {
            return ManagedPromptRecord::NeedsAttention {
                id: record.id.clone(),
                decoded: None,
                diagnostics: vec![RecordDiagnostic {
                    category: DiagnosticCategory::UnsupportedSchemaVersion,
                    message: format!("Schema version {} not supported", record.schema_version),
                }],
            };
        }

        match serde_json::from_value::<StoredPrompt>(record.payload.clone()) {
            Ok(prompt) => {
                if let Err(e) = prompt.validate() {
                    return needs_attention(
                        Some(prompt),
                        identity_state.unwrap_or(IdentityState::Ready),
                        diagnostics,
                        vec![RecordDiagnostic {
                            category: DiagnosticCategory::InvalidPayload,
                            message: format!("Prompt payload validation failed: {}", e),
                        }],
                    );
                }

                let mut identity_mismatch = Vec::new();
                if prompt.display_name != record.display_name {
                    identity_mismatch.push(RecordDiagnostic {
                        category: DiagnosticCategory::InvalidPayload,
                        message: format!(
                            "Payload display_name '{}' does not match persisted column '{}'",
                            prompt.display_name, record.display_name
                        ),
                    });
                }
                if prompt.normalized_name != record.normalized_name {
                    identity_mismatch.push(RecordDiagnostic {
                        category: DiagnosticCategory::InvalidPayload,
                        message: format!(
                            "Payload normalized_name '{}' does not match persisted column '{}'",
                            prompt.normalized_name, record.normalized_name
                        ),
                    });
                }
                if !identity_mismatch.is_empty() {
                    return needs_attention(
                        Some(prompt),
                        identity_state.unwrap_or(IdentityState::Ready),
                        diagnostics,
                        identity_mismatch,
                    );
                }

                if identity_state == Some(IdentityState::NeedsRename) {
                    return needs_attention(
                        Some(prompt),
                        IdentityState::NeedsRename,
                        diagnostics,
                        vec![RecordDiagnostic {
                            category: DiagnosticCategory::NeedsRename,
                            message: "Normalized name collided during migration; rename required"
                                .to_string(),
                        }],
                    );
                }

                ready(
                    prompt,
                    identity_state.unwrap_or(IdentityState::Ready),
                    diagnostics,
                )
            }
            Err(e) => needs_attention(
                None,
                IdentityState::Ready,
                diagnostics,
                vec![RecordDiagnostic {
                    category: DiagnosticCategory::InvalidPayload,
                    message: format!("Prompt payload could not be decoded: {}", e),
                }],
            ),
        }
    }

    /// Validate that a stored prompt's profile and skill references are sound.
    ///
    /// Ensures any non-`default` profile reference resolves to an existing
    /// profile record, rejects empty/whitespace skill identifiers, rejects
    /// duplicate skills (case-insensitive), and — when a skill inventory is
    /// attached — rejects skills unavailable to the selected profile.
    async fn validate_prompt_references(
        &self,
        prompt: &StoredPrompt,
    ) -> Result<(), ManagementError> {
        if let Some(ref pid) = prompt.profile {
            if pid.as_str() != "default" {
                match self.store.get_profile(pid.as_str()).await {
                    Ok(Some(record)) => {
                        if record.schema_version != PROFILE_SCHEMA_VERSION {
                            return Err(ManagementError::Reference(format!(
                                "Profile '{}' has an unsupported schema version",
                                pid.as_str()
                            )));
                        }
                        if serde_json::from_value::<AgentProfile>(record.payload).is_err() {
                            return Err(ManagementError::Reference(format!(
                                "Profile '{}' could not be decoded",
                                pid.as_str()
                            )));
                        }
                    }
                    Ok(None) | Err(ConfigError::Deserialization(_)) => {
                        return Err(ManagementError::Reference(format!(
                            "Profile '{}' not found",
                            pid.as_str()
                        )));
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }

        let mut seen: HashSet<String> = HashSet::new();
        for skill in &prompt.skills {
            crate::stored_prompt::validate_skill_identifier(skill).map_err(|e| {
                ManagementError::Validation(format!("Invalid skill identifier '{}': {}", skill, e))
            })?;
            let key = skill.to_ascii_lowercase();
            if !seen.insert(key) {
                return Err(ManagementError::Validation(format!(
                    "Duplicate skill identifier: '{}'",
                    skill
                )));
            }
        }

        // Best-effort skill availability validation against the attached
        // inventory, scoped by the selected profile's SkillFilter.
        if let Some(inventory) = &self.skill_inventory {
            let filter = self.resolve_skill_filter(&prompt.profile).await?;
            for skill in &prompt.skills {
                let trimmed = skill.trim();
                let available = match &filter {
                    SkillFilter::None => false,
                    SkillFilter::Inherit => inventory.contains(trimmed),
                    SkillFilter::Allow(names) => {
                        names.iter().any(|n| n.trim() == trimmed) && inventory.contains(trimmed)
                    }
                };
                if !available {
                    return Err(ManagementError::Validation(format!(
                        "Skill '{}' is not available to the selected profile",
                        trimmed
                    )));
                }
            }
        }

        Ok(())
    }

    /// Resolve the SkillFilter for a prompt's profile reference.
    ///
    /// Returns `Inherit` for absent or `default` profiles. Returns `None`
    /// (deny all) for unreadable or undecodable profiles so validation fails
    /// closed rather than broadening access.
    async fn resolve_skill_filter(
        &self,
        profile: &Option<AgentProfileId>,
    ) -> Result<SkillFilter, ManagementError> {
        let pid = match profile {
            None => return Ok(SkillFilter::Inherit),
            Some(pid) if pid.as_str() == "default" => return Ok(SkillFilter::Inherit),
            Some(pid) => pid,
        };
        match self.store.get_profile(pid.as_str()).await {
            Ok(Some(record)) => match serde_json::from_value::<AgentProfile>(record.payload) {
                Ok(p) => Ok(p.skills),
                Err(_) => Ok(SkillFilter::None),
            },
            Ok(None) => Ok(SkillFilter::None),
            Err(e) => Err(e.into()),
        }
    }

    /// Async post-processing that adds unavailable-profile and unavailable-skill
    /// diagnostics to a decoded prompt record. Skill availability is checked
    /// against both the global inventory and the selected profile's
    /// `SkillFilter`.
    async fn with_reference_diagnostics(
        &self,
        record: ManagedPromptRecord,
    ) -> Result<ManagedPromptRecord, ManagementError> {
        let (id, entry, mut diagnostics) = match record {
            ManagedRecord::Ready((id, entry)) => (id, entry, Vec::new()),
            ManagedRecord::NeedsAttention {
                id: _,
                decoded: Some((pid, entry)),
                diagnostics,
            } => (pid, entry, diagnostics),
            other => return Ok(other),
        };

        // Check profile availability.
        if let Some(profile_id) = &entry.prompt.profile {
            if profile_id.as_str() != "default" {
                let usable = match self.store.get_profile(profile_id.as_str()).await {
                    Ok(Some(record)) => {
                        record.schema_version == PROFILE_SCHEMA_VERSION
                            && serde_json::from_value::<AgentProfile>(record.payload).is_ok()
                    }
                    Ok(None) | Err(ConfigError::Deserialization(_)) => false,
                    Err(e) => return Err(e.into()),
                };
                if !usable {
                    diagnostics.push(RecordDiagnostic {
                        category: DiagnosticCategory::UnavailableProfile,
                        message: format!("Profile '{}' is not available", profile_id.as_str()),
                    });
                }
            }
        }

        // Check skill availability with profile filter.
        if let Some(inventory) = &self.skill_inventory {
            let filter = self.resolve_skill_filter(&entry.prompt.profile).await?;
            for skill in &entry.prompt.skills {
                let trimmed = skill.trim();
                let available = match &filter {
                    SkillFilter::None => false,
                    SkillFilter::Inherit => inventory.contains(trimmed),
                    SkillFilter::Allow(names) => {
                        names.iter().any(|n| n.trim() == trimmed) && inventory.contains(trimmed)
                    }
                };
                if !available {
                    diagnostics.push(RecordDiagnostic {
                        category: DiagnosticCategory::UnavailableSkill,
                        message: format!(
                            "Skill '{}' is not available to the selected profile",
                            trimmed
                        ),
                    });
                }
            }
        }

        if diagnostics.is_empty() {
            Ok(ManagedRecord::Ready((id, entry)))
        } else {
            diagnostics.sort_by(|a, b| a.message.cmp(&b.message));
            Ok(ManagedRecord::NeedsAttention {
                id: id.clone(),
                decoded: Some((id, entry)),
                diagnostics,
            })
        }
    }
}

/// Extract the provider slug from a managed profile, if any.
fn provider_slug_of(profile: &AgentProfile) -> Option<String> {
    match &profile.provider {
        AgentProfileProvider::Managed { provider_slug, .. } => {
            Some(provider_slug.as_str().to_string())
        }
        AgentProfileProvider::RuntimeDefault => None,
    }
}

/// Render a record-local profile classification failure as the single-line
/// reason used by dependency-impact diagnostics. Preserves the wording the
/// pre-centralization impact traversal produced.
fn profile_record_error_to_impact_reason(error: crate::profile::ProfileRecordError) -> String {
    use crate::profile::ProfileRecordError;
    match error {
        ProfileRecordError::UnsupportedSchemaVersion { actual } => {
            format!("unsupported schema version {}", actual)
        }
        ProfileRecordError::ReadOnlyRejected { approval } => format!(
            "could not be decoded: {} is not a valid user-facing profile approval value",
            approval
        ),
        ProfileRecordError::DecodeFailure { error } => {
            format!("could not be decoded: {}", error)
        }
        ProfileRecordError::InvalidFields { messages } => messages.join("; "),
    }
}
// ============================================================================
// Managed entry types
// ============================================================================

/// A profile entry with timestamps.
#[derive(Debug, Clone, PartialEq)]
pub struct ManagedProfileEntry {
    /// Stable profile record ID.
    pub id: AgentProfileId,
    /// Decoded and structurally valid profile.
    pub profile: AgentProfile,
    /// Time at which the profile row was first inserted.
    pub created_at: DateTime<Utc>,
    /// Time at which the profile row was last replaced.
    pub updated_at: DateTime<Utc>,
}

/// Outcome of reading a profile record.
pub type ManagedProfileRecord = ManagedRecord<ManagedProfileEntry>;

/// Outcome of reading an automation-task record.
pub type ManagedAutomationTaskRecord = ManagedRecord<AutomationTask>;

/// Outcome of reading a scheduled-task record.
pub type ManagedScheduledTaskRecord = ManagedRecord<ScheduledTask>;

/// A prompt entry with timestamps and identity state.
#[derive(Debug, Clone, PartialEq)]
pub struct ManagedPromptEntry {
    /// Decoded prompt definition.
    pub prompt: StoredPrompt,
    /// Persisted identity state controlling handle-based availability.
    pub identity_state: IdentityState,
    /// Time at which the prompt row was first inserted.
    pub created_at: DateTime<Utc>,
    /// Time at which the prompt row was last replaced or renamed.
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
    fn config_error_minimum_valid_profiles_maps_to_management_variant() {
        let config_err = ConfigError::MinimumValidProfiles {
            minimum: 1,
            remaining: 0,
        };
        let management_err = ManagementError::from(config_err);
        assert!(
            matches!(
                management_err,
                ManagementError::MinimumValidProfiles {
                    minimum: 1,
                    remaining: 0
                }
            ),
            "expected typed MinimumValidProfiles mapping, got {:?}",
            management_err
        );
    }

    #[test]
    fn config_error_minimum_valid_profiles_display_reports_values() {
        let err = ConfigError::MinimumValidProfiles {
            minimum: 2,
            remaining: 1,
        };
        let message = err.to_string();
        assert!(message.contains("minimum of 2"), "{}", message);
        assert!(message.contains("1 valid profile"), "{}", message);
    }

    struct FailingProfileRegistry;
    impl ProfileRegistry for FailingProfileRegistry {
        fn insert(&self, _id: AgentProfileId, _profile: AgentProfile) -> Result<(), String> {
            Err("simulated profile registry failure".to_string())
        }
        fn remove(&self, _id: &AgentProfileId) -> Result<(), String> {
            Ok(())
        }
        fn find_by_normalized_name(
            &self,
            _name: &str,
            _excluding_id: &AgentProfileId,
        ) -> Option<AgentProfileId> {
            None
        }
    }

    /// Registry that records removals but reports failure, to exercise the
    /// partial-operation path after a durable policy-aware deletion commits.
    struct RemoveFailingProfileRegistry;
    impl ProfileRegistry for RemoveFailingProfileRegistry {
        fn insert(&self, _id: AgentProfileId, _profile: AgentProfile) -> Result<(), String> {
            Ok(())
        }
        fn remove(&self, _id: &AgentProfileId) -> Result<(), String> {
            Err("simulated profile registry remove failure".to_string())
        }
        fn find_by_normalized_name(
            &self,
            _name: &str,
            _excluding_id: &AgentProfileId,
        ) -> Option<AgentProfileId> {
            None
        }
    }

    struct FailingPromptRegistry;
    impl PromptRegistry for FailingPromptRegistry {
        fn register(&self, _id: String, _prompt: StoredPrompt) -> Result<(), String> {
            Err("simulated prompt registry failure".to_string())
        }
        fn unregister(&self, _id: &str) -> Result<(), String> {
            Ok(())
        }
    }

    struct DesiredDeleteFailureHost {
        store: ConfigStore,
        removed: std::sync::atomic::AtomicBool,
    }

    #[async_trait::async_trait]
    impl HostScheduler for DesiredDeleteFailureHost {
        fn platform(&self) -> &'static str {
            "test"
        }

        async fn install(
            &self,
            _request: &crate::scheduled_task::host::HostInstallRequest,
        ) -> Result<(), crate::scheduled_task::host::HostSchedulerError> {
            Ok(())
        }

        async fn remove(
            &self,
            _schedule_id: &str,
        ) -> Result<(), crate::scheduled_task::host::HostSchedulerError> {
            self.removed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            sqlx::query("ALTER TABLE schedule RENAME TO schedule_after_host_removal")
                .execute(self.store.pool())
                .await
                .map_err(|e| crate::scheduled_task::host::HostSchedulerError::Io(e.to_string()))?;
            Ok(())
        }

        async fn list_owned(
            &self,
        ) -> Result<
            Vec<crate::scheduled_task::host::ObservedHostEntry>,
            crate::scheduled_task::host::HostSchedulerError,
        > {
            Ok(Vec::new())
        }

        async fn inspect(
            &self,
            _schedule_id: &str,
        ) -> Result<
            Option<crate::scheduled_task::host::ObservedHostEntry>,
            crate::scheduled_task::host::HostSchedulerError,
        > {
            Ok(None)
        }
    }

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

        let (id1, _) = svc
            .create_prompt("Check Email", "Check inbox", vec![], None)
            .await
            .unwrap();
        let (id2, _) = svc
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

        let (id, _) = svc
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

        let (id, _) = svc
            .create_prompt("Check Email", "Check inbox", vec![], None)
            .await
            .unwrap();

        svc.save_automation_task(&AutomationTaskInput {
            id: "task".to_string(),
            display_name: "Task".to_string(),
            stored_prompt_id: id.clone(),
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
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
        let task = svc.get_automation_task("task").await.unwrap().unwrap();
        assert!(matches!(
            task,
            ManagedAutomationTaskRecord::Ready(task) if task.stored_prompt_id == id
        ));
        assert!(svc
            .get_prompt_by_handle("process-inbox")
            .await
            .unwrap()
            .is_some());
        assert!(svc
            .get_prompt_by_handle("check-email")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn legacy_prompt_upgrades_on_save_without_rewriting_task_reference() {
        use crate::config::PromptInput;

        let store = ConfigStore::open_in_memory().await.unwrap();
        store
            .set_prompt(&PromptInput {
                id: "old-prompt".to_string(),
                schema_version: crate::stored_prompt::LEGACY_STORED_PROMPT_SCHEMA_VERSION,
                payload: serde_json::json!({"instructions": "Do legacy work"}),
                display_name: "old-prompt".to_string(),
                normalized_name: "old-prompt".to_string(),
            })
            .await
            .unwrap();
        let svc = ConfigManagementService::new(store.clone());
        svc.save_automation_task(&AutomationTaskInput {
            id: "legacy-task".to_string(),
            display_name: "Legacy Task".to_string(),
            stored_prompt_id: "old-prompt".to_string(),
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

        let prompt = match svc.get_prompt("old-prompt").await.unwrap().unwrap() {
            ManagedPromptRecord::Ready((id, entry)) => {
                assert_eq!(id, "old-prompt");
                entry.prompt
            }
            other => panic!("expected ready legacy prompt, got {:?}", other),
        };
        svc.save_prompt("old-prompt", &prompt).await.unwrap();

        let record = store.get_prompt("old-prompt").await.unwrap().unwrap();
        assert_eq!(
            record.schema_version,
            crate::stored_prompt::STORED_PROMPT_SCHEMA_VERSION
        );
        let task = svc
            .get_automation_task("legacy-task")
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            task,
            ManagedAutomationTaskRecord::Ready(task)
                if task.stored_prompt_id == "old-prompt"
        ));
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
    async fn oauth_summary_is_redacted_and_replaceable_by_api_key() {
        use crate::provider_credential::domain::{OAuthTokenSet, StoredCredential};
        use std::time::{Duration, SystemTime};

        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());
        let oauth = StoredCredential::OAuthBearer(OAuthTokenSet {
            access_token: "ACCESS_TOKEN_SENTINEL".to_string(),
            refresh_token: "REFRESH_TOKEN_SENTINEL".to_string(),
            expires_at: Some(SystemTime::now() + Duration::from_secs(3600)),
            id_token: Some("ID_TOKEN_SENTINEL".to_string()),
        });
        store
            .set_credential(
                "oauth-provider",
                "oauth_bearer",
                &serde_json::to_vec(&oauth).unwrap(),
            )
            .await
            .unwrap();

        let summaries = svc.list_credentials().await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].credential_mode, CredentialMode::OAuthBearer);
        assert!(matches!(
            summaries[0].auth_status,
            ProviderAuthStatus::ConnectedOAuth { .. }
        ));
        let output = format!(
            "{:?}\n{}",
            summaries,
            serde_json::to_string(&summaries).unwrap()
        );
        for secret in [
            "ACCESS_TOKEN_SENTINEL",
            "REFRESH_TOKEN_SENTINEL",
            "ID_TOKEN_SENTINEL",
        ] {
            assert!(!output.contains(secret));
        }

        svc.set_api_key("oauth-provider", "API_KEY_SENTINEL")
            .await
            .unwrap();
        let stored = store
            .get_credential("oauth-provider")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<StoredCredential>(&stored).unwrap(),
            StoredCredential::ApiKey("API_KEY_SENTINEL".to_string())
        );

        assert!(matches!(
            svc.set_api_key("oauth-provider", "   ").await,
            Err(ManagementError::Validation(_))
        ));
        assert_eq!(
            serde_json::from_slice::<StoredCredential>(
                &store
                    .get_credential("oauth-provider")
                    .await
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            StoredCredential::ApiKey("API_KEY_SENTINEL".to_string())
        );

        svc.delete_credential("oauth-provider").await.unwrap();
        assert!(svc.list_credentials().await.unwrap().is_empty());
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
    async fn successful_registry_sync_and_profile_rename_preserve_references() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let profile_registry = Arc::new(RwLock::new(HashMap::new()));
        let prompt_registry = Arc::new(RwLock::new(StoredPromptRegistry::new()));
        let svc = ConfigManagementService::new(store)
            .with_profile_registry(profile_registry.clone())
            .with_prompt_registry(prompt_registry.clone());

        svc.save_profile("profile", &AgentProfile::with_name("Original Name"))
            .await
            .unwrap();
        let (prompt_id, _) = svc
            .create_prompt(
                "Prompt",
                "Instructions",
                vec![],
                Some(AgentProfileId::from("profile")),
            )
            .await
            .unwrap();

        assert_eq!(
            profile_registry.read()[&AgentProfileId::from("profile")].name,
            "Original Name"
        );
        assert!(prompt_registry.read().get(&prompt_id).is_some());

        svc.save_profile("profile", &AgentProfile::with_name("Renamed Profile"))
            .await
            .unwrap();
        assert_eq!(
            profile_registry.read()[&AgentProfileId::from("profile")].name,
            "Renamed Profile"
        );
        let prompt = svc.get_prompt(&prompt_id).await.unwrap().unwrap();
        assert!(matches!(
            prompt,
            ManagedPromptRecord::Ready((_, entry))
                if entry.prompt.profile == Some(AgentProfileId::from("profile"))
        ));
    }

    #[tokio::test]
    async fn profile_list_reports_rejected_approval_values_without_hiding_valid_siblings() {
        use crate::config::ProfileInput;

        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());
        svc.save_profile("valid", &AgentProfile::with_name("Valid"))
            .await
            .unwrap();

        for (id, approval) in [
            ("read-only", "ReadOnly"),
            ("require-approval", "RequireApproval"),
        ] {
            let mut payload = serde_json::to_value(AgentProfile::with_name(id)).unwrap();
            payload
                .as_object_mut()
                .unwrap()
                .insert("approval".to_string(), serde_json::json!(approval));
            store
                .set_profile(&ProfileInput {
                    id: id.to_string(),
                    schema_version: PROFILE_SCHEMA_VERSION,
                    payload,
                })
                .await
                .unwrap();
        }

        let records = svc.list_profiles().await.unwrap();
        assert!(records.iter().any(|record| matches!(
            record,
            ManagedProfileRecord::Ready(entry) if entry.id.as_str() == "valid"
        )));
        for (id, approval) in [
            ("read-only", "ReadOnly"),
            ("require-approval", "RequireApproval"),
        ] {
            assert!(records.iter().any(|record| matches!(
                record,
                ManagedProfileRecord::NeedsAttention {
                    id: record_id,
                    decoded: None,
                    diagnostics,
                } if record_id == id
                    && diagnostics.iter().any(|diagnostic|
                        diagnostic.category == DiagnosticCategory::ReadOnlyRejected
                            && diagnostic.message.contains(approval))
            )));
        }
    }

    #[tokio::test]
    async fn management_reads_propagate_fatal_storage_errors() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());
        sqlx::query("DROP TABLE profiles")
            .execute(store.pool())
            .await
            .unwrap();

        assert!(matches!(
            svc.list_profiles().await,
            Err(ManagementError::Storage(_))
        ));
        assert!(matches!(
            svc.get_profile("profile").await,
            Err(ManagementError::Storage(_))
        ));
    }

    #[tokio::test]
    async fn schedule_id_with_path_separator_rejected() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let (prompt_id, _) = svc
            .create_prompt("Test", "Instructions", vec![], None)
            .await
            .unwrap();
        svc.save_automation_task(&AutomationTaskInput {
            id: "task".to_string(),
            display_name: "Task".to_string(),
            stored_prompt_id: prompt_id,
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

        for bad_id in &["bad/id", "bad\\id", "..", "foo..bar"] {
            let result = svc
                .save_scheduled_task(&ScheduledTaskInput {
                    id: bad_id.to_string(),
                    automation_task_id: "task".to_string(),
                    cron_expression: "0 9 * * *".to_string(),
                    enabled: true,
                })
                .await;
            assert!(
                matches!(result, Err(ManagementError::Validation(ref msg)) if msg.contains("path")),
                "schedule ID '{}' should be rejected, got {:?}",
                bad_id,
                result
            );
        }
    }

    #[tokio::test]
    async fn future_schema_database_rejected() {
        use crate::config::migrations::{apply_migrations, CURRENT_SCHEMA_VERSION};

        let store = ConfigStore::open_in_memory().await.unwrap();
        sqlx::query("UPDATE schema_version SET version = ? WHERE id = 1")
            .bind(CURRENT_SCHEMA_VERSION + 1)
            .execute(store.pool())
            .await
            .unwrap();

        let result = apply_migrations(store.pool()).await;
        assert!(result.is_err(), "future-schema database should be rejected");
    }

    #[tokio::test]
    async fn typed_task_write_rejects_unsupported_prompt_schema() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        let (prompt_id, _) = svc
            .create_prompt("Test", "Instructions", vec![], None)
            .await
            .unwrap();

        sqlx::query("UPDATE prompts SET schema_version = 999 WHERE id = ?")
            .bind(&prompt_id)
            .execute(store.pool())
            .await
            .unwrap();

        let result = svc
            .save_automation_task(&AutomationTaskInput {
                id: "task".to_string(),
                display_name: "Task".to_string(),
                stored_prompt_id: prompt_id,
                expected_outcome: "Done".to_string(),
                project_root: std::env::temp_dir(),
                timeout_seconds: 300,
            })
            .await;
        assert!(
            matches!(result, Err(ManagementError::Validation(ref msg)) if msg.contains("unsupported schema")),
            "task referencing unsupported prompt should be rejected, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn typed_schedule_write_rejects_unsupported_task_schema() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        let (prompt_id, _) = svc
            .create_prompt("Test", "Instructions", vec![], None)
            .await
            .unwrap();
        svc.save_automation_task(&AutomationTaskInput {
            id: "task".to_string(),
            display_name: "Task".to_string(),
            stored_prompt_id: prompt_id,
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

        sqlx::query("UPDATE automation_tasks SET schema_version = 999 WHERE id = 'task'")
            .execute(store.pool())
            .await
            .unwrap();

        let result = svc
            .save_scheduled_task(&ScheduledTaskInput {
                id: "sched".to_string(),
                automation_task_id: "task".to_string(),
                cron_expression: "0 9 * * *".to_string(),
                enabled: true,
            })
            .await;
        assert!(
            matches!(result, Err(ManagementError::Validation(ref msg)) if msg.contains("unsupported schema")),
            "schedule referencing unsupported task should be rejected, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn credential_deletion_preserves_dependent_definitions() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        svc.set_api_key("test-provider", "sk-key").await.unwrap();

        let profile = AgentProfile {
            name: "Automation".to_string(),
            provider: AgentProfileProvider::Managed {
                provider_slug: crate::provider_credential::domain::ProviderSlug::from(
                    "test-provider",
                ),
                model: "test-model".to_string(),
            },
            ..AgentProfile::with_name("Automation")
        };
        svc.save_profile("auto-prof", &profile).await.unwrap();

        let (prompt_id, _) = svc
            .create_prompt(
                "Daily Report",
                "Generate a daily report",
                vec![],
                Some(AgentProfileId::from("auto-prof")),
            )
            .await
            .unwrap();

        svc.save_automation_task(&AutomationTaskInput {
            id: "daily-task".to_string(),
            display_name: "Daily Task".to_string(),
            stored_prompt_id: prompt_id.clone(),
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

        svc.save_scheduled_task(&ScheduledTaskInput {
            id: "daily-sched".to_string(),
            automation_task_id: "daily-task".to_string(),
            cron_expression: "0 9 * * *".to_string(),
            enabled: true,
        })
        .await
        .unwrap();

        svc.delete_credential("test-provider").await.unwrap();
        assert!(svc.list_credentials().await.unwrap().is_empty());

        assert!(svc.get_profile("auto-prof").await.unwrap().is_some());
        assert!(svc.get_prompt(&prompt_id).await.unwrap().is_some());
        assert!(svc
            .get_automation_task("daily-task")
            .await
            .unwrap()
            .is_some());
        assert!(svc
            .get_scheduled_task("daily-sched")
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn profile_deletion_conflict_returns_sorted_referrer_ids() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        svc.save_profile("prof", &AgentProfile::with_name("Profile"))
            .await
            .unwrap();

        // Create prompts; IDs are generated UUIDs.
        let mut prompt_ids = Vec::new();
        for name in &["zeta", "alpha", "mid"] {
            let (id, _) = svc
                .create_prompt(
                    name,
                    "Instructions",
                    vec![],
                    Some(AgentProfileId::from("prof")),
                )
                .await
                .unwrap();
            prompt_ids.push(id);
        }
        prompt_ids.sort();

        let result = svc.delete_profile("prof").await;
        match result {
            Err(ManagementError::Conflict { referrers, .. }) => {
                assert_eq!(referrers, prompt_ids);
            }
            other => panic!("expected sorted conflict, got {:?}", other),
        }
        assert!(svc.get_profile("prof").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn delete_profile_with_policy_rejects_below_minimum() {
        use crate::profile::ProfileDeletePolicy;

        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        svc.save_profile("prof", &AgentProfile::with_name("Profile"))
            .await
            .unwrap();

        let result = svc
            .delete_profile_with_policy("prof", ProfileDeletePolicy::RequireMinimumValid(1))
            .await;
        match result {
            Err(ManagementError::MinimumValidProfiles { minimum, remaining }) => {
                assert_eq!(minimum, 1);
                assert_eq!(remaining, 0);
            }
            other => panic!("expected MinimumValidProfiles, got {:?}", other),
        }
        assert!(svc.get_profile("prof").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn delete_profile_with_policy_invalid_target_contributes_zero() {
        use crate::profile::ProfileDeletePolicy;

        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        svc.save_profile("valid", &AgentProfile::with_name("Valid"))
            .await
            .unwrap();
        // Persist a structurally invalid profile directly through the store.
        store
            .set_profile(&crate::config::ProfileInput {
                id: "invalid".to_string(),
                schema_version: crate::profile::PROFILE_SCHEMA_VERSION,
                payload: serde_json::json!({"name": ""}),
            })
            .await
            .unwrap();

        svc.delete_profile_with_policy("invalid", ProfileDeletePolicy::RequireMinimumValid(1))
            .await
            .unwrap();
        assert!(svc.get_profile("valid").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn delete_profile_with_policy_prompt_conflict_precedes_minimum() {
        use crate::profile::ProfileDeletePolicy;

        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        svc.save_profile("prof", &AgentProfile::with_name("Profile"))
            .await
            .unwrap();
        svc.create_prompt(
            "Alpha",
            "Instructions",
            vec![],
            Some(AgentProfileId::from("prof")),
        )
        .await
        .unwrap();

        let result = svc
            .delete_profile_with_policy("prof", ProfileDeletePolicy::RequireMinimumValid(1))
            .await;
        assert!(matches!(result, Err(ManagementError::Conflict { .. })));
        assert!(svc.get_profile("prof").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn delete_profile_with_policy_rejection_leaves_registry_unchanged() {
        use crate::profile::ProfileDeletePolicy;

        let store = ConfigStore::open_in_memory().await.unwrap();
        let registry = Arc::new(parking_lot::RwLock::new(HashMap::new()));
        let svc = ConfigManagementService::new(store).with_profile_registry(registry.clone());

        svc.save_profile("prof", &AgentProfile::with_name("Profile"))
            .await
            .unwrap();
        assert!(registry.read().contains_key(&AgentProfileId::from("prof")));

        let result = svc
            .delete_profile_with_policy("prof", ProfileDeletePolicy::RequireMinimumValid(1))
            .await;
        assert!(matches!(
            result,
            Err(ManagementError::MinimumValidProfiles { .. })
        ));
        // Pre-commit rejection must leave the registry entry intact.
        assert!(registry.read().contains_key(&AgentProfileId::from("prof")));
    }

    #[tokio::test]
    async fn delete_profile_with_policy_partial_when_registry_remove_fails() {
        use crate::profile::ProfileDeletePolicy;

        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone())
            .with_profile_registry_for_test(Arc::new(RemoveFailingProfileRegistry));

        svc.save_profile("prof", &AgentProfile::with_name("Profile"))
            .await
            .unwrap();
        // Provide a second valid profile so RequireMinimumValid(1) is satisfied.
        svc.save_profile("other", &AgentProfile::with_name("Other"))
            .await
            .unwrap();

        let result = svc
            .delete_profile_with_policy("prof", ProfileDeletePolicy::RequireMinimumValid(1))
            .await;
        assert!(matches!(
            result,
            Err(ManagementError::Partial {
                durable_succeeded: true,
                ..
            })
        ));
        // Durable deletion committed despite the registry failure.
        assert!(svc.get_profile("prof").await.unwrap().is_none());
        assert!(svc.get_profile("other").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn existing_delete_profile_removes_final_valid_profile() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        svc.save_profile("prof", &AgentProfile::with_name("Profile"))
            .await
            .unwrap();

        svc.delete_profile("prof").await.unwrap();
        assert!(svc.get_profile("prof").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn scheduler_unavailable_for_reconcile_and_combined_deletion() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        assert!(matches!(
            svc.reconcile_schedule("sched").await,
            Err(ManagementError::SchedulerUnavailable)
        ));
        assert!(matches!(
            svc.reconcile_all_schedules(
                crate::scheduled_task::manager::OrphanPolicy::RemoveOrphans
            )
            .await,
            Err(ManagementError::SchedulerUnavailable)
        ));
        assert!(matches!(
            svc.delete_schedule_combined("sched").await,
            Err(ManagementError::SchedulerUnavailable)
        ));
    }

    #[tokio::test]
    async fn dependency_report_exact_paths_and_ordering() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let profile = AgentProfile {
            name: "Automation".to_string(),
            provider: AgentProfileProvider::Managed {
                provider_slug: crate::provider_credential::domain::ProviderSlug::from("provider"),
                model: "model".to_string(),
            },
            ..AgentProfile::with_name("Automation")
        };
        svc.save_profile("profile", &profile).await.unwrap();

        let (prompt_id, _) = svc
            .create_prompt(
                "Daily Report",
                "Generate a daily report",
                vec![],
                Some(AgentProfileId::from("profile")),
            )
            .await
            .unwrap();

        svc.save_automation_task(&AutomationTaskInput {
            id: "task".to_string(),
            display_name: "Task".to_string(),
            stored_prompt_id: prompt_id.clone(),
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

        svc.save_scheduled_task(&ScheduledTaskInput {
            id: "sched".to_string(),
            automation_task_id: "task".to_string(),
            cron_expression: "0 9 * * *".to_string(),
            enabled: true,
        })
        .await
        .unwrap();

        let report = svc.profile_impact("profile").await.unwrap();

        // Assert exact link entities, directions, and proximity.
        let expected_links = vec![
            (
                DependencyEntity::ProviderCredential {
                    slug: "provider".to_string(),
                },
                DependencyDirection::Depends,
                DependencyProximity::Direct,
            ),
            (
                DependencyEntity::Prompt {
                    id: prompt_id.clone(),
                },
                DependencyDirection::Dependent,
                DependencyProximity::Direct,
            ),
            (
                DependencyEntity::AutomationTask {
                    id: "task".to_string(),
                },
                DependencyDirection::Dependent,
                DependencyProximity::Transitive,
            ),
            (
                DependencyEntity::ScheduledTask {
                    id: "sched".to_string(),
                },
                DependencyDirection::Dependent,
                DependencyProximity::Transitive,
            ),
        ];

        for (entity, direction, proximity) in &expected_links {
            assert!(
                report.links.iter().any(|link| {
                    &link.entity == entity
                        && &link.direction == direction
                        && &link.proximity == proximity
                }),
                "expected link {:?} {:?} {:?} not found in {:?}",
                entity,
                direction,
                proximity,
                report.links
            );
        }

        // No duplicate links.
        assert_eq!(
            report.links.len(),
            expected_links.len(),
            "link count should match expected without duplicates"
        );

        // All paths start at the report target.
        assert!(report
            .links
            .iter()
            .all(|link| { !link.path.is_empty() && link.path[0] == report.target }));

        // Repeated query produces identical results.
        let report2 = svc.profile_impact("profile").await.unwrap();
        assert_eq!(report.links, report2.links);
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

        let (id, _) = svc
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

        let (id, _) = svc
            .create_prompt("Check Email", "Check inbox", vec![], None)
            .await
            .unwrap();

        let repair_handle = "legacy-a1b2c3d4e5f67890".to_string();
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
    async fn credential_list_surfaces_unknown_mode() {
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
        assert_eq!(list.len(), 2);
        let unknown = list
            .into_iter()
            .find(|s| s.provider_slug == "unknown-provider")
            .expect("unknown provider should be listed");
        assert_eq!(unknown.credential_mode, CredentialMode::Unsupported);
        assert_eq!(
            unknown.auth_status,
            ProviderAuthStatus::UnsupportedCredential
        );
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
    async fn profile_save_partial_when_registry_fails() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone())
            .with_profile_registry_for_test(Arc::new(FailingProfileRegistry));

        let profile = AgentProfile::with_name("Test Profile");
        let result = svc.save_profile("test-prof", &profile).await;
        assert!(
            matches!(
                result,
                Err(ManagementError::Partial {
                    durable_succeeded: true,
                    ..
                })
            ),
            "registry failure should yield partial operation, got {:?}",
            result
        );

        // Durable state is committed despite registry failure.
        let persisted = svc.get_profile("test-prof").await.unwrap();
        assert!(persisted.is_some(), "profile should be persisted");
    }

    #[tokio::test]
    async fn create_prompt_partial_when_registry_fails() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone())
            .with_prompt_registry_for_test(Arc::new(FailingPromptRegistry));

        let result = svc
            .create_prompt("Check Email", "Check inbox", vec![], None)
            .await;
        let committed_id = match result {
            Err(ManagementError::Partial {
                target,
                durable_succeeded: true,
                ..
            }) => target,
            other => panic!(
                "prompt registry failure should yield partial operation, got {:?}",
                other
            ),
        };

        let list = svc.list_prompts().await.unwrap();
        assert_eq!(list.len(), 1, "prompt should be persisted");
        assert_eq!(list[0].id(), committed_id);
        assert!(svc.get_prompt(&committed_id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn legacy_prefix_rejected_for_prompt_name() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let result = svc
            .create_prompt("legacy-foo", "Instructions", vec![], None)
            .await;
        assert!(
            matches!(result, Err(ManagementError::Validation(_))),
            "legacy- prefix should be rejected, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn malformed_profile_reference_returns_reference_error() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        // Save a profile, then corrupt its payload so it cannot be decoded.
        svc.save_profile("good-profile", &AgentProfile::with_name("Good Profile"))
            .await
            .unwrap();
        sqlx::query("UPDATE profiles SET payload = 'not-json' WHERE id = ?")
            .bind("good-profile")
            .execute(store.pool())
            .await
            .unwrap();

        let result = svc
            .create_prompt(
                "Test",
                "Instructions",
                vec![],
                Some(AgentProfileId::from("good-profile")),
            )
            .await;
        assert!(
            matches!(result, Err(ManagementError::Reference(_))),
            "malformed profile should be treated as missing reference, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn unsupported_schema_profile_reference_rejected() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        svc.save_profile("good-profile", &AgentProfile::with_name("Good Profile"))
            .await
            .unwrap();
        // Corrupt the schema version so the profile exists but is unusable.
        sqlx::query("UPDATE profiles SET schema_version = 999 WHERE id = ?")
            .bind("good-profile")
            .execute(store.pool())
            .await
            .unwrap();

        let result = svc
            .create_prompt(
                "Test",
                "Instructions",
                vec![],
                Some(AgentProfileId::from("good-profile")),
            )
            .await;
        assert!(
            matches!(result, Err(ManagementError::Reference(_))),
            "unsupported-schema profile should be treated as missing reference, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn legacy_needs_rename_prompt_returns_needs_attention() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        // Insert a legacy schema-v1 prompt and mark it needs_rename (simulating
        // a collision during migration).
        let legacy_payload = serde_json::json!({
            "instructions": "Do something"
        });
        sqlx::query(
            "INSERT INTO prompts (id, schema_version, payload, display_name, normalized_name, identity_state, created_at, updated_at) \
             VALUES (?, 1, ?, '', 'legacy-deadbeef', 'needs_rename', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind("check-email")
        .bind(legacy_payload.to_string())
        .execute(store.pool())
        .await
        .unwrap();

        let result = svc.get_prompt("check-email").await.unwrap().unwrap();
        match result {
            ManagedPromptRecord::NeedsAttention {
                decoded,
                diagnostics,
                ..
            } => {
                assert!(decoded.is_some(), "legacy prompt should be decoded");
                assert!(
                    diagnostics
                        .iter()
                        .any(|d| d.category == crate::management::DiagnosticCategory::NeedsRename),
                    "should include NeedsRename diagnostic, got {:?}",
                    diagnostics
                );
            }
            other => panic!("Expected NeedsAttention, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn profile_impact_includes_malformed_prompt_diagnostic() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        svc.save_profile("test-prof", &AgentProfile::with_name("Test Profile"))
            .await
            .unwrap();

        // Insert a prompt with invalid JSON payload.
        sqlx::query(
            "INSERT INTO prompts (id, schema_version, payload, display_name, normalized_name, identity_state, created_at, updated_at) \
             VALUES ('bad-prompt', 2, 'not-json', 'Bad', 'bad', 'ready', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .execute(store.pool())
        .await
        .unwrap();

        let report = svc.profile_impact("test-prof").await.unwrap();
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.contains("bad-prompt") && d.contains("malformed")),
            "should include diagnostic about malformed prompt, got {:?}",
            report.diagnostics
        );
    }

    #[tokio::test]
    async fn dependency_impact_rejects_unsupported_and_structurally_invalid_records() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        let profile = AgentProfile {
            provider: AgentProfileProvider::Managed {
                provider_slug: crate::provider_credential::domain::ProviderSlug::from("provider"),
                model: "model".to_string(),
            },
            ..AgentProfile::with_name("Profile")
        };
        svc.save_profile("profile", &profile).await.unwrap();

        let valid_prompt_payload = serde_json::json!({
            "display_name": "Future Prompt",
            "normalized_name": "future-prompt",
            "instructions": "Do work",
            "profile": "profile"
        });
        sqlx::query(
            "INSERT INTO prompts (id, schema_version, payload, display_name, normalized_name, identity_state, created_at, updated_at) \
             VALUES ('future-prompt', 999, ?, 'Future Prompt', 'future-prompt', 'ready', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind(valid_prompt_payload.to_string())
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO prompts (id, schema_version, payload, display_name, normalized_name, identity_state, created_at, updated_at) \
             VALUES ('invalid-prompt', 2, ?, 'Invalid Prompt', 'invalid-prompt', 'ready', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind(r#"{"display_name":"Invalid Prompt","normalized_name":"invalid-prompt","instructions":"Do work","profile":123}"#)
        .execute(store.pool())
        .await
        .unwrap();

        let report = svc.profile_impact("profile").await.unwrap();
        assert!(report.links.iter().all(|link| {
            !matches!(
                &link.entity,
                DependencyEntity::Prompt { id }
                    if id == "future-prompt" || id == "invalid-prompt"
            )
        }));
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("future-prompt")
                && diagnostic.contains("unsupported schema version")));
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("invalid-prompt")
                && diagnostic.contains("could not be decoded")));

        sqlx::query("UPDATE profiles SET schema_version = 999 WHERE id = 'profile'")
            .execute(store.pool())
            .await
            .unwrap();
        let report = svc.credential_impact("provider").await.unwrap();
        assert!(report.links.iter().all(|link| {
            !matches!(&link.entity, DependencyEntity::Profile { id } if id == "profile")
        }));
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("profile")
                && diagnostic.contains("unsupported schema version")));
    }

    #[tokio::test]
    async fn unsupported_prompt_schema_cannot_be_saved_or_renamed() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());
        let (id, prompt) = svc
            .create_prompt("Future", "Instructions", vec![], None)
            .await
            .unwrap();

        let future_payload = serde_json::json!({
            "display_name": "Future",
            "normalized_name": "future",
            "instructions": "Instructions",
            "future_field": "preserve-me"
        });
        sqlx::query("UPDATE prompts SET schema_version = 999, payload = ? WHERE id = ?")
            .bind(future_payload.to_string())
            .bind(&id)
            .execute(store.pool())
            .await
            .unwrap();

        assert!(matches!(
            svc.save_prompt(&id, &prompt).await,
            Err(ManagementError::Validation(_))
        ));
        assert!(matches!(
            svc.rename_prompt(&id, "Renamed").await,
            Err(ManagementError::Validation(_))
        ));

        let row = sqlx::query("SELECT schema_version, payload FROM prompts WHERE id = ?")
            .bind(&id)
            .fetch_one(store.pool())
            .await
            .unwrap();
        use sqlx::Row;
        assert_eq!(row.get::<i64, _>("schema_version"), 999);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&row.get::<String, _>("payload")).unwrap(),
            future_payload
        );
    }

    #[tokio::test]
    async fn save_prompt_does_not_recreate_deleted_prompt() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let (id, _) = svc
            .create_prompt("Test", "Instructions", vec![], None)
            .await
            .unwrap();

        svc.delete_prompt(&id).await.unwrap();

        let prompt = StoredPrompt {
            display_name: "Updated".to_string(),
            normalized_name: "updated".to_string(),
            instructions: "New".to_string(),
            skills: Vec::new(),
            profile: None,
        };
        let result = svc.save_prompt(&id, &prompt).await;
        assert!(
            matches!(result, Err(ManagementError::Reference(_))),
            "save_prompt on deleted ID should fail, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn migration_rename_does_not_revoke_valid_handle() {
        use crate::config::migrations::apply_migrations;

        let store = ConfigStore::open_in_memory().await.unwrap();

        // Drop the unique index so we can insert colliding data.
        sqlx::query("DROP INDEX IF EXISTS idx_prompts_normalized_name")
            .execute(store.pool())
            .await
            .unwrap();

        // Insert two legacy prompts whose IDs normalize to the same handle.
        for id in &["check-email", "Check-Email"] {
            sqlx::query(
                "INSERT INTO prompts (id, schema_version, payload, display_name, normalized_name, identity_state, created_at, updated_at) \
                 VALUES (?, 1, ?, ?, ?, 'ready', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
            )
            .bind(id)
            .bind(r#"{"instructions":"x"}"#)
            .bind(id)
            .bind(id.to_lowercase())
            .execute(store.pool())
            .await
            .unwrap();
        }

        // First canonicalization: both should be marked needs_rename.
        apply_migrations(store.pool()).await.unwrap();
        for id in &["check-email", "Check-Email"] {
            let row = sqlx::query("SELECT identity_state FROM prompts WHERE id = ?")
                .bind(id)
                .fetch_one(store.pool())
                .await
                .unwrap();
            use sqlx::Row;
            let state: String = row.get("identity_state");
            assert_eq!(
                state, "needs_rename",
                "both collision participants should be needs_rename after first pass"
            );
        }

        // Rename one participant to claim the freed handle.
        let svc = ConfigManagementService::new(store.clone());
        svc.rename_prompt("check-email", "Check Email")
            .await
            .unwrap();

        // Re-run migrations: the renamed prompt should NOT be re-quarantined.
        apply_migrations(store.pool()).await.unwrap();
        let row = sqlx::query("SELECT identity_state, normalized_name FROM prompts WHERE id = ?")
            .bind("check-email")
            .fetch_one(store.pool())
            .await
            .unwrap();
        use sqlx::Row;
        let state: String = row.get("identity_state");
        let handle: String = row.get("normalized_name");
        assert_eq!(
            state, "ready",
            "renamed prompt should remain ready after re-canonicalization"
        );
        assert_eq!(
            handle, "check-email",
            "renamed prompt should keep its valid handle"
        );
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

        let (prompt_id, _) = svc
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

        // The task references the prompt via stored_prompt_id column, which is
        // schema-independent. Deletion is blocked by Conflict, not IntegrityUnknown.
        let result = svc.delete_prompt(&prompt_id).await;
        assert!(matches!(result, Err(ManagementError::Conflict { .. })));
    }

    #[tokio::test]
    async fn delete_automation_task_integrity_check() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        let (prompt_id, _) = svc
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

        let (prompt_id, _) = svc
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

        let (prompt_id, _) = svc
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

        let svc = ConfigManagementService::new(store.clone()).with_scheduler(
            Arc::new(fake),
            SchedulerInstallContext {
                runner_executable: std::path::PathBuf::from("/usr/local/bin/agent-iron"),
                config_store_path: std::path::PathBuf::from("/tmp/config.db"),
            },
        );

        let (prompt_id, _) = svc
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
        assert!(store
            .get_scheduled_task("test-sched")
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn combined_schedule_deletion_reports_desired_state_failure_after_host_removal() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let host = Arc::new(DesiredDeleteFailureHost {
            store: store.clone(),
            removed: std::sync::atomic::AtomicBool::new(false),
        });
        let svc = ConfigManagementService::new(store.clone()).with_scheduler(
            host.clone(),
            SchedulerInstallContext {
                runner_executable: std::path::PathBuf::from("/usr/local/bin/agent-iron"),
                config_store_path: std::path::PathBuf::from("/tmp/config.db"),
            },
        );
        let (prompt_id, _) = svc
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

        let outcome = svc.delete_schedule_combined("test-sched").await.unwrap();
        assert!(outcome.host_removed);
        assert!(!outcome.desired_deleted);
        assert!(host.removed.load(std::sync::atomic::Ordering::SeqCst));
        let drift = outcome.drift.expect("partial deletion should report drift");
        assert!(drift.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == crate::scheduled_task::ScheduleDiagnosticKind::DesiredDeletionFailed
        }));
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM schedule_after_host_removal WHERE id = 'test-sched'",
        )
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(
            count, 1,
            "desired state should remain after deletion failure"
        );
    }

    #[tokio::test]
    async fn automation_task_list_returns_managed_records() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let (prompt_id, _) = svc
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

        let (prompt_id, _) = svc
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

    // ---- F2: Profile validation tests ----

    #[tokio::test]
    async fn profile_name_uniqueness_enforced() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);
        svc.save_profile("prof-a", &AgentProfile::with_name("My Profile"))
            .await
            .unwrap();
        let result = svc
            .save_profile("prof-b", &AgentProfile::with_name("My Profile"))
            .await;
        assert!(matches!(result, Err(ManagementError::Validation(_))));
    }

    #[tokio::test]
    async fn default_profile_deletion_rejected() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);
        let result = svc.delete_profile("default").await;
        assert!(matches!(result, Err(ManagementError::Validation(_))));
    }

    #[tokio::test]
    async fn managed_profile_rejects_empty_provider_slug() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);
        let profile = AgentProfile {
            name: "Test".to_string(),
            provider: AgentProfileProvider::Managed {
                provider_slug: crate::provider_credential::domain::ProviderSlug::from(""),
                model: "gpt-4".to_string(),
            },
            ..AgentProfile::with_name("Test")
        };
        let result = svc.save_profile("prof-a", &profile).await;
        assert!(matches!(result, Err(ManagementError::Validation(_))));
    }

    // ---- F1: Skill availability tests ----

    #[tokio::test]
    async fn skill_unavailable_rejected_on_write() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store)
            .with_skill_inventory(HashSet::from(["email".to_string()]));
        let result = svc
            .create_prompt(
                "Test",
                "Instructions",
                vec!["email".to_string(), "missing-skill".to_string()],
                None,
            )
            .await;
        assert!(matches!(result, Err(ManagementError::Validation(_))));
    }

    #[tokio::test]
    async fn skill_available_accepted_on_write() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store)
            .with_skill_inventory(HashSet::from(["email".to_string()]));
        let result = svc
            .create_prompt("Test", "Instructions", vec!["email".to_string()], None)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn unavailable_skill_reported_on_read() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());
        let (id, _) = svc
            .create_prompt("Test", "Instructions", vec!["skill-a".to_string()], None)
            .await
            .unwrap();

        // Attach an inventory that doesn't include skill-a.
        let svc2 =
            ConfigManagementService::new(store).with_skill_inventory(HashSet::<String>::new());
        let record = svc2.get_prompt(&id).await.unwrap().unwrap();
        match record {
            ManagedPromptRecord::NeedsAttention { diagnostics, .. } => {
                assert!(diagnostics
                    .iter()
                    .any(|d| { d.category == DiagnosticCategory::UnavailableSkill }));
            }
            _ => panic!("Expected NeedsAttention for unavailable skill"),
        }
    }

    // ---- F3: Dependency impact credential edge tests ----

    #[tokio::test]
    async fn profile_impact_includes_credential_dependency() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);
        let profile = AgentProfile {
            name: "Managed".to_string(),
            provider: AgentProfileProvider::Managed {
                provider_slug: crate::provider_credential::domain::ProviderSlug::from(
                    "test-provider",
                ),
                model: "test-model".to_string(),
            },
            ..AgentProfile::with_name("Managed")
        };
        svc.save_profile("managed-prof", &profile).await.unwrap();

        let report = svc.profile_impact("managed-prof").await.unwrap();
        assert!(report.links.iter().any(|l| {
            l.direction == DependencyDirection::Depends
                && l.proximity == DependencyProximity::Direct
                && matches!(
                    &l.entity,
                    DependencyEntity::ProviderCredential { slug } if slug == "test-provider"
                )
        }));
    }

    #[tokio::test]
    async fn task_impact_includes_credential_dependency() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);
        let profile = AgentProfile {
            name: "Managed".to_string(),
            provider: AgentProfileProvider::Managed {
                provider_slug: crate::provider_credential::domain::ProviderSlug::from(
                    "test-provider",
                ),
                model: "test-model".to_string(),
            },
            ..AgentProfile::with_name("Managed")
        };
        svc.save_profile("managed-prof", &profile).await.unwrap();

        let (prompt_id, _) = svc
            .create_prompt(
                "Test",
                "Instructions",
                vec![],
                Some(AgentProfileId::from("managed-prof")),
            )
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

        let report = svc.task_impact("task-1").await.unwrap();
        assert!(report.links.iter().any(|l| {
            l.direction == DependencyDirection::Depends
                && matches!(
                    &l.entity,
                    DependencyEntity::ProviderCredential { slug } if slug == "test-provider"
                )
        }));
    }

    // ---- F7: Automation-task handle lookup ----

    #[tokio::test]
    async fn automation_task_handle_lookup() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);
        let (prompt_id, _) = svc
            .create_prompt("Test", "Instructions", vec![], None)
            .await
            .unwrap();

        svc.save_automation_task(&AutomationTaskInput {
            id: "task-1".to_string(),
            display_name: "My Task".to_string(),
            stored_prompt_id: prompt_id,
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

        let result = svc.get_automation_task_by_handle("MY TASK").await.unwrap();
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            ManagedAutomationTaskRecord::Ready(task) if task.id == "task-1"
        ));
    }

    // ---- F5: v2 invalid prompt decode ----

    #[tokio::test]
    async fn v2_prompt_with_invalid_payload_reported_as_needs_attention() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        // Insert a v2 prompt with a mismatched normalized_name directly,
        // bypassing store validation, so we can test the decode path.
        let payload = serde_json::json!({
            "display_name": "Check Email",
            "normalized_name": "wrong-handle",
            "instructions": "do thing",
            "skills": [],
            "profile": null,
        });
        let payload_str = serde_json::to_string(&payload).unwrap();
        sqlx::query(
            "INSERT INTO prompts (id, schema_version, payload, display_name, normalized_name, identity_state, created_at, updated_at) VALUES ('bad-prompt', 2, ?, 'Check Email', 'wrong-handle', 'ready', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind(&payload_str)
        .execute(store.pool())
        .await
        .unwrap();

        let record = svc.get_prompt("bad-prompt").await.unwrap().unwrap();
        assert!(matches!(record, ManagedPromptRecord::NeedsAttention { .. }));
    }

    // ---- F5: Automation-task list tolerates malformed records ----

    #[tokio::test]
    async fn automation_task_list_tolerates_unsupported_schema() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());
        let (prompt_id, _) = svc
            .create_prompt("Test", "Instructions", vec![], None)
            .await
            .unwrap();

        // Save a valid task.
        svc.save_automation_task(&AutomationTaskInput {
            id: "valid-task".to_string(),
            display_name: "Valid".to_string(),
            stored_prompt_id: prompt_id.clone(),
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

        // Insert a task with an unsupported schema version directly.
        sqlx::query(
            "INSERT INTO automation_tasks (id, name, normalized_name, stored_prompt_id, expected_outcome, project_root, timeout_seconds, schema_version, created_at, updated_at) VALUES ('bad-task', 'Bad', 'bad', ?, 'x', '/tmp', 0, 99, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind(&prompt_id)
        .execute(store.pool())
        .await
        .unwrap();

        let list = svc.list_automation_tasks().await.unwrap();
        let has_ready = list
            .iter()
            .any(|r| matches!(r, ManagedAutomationTaskRecord::Ready(t) if t.id == "valid-task"));
        let has_needs_attention = list.iter().any(|r| {
            matches!(r, ManagedAutomationTaskRecord::NeedsAttention { id, .. } if id == "bad-task")
        });
        assert!(has_ready, "valid task should be Ready");
        assert!(has_needs_attention, "bad task should be NeedsAttention");
    }

    #[tokio::test]
    async fn malformed_skill_identifier_with_whitespace_rejected() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let result = svc
            .create_prompt("Test", "instructions", vec!["bad skill".to_string()], None)
            .await;
        assert!(matches!(result, Err(ManagementError::Validation(_))));
    }

    #[tokio::test]
    async fn automation_task_missing_prompt_returns_needs_attention() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        // Insert a dummy prompt so the FK is satisfied, then remove it.
        sqlx::query(
            "INSERT INTO prompts (id, schema_version, payload, display_name, normalized_name, identity_state, created_at, updated_at) VALUES (?, 2, ?, 'Test', 'test', 'ready', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind("nonexistent-prompt")
        .bind(r#"{"instructions":"x"}"#)
        .execute(store.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO automation_tasks (id, name, normalized_name, stored_prompt_id, expected_outcome, project_root, timeout_seconds, schema_version, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("task-1")
        .bind("Task 1")
        .bind("task-1")
        .bind("nonexistent-prompt")
        .bind("Done")
        .bind(std::env::temp_dir().to_string_lossy().as_ref())
        .bind(300_i64)
        .bind(1_i64)
        .bind("2024-01-01T00:00:00Z")
        .bind("2024-01-01T00:00:00Z")
        .execute(store.pool())
        .await
        .unwrap();

        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(store.pool())
            .await
            .unwrap();
        sqlx::query("DELETE FROM prompts WHERE id = ?")
            .bind("nonexistent-prompt")
            .execute(store.pool())
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(store.pool())
            .await
            .unwrap();

        let result = svc.get_automation_task("task-1").await.unwrap();
        assert!(
            matches!(
                result,
                Some(ManagedAutomationTaskRecord::NeedsAttention { .. })
            ),
            "task referencing missing prompt should be NeedsAttention"
        );
    }

    #[tokio::test]
    async fn schedule_missing_task_returns_needs_attention() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        // Insert a schedule referencing a missing automation task directly.
        sqlx::query(
            "INSERT INTO schedule (id, schema_version, payload, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind("sched-1")
        .bind(1_i64)
        .bind(r#"{"automation_task_id":"nonexistent-task","cron_expression":"0 9 * * *","enabled":true}"#)
        .bind("2024-01-01T00:00:00Z")
        .bind("2024-01-01T00:00:00Z")
        .execute(store.pool())
        .await
        .unwrap();

        let result = svc.get_scheduled_task("sched-1").await.unwrap();
        assert!(
            matches!(
                result,
                Some(ManagedScheduledTaskRecord::NeedsAttention { .. })
            ),
            "schedule referencing missing task should be NeedsAttention"
        );
    }

    #[tokio::test]
    async fn schedule_invalid_cron_returns_needs_attention() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        let (prompt_id, _) = svc
            .create_prompt("Test", "instructions", vec![], None)
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

        // Insert schedule with malformed cron directly.
        sqlx::query(
            "INSERT INTO schedule (id, schema_version, payload, created_at, updated_at) VALUES (?, 1, ?, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind("bad-sched")
        .bind(r#"{"automation_task_id":"task-1","cron_expression":"not a cron","enabled":true}"#)
        .execute(store.pool())
        .await
        .unwrap();

        let result = svc.get_scheduled_task("bad-sched").await.unwrap();
        assert!(
            matches!(
                result,
                Some(ManagedScheduledTaskRecord::NeedsAttention { .. })
            ),
            "schedule with invalid cron should be NeedsAttention"
        );
    }

    #[tokio::test]
    async fn profile_deletion_blocked_by_malformed_prompt_payload() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        let profile = AgentProfile::with_name("Test Profile");
        svc.save_profile("test-prof", &profile).await.unwrap();

        // Insert a malformed prompt that claims to reference the profile.
        sqlx::query(
            "INSERT INTO prompts (id, schema_version, payload, display_name, normalized_name, identity_state, created_at, updated_at) VALUES (?, 2, ?, '', '', 'ready', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind("bad-prompt")
        .bind(r#"{"profile": 123}"#)
        .execute(store.pool())
        .await
        .unwrap();

        let result = svc.delete_profile("test-prof").await;
        assert!(
            matches!(result, Err(ManagementError::IntegrityUnknown { .. })),
            "malformed prompt should yield IntegrityUnknown"
        );
    }

    #[tokio::test]
    async fn dependency_impact_reports_malformed_profile() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        // Insert a malformed profile with a provider slug.
        sqlx::query(
            "INSERT INTO profiles (id, schema_version, payload, created_at, updated_at) VALUES (?, 2, ?, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind("bad-prof")
        .bind(r#"{"name":"Bad","provider":{"provider_slug":"test-provider","model":"m"},"approval":"PerTool"}"#)
        .execute(store.pool())
        .await
        .unwrap();

        // Corrupt the payload so it no longer decodes.
        sqlx::query("UPDATE profiles SET payload = ? WHERE id = ?")
            .bind(r#"{"name":"Bad""#)
            .bind("bad-prof")
            .execute(store.pool())
            .await
            .unwrap();

        let report = svc.credential_impact("test-provider").await.unwrap();
        assert!(
            report.diagnostics.iter().any(|d| d.contains("bad-prof")),
            "credential impact should report malformed profile"
        );
    }

    #[tokio::test]
    async fn legacy_v1_prompt_identity_derived_from_record_id() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        // Insert a v1 prompt with a kebab-case ID.
        sqlx::query(
            "INSERT INTO prompts (id, schema_version, payload, display_name, normalized_name, identity_state, created_at, updated_at) VALUES (?, 1, ?, '', '', 'ready', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind("check-email")
        .bind(r#"{"instructions":"Check inbox"}"#)
        .execute(store.pool())
        .await
        .unwrap();

        let result = svc.get_prompt("check-email").await.unwrap();
        if let Some(ManagedPromptRecord::Ready((_, entry))) = result {
            assert_eq!(entry.prompt.display_name, "Check Email");
            assert_eq!(entry.prompt.normalized_name, "check-email");
        } else {
            panic!("Expected Ready legacy v1 prompt");
        }
    }

    #[tokio::test]
    async fn config_error_reference_maps_to_management_reference() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);

        let result = svc
            .save_scheduled_task(&ScheduledTaskInput {
                id: "sched-1".to_string(),
                automation_task_id: "missing-task".to_string(),
                cron_expression: "0 9 * * *".to_string(),
                enabled: true,
            })
            .await;
        assert!(
            matches!(result, Err(ManagementError::Reference(_))),
            "missing automation task should yield Reference error, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn profile_registry_name_conflict_prevents_save() {
        use std::collections::HashMap;

        let store = ConfigStore::open_in_memory().await.unwrap();
        let registry = Arc::new(RwLock::new(HashMap::new()));
        let svc =
            ConfigManagementService::new(store.clone()).with_profile_registry(registry.clone());

        // Seed the registry with a profile that has the same name but different ID.
        registry.write().insert(
            AgentProfileId::from("existing-id"),
            AgentProfile::with_name("Shared Name"),
        );

        let result = svc
            .save_profile("new-id", &AgentProfile::with_name("Shared Name"))
            .await;
        assert!(
            matches!(result, Err(ManagementError::Conflict { .. })),
            "registry name conflict should yield Conflict"
        );
    }

    // ---- F9: create_prompt returns persisted entry ----

    #[tokio::test]
    async fn create_prompt_returns_persisted_entry() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);
        let (id, prompt) = svc
            .create_prompt("Check Email", "Check inbox", vec![], None)
            .await
            .unwrap();
        assert!(id.starts_with("prompt-"));
        assert_eq!(prompt.display_name, "Check Email");
        assert_eq!(prompt.normalized_name, "check-email");
        assert_eq!(prompt.instructions, "Check inbox");
    }

    #[tokio::test]
    async fn duplicate_skills_rejected() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);
        let result = svc
            .create_prompt(
                "Test",
                "instructions",
                vec!["skill-a".to_string(), "Skill-A".to_string()],
                None,
            )
            .await;
        assert!(
            matches!(result, Err(ManagementError::Validation(ref msg)) if msg.contains("duplicate")),
            "duplicate skills should be rejected, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn leading_trailing_whitespace_skill_rejected() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store);
        let result = svc
            .create_prompt("Test", "instructions", vec![" skill-a ".to_string()], None)
            .await;
        assert!(
            matches!(result, Err(ManagementError::Validation(ref msg)) if msg.contains("whitespace")),
            "leading/trailing whitespace skill should be rejected, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn malformed_schedule_missing_enabled_returns_needs_attention() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        let (prompt_id, _) = svc
            .create_prompt("Test", "instructions", vec![], None)
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

        sqlx::query(
            "INSERT INTO schedule (id, schema_version, payload, created_at, updated_at) VALUES (?, 1, ?, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind("bad-sched")
        .bind(r#"{"automation_task_id":"task-1","cron_expression":"0 9 * * *"}"#)
        .execute(store.pool())
        .await
        .unwrap();

        let result = svc.get_scheduled_task("bad-sched").await.unwrap();
        assert!(
            matches!(
                result,
                Some(ManagedScheduledTaskRecord::NeedsAttention { .. })
            ),
            "schedule missing 'enabled' should be NeedsAttention"
        );
    }

    #[tokio::test]
    async fn malformed_schedule_blocks_task_deletion() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        let (prompt_id, _) = svc
            .create_prompt("Test", "instructions", vec![], None)
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

        sqlx::query(
            "INSERT INTO schedule (id, schema_version, payload, created_at, updated_at) VALUES (?, 1, ?, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind("bad-sched")
        .bind(r#"{"foo":"bar"}"#)
        .execute(store.pool())
        .await
        .unwrap();

        let result = svc.delete_automation_task("task-1").await;
        assert!(
            matches!(result, Err(ManagementError::IntegrityUnknown { .. })),
            "malformed schedule should block task deletion with IntegrityUnknown"
        );
    }

    #[tokio::test]
    async fn credential_impact_tolerates_malformed_prompt_reference() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        let (prompt_id, _) = svc
            .create_prompt("Test", "instructions", vec![], None)
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

        // Insert a schedule with malformed JSON payload.
        sqlx::query(
            "INSERT INTO schedule (id, schema_version, payload, created_at, updated_at) VALUES (?, 1, ?, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind("malformed-sched")
        .bind(r#"not valid json"#)
        .execute(store.pool())
        .await
        .unwrap();

        // Credential impact should not fail; it should return a report.
        let report = svc.credential_impact("test-provider").await.unwrap();
        assert_eq!(
            report.target,
            DependencyEntity::ProviderCredential {
                slug: "test-provider".to_string()
            }
        );
    }

    #[tokio::test]
    async fn legacy_prompt_with_empty_derived_handle_returns_needs_attention() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        // Insert a v1 prompt whose ID produces an empty normalized handle.
        sqlx::query(
            "INSERT INTO prompts (id, schema_version, payload, display_name, normalized_name, identity_state, created_at, updated_at) VALUES (?, 1, ?, '', '', 'ready', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind("!!!")
        .bind(r#"{"instructions":"Check inbox"}"#)
        .execute(store.pool())
        .await
        .unwrap();

        let result = svc.get_prompt("!!!").await.unwrap();
        assert!(
            matches!(result, Some(ManagedPromptRecord::NeedsAttention { .. })),
            "legacy prompt with empty derived handle should be NeedsAttention"
        );
    }

    #[tokio::test]
    async fn handle_lookup_uses_record_id_on_deserialization_error() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        // Insert a prompt with a valid handle but malformed JSON payload.
        sqlx::query(
            "INSERT INTO prompts (id, schema_version, payload, display_name, normalized_name, identity_state, created_at, updated_at) VALUES (?, 2, ?, 'Test', 'test', 'ready', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .bind("stable-prompt-id")
        .bind(r#"{"display_name":"Test","normalized_name":"test","instructions":"x""#)
        .execute(store.pool())
        .await
        .unwrap();

        let result = svc.get_prompt_by_handle("test").await.unwrap();
        match result {
            Some(ManagedPromptRecord::NeedsAttention { id, .. }) => {
                assert_eq!(
                    id, "stable-prompt-id",
                    "NeedsAttention should use the record ID, not the handle"
                );
            }
            other => panic!("Expected NeedsAttention, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn automation_task_error_categorized_correctly() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let svc = ConfigManagementService::new(store.clone());

        let (prompt_id, _) = svc
            .create_prompt("Test", "instructions", vec![], None)
            .await
            .unwrap();

        // Insert a task with unsupported schema version.
        sqlx::query(
            "INSERT INTO automation_tasks (id, name, normalized_name, stored_prompt_id, expected_outcome, project_root, timeout_seconds, schema_version, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("bad-task")
        .bind("Bad Task")
        .bind("bad-task")
        .bind(&prompt_id)
        .bind("Done")
        .bind(std::env::temp_dir().to_string_lossy().as_ref())
        .bind(300_i64)
        .bind(99_i64)
        .bind("2024-01-01T00:00:00Z")
        .bind("2024-01-01T00:00:00Z")
        .execute(store.pool())
        .await
        .unwrap();

        let result = svc.get_automation_task("bad-task").await.unwrap();
        match result {
            Some(ManagedAutomationTaskRecord::NeedsAttention { diagnostics, .. }) => {
                assert!(
                    diagnostics
                        .iter()
                        .any(|d| { d.category == DiagnosticCategory::UnsupportedSchemaVersion }),
                    "unsupported schema should be categorized as UnsupportedSchemaVersion"
                );
            }
            other => panic!("Expected NeedsAttention, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn migration_collision_assigns_repair_handles() {
        use crate::config::migrations::apply_migrations;

        let store = ConfigStore::open_in_memory().await.unwrap();

        // Drop the unique index so we can insert colliding data.
        sqlx::query("DROP INDEX IF EXISTS idx_prompts_normalized_name")
            .execute(store.pool())
            .await
            .unwrap();

        // Insert two prompts whose IDs normalize to the same handle.
        // "Check-Email" and "check-email" both produce normalized_name "check-email"
        // when the SQL backfill runs: lower(trim(id)).
        for id in &["Check-Email", "check-email"] {
            sqlx::query(
                "INSERT INTO prompts (id, schema_version, payload, display_name, normalized_name, identity_state, created_at, updated_at) \
                 VALUES (?, 2, ?, ?, ?, 'ready', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
            )
            .bind(id)
            .bind(r#"{"display_name":"Check Email","normalized_name":"check-email","instructions":"x"}"#)
            .bind("Check Email")
            .bind("check-email")
            .execute(store.pool())
            .await
            .unwrap();
        }

        // apply_migrations runs canonicalization and recreates the unique index.
        // This should succeed because canonicalization assigns repair handles.
        apply_migrations(store.pool()).await.unwrap();

        // Verify both records are retrievable and marked needs_rename.
        let result1 = check_prompt_needs_rename(&store, "Check-Email").await;
        let result2 = check_prompt_needs_rename(&store, "check-email").await;
        assert!(result1.is_some(), "first colliding record should exist");
        assert!(result2.is_some(), "second colliding record should exist");
    }

    #[tokio::test]
    async fn unicode_normalization_removes_non_ascii() {
        use crate::stored_prompt::normalize_prompt_name;
        // Kelvin sign (K, U+212A) lowercases to ASCII 'k' via to_lowercase.
        // After the fix, it should be removed, not converted to 'k'.
        let normalized = normalize_prompt_name("Kelvin");
        assert!(
            !normalized.contains('k'),
            "non-ASCII character should be removed, not lowercased to ASCII; got '{}'",
            normalized
        );
    }

    async fn check_prompt_needs_rename(store: &ConfigStore, id: &str) -> Option<()> {
        let row = sqlx::query("SELECT identity_state FROM prompts WHERE id = ?")
            .bind(id)
            .fetch_optional(store.pool())
            .await
            .ok()??;
        use sqlx::Row;
        let state: String = row.get("identity_state");
        assert_eq!(
            state, "needs_rename",
            "colliding record should be needs_rename"
        );
        Some(())
    }
}
