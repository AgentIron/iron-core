use crate::{
    config::{Config, ContextWindowPolicy},
    error::LoopError,
    tool::ToolRegistry,
};
use iron_providers::{InferenceRequest, Message, ToolPolicy};

pub fn build_inference_request(
    config: &Config,
    messages: &[Message],
    instructions: Option<&str>,
    tool_registry: &ToolRegistry,
) -> Result<InferenceRequest, LoopError> {
    build_inference_request_with_repo(config, messages, instructions, None, tool_registry)
}

pub fn build_inference_request_with_repo(
    config: &Config,
    messages: &[Message],
    instructions: Option<&str>,
    repo_instruction_payload: Option<&crate::prompt::config::RepoInstructionPayload>,
    tool_registry: &ToolRegistry,
) -> Result<InferenceRequest, LoopError> {
    let mut pruned = messages.to_vec();
    apply_context_window_policy(config, &mut pruned)?;

    let transcript = iron_providers::Transcript::with_messages(pruned);

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
    );
    if !composed.is_empty() {
        request = request.with_instructions(composed);
    }

    Ok(request)
}

fn build_composed_instructions(
    config: &Config,
    session_instructions: Option<&str>,
    repo_instruction_payload: Option<&crate::prompt::config::RepoInstructionPayload>,
    python_exec_available: bool,
) -> String {
    let baseline = crate::prompt::baseline::BASELINE_PROMPT;

    let repo_payload = repo_instruction_payload.cloned().unwrap_or_default();

    let working_dir = std::env::current_dir().unwrap_or_default();
    let is_git_repo = working_dir.join(".git").exists();

    let runtime_context = crate::prompt::RuntimeContextRenderer::render(
        config,
        None,
        &working_dir,
        &[],
        is_git_repo,
        python_exec_available,
    );

    crate::prompt::PromptAssembler::assemble(
        baseline,
        &repo_payload,
        &config.prompt_composition.additional_inline,
        session_instructions,
        &runtime_context,
    )
}

fn apply_context_window_policy(
    config: &Config,
    messages: &mut Vec<Message>,
) -> Result<(), LoopError> {
    match config.context_window_policy {
        ContextWindowPolicy::KeepAll => Ok(()),
        ContextWindowPolicy::KeepRecent(count) => {
            if messages.len() > count {
                let start = messages.len() - count;
                *messages = messages.split_off(start);
            }
            Ok(())
        }
        ContextWindowPolicy::SummarizeAfter(_) => Err(LoopError::invalid_config(
            "ContextWindowPolicy::SummarizeAfter is not implemented",
        )),
    }
}
