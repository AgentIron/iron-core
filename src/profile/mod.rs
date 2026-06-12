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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AgentApproval {
    /// Require approval for each tool call.
    #[default]
    PerTool,
    /// Auto-approve tool calls (plumbing only; not enforced by this slice).
    AutoApprove,
    /// Read-only execution; no tool calls are allowed.
    ReadOnly,
}

/// An agent identity profile.
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
}

impl ResolvedProfileProvider {
    /// Borrow the provider as a trait object.
    pub fn as_provider(&self) -> &dyn Provider {
        match self {
            ResolvedProfileProvider::RuntimeDefault(arc) => arc.as_ref(),
            ResolvedProfileProvider::Managed(boxed) => boxed.as_ref(),
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
}
