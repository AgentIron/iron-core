//! Agent identity profile types and registry helpers.
//!
//! Profiles represent durable agent identities: provider/model selection, tool
//! and skill boundaries, approval posture, and a profile-specific identity
//! prompt layer. They intentionally do not store credential secret material;
//! managed provider credentials are resolved from `iron-core`'s credential
//! state at execution time.

use crate::provider_credential::domain::{ProviderPromptContext, ProviderSlug};
use iron_providers::Provider;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Schema version for typed `AgentProfile` payloads stored in `ConfigStore`.
pub const PROFILE_SCHEMA_VERSION: i64 = 1;

/// Stable identifier for an agent profile.
///
/// For durable profiles, the `ConfigStore` profile record ID is the stable
/// profile ID. The user-facing display name lives inside [`AgentProfile::name`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentProfileId(pub String);

impl AgentProfileId {
    /// Borrow the underlying ID string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for AgentProfileId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for AgentProfileId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// A user-facing agent profile entry pairing a stable ID with its profile.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentProfileEntry {
    /// Stable profile identifier.
    pub id: AgentProfileId,
    /// Profile value, including the user-facing name.
    pub profile: AgentProfile,
}

/// Provider selection context stored in an agent profile.
///
/// Profiles intentionally do not include API keys or other credential secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentProfileProvider {
    /// Use the runtime's injected/default provider path.
    RuntimeDefault,
    /// Use a managed provider resolved from `iron-core` credential state.
    Managed {
        /// Provider slug used for registry and credential lookup.
        provider_slug: ProviderSlug,
        /// Model identifier passed to the provider.
        model: String,
    },
}

/// Tool availability policy for a profile.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ToolFilter {
    /// Inherit the runtime's available tool set.
    #[default]
    Inherit,
    /// Only allow tools with these names.
    Allow(Vec<String>),
    /// Deny tools with these names.
    Deny(Vec<String>),
}

/// Skill availability policy for a profile.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SkillFilter {
    /// No skills are available.
    None,
    /// Only allow skills with these names.
    Allow(Vec<String>),
    /// Inherit the runtime's available skill catalog.
    #[default]
    Inherit,
}

/// Approval posture for a profile.
///
/// Only `PerTool` and `AutoApprove` are valid user-facing values.
/// `ReadOnly` and `RequireApproval` are rejected during deserialization
/// and profile validation/registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "AgentApprovalRaw")]
pub enum AgentApproval {
    /// Require approval for each tool call.
    #[default]
    PerTool,
    /// Auto-approve all tool calls for this profile.
    AutoApprove,
}

/// Raw deserialization helper for `AgentApproval`.
/// Used to reject `ReadOnly` and `RequireApproval` at the deserialization boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum AgentApprovalRaw {
    PerTool,
    AutoApprove,
    ReadOnly,
    RequireApproval,
}

impl TryFrom<AgentApprovalRaw> for AgentApproval {
    type Error = String;

    fn try_from(value: AgentApprovalRaw) -> Result<Self, Self::Error> {
        match value {
            AgentApprovalRaw::PerTool => Ok(AgentApproval::PerTool),
            AgentApprovalRaw::AutoApprove => Ok(AgentApproval::AutoApprove),
            AgentApprovalRaw::ReadOnly => {
                Err("ReadOnly is not a valid user-facing profile approval value".to_string())
            }
            AgentApprovalRaw::RequireApproval => Err(
                "RequireApproval is not a valid user-facing profile approval value; use PerTool"
                    .to_string(),
            ),
        }
    }
}
///
/// `name` is a user-facing unique label. The stable identity handle is the
/// profile ID supplied at registration or loaded from `ConfigStore`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentProfile {
    /// User-facing profile name. Must be unique within the registered set.
    pub name: String,
    /// Provider/model selection for this profile.
    #[serde(flatten)]
    pub provider: AgentProfileProvider,
    /// Tool availability policy.
    #[serde(default)]
    pub tools: ToolFilter,
    /// Skill availability policy.
    #[serde(default)]
    pub skills: SkillFilter,
    /// Approval posture.
    #[serde(default)]
    pub approval: AgentApproval,
    /// Optional profile-specific model-facing identity instructions.
    ///
    /// A non-blank value replaces the default identity prompt. A blank or
    /// absent value falls back to the built-in default profile's identity
    /// prompt during execution preparation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_prompt: Option<String>,
}

impl AgentProfile {
    /// Create a profile with the given name using the runtime default provider.
    pub fn with_name<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            provider: AgentProfileProvider::RuntimeDefault,
            tools: ToolFilter::default(),
            skills: SkillFilter::default(),
            approval: AgentApproval::default(),
            identity_prompt: None,
        }
    }

    /// Return the effective identity prompt for this profile.
    ///
    /// If the profile has a non-blank custom identity prompt, that is returned.
    /// Otherwise the built-in default identity prompt is returned.
    pub fn effective_identity_prompt(&self) -> &str {
        match self.identity_prompt {
            Some(ref prompt) if !prompt.trim().is_empty() => prompt,
            _ => default_identity_prompt(),
        }
    }
}

// ============================================================================
// Shipped default profile definitions
// ============================================================================

/// Core-owned shipped default profile IDs.
pub const SHIPPED_PROFILE_IDS: &[&str] = &["explore", "plan", "apply"];

/// Build the shipped default `AgentProfile` definitions.
///
/// These are bootstrap templates for ordinary persisted profile records.
/// They use `RuntimeDefault`, `ToolFilter::Inherit`, `SkillFilter::Inherit`,
/// and `PerTool` approval. Each carries a profile-specific identity prompt.
pub fn shipped_default_profiles() -> Vec<(AgentProfileId, AgentProfile)> {
    vec![
        (
            AgentProfileId::from("explore"),
            AgentProfile {
                name: "Explore".to_string(),
                provider: AgentProfileProvider::RuntimeDefault,
                tools: ToolFilter::Inherit,
                skills: SkillFilter::Inherit,
                approval: AgentApproval::PerTool,
                identity_prompt: Some(
                    "You are an exploratory research agent. Your goal is to broadly investigate \
                     topics, gather information, and surface options without committing to a specific \
                     implementation. You should ask clarifying questions, consider alternatives, and \
                     summarize findings."
                        .to_string(),
                ),
            },
        ),
        (
            AgentProfileId::from("plan"),
            AgentProfile {
                name: "Plan".to_string(),
                provider: AgentProfileProvider::RuntimeDefault,
                tools: ToolFilter::Inherit,
                skills: SkillFilter::Inherit,
                approval: AgentApproval::PerTool,
                identity_prompt: Some(
                    "You are a planning agent. Your goal is to analyze requirements, break work \
                     into actionable steps, and produce structured plans. You should identify \
                     dependencies, estimate effort, and propose milestones before any implementation \
                     begins."
                        .to_string(),
                ),
            },
        ),
        (
            AgentProfileId::from("apply"),
            AgentProfile {
                name: "Apply".to_string(),
                provider: AgentProfileProvider::RuntimeDefault,
                tools: ToolFilter::Inherit,
                skills: SkillFilter::Inherit,
                approval: AgentApproval::PerTool,
                identity_prompt: Some(
                    "You are an implementation agent. Your goal is to execute plans, write code, \
                     run tests, and deliver working solutions. You should focus on correctness, \
                     test coverage, and incremental progress toward the stated goal."
                        .to_string(),
                ),
            },
        ),
    ]
}

/// Built-in default identity prompt for profiles without a custom prompt.
pub fn default_identity_prompt() -> &'static str {
    "You are a helpful software engineering agent."
}

/// A resolved provider ready for execution, preserving ownership semantics.
pub enum ResolvedProfileProvider {
    /// The runtime's injected/default provider.
    RuntimeDefault(Arc<dyn Provider>),
    /// An owned managed provider constructed through credential resolution.
    Managed(Box<dyn Provider>),
    /// Fallback to the runtime default provider because the explicit
    /// provider/model reference was unavailable or credential resolution
    /// failed. The diagnostic explains why the fallback occurred.
    Fallback {
        provider: Arc<dyn Provider>,
        diagnostic: String,
    },
}

impl std::fmt::Debug for ResolvedProfileProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolvedProfileProvider::RuntimeDefault(_) => f.debug_tuple("RuntimeDefault").finish(),
            ResolvedProfileProvider::Managed(_) => f.debug_tuple("Managed").finish(),
            ResolvedProfileProvider::Fallback { diagnostic, .. } => f
                .debug_struct("Fallback")
                .field("diagnostic", diagnostic)
                .finish(),
        }
    }
}

impl ResolvedProfileProvider {
    /// Borrow the provider as a trait object.
    pub fn as_provider(&self) -> &dyn Provider {
        match self {
            ResolvedProfileProvider::RuntimeDefault(arc) => arc.as_ref(),
            ResolvedProfileProvider::Managed(boxed) => boxed.as_ref(),
            ResolvedProfileProvider::Fallback { provider, .. } => provider.as_ref(),
        }
    }

    /// Return the fallback diagnostic, if any.
    pub fn fallback_diagnostic(&self) -> Option<&str> {
        match self {
            ResolvedProfileProvider::Fallback { diagnostic, .. } => Some(diagnostic.as_str()),
            _ => None,
        }
    }
}

/// Issue category reported for a skipped profile during `load_profiles`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileLoadIssue {
    /// The stored profile schema version is not supported.
    UnsupportedSchemaVersion { version: i64 },
    /// The stored payload could not be decoded as an `AgentProfile`.
    InvalidPayload,
    /// The profile ID is invalid or reserved.
    InvalidProfileId,
    /// The profile name is empty, contains control characters, or is reserved.
    InvalidName,
    /// The profile ID or name equals the reserved `default` identifier.
    ReservedDefault,
    /// Another registered profile already uses the trimmed name.
    DuplicateName,
    /// The listed record was missing when read.
    MissingRecord,
    /// The profile approval value `ReadOnly` was rejected.
    ReadOnlyRejected,
}

/// Per-profile diagnostic returned by best-effort profile loading.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileLoadDiagnostic {
    /// Stable profile ID from the store listing.
    pub profile_id: AgentProfileId,
    /// Parsed profile name, if available from the payload.
    pub name: Option<String>,
    /// Issue category describing why the profile was skipped.
    pub issue: ProfileLoadIssue,
}

/// Result of loading typed profiles from `ConfigStore`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileLoadReport {
    /// Profiles that were successfully loaded/merged into the registry.
    pub loaded: Vec<AgentProfileEntry>,
    /// Profiles that were skipped, with per-profile diagnostics.
    pub diagnostics: Vec<ProfileLoadDiagnostic>,
}

impl ProfileLoadReport {
    /// Create an empty load report.
    pub fn empty() -> Self {
        Self {
            loaded: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

/// Validate a non-default profile ID.
///
/// Returns `true` for non-empty strings that contain no control characters and
/// are not ASCII-case-insensitive equal to `"default"`.
pub fn is_valid_profile_id(id: &str) -> bool {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.as_bytes().iter().any(|b| b.is_ascii_control()) {
        return false;
    }
    !trimmed.eq_ignore_ascii_case("default")
}

/// Validate and normalize a profile name.
///
/// Returns the trimmed name if it is non-empty, contains no control characters,
/// and is not the reserved `"default"` identifier. Returns `None` otherwise.
pub fn normalize_profile_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.as_bytes().iter().any(|b| b.is_ascii_control()) {
        return None;
    }
    if trimmed.eq_ignore_ascii_case("default") {
        return None;
    }
    Some(trimmed.to_string())
}

/// Opt-in policy governing how many valid persisted profiles must remain after
/// a profile deletion.
///
/// The unrestricted default ([`ProfileDeletePolicy::AllowZero`]) preserves the
/// original `delete_profile` semantics, which permit zero persisted profiles.
/// [`ProfileDeletePolicy::RequireMinimumValid`] requests an atomic check that
/// at least `minimum` valid persisted profiles remain after the deletion
/// commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileDeletePolicy {
    /// Permit zero valid persisted profiles after deletion (unrestricted).
    AllowZero,
    /// Require at least `minimum` valid persisted profiles to remain.
    RequireMinimumValid(usize),
}

impl ProfileDeletePolicy {
    /// The minimum valid-profile count this policy requires to remain, or zero
    /// for [`ProfileDeletePolicy::AllowZero`].
    pub fn minimum(self) -> usize {
        match self {
            ProfileDeletePolicy::AllowZero => 0,
            ProfileDeletePolicy::RequireMinimumValid(minimum) => minimum,
        }
    }
}

/// Why a raw persisted profile record failed record-local validity
/// classification.
///
/// This is the single source of truth used by both management reads (to build
/// per-record diagnostics) and policy-aware deletion counting (to decide
/// whether a row counts as a valid profile). A record counts as valid only when
/// [`classify_profile_record`] returns [`Ok`].
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileRecordError {
    /// The record's schema version is not the supported profile schema version.
    UnsupportedSchemaVersion {
        /// The unsupported schema version stored alongside the payload.
        actual: i64,
    },
    /// The payload decoded far enough to reveal an approval value that is not a
    /// valid user-facing managed profile approval policy.
    ReadOnlyRejected {
        /// The rejected approval value found in the payload.
        approval: String,
    },
    /// The payload could not be decoded as an [`AgentProfile`] for any other
    /// reason.
    DecodeFailure {
        /// The underlying deserialization error message.
        error: String,
    },
    /// The payload decoded but one or more structural fields failed validation.
    InvalidFields {
        /// One human-readable message per failed field check.
        messages: Vec<String>,
    },
}

/// Classify a raw persisted profile record using the same rules that decide
/// whether management listing returns a ready profile record.
///
/// A record is valid only when its schema version equals
/// [`PROFILE_SCHEMA_VERSION`], its payload decodes as [`AgentProfile`], and its
/// stable ID and decoded fields pass the structural validation shared with
/// management reads (protected-default identity and supported approval rules).
///
/// This function is pure and allocation-bounded; it performs no I/O and does
/// not consult registry membership, provider credentials, or prompt references.
pub fn classify_profile_record(
    id: &str,
    schema_version: i64,
    payload: &serde_json::Value,
) -> Result<AgentProfile, ProfileRecordError> {
    if schema_version != PROFILE_SCHEMA_VERSION {
        return Err(ProfileRecordError::UnsupportedSchemaVersion {
            actual: schema_version,
        });
    }
    match serde_json::from_value::<AgentProfile>(payload.clone()) {
        Ok(profile) => {
            let messages = validate_profile_fields(id, &profile);
            if messages.is_empty() {
                Ok(profile)
            } else {
                Err(ProfileRecordError::InvalidFields { messages })
            }
        }
        Err(error) => {
            let rejected = payload
                .get("approval")
                .and_then(|value| value.as_str())
                .filter(|approval| matches!(*approval, "ReadOnly" | "RequireApproval"));
            if let Some(approval) = rejected {
                Err(ProfileRecordError::ReadOnlyRejected {
                    approval: approval.to_string(),
                })
            } else {
                Err(ProfileRecordError::DecodeFailure {
                    error: error.to_string(),
                })
            }
        }
    }
}

/// Validate a decoded profile's structural fields, returning one message per
/// failed check. An empty vector means the profile is structurally valid.
pub(crate) fn validate_profile_fields(id: &str, profile: &AgentProfile) -> Vec<String> {
    let mut messages = Vec::new();
    if !is_valid_profile_id(id) {
        messages.push(format!("Profile ID '{}' is invalid or reserved", id));
    }
    if normalize_profile_name(&profile.name).is_none() {
        messages.push(format!(
            "Profile name '{}' is invalid or reserved",
            profile.name
        ));
    }
    if let AgentProfileProvider::Managed {
        provider_slug,
        model,
    } = &profile.provider
    {
        if provider_slug.as_str().trim().is_empty() || model.trim().is_empty() {
            messages.push("Managed profile has empty provider_slug or model".to_string());
        }
    }
    messages
}

/// Build a managed `ProviderPromptContext` from a profile without a profile API key.
pub fn managed_profile_prompt_context(
    provider: &AgentProfileProvider,
) -> Option<ProviderPromptContext> {
    match provider {
        AgentProfileProvider::RuntimeDefault => None,
        AgentProfileProvider::Managed {
            provider_slug,
            model,
        } => Some(ProviderPromptContext {
            provider_slug: provider_slug.clone(),
            model: model.clone(),
            api_key: None,
        }),
    }
}

// ============================================================================
// Default profile seeding
// ============================================================================

/// Domain for bootstrap metadata used by default-profile seeding.
pub const DEFAULT_PROFILE_SEED_DOMAIN: &str = "agent_profiles";
/// Key for the durable seed marker.
pub const DEFAULT_PROFILE_SEED_KEY: &str = "default_seed";
/// Current seed marker version.
pub const DEFAULT_PROFILE_SEED_VERSION: &str = "1";

/// Policy for the default-profile seed operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultProfileSeedPolicy {
    /// Create missing shipped defaults only when the seed marker is absent,
    /// then write the marker.
    FirstRunOnly,
    /// Create missing shipped defaults regardless of marker state, but still
    /// preserve existing records.
    RestoreMissing,
}

/// Diagnostic for a single shipped default during seeding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultProfileSeedDiagnostic {
    /// The profile was created successfully.
    Created(AgentProfileId),
    /// The profile already existed and was skipped.
    SkippedExisting(AgentProfileId),
    /// The profile was missing and not recreated (first-run already done).
    SkippedFirstRunDone(AgentProfileId),
    /// Storage failure for this profile.
    StorageFailure {
        profile_id: AgentProfileId,
        reason: String,
    },
}

/// Result of a default-profile seed operation.
#[derive(Debug, Clone, PartialEq)]
pub struct DefaultProfileSeedReport {
    /// Policy used for this seed operation.
    pub policy: DefaultProfileSeedPolicy,
    /// Whether the durable seed marker was present before the operation.
    pub marker_was_present: bool,
    /// Whether the marker was written during this operation.
    pub marker_written: bool,
    /// Profiles that were created.
    pub created: Vec<AgentProfileId>,
    /// Profiles that were skipped because they already existed.
    pub skipped_existing: Vec<AgentProfileId>,
    /// Diagnostics for each shipped default processed.
    pub diagnostics: Vec<DefaultProfileSeedDiagnostic>,
}

impl DefaultProfileSeedReport {
    /// Create a report for a seed operation that did nothing.
    pub fn no_op(policy: DefaultProfileSeedPolicy, marker_was_present: bool) -> Self {
        Self {
            policy,
            marker_was_present,
            marker_written: false,
            created: Vec::new(),
            skipped_existing: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

/// Seed shipped default profiles into the config store.
///
/// This is a core-owned, non-destructive operation. Existing profiles are never
/// overwritten. Under `FirstRunOnly`, missing defaults are only created when
/// the durable seed marker is absent; the marker is then written so later
/// startups do not recreate user-deleted defaults.
///
/// Returns a structured report describing created, skipped, and diagnostic
/// outcomes for each shipped default.
pub async fn seed_default_profiles(
    store: &crate::config::ConfigStore,
    policy: DefaultProfileSeedPolicy,
) -> Result<DefaultProfileSeedReport, crate::config::ConfigError> {
    use crate::config::records::{BootstrapMetadataInput, ProfileInput};

    let marker = store
        .get_bootstrap_metadata(DEFAULT_PROFILE_SEED_DOMAIN, DEFAULT_PROFILE_SEED_KEY)
        .await?;
    let marker_was_present = marker.is_some();

    // Under FirstRunOnly, if marker is present we do nothing.
    if matches!(policy, DefaultProfileSeedPolicy::FirstRunOnly) && marker_was_present {
        let mut report = DefaultProfileSeedReport::no_op(policy, true);
        for (id, _profile) in shipped_default_profiles() {
            report
                .diagnostics
                .push(DefaultProfileSeedDiagnostic::SkippedFirstRunDone(id));
        }
        return Ok(report);
    }

    let mut report = DefaultProfileSeedReport {
        policy,
        marker_was_present,
        marker_written: false,
        created: Vec::new(),
        skipped_existing: Vec::new(),
        diagnostics: Vec::new(),
    };

    for (id, profile) in shipped_default_profiles() {
        let payload = serde_json::to_value(&profile).map_err(|e| {
            crate::config::ConfigError::Serialization(format!(
                "Failed to serialize shipped default profile {}: {}",
                id.as_str(),
                e
            ))
        })?;
        let input = ProfileInput {
            id: id.as_str().to_string(),
            schema_version: PROFILE_SCHEMA_VERSION,
            payload,
        };
        let inserted = store.insert_profile_if_missing(&input).await?;
        if !inserted {
            report.skipped_existing.push(id.clone());
            report
                .diagnostics
                .push(DefaultProfileSeedDiagnostic::SkippedExisting(id));
            continue;
        }

        report.created.push(id.clone());
        report
            .diagnostics
            .push(DefaultProfileSeedDiagnostic::Created(id));
    }

    // Write the seed marker so that subsequent startups do not recreate
    // user-deleted defaults (under FirstRunOnly policy).
    if matches!(policy, DefaultProfileSeedPolicy::FirstRunOnly) || !report.created.is_empty() {
        let marker_input = BootstrapMetadataInput {
            domain: DEFAULT_PROFILE_SEED_DOMAIN.to_string(),
            key: DEFAULT_PROFILE_SEED_KEY.to_string(),
            value: DEFAULT_PROFILE_SEED_VERSION.to_string(),
        };
        match store.set_bootstrap_metadata(&marker_input).await {
            Ok(()) => report.marker_written = true,
            Err(e) => {
                // Do not report successful seed if marker could not be persisted.
                return Err(crate::config::ConfigError::Migration(format!(
                    "Failed to write default-profile seed marker: {}",
                    e
                )));
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_profile_id_accepts_non_reserved() {
        assert!(is_valid_profile_id("my-profile"));
        assert!(is_valid_profile_id("  spaced-id  "));
    }

    #[test]
    fn valid_profile_id_rejects_default_variants() {
        assert!(!is_valid_profile_id("default"));
        assert!(!is_valid_profile_id("Default"));
        assert!(!is_valid_profile_id("DEFAULT"));
    }

    #[test]
    fn valid_profile_id_rejects_empty_and_control() {
        assert!(!is_valid_profile_id(""));
        assert!(!is_valid_profile_id("   "));
        assert!(!is_valid_profile_id("id\0"));
    }

    #[test]
    fn normalize_profile_name_trims_and_rejects_reserved() {
        assert_eq!(normalize_profile_name("  Foo  "), Some("Foo".to_string()));
        assert_eq!(normalize_profile_name("default"), None);
        assert_eq!(normalize_profile_name(""), None);
        assert_eq!(normalize_profile_name("\t"), None);
        assert_eq!(normalize_profile_name("a\nb"), None);
    }

    #[test]
    fn default_profile_identity_prompt() {
        assert_eq!(
            default_identity_prompt(),
            "You are a helpful software engineering agent."
        );
    }

    #[test]
    fn profile_effective_identity_prompt() {
        let mut profile = AgentProfile::with_name("test");
        assert_eq!(
            profile.effective_identity_prompt(),
            default_identity_prompt()
        );

        profile.identity_prompt = Some("Custom.".to_string());
        assert_eq!(profile.effective_identity_prompt(), "Custom.");

        profile.identity_prompt = Some("   ".to_string());
        assert_eq!(
            profile.effective_identity_prompt(),
            default_identity_prompt()
        );
    }

    #[test]
    fn classify_valid_profile_record() {
        let payload = serde_json::to_value(AgentProfile::with_name("Valid")).unwrap();
        let result = classify_profile_record("prof", PROFILE_SCHEMA_VERSION, &payload);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().name, "Valid");
    }

    #[test]
    fn classify_unsupported_schema_version() {
        let payload = serde_json::to_value(AgentProfile::with_name("Valid")).unwrap();
        let result = classify_profile_record("prof", 999, &payload);
        assert_eq!(
            result,
            Err(ProfileRecordError::UnsupportedSchemaVersion { actual: 999 })
        );
    }

    #[test]
    fn classify_malformed_json_payload() {
        let payload = serde_json::json!({"unexpected": true});
        let result = classify_profile_record("prof", PROFILE_SCHEMA_VERSION, &payload);
        assert!(matches!(
            result,
            Err(ProfileRecordError::DecodeFailure { .. })
        ));
    }

    #[test]
    fn classify_read_only_approval_rejected() {
        let mut payload = serde_json::to_value(AgentProfile::with_name("Valid")).unwrap();
        payload
            .as_object_mut()
            .unwrap()
            .insert("approval".to_string(), serde_json::json!("ReadOnly"));
        let result = classify_profile_record("prof", PROFILE_SCHEMA_VERSION, &payload);
        assert_eq!(
            result,
            Err(ProfileRecordError::ReadOnlyRejected {
                approval: "ReadOnly".to_string()
            })
        );
    }

    #[test]
    fn classify_reserved_default_id_invalid() {
        let payload = serde_json::to_value(AgentProfile::with_name("Valid")).unwrap();
        let result = classify_profile_record("default", PROFILE_SCHEMA_VERSION, &payload);
        assert!(matches!(
            result,
            Err(ProfileRecordError::InvalidFields { .. })
        ));
    }

    #[test]
    fn classify_structurally_invalid_fields() {
        let profile = AgentProfile {
            name: String::new(),
            ..AgentProfile::with_name("placeholder")
        };
        let mut payload = serde_json::to_value(profile).unwrap();
        payload
            .as_object_mut()
            .unwrap()
            .insert("name".to_string(), serde_json::json!(""));
        let result = classify_profile_record("prof", PROFILE_SCHEMA_VERSION, &payload);
        match result {
            Err(ProfileRecordError::InvalidFields { messages }) => {
                assert!(messages.iter().any(|m| m.contains("Profile name")));
            }
            other => panic!("expected InvalidFields, got {:?}", other),
        }
    }

    #[test]
    fn delete_policy_minimum_resolves() {
        assert_eq!(ProfileDeletePolicy::AllowZero.minimum(), 0);
        assert_eq!(ProfileDeletePolicy::RequireMinimumValid(3).minimum(), 3);
    }
}
