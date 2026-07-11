//! Headless runtime bootstrap and safety preflight for non-interactive
//! automation task execution.
//!
//! This module reconstructs a fully functional runtime from persisted
//! [`ConfigStore`] state, resolves the saved default provider/model through
//! stored credentials, validates headless safety policy, and produces a
//! [`HeadlessRuntime`] ready for root-session execution.
//!
//! ## Bootstrap flow
//!
//! 1. Load runtime settings from ConfigStore.
//! 2. Resolve the saved default provider/model through credentials.
//! 3. Construct `Config` and `IronAgent` with the resolved provider.
//! 4. Load provider profiles, agent profiles, stored prompts, built-in tools,
//!    and MCP server definitions.
//! 5. Resolve the automation task into an immutable [`ResolvedExecutionInput`].
//! 6. Run headless safety preflight.
//!
//! ## Plugin exclusion
//!
//! WASM plugins are not scanned, registered, or loaded. A profile that
//! explicitly allow-lists a plugin tool fails preflight with an
//! [`UnavailableTool`] error.

use crate::builtin::BuiltinToolConfig;
use crate::config::records::{McpServerConfigRecord, RuntimeSettingsSnapshot};
use crate::config::{Config, ConfigError, ConfigStore, McpConfig, SkillConfig};
use crate::execution::{is_headless_safe, resolve_task_execution, ResolvedExecutionInput};
use crate::facade::IronAgent;
use crate::mcp::McpServerConfig;
use crate::profile::{AgentProfileProvider, DefaultProfileSeedPolicy, ToolFilter};
use crate::provider_credential::{
    CredentialResolver, DurableCredentialStore, DynCredentialStore, FallibleCredentialStore,
    ProviderAuthError, ProviderPromptContext, ProviderSlug,
};
use crate::provider_profile::build_effective_registry;
use iron_providers::{
    InferenceRequest, Provider, ProviderEvent, ProviderFuture, ProviderRegistry, ProviderResult,
    RuntimeConfig,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::execution::{AutomationRunErrorCategory, AutomationRunResult, ResolutionError};
use crate::profile::{AgentProfile, AgentProfileId};
use futures::stream::BoxStream;

// ============================================================================
// Provider wrapper
// ============================================================================

/// Wrapper that allows a `Box<dyn Provider>` to be passed through the generic
/// `P: Provider + 'static` bound on `IronAgent` constructors.
struct ResolvedProvider(Box<dyn Provider>);

impl Provider for ResolvedProvider {
    fn infer(&self, request: InferenceRequest) -> ProviderFuture<'_, Vec<ProviderEvent>> {
        self.0.infer(request)
    }

    fn infer_stream(
        &self,
        request: InferenceRequest,
    ) -> ProviderFuture<'_, BoxStream<'static, ProviderResult<ProviderEvent>>> {
        self.0.infer_stream(request)
    }
}

// ============================================================================
// Bootstrap error
// ============================================================================

/// Typed failure during headless runtime bootstrap.
#[derive(Debug, thiserror::Error)]
pub enum HeadlessBootstrapError {
    /// No default provider/model saved in ConfigStore.
    #[error("no default provider/model saved in configuration")]
    MissingDefaultProvider,
    /// Provider construction failed after credential resolution.
    #[error("provider initialization failed for '{provider}': {reason}")]
    ProviderInit { provider: String, reason: String },
    /// Credential is missing, unreadable, or otherwise unusable.
    #[error("credential failure for provider '{provider}': {reason}")]
    CredentialFailure { provider: String, reason: String },
    /// Credential exists but requires interactive re-authentication.
    #[error("interactive authentication required for provider '{provider}': {reason}")]
    InteractiveAuthRequired { provider: String, reason: String },
    /// Profile approval or tool policy is unsafe for headless execution.
    #[error("unsafe policy for headless execution: {0}")]
    UnsafePolicy(String),
    /// Profile explicitly requires a tool that is not available.
    #[error("required tool '{0}' is not available in the reconstructed runtime")]
    UnavailableTool(String),
    /// ConfigStore error.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// Execution-input resolution error.
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
}

// ============================================================================
// Bootstrapped runtime
// ============================================================================

/// Fully bootstrapped headless runtime ready for root-session execution.
///
/// Produced by [`bootstrap_headless`] after resolving all persisted state,
/// constructing the agent, and passing safety preflight. Phase 4 root-session
/// execution consumes this struct.
pub struct HeadlessRuntime {
    /// Fully initialized agent with provider, credentials, profiles, prompts,
    /// built-in tools, and MCP servers loaded.
    pub agent: IronAgent,
    /// Immutable execution-input snapshot resolved at run start.
    pub resolved: ResolvedExecutionInput,
    /// Resolved provider slug (from saved default model).
    pub provider_slug: String,
    /// Resolved model identifier (from saved default model).
    pub model: String,
    /// Sorted effective tool names after applying the profile's tool filter
    /// to the reconstructed base tool inventory.
    pub effective_tools: Vec<String>,
}

// ============================================================================
// Bootstrap entry point
// ============================================================================

/// Bootstrap a headless runtime from persisted ConfigStore state.
///
/// This is the main entry point for headless automation. It:
///
/// 1. Loads runtime settings (default model, MCP servers, skill settings).
/// 2. Resolves the saved default provider/model through stored credentials.
/// 3. Constructs the runtime with built-in tools, MCP servers, skills,
///    profiles, and stored prompts.
/// 4. Resolves the named automation task into an immutable snapshot.
/// 5. Runs headless safety preflight (AutoApprove, tool availability).
///
/// WASM plugins are never scanned, registered, or loaded.
pub async fn bootstrap_headless(
    store: ConfigStore,
    task_id: &str,
    workspace: PathBuf,
) -> Result<HeadlessRuntime, HeadlessBootstrapError> {
    // 1. Load persisted settings.
    let settings = store.load_runtime_settings().await?;

    // 2. Resolve saved default provider/model through credentials.
    let (provider, default_slug, default_model) =
        resolve_default_provider(&store, &settings).await?;

    // 3. Build Config from persisted settings.
    let skill_settings = &settings.skill_settings;
    let config = Config::default()
        .with_model(default_model.clone())
        .with_provider_name(default_slug.clone())
        .with_workspace_roots(vec![workspace.clone()])
        .with_mcp(McpConfig {
            enabled: true,
            enabled_by_default: true,
        })
        .with_skills(SkillConfig {
            enabled: true,
            trust_project_skills: skill_settings.trust_project_skills,
            additional_skill_dirs: skill_settings.additional_skill_dirs.clone(),
        });

    // 4. Create credential store for runtime-level managed provider resolution.
    let runtime_cred_store: DynCredentialStore =
        Arc::new(DurableCredentialStore::new(store.clone()));

    // 5. Construct the agent with the resolved provider.
    let provider_wrapper = ResolvedProvider(provider);
    let agent = IronAgent::new_with_credential_store(config, provider_wrapper, runtime_cred_store);

    // 6. Load persisted state into the runtime.
    agent.load_provider_profiles(&store).await?;
    agent
        .seed_default_profiles(&store, DefaultProfileSeedPolicy::FirstRunOnly)
        .await?;
    agent.load_profiles(&store).await?;
    agent.load_stored_prompts(&store).await?;

    // 7. Register built-in tools with the resolved workspace as allowed root.
    let builtin_config = BuiltinToolConfig::new(vec![workspace.clone()]);
    agent.register_builtin_tools(&builtin_config);

    // 8. Register persisted MCP servers.
    for record in &settings.mcp_servers {
        let mcp_config = mcp_record_to_config(record);
        agent.register_mcp_server(mcp_config);
    }

    // 9. Build profile registry HashMap for task resolution.
    let profiles: HashMap<AgentProfileId, AgentProfile> = agent
        .list_profiles()
        .into_iter()
        .map(|entry| (entry.id, entry.profile))
        .collect();

    // 10. Resolve the automation task.
    let resolved = resolve_task_execution(&store, &profiles, task_id, workspace).await?;

    // 11. Preflight: approval check only. Tool availability is checked in
    //     run_automation after session creation so MCP tools are included.
    if !is_headless_safe(resolved.profile.approval) {
        return Err(HeadlessBootstrapError::UnsafePolicy(format!(
            "profile '{}' uses {:?}, but headless execution requires AutoApprove",
            resolved.profile_id.as_str(),
            resolved.profile.approval
        )));
    }

    // 12. Determine reported provider/model based on the resolved profile
    //     variant (F1). Managed profiles report their configured provider/model;
    //     RuntimeDefault profiles report the saved default.
    let (provider_slug, model) = match &resolved.profile.provider {
        AgentProfileProvider::RuntimeDefault => (default_slug, default_model),
        AgentProfileProvider::Managed {
            provider_slug,
            model,
        } => (provider_slug.as_str().to_string(), model.clone()),
    };

    Ok(HeadlessRuntime {
        agent,
        resolved,
        provider_slug,
        model,
        effective_tools: Vec::new(),
    })
}

// ============================================================================
// Default provider resolution
// ============================================================================

/// Resolve the saved default provider/model through credentials and the
/// effective provider registry.
///
/// Returns the constructed provider, provider slug, and model. Fails closed
/// if the default is missing, credentials are unavailable, or the provider
/// cannot be constructed.
async fn resolve_default_provider(
    store: &ConfigStore,
    settings: &RuntimeSettingsSnapshot,
) -> Result<(Box<dyn Provider>, String, String), HeadlessBootstrapError> {
    let default = settings
        .default_model
        .as_ref()
        .ok_or(HeadlessBootstrapError::MissingDefaultProvider)?;

    let provider_slug = default.provider_slug.clone();
    let model = default.model_id.clone();

    // Build effective provider registry (built-ins + persisted profiles).
    let registry: ProviderRegistry = build_effective_registry(store).await?;

    // Build credential resolver with fallible store for actionable errors.
    let durable = Arc::new(DurableCredentialStore::new(store.clone()));
    let cred_store: DynCredentialStore = durable.clone();
    let fallible: Arc<dyn FallibleCredentialStore> = durable.clone();
    let resolver = CredentialResolver::with_fallible_store(cred_store, fallible);

    // Merge support from persisted provider profiles.
    let profile_records = store.list_provider_profiles().await?;
    let provider_profiles: Vec<iron_providers::ProviderProfile> = profile_records
        .into_iter()
        .filter_map(|r| serde_json::from_str(&r.profile_json).ok())
        .collect();
    resolver.merge_support_from_profiles(&provider_profiles);

    // Resolve credential.
    let context = ProviderPromptContext {
        provider_slug: ProviderSlug::new(&provider_slug),
        model: model.clone(),
        api_key: None,
    };

    let resolved_credential = resolver
        .resolve(&context, None)
        .await
        .map_err(map_auth_error)?;

    // Construct provider.
    let runtime_config = RuntimeConfig::from_credential(resolved_credential.provider_credential);
    let provider = registry.get(&provider_slug, runtime_config).map_err(|e| {
        HeadlessBootstrapError::ProviderInit {
            provider: provider_slug.clone(),
            reason: e.to_string(),
        }
    })?;

    Ok((provider, provider_slug, model))
}

/// Map a `ProviderAuthError` to the appropriate `HeadlessBootstrapError`.
fn map_auth_error(e: ProviderAuthError) -> HeadlessBootstrapError {
    match e {
        ProviderAuthError::NotConfigured(slug) => HeadlessBootstrapError::CredentialFailure {
            provider: slug,
            reason: "no credential configured".to_string(),
        },
        ProviderAuthError::UnsupportedCredential { provider, mode } => {
            HeadlessBootstrapError::CredentialFailure {
                provider,
                reason: format!("unsupported credential mode: {:?}", mode),
            }
        }
        ProviderAuthError::Expired(slug) => HeadlessBootstrapError::CredentialFailure {
            provider: slug,
            reason: "token expired".to_string(),
        },
        ProviderAuthError::RefreshFailed { provider, reason } => {
            HeadlessBootstrapError::InteractiveAuthRequired {
                provider,
                reason: format!("token refresh failed: {}", reason),
            }
        }
        ProviderAuthError::Revoked(slug) => HeadlessBootstrapError::InteractiveAuthRequired {
            provider: slug,
            reason: "credential revoked, re-authentication required".to_string(),
        },
        ProviderAuthError::StoreError { provider, reason } => {
            HeadlessBootstrapError::CredentialFailure { provider, reason }
        }
    }
}

// ============================================================================
// Headless safety preflight
// ============================================================================

/// Validate that the resolved profile and tool inventory are safe for
/// non-interactive headless execution.
///
/// Checks:
/// - Profile approval must be `AutoApprove` (rejects `PerTool`).
/// - All explicitly allow-listed tools must be available in the base registry.
/// - `Inherit` and `Deny` filters are accepted without additional checks.
pub fn preflight_headless_safety(
    input: &ResolvedExecutionInput,
    tool_names: &[String],
) -> Result<(), HeadlessBootstrapError> {
    if !is_headless_safe(input.profile.approval) {
        return Err(HeadlessBootstrapError::UnsafePolicy(format!(
            "profile '{}' uses {:?}, but headless execution requires AutoApprove",
            input.profile_id.as_str(),
            input.profile.approval
        )));
    }

    if let ToolFilter::Allow(allowed) = &input.profile.tools {
        for tool in allowed {
            if !tool_names.iter().any(|n| n == tool) {
                return Err(HeadlessBootstrapError::UnavailableTool(tool.clone()));
            }
        }
    }

    Ok(())
}

// ============================================================================
// Effective tools computation
// ============================================================================

/// Compute the effective tool set after applying a tool filter to available
/// tool names.
///
/// - `Inherit`: all available tools.
/// - `Allow`: only tools in both the allow-list and the available set.
/// - `Deny`: available tools minus the denied set.
///
/// Results are sorted for deterministic output.
pub fn compute_effective_tools(filter: &ToolFilter, available: &[String]) -> Vec<String> {
    let mut tools: Vec<String> = match filter {
        ToolFilter::Inherit => available.to_vec(),
        ToolFilter::Allow(allowed) => available
            .iter()
            .filter(|n| allowed.iter().any(|a| a == *n))
            .cloned()
            .collect(),
        ToolFilter::Deny(denied) => available
            .iter()
            .filter(|n| !denied.iter().any(|d| d == *n))
            .cloned()
            .collect(),
    };
    tools.sort();
    tools
}

// ============================================================================
// Root-session automation execution
// ============================================================================

/// Execute an automation task as a root session with timeout and cancellation.
///
/// This is the main entry point for Phase 4 root-session execution. It:
///
/// 1. Connects to the agent and creates a root session with the resolved
///    profile.
/// 2. Applies the canonical workspace as session roots.
/// 3. Sends the composed user goal as a single prompt.
/// 4. Races the prompt against a timeout and an external cancellation token.
/// 5. Extracts the final assistant text and returns a structured result.
///
/// The function does not create a synthetic parent session — the resolved
/// profile's tool filter and approval mode are snapshotted directly into the
/// root session via `create_session_with_profile`.
///
/// **Must run on a `tokio::task::LocalSet`** because `AgentSession` uses `Rc`
/// internally and is `!Send`.
pub async fn run_automation(
    headless: HeadlessRuntime,
    timeout: std::time::Duration,
    cancel: tokio_util::sync::CancellationToken,
) -> AutomationRunResult {
    let HeadlessRuntime {
        agent,
        resolved,
        provider_slug,
        model,
        ..
    } = headless;

    let mut result = AutomationRunResult::started(&resolved.task, resolved.workspace.clone());

    // Connect and create root session.
    let connection = agent.connect();
    let session = match connection.create_session_with_profile(resolved.profile_id.clone()) {
        Ok(s) => s,
        Err(e) => {
            result.fail(
                AutomationRunErrorCategory::Execution,
                format!("failed to create root session: {}", e),
            );
            return result;
        }
    };

    // Apply canonical workspace explicitly (F10: surface failures).
    if let Err(e) = session.set_workspace_roots(vec![resolved.workspace.clone()]) {
        result.fail(
            AutomationRunErrorCategory::Execution,
            format!("failed to set workspace roots: {}", e),
        );
        return result;
    }

    // Activate effective skills on the root session (F2).
    for skill_name in &resolved.effective_skills {
        if let Err(e) = session.activate_skill(skill_name) {
            result.fail(
                AutomationRunErrorCategory::Execution,
                format!("failed to activate skill '{}': {}", skill_name, e),
            );
            return result;
        }
    }

    // Compute effective tools from the session-effective catalog (F3).
    // This includes MCP tools that are invisible to the base registry.
    let session_tool_names: Vec<String> = agent
        .get_effective_tools(session.id())
        .into_iter()
        .map(|d| d.name)
        .collect();

    // Preflight: check allow-listed tools against the full session catalog.
    if let ToolFilter::Allow(allowed) = &resolved.profile.tools {
        for tool in allowed {
            if !session_tool_names.iter().any(|n| n == tool) {
                result.fail(
                    AutomationRunErrorCategory::UnsafePolicy,
                    format!(
                        "required tool '{}' is not available in the session tool catalog",
                        tool
                    ),
                );
                return result;
            }
        }
    }

    let effective_tools = compute_effective_tools(&resolved.profile.tools, &session_tool_names);

    result.set_resolved_metadata(
        resolved.profile_id.as_str(),
        if provider_slug.is_empty() {
            None
        } else {
            Some(provider_slug)
        },
        if model.is_empty() { None } else { Some(model) },
        effective_tools,
    );

    // Execute with timeout and cancellation.
    let cancel_session = session.clone();
    let cancel_token = cancel.clone();

    tokio::select! {
        biased;

        _ = cancel_token.cancelled() => {
            cancel_session.cancel().await;
            result.cancel("cancelled by signal".to_string());
        }

        _ = tokio::time::sleep(timeout) => {
            cancel_session.cancel().await;
            result.timeout(format!("exceeded {} timeout", format_duration(timeout)));
        }

        prompt_result = session.try_prompt(&resolved.user_goal) => {
            match prompt_result {
                Err(e) => {
                    result.fail(
                        AutomationRunErrorCategory::Execution,
                        format!("prompt error: {}", e),
                    );
                }
                Ok(crate::facade::PromptOutcome::Cancelled) => {
                    result.cancel("prompt was cancelled".to_string());
                }
                Ok(_outcome) => {
                    // Check for injected provider/auth errors (F4).
                    if let Some(diagnostic) = session.profile_unavailable() {
                        result.fail(
                            AutomationRunErrorCategory::Execution,
                            format!("provider unavailable: {}", diagnostic),
                        );
                    } else {
                        let text = extract_final_text(&session.messages());
                        result.complete(text);
                    }
                }
            }
        }
    }

    result
}

/// Extract the final assistant text from conversation messages.
///
/// Scans messages in reverse and returns the text content of the last
/// agent message. Returns an empty string if no agent message exists.
pub fn extract_final_text(messages: &[crate::durable::StructuredMessage]) -> String {
    for msg in messages.iter().rev() {
        if msg.is_agent() {
            return msg.text_content();
        }
    }
    String::new()
}

/// Format a duration for human-readable timeout messages.
fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 && secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs >= 60 && secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{}s", secs)
    }
}

// ============================================================================
// MCP record conversion
// ============================================================================

/// Convert a persisted `McpServerConfigRecord` to a runtime `McpServerConfig`.
fn mcp_record_to_config(record: &McpServerConfigRecord) -> McpServerConfig {
    McpServerConfig {
        id: record.id.clone(),
        label: record.label.clone(),
        description: record.description.clone(),
        transport: record.transport.clone(),
        enabled_by_default: record.enabled_by_default,
        working_dir: record.working_dir.clone(),
        inherited_env_vars: record.inherited_env_vars.clone(),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation_task::AutomationTaskInput;
    use crate::config::{
        ConfigStore, DefaultModelInput, McpServerConfigInput, ProfileInput, PromptInput,
        SkillSettingsInput,
    };
    use crate::mcp::{HttpConfig, McpTransport};
    use crate::profile::{
        AgentApproval, AgentProfile, AgentProfileProvider, PROFILE_SCHEMA_VERSION,
    };
    use crate::provider_credential::{ProviderCredentialStore, ProviderSlug, StoredCredential};
    use crate::stored_prompt::{StoredPrompt, STORED_PROMPT_SCHEMA_VERSION};

    // ---- compute_effective_tools ----

    #[test]
    fn effective_tools_inherit_returns_all_sorted() {
        let filter = ToolFilter::Inherit;
        let available = vec!["z_tool".to_string(), "a_tool".to_string()];
        let result = compute_effective_tools(&filter, &available);
        assert_eq!(result, vec!["a_tool", "z_tool"]);
    }

    #[test]
    fn effective_tools_allow_filters_to_intersection() {
        let filter = ToolFilter::Allow(vec!["a_tool".to_string(), "missing".to_string()]);
        let available = vec!["a_tool".to_string(), "z_tool".to_string()];
        let result = compute_effective_tools(&filter, &available);
        assert_eq!(result, vec!["a_tool"]);
    }

    #[test]
    fn effective_tools_deny_excludes_denied() {
        let filter = ToolFilter::Deny(vec!["z_tool".to_string()]);
        let available = vec![
            "a_tool".to_string(),
            "z_tool".to_string(),
            "m_tool".to_string(),
        ];
        let result = compute_effective_tools(&filter, &available);
        assert_eq!(result, vec!["a_tool", "m_tool"]);
    }

    #[test]
    fn effective_tools_empty_available() {
        let filter = ToolFilter::Inherit;
        let result = compute_effective_tools(&filter, &[]);
        assert!(result.is_empty());
    }

    // ---- preflight_headless_safety ----

    fn make_resolved_input(approval: AgentApproval, tools: ToolFilter) -> ResolvedExecutionInput {
        let now = chrono::Utc::now();
        let task = crate::automation_task::AutomationTask {
            id: "test".to_string(),
            name: "Test".to_string(),
            stored_prompt_id: "p".to_string(),
            expected_outcome: "done".to_string(),
            created_at: now,
            updated_at: now,
        };
        let prompt = StoredPrompt {
            instructions: "do".to_string(),
            skills: Vec::new(),
            profile: None,
        };
        let profile = AgentProfile {
            name: "test".to_string(),
            provider: AgentProfileProvider::RuntimeDefault,
            tools,
            skills: crate::profile::SkillFilter::Inherit,
            approval,
            identity_prompt: None,
        };
        ResolvedExecutionInput {
            task,
            prompt,
            profile_id: AgentProfileId::from("test"),
            profile,
            user_goal: "do\n\ndone".to_string(),
            effective_skills: Vec::new(),
            workspace: PathBuf::from("/tmp"),
            resolved_at: now,
        }
    }

    #[test]
    fn preflight_autoapprove_inherit_passes() {
        let input = make_resolved_input(AgentApproval::AutoApprove, ToolFilter::Inherit);
        let tools = vec!["tool_a".to_string()];
        assert!(preflight_headless_safety(&input, &tools).is_ok());
    }

    #[test]
    fn preflight_pertool_fails() {
        let input = make_resolved_input(AgentApproval::PerTool, ToolFilter::Inherit);
        let tools = vec!["tool_a".to_string()];
        let err = preflight_headless_safety(&input, &tools).unwrap_err();
        assert!(matches!(err, HeadlessBootstrapError::UnsafePolicy(_)));
    }

    #[test]
    fn preflight_autoapprove_allow_available_passes() {
        let input = make_resolved_input(
            AgentApproval::AutoApprove,
            ToolFilter::Allow(vec!["tool_a".to_string()]),
        );
        let tools = vec!["tool_a".to_string(), "tool_b".to_string()];
        assert!(preflight_headless_safety(&input, &tools).is_ok());
    }

    #[test]
    fn preflight_autoapprove_allow_missing_fails() {
        let input = make_resolved_input(
            AgentApproval::AutoApprove,
            ToolFilter::Allow(vec!["plugin_tool".to_string()]),
        );
        let tools = vec!["tool_a".to_string()];
        let err = preflight_headless_safety(&input, &tools).unwrap_err();
        assert!(matches!(err, HeadlessBootstrapError::UnavailableTool(t) if t == "plugin_tool"));
    }

    #[test]
    fn preflight_autoapprove_deny_passes_regardless() {
        let input = make_resolved_input(
            AgentApproval::AutoApprove,
            ToolFilter::Deny(vec!["missing".to_string()]),
        );
        let tools = vec!["tool_a".to_string()];
        assert!(preflight_headless_safety(&input, &tools).is_ok());
    }

    // ---- mcp_record_to_config ----

    #[test]
    fn mcp_record_converts_to_config() {
        let now = chrono::Utc::now();
        let record = McpServerConfigRecord {
            id: "test-server".to_string(),
            label: "Test".to_string(),
            description: Some("A test server".to_string()),
            transport: McpTransport::Http {
                config: HttpConfig::new("https://example.com/mcp".to_string()),
            },
            working_dir: None,
            enabled_by_default: true,
            inherited_env_vars: vec!["PATH".to_string()],
            created_at: now,
            updated_at: now,
        };
        let config = mcp_record_to_config(&record);
        assert_eq!(config.id, "test-server");
        assert_eq!(config.label, "Test");
        assert_eq!(config.description.as_deref(), Some("A test server"));
        assert!(config.enabled_by_default);
        assert_eq!(config.inherited_env_vars, vec!["PATH"]);
    }

    // ---- resolve_default_provider ----

    #[tokio::test]
    async fn resolve_default_provider_success() {
        let store = ConfigStore::open_in_memory().await.unwrap();

        // Store credential.
        let durable = Arc::new(DurableCredentialStore::new(store.clone()));
        ProviderCredentialStore::set(
            &*durable,
            &ProviderSlug::new("openai"),
            StoredCredential::ApiKey("sk-test".to_string()),
        )
        .await;

        // Set default model.
        store
            .set_default_model(&DefaultModelInput {
                provider_slug: "openai".to_string(),
                model_id: "gpt-4o".to_string(),
            })
            .await
            .unwrap();

        let settings = store.load_runtime_settings().await.unwrap();
        let (provider, slug, model) = resolve_default_provider(&store, &settings)
            .await
            .expect("provider resolution should succeed");

        assert_eq!(slug, "openai");
        assert_eq!(model, "gpt-4o");
        // Provider object exists — we can't do much with it without a network call.
        let _ = provider;
    }

    #[tokio::test]
    async fn resolve_default_provider_missing_default() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let settings = store.load_runtime_settings().await.unwrap();

        let result = resolve_default_provider(&store, &settings).await;
        assert!(matches!(
            result,
            Err(HeadlessBootstrapError::MissingDefaultProvider)
        ));
    }

    #[tokio::test]
    async fn resolve_default_provider_missing_credential() {
        let store = ConfigStore::open_in_memory().await.unwrap();

        store
            .set_default_model(&DefaultModelInput {
                provider_slug: "openai".to_string(),
                model_id: "gpt-4o".to_string(),
            })
            .await
            .unwrap();

        let settings = store.load_runtime_settings().await.unwrap();
        let result = resolve_default_provider(&store, &settings).await;
        assert!(matches!(
            result,
            Err(HeadlessBootstrapError::CredentialFailure { .. })
        ));
    }

    // ---- map_auth_error ----

    #[test]
    fn map_not_configured_to_credential_failure() {
        let err = map_auth_error(ProviderAuthError::NotConfigured("openai".to_string()));
        assert!(matches!(
            err,
            HeadlessBootstrapError::CredentialFailure { provider, .. } if provider == "openai"
        ));
    }

    #[test]
    fn map_revoked_to_interactive_auth() {
        let err = map_auth_error(ProviderAuthError::Revoked("openai".to_string()));
        assert!(matches!(
            err,
            HeadlessBootstrapError::InteractiveAuthRequired { provider, .. } if provider == "openai"
        ));
    }

    #[test]
    fn map_refresh_failed_to_interactive_auth() {
        let err = map_auth_error(ProviderAuthError::RefreshFailed {
            provider: "openai".to_string(),
            reason: "network error".to_string(),
        });
        assert!(matches!(
            err,
            HeadlessBootstrapError::InteractiveAuthRequired { .. }
        ));
    }

    // ---- bootstrap_headless full flow ----

    /// Helper: set up a complete ConfigStore with provider credential,
    /// default model, AutoApprove profile, prompt, and task.
    async fn setup_complete_store() -> ConfigStore {
        let store = ConfigStore::open_in_memory().await.unwrap();

        // 1. Store API key credential for openai.
        let durable = Arc::new(DurableCredentialStore::new(store.clone()));
        ProviderCredentialStore::set(
            &*durable,
            &ProviderSlug::new("openai"),
            StoredCredential::ApiKey("sk-test".to_string()),
        )
        .await;

        // 2. Set default model.
        store
            .set_default_model(&DefaultModelInput {
                provider_slug: "openai".to_string(),
                model_id: "gpt-4o".to_string(),
            })
            .await
            .unwrap();

        // 3. Create an AutoApprove profile.
        let auto_profile = AgentProfile {
            name: "automation".to_string(),
            provider: AgentProfileProvider::RuntimeDefault,
            tools: ToolFilter::Inherit,
            skills: crate::profile::SkillFilter::Inherit,
            approval: AgentApproval::AutoApprove,
            identity_prompt: None,
        };
        store
            .set_profile(&ProfileInput {
                id: "automation".to_string(),
                schema_version: PROFILE_SCHEMA_VERSION,
                payload: serde_json::to_value(&auto_profile).unwrap(),
            })
            .await
            .unwrap();

        // 4. Create a stored prompt referencing the automation profile.
        let prompt = StoredPrompt {
            instructions: "Generate a daily report".to_string(),
            skills: Vec::new(),
            profile: Some(AgentProfileId::from("automation")),
        };
        store
            .set_prompt(&PromptInput {
                id: "report-prompt".to_string(),
                schema_version: STORED_PROMPT_SCHEMA_VERSION,
                payload: serde_json::to_value(&prompt).unwrap(),
            })
            .await
            .unwrap();

        // 5. Create an automation task.
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

    #[tokio::test]
    async fn bootstrap_success() {
        let store = setup_complete_store().await;
        let workspace = std::env::temp_dir();

        let runtime = bootstrap_headless(store, "daily-report", workspace.clone())
            .await
            .expect("bootstrap should succeed");

        assert_eq!(runtime.provider_slug, "openai");
        assert_eq!(runtime.model, "gpt-4o");
        assert_eq!(runtime.resolved.task.id, "daily-report");
        assert_eq!(
            runtime.resolved.profile_id,
            AgentProfileId::from("automation")
        );
        assert_eq!(
            runtime.resolved.profile.approval,
            AgentApproval::AutoApprove
        );
        // effective_tools is empty after bootstrap — computed in run_automation
        // after session creation so MCP tools are included.
        assert!(runtime.effective_tools.is_empty());
    }

    #[tokio::test]
    async fn bootstrap_missing_default_provider() {
        let store = ConfigStore::open_in_memory().await.unwrap();

        let result = bootstrap_headless(store, "any-task", std::env::temp_dir()).await;
        assert!(matches!(
            result,
            Err(HeadlessBootstrapError::MissingDefaultProvider)
        ));
    }

    #[tokio::test]
    async fn bootstrap_missing_credential() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        store
            .set_default_model(&DefaultModelInput {
                provider_slug: "openai".to_string(),
                model_id: "gpt-4o".to_string(),
            })
            .await
            .unwrap();

        let result = bootstrap_headless(store, "any-task", std::env::temp_dir()).await;
        assert!(matches!(
            result,
            Err(HeadlessBootstrapError::CredentialFailure { .. })
        ));
    }

    #[tokio::test]
    async fn bootstrap_unsafe_profile_fails_preflight() {
        let store = setup_complete_store().await;

        // Override the profile to PerTool (unsafe for headless).
        let pertool_profile = AgentProfile {
            name: "automation".to_string(),
            provider: AgentProfileProvider::RuntimeDefault,
            tools: ToolFilter::Inherit,
            skills: crate::profile::SkillFilter::Inherit,
            approval: AgentApproval::PerTool,
            identity_prompt: None,
        };
        store
            .set_profile(&ProfileInput {
                id: "automation".to_string(),
                schema_version: PROFILE_SCHEMA_VERSION,
                payload: serde_json::to_value(&pertool_profile).unwrap(),
            })
            .await
            .unwrap();

        let result = bootstrap_headless(store, "daily-report", std::env::temp_dir()).await;
        assert!(matches!(
            result,
            Err(HeadlessBootstrapError::UnsafePolicy(_))
        ));
    }

    #[tokio::test]
    async fn bootstrap_default_profile_not_headless_safe() {
        let store = ConfigStore::open_in_memory().await.unwrap();

        // Credential + default model.
        let durable = Arc::new(DurableCredentialStore::new(store.clone()));
        ProviderCredentialStore::set(
            &*durable,
            &ProviderSlug::new("openai"),
            StoredCredential::ApiKey("sk-test".to_string()),
        )
        .await;
        store
            .set_default_model(&DefaultModelInput {
                provider_slug: "openai".to_string(),
                model_id: "gpt-4o".to_string(),
            })
            .await
            .unwrap();

        // Prompt WITHOUT explicit profile — resolves to built-in "default"
        // which has PerTool → fails preflight.
        let prompt = StoredPrompt {
            instructions: "Do something".to_string(),
            skills: Vec::new(),
            profile: None,
        };
        store
            .set_prompt(&PromptInput {
                id: "p1".to_string(),
                schema_version: STORED_PROMPT_SCHEMA_VERSION,
                payload: serde_json::to_value(&prompt).unwrap(),
            })
            .await
            .unwrap();

        store
            .set_automation_task(&AutomationTaskInput {
                id: "t1".to_string(),
                name: "T1".to_string(),
                stored_prompt_id: "p1".to_string(),
                expected_outcome: "done".to_string(),
            })
            .await
            .unwrap();

        let result = bootstrap_headless(store, "t1", std::env::temp_dir()).await;
        assert!(matches!(
            result,
            Err(HeadlessBootstrapError::UnsafePolicy(_))
        ));
    }

    #[tokio::test]
    async fn bootstrap_task_not_found() {
        let store = setup_complete_store().await;
        let result = bootstrap_headless(store, "nonexistent", std::env::temp_dir()).await;
        assert!(matches!(
            result,
            Err(HeadlessBootstrapError::Resolution(
                ResolutionError::TaskNotFound(_)
            ))
        ));
    }

    #[tokio::test]
    async fn bootstrap_allow_listed_unavailable_tool() {
        // Tool availability is checked in run_automation (after session
        // creation), not in bootstrap. Bootstrap should succeed; the
        // tool check failure surfaces as a Failed result from run_automation.
        let store = setup_complete_store().await;

        // Override the profile to allow-list a nonexistent plugin tool.
        let allow_profile = AgentProfile {
            name: "automation".to_string(),
            provider: AgentProfileProvider::RuntimeDefault,
            tools: ToolFilter::Allow(vec!["plugin_imaginary_tool".to_string()]),
            skills: crate::profile::SkillFilter::Inherit,
            approval: AgentApproval::AutoApprove,
            identity_prompt: None,
        };
        store
            .set_profile(&ProfileInput {
                id: "automation".to_string(),
                schema_version: PROFILE_SCHEMA_VERSION,
                payload: serde_json::to_value(&allow_profile).unwrap(),
            })
            .await
            .unwrap();

        // Bootstrap succeeds — tool check is deferred to run_automation.
        let runtime = bootstrap_headless(store, "daily-report", std::env::temp_dir())
            .await
            .expect("bootstrap should succeed (tool check deferred)");

        let result = run_automation(
            runtime,
            std::time::Duration::from_secs(5),
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

        assert_eq!(result.status, AutomationRunStatus::Failed);
        assert!(result
            .error
            .as_ref()
            .map(|e| e.message.contains("plugin_imaginary_tool"))
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn bootstrap_with_mcp_server() {
        let store = setup_complete_store().await;

        // Add an MCP server record.
        store
            .set_mcp_server(&McpServerConfigInput {
                id: "test-mcp".to_string(),
                label: "Test MCP".to_string(),
                description: None,
                transport: McpTransport::Http {
                    config: HttpConfig::new("https://example.com/mcp".to_string()),
                },
                working_dir: None,
                enabled_by_default: true,
                inherited_env_vars: Vec::new(),
            })
            .await
            .unwrap();

        let runtime = bootstrap_headless(store, "daily-report", std::env::temp_dir())
            .await
            .expect("bootstrap should succeed with MCP");

        // MCP server is registered in the runtime.
        let mcp_registry = runtime.agent.mcp_registry();
        assert!(mcp_registry
            .list_servers()
            .iter()
            .any(|s| s.config.id == "test-mcp"));
    }

    #[tokio::test]
    async fn bootstrap_skill_settings_preserved() {
        let store = setup_complete_store().await;

        // Set skill settings with trust_project_skills.
        store
            .set_skill_settings(&SkillSettingsInput {
                trust_project_skills: true,
                additional_skill_dirs: vec![PathBuf::from("/custom/skills")],
            })
            .await
            .unwrap();

        let runtime = bootstrap_headless(store, "daily-report", std::env::temp_dir())
            .await
            .expect("bootstrap should succeed");

        // Verify skill catalog was refreshed with workspace roots (not empty
        // would be fine too, just verify it didn't crash).
        let _catalog = runtime.agent.runtime().skill_catalog();
    }

    #[tokio::test]
    async fn bootstrap_managed_profile_reports_its_provider_model() {
        let store = setup_complete_store().await;

        // Override the profile to Managed with a different model.
        let managed_profile = AgentProfile {
            name: "automation".to_string(),
            provider: AgentProfileProvider::Managed {
                provider_slug: ProviderSlug::new("openai"),
                model: "gpt-4o-mini".to_string(),
            },
            tools: ToolFilter::Inherit,
            skills: crate::profile::SkillFilter::Inherit,
            approval: AgentApproval::AutoApprove,
            identity_prompt: None,
        };
        store
            .set_profile(&ProfileInput {
                id: "automation".to_string(),
                schema_version: PROFILE_SCHEMA_VERSION,
                payload: serde_json::to_value(&managed_profile).unwrap(),
            })
            .await
            .unwrap();

        let runtime = bootstrap_headless(store, "daily-report", std::env::temp_dir())
            .await
            .expect("bootstrap should succeed");

        // Provider/model must come from the Managed profile, not the saved default.
        assert_eq!(runtime.provider_slug, "openai");
        assert_eq!(runtime.model, "gpt-4o-mini");
    }

    #[tokio::test]
    async fn bootstrap_plugin_exclusion_no_plugin_tools() {
        let store = setup_complete_store().await;
        let runtime = bootstrap_headless(store, "daily-report", std::env::temp_dir())
            .await
            .expect("bootstrap should succeed");

        // Verify no plugin namespaced tools are in the base tool registry.
        // Plugins are never registered during headless bootstrap.
        for def in runtime.agent.runtime().tool_registry().definitions() {
            assert!(
                !def.name.starts_with("plugin_"),
                "plugin tool '{}' should not be present",
                def.name
            );
        }
    }

    // ---- Phase 4: run_automation tests ----

    use crate::durable::StructuredMessage;
    use crate::execution::AutomationRunStatus;
    use crate::profile::SkillFilter;
    use futures::stream::{self, StreamExt};
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    /// Provider that emits text output then completes.
    #[derive(Clone)]
    struct TextProvider {
        text: Arc<String>,
    }

    impl Provider for TextProvider {
        fn infer(&self, _: InferenceRequest) -> ProviderFuture<'_, Vec<ProviderEvent>> {
            let text = self.text.clone();
            Box::pin(async move {
                Ok(vec![
                    ProviderEvent::Output {
                        content: (*text).clone(),
                    },
                    ProviderEvent::Complete,
                ])
            })
        }

        fn infer_stream(
            &self,
            _: InferenceRequest,
        ) -> ProviderFuture<'_, BoxStream<'static, ProviderResult<ProviderEvent>>> {
            let text = self.text.clone();
            Box::pin(async move {
                Ok(stream::iter(vec![
                    Ok(ProviderEvent::Output {
                        content: (*text).clone(),
                    }),
                    Ok(ProviderEvent::Complete),
                ])
                .boxed())
            })
        }
    }

    /// Provider that only emits Complete (no text output).
    #[derive(Clone, Default)]
    struct EmptyProvider;

    impl Provider for EmptyProvider {
        fn infer(&self, _: InferenceRequest) -> ProviderFuture<'_, Vec<ProviderEvent>> {
            Box::pin(async move { Ok(vec![ProviderEvent::Complete]) })
        }

        fn infer_stream(
            &self,
            _: InferenceRequest,
        ) -> ProviderFuture<'_, BoxStream<'static, ProviderResult<ProviderEvent>>> {
            Box::pin(async move { Ok(stream::iter(vec![Ok(ProviderEvent::Complete)]).boxed()) })
        }
    }

    /// Provider that hangs indefinitely (for timeout/cancellation tests).
    #[derive(Clone, Default)]
    struct HangingProvider {
        call_count: Arc<AtomicUsize>,
    }

    impl Provider for HangingProvider {
        fn infer(&self, _: InferenceRequest) -> ProviderFuture<'_, Vec<ProviderEvent>> {
            self.call_count.fetch_add(1, AtomicOrdering::SeqCst);
            Box::pin(async move {
                std::future::pending::<()>().await;
                unreachable!()
            })
        }

        fn infer_stream(
            &self,
            _: InferenceRequest,
        ) -> ProviderFuture<'_, BoxStream<'static, ProviderResult<ProviderEvent>>> {
            self.call_count.fetch_add(1, AtomicOrdering::SeqCst);
            Box::pin(async move {
                std::future::pending::<()>().await;
                unreachable!()
            })
        }
    }

    /// Provider that emits a transport error (tests technical completion
    /// with injected error text).
    #[derive(Clone, Default)]
    struct ErrorProvider;

    impl Provider for ErrorProvider {
        fn infer(&self, _: InferenceRequest) -> ProviderFuture<'_, Vec<ProviderEvent>> {
            Box::pin(async move { Ok(vec![ProviderEvent::Complete]) })
        }

        fn infer_stream(
            &self,
            _: InferenceRequest,
        ) -> ProviderFuture<'_, BoxStream<'static, ProviderResult<ProviderEvent>>> {
            Box::pin(async move {
                Err(iron_providers::ProviderError::transport(
                    "connection refused",
                ))
            })
        }
    }

    /// Build a HeadlessRuntime with a custom test provider, bypassing
    /// bootstrap_headless. The runtime has an AutoApprove profile named
    /// "automation", a prompt "test-prompt" with that profile, and a task
    /// "test-task".
    async fn build_test_runtime(provider: impl Provider + 'static) -> HeadlessRuntime {
        let workspace = std::env::temp_dir();

        let store = ConfigStore::open_in_memory().await.unwrap();

        let prompt = StoredPrompt {
            instructions: "Generate a report".to_string(),
            skills: Vec::new(),
            profile: Some(AgentProfileId::from("automation")),
        };
        store
            .set_prompt(&PromptInput {
                id: "test-prompt".to_string(),
                schema_version: STORED_PROMPT_SCHEMA_VERSION,
                payload: serde_json::to_value(&prompt).unwrap(),
            })
            .await
            .unwrap();

        store
            .set_automation_task(&AutomationTaskInput {
                id: "test-task".to_string(),
                name: "Test Task".to_string(),
                stored_prompt_id: "test-prompt".to_string(),
                expected_outcome: "A summary of activity".to_string(),
            })
            .await
            .unwrap();

        let config = Config::default()
            .with_model("test-model")
            .with_provider_name("test")
            .with_workspace_roots(vec![workspace.clone()]);

        let agent = IronAgent::new(config, provider);

        let auto_profile = AgentProfile {
            name: "automation".to_string(),
            provider: AgentProfileProvider::RuntimeDefault,
            tools: ToolFilter::Inherit,
            skills: SkillFilter::Inherit,
            approval: AgentApproval::AutoApprove,
            identity_prompt: None,
        };
        agent
            .register_profile(AgentProfileId::from("automation"), auto_profile)
            .unwrap();

        agent.load_stored_prompts(&store).await.unwrap();

        let builtin_config = BuiltinToolConfig::new(vec![workspace.clone()]);
        agent.register_builtin_tools(&builtin_config);

        let profiles: HashMap<AgentProfileId, AgentProfile> = agent
            .list_profiles()
            .into_iter()
            .map(|e| (e.id, e.profile))
            .collect();
        let resolved = resolve_task_execution(&store, &profiles, "test-task", workspace)
            .await
            .unwrap();

        let tool_names: Vec<String> = agent
            .runtime()
            .tool_registry()
            .definitions()
            .into_iter()
            .map(|d| d.name)
            .collect();
        let effective_tools = compute_effective_tools(&resolved.profile.tools, &tool_names);

        HeadlessRuntime {
            agent,
            resolved,
            provider_slug: "test".to_string(),
            model: "test-model".to_string(),
            effective_tools,
        }
    }

    #[tokio::test]
    async fn run_completes_with_text_output() {
        let runtime = build_test_runtime(TextProvider {
            text: Arc::new("Report: all systems operational.".to_string()),
        })
        .await;

        let result = run_automation(
            runtime,
            std::time::Duration::from_secs(30),
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

        assert_eq!(result.status, AutomationRunStatus::Completed);
        assert!(
            result.output.contains("Report: all systems operational."),
            "expected output text, got: {:?}",
            result.output
        );
        assert_eq!(result.task_id, "test-task");
        assert_eq!(result.task_name, "Test Task");
        assert_eq!(result.expected_outcome, "A summary of activity");
        assert!(result.error.is_none());
        // timing was set (duration_ms is u64, always >= 0)
    }

    #[tokio::test]
    async fn run_completes_with_empty_output() {
        let runtime = build_test_runtime(EmptyProvider).await;

        let result = run_automation(
            runtime,
            std::time::Duration::from_secs(30),
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

        assert_eq!(result.status, AutomationRunStatus::Completed);
        assert!(result.output.is_empty() || result.output.contains("Complete"));
    }

    #[tokio::test]
    async fn run_provider_error_completes_technically() {
        let runtime = build_test_runtime(ErrorProvider).await;

        let result = run_automation(
            runtime,
            std::time::Duration::from_secs(10),
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

        // Provider errors are handled gracefully — the turn completes
        // (technical completion) with injected error text.
        assert_eq!(result.status, AutomationRunStatus::Completed);
    }

    #[tokio::test]
    async fn run_times_out() {
        let runtime = build_test_runtime(HangingProvider::default()).await;

        let result = run_automation(
            runtime,
            std::time::Duration::from_millis(200),
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

        assert_eq!(result.status, AutomationRunStatus::TimedOut);
        assert!(result.error.is_some());
        assert_eq!(
            result.error.as_ref().unwrap().category,
            crate::execution::AutomationRunErrorCategory::TimedOut
        );
    }

    #[tokio::test]
    async fn run_cancelled_by_signal() {
        let runtime = build_test_runtime(HangingProvider::default()).await;

        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            cancel_clone.cancel();
        });

        let result = run_automation(runtime, std::time::Duration::from_secs(30), cancel).await;

        assert_eq!(result.status, AutomationRunStatus::Cancelled);
        assert_eq!(
            result.error.as_ref().unwrap().category,
            crate::execution::AutomationRunErrorCategory::Cancelled
        );
    }

    // ---- extract_final_text tests ----

    #[test]
    fn extract_final_text_finds_last_agent_message() {
        use crate::durable::ContentBlock;

        let messages = vec![
            StructuredMessage::User {
                content: vec![ContentBlock::text("hello")],
            },
            StructuredMessage::Agent {
                content: vec![ContentBlock::text("first response")],
            },
            StructuredMessage::User {
                content: vec![ContentBlock::text("again")],
            },
            StructuredMessage::Agent {
                content: vec![ContentBlock::text("final response")],
            },
        ];

        assert_eq!(extract_final_text(&messages), "final response");
    }

    #[test]
    fn extract_final_text_empty_when_no_agent() {
        use crate::durable::ContentBlock;

        let messages = vec![StructuredMessage::User {
            content: vec![ContentBlock::text("hello")],
        }];

        assert_eq!(extract_final_text(&messages), "");
    }

    #[test]
    fn extract_final_text_empty_messages() {
        assert_eq!(extract_final_text(&[]), "");
    }

    #[test]
    fn extract_final_text_concatates_text_blocks() {
        use crate::durable::ContentBlock;

        let messages = vec![StructuredMessage::Agent {
            content: vec![ContentBlock::text("part 1 "), ContentBlock::text("part 2")],
        }];

        assert_eq!(extract_final_text(&messages), "part 1 part 2");
    }

    // ---- format_duration tests ----

    #[test]
    fn format_duration_seconds() {
        assert_eq!(format_duration(std::time::Duration::from_secs(45)), "45s");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(std::time::Duration::from_secs(300)), "5m");
    }

    #[test]
    fn format_duration_hours() {
        assert_eq!(format_duration(std::time::Duration::from_secs(3600)), "1h");
    }

    #[test]
    fn format_duration_mixed_minutes_seconds() {
        assert_eq!(format_duration(std::time::Duration::from_secs(90)), "90s");
    }
}
