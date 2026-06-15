//! Delegated child-agent execution types and helpers.
//!
//! This module defines the input, policy, result, and audit metadata types for
//! `delegate_task` and for stored-prompt invocation, which both reuse the same
//! child-session machinery.

pub mod runtime;
pub mod sink;

use crate::durable::SessionId;
use crate::profile::{AgentProfileId, SkillFilter, ToolFilter};
use crate::tool::ToolDefinition;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Schema version for typed delegation payloads stored externally.
pub const DELEGATION_SCHEMA_VERSION: i64 = 1;

/// How child tool approval requests are handled during a delegated run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ChildApprovalMode {
    /// Propagate each child approval request to the parent/main UI workflow.
    #[default]
    PropagateToParent,
    /// Auto-approve all visible child tool calls for this delegated run.
    AutoApprove,
}

/// Base catalog used when deriving a child tool set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SubAgentToolBase {
    /// Inherit the parent session's effective tool catalog.
    #[default]
    ParentEffective,
    /// Build from a fresh child session's runtime-default catalog.
    ChildDefault,
}

/// Configurable tool policy for a delegated child run.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubAgentToolPolicy {
    /// Which catalog to start from.
    #[serde(default)]
    pub base: SubAgentToolBase,
    /// Tool names to remove from the candidate catalog after profile filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
    /// Tool names to add from the runtime/default catalog.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additions: Vec<String>,
}

impl SubAgentToolPolicy {
    /// Default safe policy: inherit parent effective tools, apply profile filter.
    pub fn inherit_parent() -> Self {
        Self {
            base: SubAgentToolBase::ParentEffective,
            deny: Vec::new(),
            additions: Vec::new(),
        }
    }
}

/// Input for starting a delegated child run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DelegationRequest {
    /// The high-level goal passed to the child agent.
    pub goal: String,
    /// Optional extra context for the child run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Optional profile to use for the child run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<AgentProfileId>,
    /// Requested skills to activate in the child run after profile filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_skills: Vec<String>,
    /// Maximum inference/tool iterations for the child run.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// How child tool approval requests are handled.
    #[serde(default)]
    pub child_approval_mode: ChildApprovalMode,
    /// Tool policy for the child run.
    #[serde(default)]
    pub tool_policy: SubAgentToolPolicy,
}

fn default_max_iterations() -> u32 {
    10
}

impl DelegationRequest {
    /// Validate request invariants.
    pub fn validate(&self) -> Result<(), String> {
        let goal = self.goal.trim();
        if goal.is_empty() {
            return Err("delegation goal is required".to_string());
        }
        if self.max_iterations == 0 {
            return Err("max_iterations must be greater than zero".to_string());
        }
        Ok(())
    }
}

/// Outcome of a delegated child run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DelegationOutcome {
    EndTurn,
    Cancelled,
    MaxTurnRequests,
}

impl From<agent_client_protocol::schema::StopReason> for DelegationOutcome {
    fn from(reason: agent_client_protocol::schema::StopReason) -> Self {
        match reason {
            agent_client_protocol::schema::StopReason::EndTurn => DelegationOutcome::EndTurn,
            agent_client_protocol::schema::StopReason::Cancelled => DelegationOutcome::Cancelled,
            agent_client_protocol::schema::StopReason::MaxTurnRequests => {
                DelegationOutcome::MaxTurnRequests
            }
            _ => DelegationOutcome::EndTurn,
        }
    }
}

/// Result returned to the caller/model from a delegated run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DelegationResult {
    /// Stable delegation ID for audit.
    pub delegation_id: String,
    /// Child session ID.
    pub child_session_id: SessionId,
    /// Final outcome of the child run.
    pub outcome: DelegationOutcome,
    /// Final model-readable result text, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_text: Option<String>,
}

/// Audit/diagnostic metadata for a delegated run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DelegationMetadata {
    pub delegation_id: String,
    pub parent_session_id: SessionId,
    pub parent_tool_call_id: Option<String>,
    pub child_session_id: SessionId,
    pub profile_id: Option<AgentProfileId>,
    pub child_approval_mode: ChildApprovalMode,
    pub max_iterations: u32,
    pub outcome: Option<DelegationOutcome>,
    pub tool_catalog_digest: String,
    pub inherited_tools: Vec<String>,
    pub removed_tools: Vec<String>,
    pub added_tools: Vec<String>,
    pub unavailable_requested_additions: Vec<String>,
    pub excluded_by_profile: Vec<String>,
    pub tool_policy_diagnostics: Vec<ToolPolicyDiagnostic>,
    pub requested_skills: Vec<String>,
    pub activated_skills: Vec<String>,
    pub excluded_skills_by_profile: Vec<String>,
    pub unavailable_requested_skills: Vec<String>,
}

/// Reason a requested child tool policy change could not be applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason")]
pub enum ToolPolicyDiagnosticReason {
    /// The requested tool is not known to the runtime/session-effective catalog.
    Missing,
    /// The selected profile's `ToolFilter` excluded the requested tool.
    ExcludedByProfile,
    /// The MCP server providing this tool is disabled for the session.
    McpServerNotEnabled {
        #[serde(rename = "server_id")]
        server_id: String,
    },
    /// The MCP server providing this tool is not healthy.
    McpServerNotHealthy {
        #[serde(rename = "server_id")]
        server_id: String,
        #[serde(rename = "health")]
        health: String,
    },
    /// The plugin providing this tool is not enabled for the session.
    PluginNotEnabled {
        #[serde(rename = "plugin_id")]
        plugin_id: String,
    },
    /// The plugin providing this tool is not installed.
    PluginNotInstalled {
        #[serde(rename = "plugin_id")]
        plugin_id: String,
    },
    /// The plugin providing this tool has no loaded manifest.
    PluginManifestMissing {
        #[serde(rename = "plugin_id")]
        plugin_id: String,
    },
    /// The plugin providing this tool is not healthy.
    PluginNotHealthy {
        #[serde(rename = "plugin_id")]
        plugin_id: String,
        #[serde(rename = "health")]
        health: String,
    },
    /// The plugin tool requires authentication that is not satisfied.
    PluginAuthRequired {
        #[serde(rename = "plugin_id")]
        plugin_id: String,
    },
    /// The plugin tool requires scopes that are not granted.
    PluginScopeMissing {
        #[serde(rename = "plugin_id")]
        plugin_id: String,
        #[serde(rename = "required")]
        required: Vec<String>,
        #[serde(rename = "missing")]
        missing: Vec<String>,
    },
}

/// Frontend/audit diagnostic for child tool policy derivation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPolicyDiagnostic {
    pub tool_name: String,
    pub reason: ToolPolicyDiagnosticReason,
}

/// Result of deriving a child tool catalog from policy.
#[derive(Debug, Clone, PartialEq)]
pub struct DerivedToolCatalog {
    /// Final model-visible tool definitions.
    pub definitions: Vec<ToolDefinition>,
    /// Tools inherited from the parent effective catalog.
    pub inherited_tools: Vec<String>,
    /// Tools removed by blocklist.
    pub removed_tools: Vec<String>,
    /// Tools added beyond the parent effective catalog.
    pub added_tools: Vec<String>,
    /// Requested additions that could not be made available.
    pub unavailable_requested_additions: Vec<String>,
    /// Tools excluded by profile filter.
    pub excluded_by_profile: Vec<String>,
    /// Structured diagnostics for requested policy changes that were not applied.
    pub diagnostics: Vec<ToolPolicyDiagnostic>,
    /// Deterministic digest of the final catalog.
    pub digest: String,
}

/// Compute a deterministic digest over tool definitions and approval flags.
pub fn compute_tool_catalog_digest(definitions: &[ToolDefinition]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    for def in definitions {
        def.name.hash(&mut hasher);
        def.requires_approval.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

/// Apply a profile tool filter to a candidate tool list.
pub fn apply_tool_filter(
    definitions: &[ToolDefinition],
    filter: &ToolFilter,
) -> (Vec<ToolDefinition>, Vec<String>) {
    let mut allowed = Vec::new();
    let mut excluded = Vec::new();

    match filter {
        ToolFilter::Inherit => {
            allowed = definitions.to_vec();
        }
        ToolFilter::Allow(names) => {
            let set: HashSet<_> = names.iter().cloned().collect();
            for def in definitions {
                if set.contains(&def.name) {
                    allowed.push(def.clone());
                } else {
                    excluded.push(def.name.clone());
                }
            }
        }
        ToolFilter::Deny(names) => {
            let set: HashSet<_> = names.iter().cloned().collect();
            for def in definitions {
                if set.contains(&def.name) {
                    excluded.push(def.name.clone());
                } else {
                    allowed.push(def.clone());
                }
            }
        }
    }

    (allowed, excluded)
}

/// Apply a profile skill filter to requested skill names.
pub fn apply_skill_filter(
    requested: &[String],
    filter: &SkillFilter,
) -> (Vec<String>, Vec<String>) {
    let mut allowed = Vec::new();
    let mut excluded = Vec::new();

    match filter {
        SkillFilter::None => {
            excluded.extend(requested.iter().cloned());
        }
        SkillFilter::Inherit => {
            allowed.extend(requested.iter().cloned());
        }
        SkillFilter::Allow(names) => {
            let set: HashSet<_> = names.iter().cloned().collect();
            for skill in requested {
                if set.contains(skill) {
                    allowed.push(skill.clone());
                } else {
                    excluded.push(skill.clone());
                }
            }
        }
    }

    (allowed, excluded)
}

/// Validate a delegation tool call argument payload.
pub fn validate_delegation_arguments(
    value: &serde_json::Value,
) -> Result<DelegationRequest, String> {
    serde_json::from_value(value.clone()).map_err(|e| format!("invalid delegate_task args: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::ToolDefinition;

    #[test]
    fn delegation_request_rejects_empty_goal() {
        let req = DelegationRequest {
            goal: "   ".to_string(),
            context: None,
            profile_id: None,
            requested_skills: Vec::new(),
            max_iterations: 5,
            child_approval_mode: ChildApprovalMode::AutoApprove,
            tool_policy: SubAgentToolPolicy::default(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn delegation_request_rejects_zero_iterations() {
        let req = DelegationRequest {
            goal: "do work".to_string(),
            context: None,
            profile_id: None,
            requested_skills: Vec::new(),
            max_iterations: 0,
            child_approval_mode: ChildApprovalMode::AutoApprove,
            tool_policy: SubAgentToolPolicy::default(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn apply_tool_filter_inherit_passes_through() {
        let defs = vec![ToolDefinition::new("read", "read", serde_json::json!({}))];
        let (allowed, excluded) = apply_tool_filter(&defs, &ToolFilter::Inherit);
        assert_eq!(allowed.len(), 1);
        assert!(excluded.is_empty());
    }

    #[test]
    fn apply_tool_filter_allow_excludes_others() {
        let defs = vec![
            ToolDefinition::new("read", "read", serde_json::json!({})),
            ToolDefinition::new("write", "write", serde_json::json!({})),
        ];
        let (allowed, excluded) =
            apply_tool_filter(&defs, &ToolFilter::Allow(vec!["read".to_string()]));
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0].name, "read");
        assert_eq!(excluded, vec!["write".to_string()]);
    }

    #[test]
    fn apply_skill_filter_respects_allow() {
        let (allowed, excluded) = apply_skill_filter(
            &["rust".to_string(), "python".to_string()],
            &SkillFilter::Allow(vec!["rust".to_string()]),
        );
        assert_eq!(allowed, vec!["rust".to_string()]);
        assert_eq!(excluded, vec!["python".to_string()]);
    }
}
