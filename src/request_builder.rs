use crate::{
    config::{Config, ContextWindowPolicy},
    context::models::CompressedBlock,
    error::RuntimeError,
    tool::ToolRegistry,
};
use iron_providers::{InferenceRequest, Message, ProviderRegistry, ToolPolicy};

pub struct EffectiveToolRequestContext<'a> {
    pub compressed_blocks: &'a [CompressedBlock],
    pub instructions: Option<&'a str>,
    pub repo_instruction_payload: Option<&'a crate::prompt::config::RepoInstructionPayload>,
    pub python_exec_available: bool,
    pub skill_instructions: Option<&'a str>,
    pub context_pressure: crate::context::ContextPressure,
    /// Optional session workspace root snapshot. When provided, this overrides
    /// Config.workspace_roots for runtime context rendering.
    pub workspace_roots: Option<&'a [std::path::PathBuf]>,
    /// When set, overrides `config.model` in the inference request.
    /// Populated from `DurableSession::current_model` after a model switch.
    pub effective_model: Option<&'a str>,
    /// Selected profile identity prompt for the executing agent profile.
    /// When present and non-empty, this is rendered in `## 1. Identity`.
    pub profile_identity: Option<&'a str>,
    /// Effective provider guidance resolved from the effective provider
    /// profile. When present, this overrides the built-in registry lookup
    /// for `## 7. Provider-Specific Guidance`.
    pub effective_provider_guidance: Option<&'a str>,
}

struct ComposedInstructionInputs<'a> {
    session_instructions: Option<&'a str>,
    repo_instruction_payload: Option<&'a crate::prompt::config::RepoInstructionPayload>,
    python_exec_available: bool,
    skill_instructions: Option<&'a str>,
    context_pressure: crate::context::ContextPressure,
    workspace_roots: Option<&'a [std::path::PathBuf]>,
    profile_identity: Option<&'a str>,
    effective_provider_guidance: Option<&'a str>,
}

/// Build an inference request using an effective tool view.
/// This allows MCP tools to be included based on session state.
pub fn build_inference_request_with_effective_tools(
    config: &Config,
    messages: &[Message],
    context: EffectiveToolRequestContext<'_>,
    effective_tools: &[crate::tool::ToolDefinition],
) -> Result<InferenceRequest, RuntimeError> {
    let mut pruned = messages.to_vec();
    apply_context_window_policy(config, &mut pruned)?;

    let mut provider_messages = Vec::new();
    for block in context.compressed_blocks {
        provider_messages.push(Message::Assistant {
            content: block.render_to_text(),
        });
    }
    provider_messages.extend(pruned);

    let transcript = iron_providers::Transcript::with_messages(provider_messages);

    let tool_policy = if effective_tools.is_empty() {
        ToolPolicy::None
    } else {
        config.default_tool_policy.clone()
    };

    // Convert to provider tool definitions
    let provider_tools: Vec<iron_providers::ToolDefinition> = effective_tools
        .iter()
        .map(|t| t.to_provider_definition())
        .collect();

    let model = context
        .effective_model
        .map(|m| m.to_string())
        .unwrap_or_else(|| config.model.clone());
    let mut request = InferenceRequest::new(model, transcript)
        .with_tools(provider_tools)
        .with_tool_policy(tool_policy)
        .with_generation(config.default_generation.clone());

    let composed = build_composed_instructions(
        config,
        ComposedInstructionInputs {
            session_instructions: context.instructions,
            repo_instruction_payload: context.repo_instruction_payload,
            python_exec_available: context.python_exec_available,
            skill_instructions: context.skill_instructions,
            context_pressure: context.context_pressure,
            workspace_roots: context.workspace_roots,
            profile_identity: context.profile_identity,
            effective_provider_guidance: context.effective_provider_guidance,
        },
    );
    if !composed.is_empty() {
        request = request.with_instructions(composed);
    }

    Ok(request)
}

pub fn build_inference_request(
    config: &Config,
    messages: &[Message],
    instructions: Option<&str>,
    tool_registry: &ToolRegistry,
) -> Result<InferenceRequest, RuntimeError> {
    build_inference_request_with_context_and_repo(
        config,
        messages,
        EffectiveToolRequestContext {
            compressed_blocks: &[],
            instructions,
            repo_instruction_payload: None,
            python_exec_available: tool_registry.contains("python_exec"),
            skill_instructions: None,
            context_pressure: crate::context::ContextPressure::None,
            workspace_roots: None,
            effective_model: None,
            profile_identity: None,
            effective_provider_guidance: None,
        },
        tool_registry,
    )
}

pub fn build_inference_request_with_context(
    config: &Config,
    messages: &[Message],
    compressed_blocks: &[CompressedBlock],
    instructions: Option<&str>,
    tool_registry: &ToolRegistry,
) -> Result<InferenceRequest, RuntimeError> {
    build_inference_request_with_context_and_repo(
        config,
        messages,
        EffectiveToolRequestContext {
            compressed_blocks,
            instructions,
            repo_instruction_payload: None,
            python_exec_available: tool_registry.contains("python_exec"),
            skill_instructions: None,
            context_pressure: crate::context::ContextPressure::None,
            workspace_roots: None,
            effective_model: None,
            profile_identity: None,
            effective_provider_guidance: None,
        },
        tool_registry,
    )
}

pub fn build_inference_request_with_repo(
    config: &Config,
    messages: &[Message],
    instructions: Option<&str>,
    repo_instruction_payload: Option<&crate::prompt::config::RepoInstructionPayload>,
    tool_registry: &ToolRegistry,
) -> Result<InferenceRequest, RuntimeError> {
    build_inference_request_with_context_and_repo(
        config,
        messages,
        EffectiveToolRequestContext {
            compressed_blocks: &[],
            instructions,
            repo_instruction_payload,
            python_exec_available: tool_registry.contains("python_exec"),
            skill_instructions: None,
            context_pressure: crate::context::ContextPressure::None,
            workspace_roots: None,
            effective_model: None,
            profile_identity: None,
            effective_provider_guidance: None,
        },
        tool_registry,
    )
}

pub fn build_inference_request_with_context_and_repo(
    config: &Config,
    messages: &[Message],
    context: EffectiveToolRequestContext<'_>,
    tool_registry: &ToolRegistry,
) -> Result<InferenceRequest, RuntimeError> {
    let mut pruned = messages.to_vec();
    apply_context_window_policy(config, &mut pruned)?;

    let mut provider_messages = Vec::new();
    for block in context.compressed_blocks {
        provider_messages.push(Message::Assistant {
            content: block.render_to_text(),
        });
    }
    provider_messages.extend(pruned);

    let transcript = iron_providers::Transcript::with_messages(provider_messages);

    let tool_policy = if tool_registry.is_empty() {
        ToolPolicy::None
    } else {
        config.default_tool_policy.clone()
    };

    let model = context
        .effective_model
        .map(|m| m.to_string())
        .unwrap_or_else(|| config.model.clone());
    let mut request = InferenceRequest::new(model, transcript)
        .with_tools(tool_registry.provider_definitions())
        .with_tool_policy(tool_policy)
        .with_generation(config.default_generation.clone());

    let python_exec_available = tool_registry.contains("python_exec");

    let composed = build_composed_instructions(
        config,
        ComposedInstructionInputs {
            session_instructions: context.instructions,
            repo_instruction_payload: context.repo_instruction_payload,
            python_exec_available: context.python_exec_available || python_exec_available,
            skill_instructions: context.skill_instructions,
            context_pressure: context.context_pressure,
            workspace_roots: context.workspace_roots,
            profile_identity: context.profile_identity,
            effective_provider_guidance: context.effective_provider_guidance,
        },
    );
    if !composed.is_empty() {
        request = request.with_instructions(composed);
    }

    Ok(request)
}

fn build_composed_instructions(config: &Config, inputs: ComposedInstructionInputs<'_>) -> String {
    let baseline = crate::prompt::baseline::BASELINE_PROMPT;

    let repo_payload = inputs.repo_instruction_payload.cloned().unwrap_or_default();

    let (working_dir, workspace_roots) = if let Some(roots) = inputs.workspace_roots {
        if roots.is_empty() {
            (std::env::current_dir().unwrap_or_default(), Vec::new())
        } else {
            (roots[0].clone(), roots.to_vec())
        }
    } else if config.workspace_roots.is_empty() {
        (std::env::current_dir().unwrap_or_default(), Vec::new())
    } else {
        (
            config.workspace_roots[0].clone(),
            config.workspace_roots.clone(),
        )
    };
    let is_git_repo = working_dir.join(".git").exists();

    let runtime_context = crate::prompt::RuntimeContextRenderer::render(
        config,
        None,
        &working_dir,
        &workspace_roots,
        is_git_repo,
        inputs.python_exec_available,
    );

    let provider_guidance = resolve_provider_guidance(config, inputs.effective_provider_guidance);

    crate::prompt::SystemPromptRenderer::render(&crate::prompt::SystemPromptInputs {
        baseline,
        runtime_context: &runtime_context,
        repo_payload: &repo_payload,
        additional_inline: &config.prompt_composition.additional_inline,
        profile_identity: inputs.profile_identity,
        session_instructions: inputs.session_instructions,
        skill_instructions: inputs.skill_instructions,
        provider_guidance: provider_guidance.as_deref(),
        client_editing_guidance: config.prompt_composition.client_editing_guidance.as_deref(),
        client_injections: &config.prompt_composition.client_injections,
        python_exec_available: inputs.python_exec_available,
        context_pressure: inputs.context_pressure,
    })
}

/// Resolve provider-specific guidance.
///
/// When `effective_guidance` is provided (from the effective provider
/// registry), it takes precedence. Otherwise falls back to the built-in
/// registry lookup by provider name, then to the manually set
/// `provider_guidance` in `PromptCompositionConfig`.
fn resolve_provider_guidance(config: &Config, effective_guidance: Option<&str>) -> Option<String> {
    if let Some(guidance) = effective_guidance {
        return Some(guidance.to_string());
    }
    if let Some(ref name) = config.provider_name {
        let registry = ProviderRegistry::default();
        match registry.system_prompt_fragment(name) {
            Ok(fragment) => return Some(fragment.to_string()),
            Err(_) => {
                // Unknown provider name — fall through to manual guidance
            }
        }
    }
    config.prompt_composition.provider_guidance.clone()
}

fn apply_context_window_policy(
    config: &Config,
    messages: &mut Vec<Message>,
) -> Result<(), RuntimeError> {
    match config.context_window_policy {
        ContextWindowPolicy::KeepAll => Ok(()),
        ContextWindowPolicy::KeepRecent(count) => {
            if messages.len() > count {
                let start = messages.len() - count;
                *messages = messages.split_off(start);
            }
            Ok(())
        }
    }
}
