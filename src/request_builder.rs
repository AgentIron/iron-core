use crate::{
    config::{Config, ContextWindowPolicy},
    context::models::CompactedContext,
    error::RuntimeError,
    tool::ToolRegistry,
};
use iron_providers::{InferenceRequest, Message, ToolPolicy};

pub struct EffectiveToolRequestContext<'a> {
    pub compacted_context: Option<&'a CompactedContext>,
    pub instructions: Option<&'a str>,
    pub repo_instruction_payload: Option<&'a crate::prompt::config::RepoInstructionPayload>,
    pub python_exec_available: bool,
    pub skill_instructions: Option<&'a str>,
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
    if let Some(summary) = compacted_context_message(context.compacted_context) {
        provider_messages.push(summary);
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

    let mut request = InferenceRequest::new(config.model.clone(), transcript)
        .with_tools(provider_tools)
        .with_tool_policy(tool_policy)
        .with_generation(config.default_generation.clone());

    let composed = build_composed_instructions(
        config,
        context.instructions,
        context.repo_instruction_payload,
        context.python_exec_available,
        context.skill_instructions,
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
        None,
        instructions,
        None,
        tool_registry,
        None,
    )
}

pub fn build_inference_request_with_context(
    config: &Config,
    messages: &[Message],
    compacted_context: Option<&CompactedContext>,
    instructions: Option<&str>,
    tool_registry: &ToolRegistry,
) -> Result<InferenceRequest, RuntimeError> {
    build_inference_request_with_context_and_repo(
        config,
        messages,
        compacted_context,
        instructions,
        None,
        tool_registry,
        None,
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
        None,
        instructions,
        repo_instruction_payload,
        tool_registry,
        None,
    )
}

pub fn build_inference_request_with_context_and_repo(
    config: &Config,
    messages: &[Message],
    compacted_context: Option<&CompactedContext>,
    instructions: Option<&str>,
    repo_instruction_payload: Option<&crate::prompt::config::RepoInstructionPayload>,
    tool_registry: &ToolRegistry,
    skill_instructions: Option<&str>,
) -> Result<InferenceRequest, RuntimeError> {
    let mut pruned = messages.to_vec();
    apply_context_window_policy(config, &mut pruned)?;

    let mut provider_messages = Vec::new();
    if let Some(summary) = compacted_context_message(compacted_context) {
        provider_messages.push(summary);
    }
    provider_messages.extend(pruned);

    let transcript = iron_providers::Transcript::with_messages(provider_messages);

    let tool_policy = if tool_registry.is_empty() {
        ToolPolicy::None
    } else {
        config.default_tool_policy.clone()
    };

    let mut request = InferenceRequest::new(config.model.clone(), transcript)
        .with_tools(tool_registry.provider_definitions())
        .with_tool_policy(tool_policy)
        .with_generation(config.default_generation.clone());

    let python_exec_available = tool_registry.contains("python_exec");

    let composed = build_composed_instructions(
        config,
        instructions,
        repo_instruction_payload,
        python_exec_available,
        skill_instructions,
    );
    if !composed.is_empty() {
        request = request.with_instructions(composed);
    }

    Ok(request)
}

fn compacted_context_message(compacted_context: Option<&CompactedContext>) -> Option<Message> {
    let rendered = compacted_context?.render_to_text();
    if rendered.is_empty() {
        return None;
    }

    Some(Message::Assistant {
        content: format!("[Compacted session context]\n{}", rendered),
    })
}

fn build_composed_instructions(
    config: &Config,
    session_instructions: Option<&str>,
    repo_instruction_payload: Option<&crate::prompt::config::RepoInstructionPayload>,
    python_exec_available: bool,
    skill_instructions: Option<&str>,
) -> String {
    let baseline = crate::prompt::baseline::BASELINE_PROMPT;

    let repo_payload = repo_instruction_payload.cloned().unwrap_or_default();

    let (working_dir, workspace_roots) = if config.workspace_roots.is_empty() {
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
        python_exec_available,
    );

    crate::prompt::PromptAssembler::assemble(
        baseline,
        &repo_payload,
        &config.prompt_composition.additional_inline,
        session_instructions,
        skill_instructions,
        &runtime_context,
    )
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
