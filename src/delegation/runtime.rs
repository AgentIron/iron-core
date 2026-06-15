//! Runtime extension methods for delegated child execution.

use crate::connection::SharedClientChannel;
use crate::delegation::sink::DelegationPromptSink;
use crate::delegation::{
    apply_skill_filter, apply_tool_filter, compute_tool_catalog_digest, DelegationMetadata,
    DelegationOutcome, DelegationRequest, DelegationResult, DerivedToolCatalog, SubAgentToolBase,
    ToolPolicyDiagnostic, ToolPolicyDiagnosticReason,
};
use crate::durable::SessionId;
use crate::profile::AgentProfile;
use crate::prompt_runner::PromptRunner;
use crate::runtime::IronRuntime;
use crate::tool::ToolDefinition;
use agent_client_protocol::schema as acp;
use std::collections::{HashMap, HashSet};

use crate::mcp::session_catalog::ToolSource;
use crate::plugin::effective_tools::UnavailableReason;

/// Map a session-catalog unavailability reason to a delegation policy diagnostic.
fn map_unavailable_reason(
    source: &ToolSource,
    reason: UnavailableReason,
) -> Option<ToolPolicyDiagnosticReason> {
    match (source, reason) {
        (ToolSource::Mcp { server_id }, UnavailableReason::McpServerNotEnabled) => {
            Some(ToolPolicyDiagnosticReason::McpServerNotEnabled {
                server_id: server_id.clone(),
            })
        }
        (ToolSource::Mcp { server_id }, UnavailableReason::McpServerNotHealthy(health)) => {
            Some(ToolPolicyDiagnosticReason::McpServerNotHealthy {
                server_id: server_id.clone(),
                health: format!("{:?}", health),
            })
        }
        (ToolSource::Plugin { plugin_id }, UnavailableReason::PluginNotEnabled) => {
            Some(ToolPolicyDiagnosticReason::PluginNotEnabled {
                plugin_id: plugin_id.clone(),
            })
        }
        (ToolSource::Plugin { plugin_id }, UnavailableReason::PluginNotInstalled) => {
            Some(ToolPolicyDiagnosticReason::PluginNotInstalled {
                plugin_id: plugin_id.clone(),
            })
        }
        (ToolSource::Plugin { plugin_id }, UnavailableReason::ManifestMissing) => {
            Some(ToolPolicyDiagnosticReason::PluginManifestMissing {
                plugin_id: plugin_id.clone(),
            })
        }
        (ToolSource::Plugin { plugin_id }, UnavailableReason::PluginNotHealthy(health)) => {
            Some(ToolPolicyDiagnosticReason::PluginNotHealthy {
                plugin_id: plugin_id.clone(),
                health: format!("{:?}", health),
            })
        }
        (ToolSource::Plugin { plugin_id }, UnavailableReason::AuthRequired) => {
            Some(ToolPolicyDiagnosticReason::PluginAuthRequired {
                plugin_id: plugin_id.clone(),
            })
        }
        (
            ToolSource::Plugin { plugin_id },
            UnavailableReason::ScopeMissing { required, missing },
        ) => Some(ToolPolicyDiagnosticReason::PluginScopeMissing {
            plugin_id: plugin_id.clone(),
            required,
            missing,
        }),
        _ => None,
    }
}

impl IronRuntime {
    /// Derive a child tool catalog from policy, profile filter, and runtime state.
    pub fn derive_child_tool_catalog(
        &self,
        parent_session_id: SessionId,
        profile: &AgentProfile,
        policy: &crate::delegation::SubAgentToolPolicy,
    ) -> Result<DerivedToolCatalog, String> {
        let parent_defs = self.get_effective_tool_definitions(parent_session_id);
        let parent_names: HashSet<String> = parent_defs.iter().map(|d| d.name.clone()).collect();

        let base_defs = match policy.base {
            SubAgentToolBase::ParentEffective => parent_defs.clone(),
            SubAgentToolBase::ChildDefault => {
                // Build from a fresh hidden session's default catalog.
                let conn_id = self.new_connection();
                let (session_id, _) = self
                    .create_hidden_session(conn_id)
                    .map_err(|e| format!("failed to create child-default session: {}", e))?;
                let defs = self.get_effective_tool_definitions(session_id);
                self.close_session(session_id);
                self.close_connection(conn_id);
                defs
            }
        };

        // Apply additions from runtime tool registry or parent session diagnostics.
        let mut candidate_defs: HashMap<String, ToolDefinition> =
            base_defs.into_iter().map(|d| (d.name.clone(), d)).collect();

        let mut unavailable_requested_additions = Vec::new();
        let mut diagnostics: Vec<ToolPolicyDiagnostic> = Vec::new();
        let runtime_defs = self.tool_registry().definitions();
        let runtime_map: HashMap<String, ToolDefinition> = runtime_defs
            .into_iter()
            .map(|d| (d.name.clone(), d))
            .collect();

        let parent_catalog = self.get_session_tool_catalog(parent_session_id);
        let parent_durable = self.get_session(parent_session_id);

        for name in &policy.additions {
            if let Some(def) = runtime_map.get(name) {
                candidate_defs.insert(name.clone(), def.clone());
                continue;
            }

            let diagnostic = parent_catalog
                .as_ref()
                .zip(parent_durable.as_ref())
                .and_then(|(catalog, durable)| catalog.inspect_tool(&*durable.lock(), name));

            match diagnostic {
                Some(diagnostic) if diagnostic.available => {
                    // Tool is known to the parent session and available there;
                    // treat as an addition request that is already satisfied.
                    if let Some(def) = parent_catalog.as_ref().and_then(|c| c.get_definition(name))
                    {
                        candidate_defs.insert(name.clone(), def.clone());
                    }
                }
                Some(diagnostic) => {
                    unavailable_requested_additions.push(name.clone());
                    if let Some(reason) = diagnostic
                        .unavailable_reason
                        .and_then(|reason| map_unavailable_reason(&diagnostic.source, reason))
                    {
                        diagnostics.push(ToolPolicyDiagnostic {
                            tool_name: name.clone(),
                            reason,
                        });
                    }
                }
                None => {
                    unavailable_requested_additions.push(name.clone());
                    diagnostics.push(ToolPolicyDiagnostic {
                        tool_name: name.clone(),
                        reason: ToolPolicyDiagnosticReason::Missing,
                    });
                }
            }
        }

        let requested_additions: HashSet<String> = policy.additions.iter().cloned().collect();
        let candidate: Vec<ToolDefinition> = candidate_defs.into_values().collect();

        // Apply profile tool filter.
        let (profile_bounded, excluded_by_profile) = apply_tool_filter(&candidate, &profile.tools);
        diagnostics.extend(
            excluded_by_profile
                .iter()
                .filter(|name| requested_additions.contains(*name))
                .cloned()
                .map(|tool_name| ToolPolicyDiagnostic {
                    tool_name,
                    reason: ToolPolicyDiagnosticReason::ExcludedByProfile,
                }),
        );

        // Apply blocklist.
        let deny_set: HashSet<String> = policy.deny.iter().cloned().collect();
        let mut final_defs = Vec::new();
        let mut removed_tools = Vec::new();
        for def in profile_bounded {
            if deny_set.contains(&def.name) {
                removed_tools.push(def.name.clone());
            } else {
                final_defs.push(def);
            }
        }

        final_defs.sort_by(|a, b| a.name.cmp(&b.name));

        let final_names: HashSet<String> = final_defs.iter().map(|d| d.name.clone()).collect();

        let inherited_tools: Vec<String> = final_names
            .iter()
            .cloned()
            .filter(|n| parent_names.contains(n))
            .collect();

        let added_tools: Vec<String> = final_names
            .iter()
            .cloned()
            .filter(|n| !parent_names.contains(n))
            .collect();

        let digest = compute_tool_catalog_digest(&final_defs);

        Ok(DerivedToolCatalog {
            definitions: final_defs,
            inherited_tools,
            removed_tools,
            added_tools,
            unavailable_requested_additions,
            excluded_by_profile,
            diagnostics,
            digest,
        })
    }

    /// Run a delegated child prompt and return the result.
    ///
    /// This creates a hidden child session, sets up a delegation sink, resolves the
    /// selected profile/provider, runs the prompt, and cleans up the relationship.
    pub async fn run_delegation(
        &self,
        parent_session_id: SessionId,
        parent_tool_call_id: Option<String>,
        request: DelegationRequest,
        profile: AgentProfile,
        parent_client: SharedClientChannel,
        parent_session_acp_id: acp::SessionId,
    ) -> Result<(DelegationResult, DelegationMetadata), String> {
        request.validate()?;

        let parent_connection_id = self
            .get_session_connection(parent_session_id)
            .ok_or_else(|| "parent session not found".to_string())?;

        let (child_session_id, durable) = self
            .create_hidden_session(parent_connection_id)
            .map_err(|e| format!("failed to create child session: {}", e))?;

        if let Err(e) = self.register_child(parent_session_id, child_session_id) {
            self.close_session(child_session_id);
            return Err(format!("failed to register child session: {}", e));
        }

        let delegation_id = format!("dlg-{}", uuid::Uuid::new_v4());

        let catalog =
            match self.derive_child_tool_catalog(parent_session_id, &profile, &request.tool_policy)
            {
                Ok(c) => c,
                Err(e) => {
                    self.unregister_child(parent_session_id, child_session_id);
                    self.close_session(child_session_id);
                    return Err(e);
                }
            };

        // Hide tools not in the derived catalog.
        let allowed_names: HashSet<String> =
            catalog.definitions.iter().map(|d| d.name.clone()).collect();
        let all_runtime_defs = self.tool_registry().definitions();
        let hidden: Vec<String> = all_runtime_defs
            .into_iter()
            .map(|d| d.name)
            .filter(|n| !allowed_names.contains(n))
            .collect();
        {
            let mut session = durable.lock();
            session.hidden_tools = hidden;
        }

        // Set identity prompt.
        {
            let mut session = durable.lock();
            session.set_instructions(profile.effective_identity_prompt());
        }

        let (activated_skills, excluded_skills_by_profile, unavailable_requested_skills) = {
            let (allowed_skills, excluded_skills) =
                apply_skill_filter(&request.requested_skills, &profile.skills);
            let mut activated = Vec::new();
            let mut unavailable = Vec::new();
            let mut session = durable.lock();
            for skill_name in allowed_skills {
                match session.load_available_skill(&skill_name) {
                    Some(skill) if !skill.metadata.requires_trust => {
                        session.activate_skill(
                            &skill.metadata.id,
                            &skill.body,
                            skill.resources.clone(),
                        );
                        activated.push(skill.metadata.id.clone());
                    }
                    _ => unavailable.push(skill_name),
                }
            }
            (activated, excluded_skills, unavailable)
        };

        // Build initial user message.
        let user_text = if let Some(context) = &request.context {
            format!("{}\n\nContext:\n{}", request.goal, context)
        } else {
            request.goal.clone()
        };
        {
            let mut session = durable.lock();
            session.add_user_text(user_text);
        }

        let ephemeral = match self.try_start_prompt(child_session_id) {
            Ok(e) => e,
            Err(e) => {
                self.unregister_child(parent_session_id, child_session_id);
                self.close_session(child_session_id);
                return Err(format!("failed to start child prompt: {}", e));
            }
        };

        let sink = DelegationPromptSink::new(
            request.child_approval_mode,
            parent_client,
            parent_session_acp_id,
        );

        let runner = match self.resolve_profile_provider(&profile).await {
            Ok(crate::profile::ResolvedProfileProvider::RuntimeDefault(_)) => {
                PromptRunner::new(self.clone())
            }
            Ok(crate::profile::ResolvedProfileProvider::Managed(provider)) => {
                let context = crate::profile::managed_profile_prompt_context(&profile.provider)
                    .expect("managed profile always yields a prompt context");
                PromptRunner::new_managed(self.clone(), provider, context)
            }
            Err(e) => {
                self.finish_prompt(child_session_id);
                self.unregister_child(parent_session_id, child_session_id);
                self.close_session(child_session_id);
                return Err(format!("failed to resolve profile provider: {}", e));
            }
        };

        let config = self.config().clone();
        let stop_reason =
            Box::pin(runner.run(&durable, &ephemeral, &sink, &config, request.max_iterations))
                .await;

        self.finish_prompt(child_session_id);

        let final_text = {
            let session = durable.lock();
            session
                .messages
                .last()
                .and_then(|m| Some(m.text_content()))
                .filter(|t| !t.is_empty())
                .map(|t| t.to_string())
        };

        let outcome: DelegationOutcome = stop_reason.into();

        let result = DelegationResult {
            delegation_id: delegation_id.clone(),
            child_session_id,
            outcome,
            final_text,
        };

        let metadata = DelegationMetadata {
            delegation_id,
            parent_session_id,
            parent_tool_call_id,
            child_session_id,
            profile_id: request.profile_id.clone(),
            child_approval_mode: request.child_approval_mode,
            max_iterations: request.max_iterations,
            outcome: Some(outcome),
            tool_catalog_digest: catalog.digest.clone(),
            inherited_tools: catalog.inherited_tools,
            removed_tools: catalog.removed_tools,
            added_tools: catalog.added_tools,
            unavailable_requested_additions: catalog.unavailable_requested_additions,
            excluded_by_profile: catalog.excluded_by_profile,
            tool_policy_diagnostics: catalog.diagnostics,
            requested_skills: request.requested_skills.clone(),
            activated_skills,
            excluded_skills_by_profile,
            unavailable_requested_skills,
        };

        Ok((result, metadata))
    }
}
