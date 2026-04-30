use iron_core::{
    config::Config,
    prompt::config::PromptCompositionConfig,
    prompt::{
        AdditionalInstructionFile, ClientPromptFragment, PromptAssembler, PromptSectionOwner,
        RepoInstructionConfig, RepoInstructionFamily, RepoInstructionLoader,
        RepoInstructionPayload, RepoInstructionSource, RuntimeContextRenderer, SystemPromptCache,
        SystemPromptInputs, SystemPromptRenderer, PROMPT_SECTION_ORDER,
    },
};
use iron_providers::Message;
use std::path::PathBuf;

fn section_position(prompt: &str, title: &str) -> usize {
    prompt
        .find(title)
        .unwrap_or_else(|| panic!("missing section title: {title}"))
}

fn render_system_prompt<'a>(
    payload: &'a RepoInstructionPayload,
    runtime_context: &'a str,
    provider_guidance: Option<&'a str>,
    client_editing_guidance: Option<&'a str>,
    client_injections: &'a [ClientPromptFragment],
    python_exec_available: bool,
) -> String {
    SystemPromptRenderer::render(&SystemPromptInputs {
        baseline: "BASELINE",
        runtime_context,
        repo_payload: payload,
        additional_inline: &[],
        session_instructions: None,
        skill_instructions: None,
        provider_guidance,
        client_editing_guidance,
        client_injections,
        python_exec_available,
    })
}

fn make_source(filename: &str, content: &str) -> RepoInstructionSource {
    RepoInstructionSource {
        scope: PathBuf::from("/test"),
        filename: filename.to_string(),
        content: content.to_string(),
    }
}

fn make_additional(path: &str, content: &str) -> AdditionalInstructionFile {
    AdditionalInstructionFile {
        path: PathBuf::from(path),
        content: content.to_string(),
    }
}

#[test]
fn prompt_section_ordering_is_fixed() {
    let payload = RepoInstructionPayload::default();
    let result = render_system_prompt(&payload, "RUNTIME", None, None, &[], false);

    let mut last = 0;
    for (idx, section) in PROMPT_SECTION_ORDER.iter().enumerate() {
        let title = format!("## {}. {}", idx + 1, section.metadata().title);
        let pos = section_position(&result, &title);
        assert!(pos >= last, "section out of order: {title}");
        last = pos;
    }
}

#[test]
fn prompt_section_metadata_declares_expected_owners() {
    assert_eq!(PROMPT_SECTION_ORDER.len(), 9);
    assert_eq!(
        PROMPT_SECTION_ORDER[0].metadata().owner,
        PromptSectionOwner::Core
    );
    assert_eq!(
        PROMPT_SECTION_ORDER[6].metadata().owner,
        PromptSectionOwner::Provider
    );
    assert_eq!(
        PROMPT_SECTION_ORDER[8].metadata().owner,
        PromptSectionOwner::Client
    );
}

#[test]
fn prompt_preserves_repo_and_session_content_inside_client_injection() {
    let payload = RepoInstructionPayload {
        sources: vec![make_source("AGENTS.md", "repo content")],
        additional_files: vec![],
    };
    let result = SystemPromptRenderer::render(&SystemPromptInputs {
        baseline: "BASELINE",
        runtime_context: "RUNTIME",
        repo_payload: &payload,
        additional_inline: &["INLINE".to_string()],
        session_instructions: Some("SESSION"),
        skill_instructions: Some("SKILLS"),
        provider_guidance: None,
        client_editing_guidance: None,
        client_injections: &[],
        python_exec_available: false,
    });

    let client_pos = section_position(&result, "## 9. Client Injection");
    let repo_pos = result.find("repo content").unwrap();
    let inline_pos = result.find("INLINE").unwrap();
    let session_pos = result.find("SESSION").unwrap();
    let skill_pos = result.find("SKILLS").unwrap();
    assert!(client_pos < repo_pos);
    assert!(repo_pos < inline_pos);
    assert!(inline_pos < session_pos);
    assert!(session_pos < skill_pos);
}

#[test]
fn provider_guidance_only_appears_in_provider_section() {
    let payload = RepoInstructionPayload::default();
    let result = render_system_prompt(
        &payload,
        "RUNTIME",
        Some("provider fragment"),
        None,
        &[],
        false,
    );
    let provider_pos = section_position(&result, "## 7. Provider-Specific Guidance");
    let communication_pos = section_position(&result, "## 8. Communication & Formatting");
    let fragment_pos = result.find("provider fragment").unwrap();
    assert!(provider_pos < fragment_pos);
    assert!(fragment_pos < communication_pos);
}

#[test]
fn client_editing_guidance_overrides_fallback_without_overriding_core() {
    let payload = RepoInstructionPayload::default();
    let fallback = render_system_prompt(&payload, "RUNTIME", None, None, &[], false);
    assert!(fallback.contains("Make the smallest correct change"));

    let custom = render_system_prompt(
        &payload,
        "RUNTIME",
        None,
        Some("client edit policy"),
        &[],
        false,
    );
    assert!(custom.contains("client edit policy"));
    assert!(!custom.contains("Make the smallest correct change"));
    assert!(custom.contains("## 1. Identity"));
    assert!(custom.contains("## 6. Safety & Destructive Actions"));
}

#[test]
fn client_injection_fragments_render_in_order() {
    let payload = RepoInstructionPayload::default();
    let fragments = vec![
        ClientPromptFragment::titled("First", "alpha"),
        ClientPromptFragment::new("beta"),
    ];
    let result = render_system_prompt(&payload, "RUNTIME", None, None, &fragments, false);
    let first_pos = result.find("### First").unwrap();
    let alpha_pos = result.find("alpha").unwrap();
    let beta_pos = result.find("beta").unwrap();
    assert!(first_pos < alpha_pos);
    assert!(alpha_pos < beta_pos);
}

#[test]
fn repo_instructions_renders_sources_metadata() {
    let payload = RepoInstructionPayload {
        sources: vec![make_source("AGENTS.md", "do good things")],
        additional_files: vec![make_additional("/extra.md", "extra stuff")],
    };
    let result = PromptAssembler::assemble("", &payload, &[], None, None, "");
    assert!(result.contains("<repository_instruction_sources>"));
    assert!(result.contains("AGENTS.md"));
    assert!(result.contains("/extra.md"));
    assert!(result.contains("<file_content path="));
    assert!(result.contains("do good things"));
    assert!(result.contains("extra stuff"));
}

#[test]
fn repo_loader_disabled_returns_empty() {
    let config = RepoInstructionConfig::new().with_enabled(false);
    let result = RepoInstructionLoader::resolve(&config).unwrap();
    assert!(result.sources.is_empty());
    assert!(result.additional_files.is_empty());
}

#[test]
fn repo_family_candidates() {
    assert_eq!(
        RepoInstructionFamily::PreferAgentsFallbackClaude.candidates(),
        &["AGENTS.md", "CLAUDE.md"]
    );
    assert_eq!(
        RepoInstructionFamily::AgentsOnly.candidates(),
        &["AGENTS.md"]
    );
    assert_eq!(
        RepoInstructionFamily::ClaudeOnly.candidates(),
        &["CLAUDE.md"]
    );
}

#[test]
fn prompt_composition_config_defaults() {
    let config = PromptCompositionConfig::default();
    assert!(config.repo_instructions.enabled);
    assert!(config.additional_files.is_empty());
    assert!(config.additional_inline.is_empty());
    assert!(config.provider_guidance.is_none());
    assert!(config.client_editing_guidance.is_none());
    assert!(config.client_injections.is_empty());
    assert!(config.protected_resources.contains(&".git".to_string()));
    assert!(config.protected_resources.contains(&".ssh".to_string()));
    assert!(config.protected_resources.contains(&".env".to_string()));
    assert!(config.protected_resources.contains(&".envrc".to_string()));
}

#[test]
fn runtime_context_includes_protected_resources() {
    let config = Config::default();
    let ctx = RuntimeContextRenderer::render(
        &config,
        None,
        std::path::Path::new("/tmp"),
        &[],
        false,
        false,
    );
    assert!(ctx.contains("<runtime_context>"));
    assert!(ctx.contains("Protected resources"));
    assert!(ctx.contains(".git"));
    assert!(ctx.contains(".ssh"));
    assert!(ctx.contains("Approval strategy: per-tool"));
}

#[test]
fn runtime_context_includes_date_and_platform() {
    let config = Config::default();
    let ctx = RuntimeContextRenderer::render(
        &config,
        None,
        std::path::Path::new("/tmp"),
        &[],
        false,
        false,
    );
    assert!(ctx.contains("Date:"));
    assert!(ctx.contains("Platform:"));
    assert!(ctx.contains("Working directory: /tmp"));
}

#[test]
fn runtime_context_git_repo_flag() {
    let config = Config::default();
    let with_git = RuntimeContextRenderer::render(
        &config,
        None,
        std::path::Path::new("/tmp"),
        &[],
        true,
        false,
    );
    let without_git = RuntimeContextRenderer::render(
        &config,
        None,
        std::path::Path::new("/tmp"),
        &[],
        false,
        false,
    );
    assert!(with_git.contains("Git repository: yes"));
    assert!(!without_git.contains("Git repository: yes"));
}

#[test]
fn runtime_context_python_disabled() {
    let config = Config::default();
    let ctx = RuntimeContextRenderer::render(
        &config,
        None,
        std::path::Path::new("/tmp"),
        &[],
        false,
        false,
    );
    assert!(!ctx.contains("Embedded Python (python_exec)"));
    assert!(!ctx.contains("pip is unavailable"));
}

#[test]
fn runtime_context_python_enabled() {
    let config = Config::default().with_embedded_python_enabled();
    let ctx = RuntimeContextRenderer::render(
        &config,
        None,
        std::path::Path::new("/tmp"),
        &[],
        false,
        true,
    );
    assert!(ctx.contains("Embedded Python (python_exec)"));
    assert!(ctx.contains("pip"));
    assert!(ctx.contains("third-party libraries"));
}

#[test]
fn runtime_context_python_sandbox_boundary() {
    let config = Config::default().with_embedded_python_enabled();
    let ctx = RuntimeContextRenderer::render(
        &config,
        None,
        std::path::Path::new("/tmp"),
        &[],
        false,
        true,
    );
    assert!(
        ctx.contains("Sandbox boundary"),
        "runtime context should have sandbox boundary section"
    );
    assert!(
        ctx.contains("direct OS, filesystem, and network access from Python is unavailable"),
        "runtime context should state direct access is unavailable"
    );
    assert!(
        ctx.contains("tools.<alias>(payload)"),
        "runtime context should mention tools alias"
    );
    assert!(
        ctx.contains("tools.call(name, payload)"),
        "runtime context should mention tools.call"
    );
    assert!(
        ctx.contains("pathlib"),
        "runtime context should mention pathlib as unsupported"
    );
}

#[test]
fn baseline_prompt_mentions_protected_resources() {
    let prompt = iron_core::prompt::BASELINE_PROMPT;
    assert!(prompt.contains("Protected Resources"));
    assert!(prompt.contains("protected resource"));
    assert!(prompt.contains("python_exec"));
}

#[test]
fn baseline_prompt_describes_sandbox_boundary() {
    let prompt = iron_core::prompt::BASELINE_PROMPT;
    assert!(
        prompt.contains("sandboxed"),
        "baseline should describe python_exec as sandboxed"
    );
    assert!(
        prompt.contains("tools.<tool>(payload)"),
        "baseline should mention tools namespace"
    );
    assert!(
        prompt.contains("pathlib"),
        "baseline should mention pathlib as unsupported"
    );
    assert!(
        prompt.contains("os"),
        "baseline should mention os as unsupported"
    );
}

#[test]
fn request_builder_composes_instructions() {
    let config = Config::default();
    let registry = iron_core::ToolRegistry::new();
    let messages: Vec<Message> = vec![];
    let result = iron_core::request_builder::build_inference_request(
        &config,
        &messages,
        Some("session instructions"),
        &registry,
    );
    assert!(result.is_ok());
    let req = result.unwrap();
    assert!(req.instructions.is_some());
    let instr = req.instructions.unwrap();
    assert!(instr.contains("<baseline_instructions>"));
    assert!(instr.contains("session instructions"));
    assert!(instr.contains("<runtime_context>"));
}

#[test]
fn handoff_excludes_repo_payload() {
    use iron_core::context::{ContextManagementConfig, HandoffExporter};
    use iron_core::durable::{DurableSession, SessionId};

    let mut session = DurableSession::new(SessionId::new());
    session.instructions = Some("portable instructions".to_string());
    session.repo_instruction_payload = Some(RepoInstructionPayload {
        sources: vec![make_source("AGENTS.md", "repo only")],
        additional_files: vec![],
    });

    let config = ContextManagementConfig::default();
    let bundle =
        HandoffExporter::export(&session, "test-model", None, vec![], &config, None).unwrap();
    assert_eq!(
        bundle.instructions,
        Some("portable instructions".to_string())
    );
    assert!(!bundle.instructions.as_ref().unwrap().contains("repo only"));
}

#[test]
fn additional_files_loaded_into_payload() {
    let dir = std::env::temp_dir();
    let file_path = dir.join("test_instruction_extra.md");
    std::fs::write(&file_path, "extra instructions content").unwrap();

    let mut payload = RepoInstructionPayload::default();
    let result = RepoInstructionLoader::load_additional_files(
        &mut payload,
        std::slice::from_ref(&file_path),
    );
    assert!(result.is_ok());
    assert_eq!(payload.additional_files.len(), 1);
    assert_eq!(
        payload.additional_files[0].content,
        "extra instructions content"
    );

    let _ = std::fs::remove_file(&file_path);
}

#[test]
fn additional_files_error_on_missing() {
    let mut payload = RepoInstructionPayload::default();
    let result = RepoInstructionLoader::load_additional_files(
        &mut payload,
        &[PathBuf::from("/nonexistent/path/instructions.md")],
    );
    assert!(result.is_err());
}

#[test]
fn prompt_composition_builder_chaining() {
    let config = PromptCompositionConfig::new()
        .with_repo_instructions(RepoInstructionConfig::new().with_enabled(false))
        .with_additional_inline(vec!["inline block".to_string()])
        .with_protected_resources(vec![".secret".to_string()])
        .with_provider_guidance("provider guidance")
        .with_client_editing_guidance("client editing")
        .with_client_injections(vec![ClientPromptFragment::titled("Client", "markdown")]);

    assert!(!config.repo_instructions.enabled);
    assert_eq!(config.additional_inline, vec!["inline block"]);
    assert_eq!(config.protected_resources, vec![".secret"]);
    assert_eq!(
        config.provider_guidance.as_deref(),
        Some("provider guidance")
    );
    assert_eq!(
        config.client_editing_guidance.as_deref(),
        Some("client editing")
    );
    assert_eq!(config.client_injections[0].title.as_deref(), Some("Client"));
    assert_eq!(config.client_injections[0].markdown, "markdown");
}

#[test]
fn runtime_context_uses_first_workspace_root_as_working_dir() {
    let config = Config::default().with_workspace_roots(vec![
        PathBuf::from("/project/alpha"),
        PathBuf::from("/project/beta"),
    ]);
    let ctx = RuntimeContextRenderer::render(
        &config,
        None,
        config.workspace_roots.first().unwrap(),
        &config.workspace_roots,
        false,
        false,
    );
    assert!(ctx.contains("Working directory: /project/alpha"));
    assert!(ctx.contains("Workspace root: /project/alpha"));
    assert!(ctx.contains("Workspace root: /project/beta"));
}

#[test]
fn runtime_context_lists_all_workspace_roots() {
    let config = Config::default().with_workspace_roots(vec![
        PathBuf::from("/a"),
        PathBuf::from("/b"),
        PathBuf::from("/c"),
    ]);
    let ctx = RuntimeContextRenderer::render(
        &config,
        None,
        config.workspace_roots.first().unwrap(),
        &config.workspace_roots,
        false,
        false,
    );
    assert!(ctx.contains("Workspace root: /a"));
    assert!(ctx.contains("Workspace root: /b"));
    assert!(ctx.contains("Workspace root: /c"));
}

#[test]
fn runtime_context_falls_back_to_current_dir_when_no_roots() {
    let config = Config::default();
    assert!(config.workspace_roots.is_empty());
    let cwd = std::env::current_dir().unwrap_or_default();
    let ctx = RuntimeContextRenderer::render(&config, None, &cwd, &[], false, false);
    assert!(ctx.contains(&format!("Working directory: {}", cwd.display())));
    assert!(!ctx.contains("Workspace root:"));
}

#[test]
fn request_builder_uses_configured_workspace_roots() {
    let config = Config::default().with_workspace_roots(vec![PathBuf::from("/configured/root")]);
    let registry = iron_core::ToolRegistry::new();
    let messages: Vec<Message> = vec![];
    let result = iron_core::request_builder::build_inference_request(
        &config,
        &messages,
        Some("session instructions"),
        &registry,
    );
    assert!(result.is_ok());
    let req = result.unwrap();
    let instr = req.instructions.unwrap();
    assert!(
        instr.contains("Working directory: /configured/root"),
        "request builder should use configured workspace root as working directory, got: {}",
        instr
    );
    assert!(
        instr.contains("Workspace root: /configured/root"),
        "request builder should pass workspace roots, got: {}",
        instr
    );
}

#[test]
fn request_builder_uses_sectioned_system_prompt() {
    let prompt_config = PromptCompositionConfig::default()
        .with_provider_guidance("provider guidance")
        .with_client_editing_guidance("client editing")
        .with_client_injections(vec![ClientPromptFragment::new("client fragment")]);
    let config = Config::default().with_prompt_composition(prompt_config);
    let registry = iron_core::ToolRegistry::new();
    let messages: Vec<Message> = vec![];
    let req = iron_core::request_builder::build_inference_request(
        &config,
        &messages,
        Some("session instructions"),
        &registry,
    )
    .unwrap();
    let instr = req.instructions.unwrap();
    assert!(instr.contains("## 1. Identity"));
    assert!(instr.contains("## 7. Provider-Specific Guidance"));
    assert!(instr.contains("provider guidance"));
    assert!(instr.contains("client editing"));
    assert!(instr.contains("client fragment"));
    assert!(instr.contains("session instructions"));
}

#[test]
fn system_prompt_cache_reuses_output_until_inputs_change_or_invalidate() {
    let payload = RepoInstructionPayload::default();
    let mut cache = SystemPromptCache::default();
    let first = cache
        .render(&SystemPromptInputs {
            baseline: "BASELINE",
            runtime_context: "Working directory: /one",
            repo_payload: &payload,
            additional_inline: &[],
            session_instructions: None,
            skill_instructions: None,
            provider_guidance: None,
            client_editing_guidance: None,
            client_injections: &[],
            python_exec_available: false,
        })
        .to_string();
    let second = cache
        .render(&SystemPromptInputs {
            baseline: "BASELINE",
            runtime_context: "Working directory: /one",
            repo_payload: &payload,
            additional_inline: &[],
            session_instructions: None,
            skill_instructions: None,
            provider_guidance: None,
            client_editing_guidance: None,
            client_injections: &[],
            python_exec_available: false,
        })
        .to_string();
    assert_eq!(first, second);

    let changed = cache
        .render(&SystemPromptInputs {
            baseline: "BASELINE",
            runtime_context: "Working directory: /two",
            repo_payload: &payload,
            additional_inline: &[],
            session_instructions: None,
            skill_instructions: None,
            provider_guidance: None,
            client_editing_guidance: None,
            client_injections: &[],
            python_exec_available: false,
        })
        .to_string();
    assert_ne!(first, changed);

    cache.invalidate_working_directory();
    let after_invalidate = cache
        .render(&SystemPromptInputs {
            baseline: "BASELINE",
            runtime_context: "Working directory: /two",
            repo_payload: &payload,
            additional_inline: &[],
            session_instructions: None,
            skill_instructions: None,
            provider_guidance: None,
            client_editing_guidance: None,
            client_injections: &[],
            python_exec_available: false,
        })
        .to_string();
    assert_eq!(changed, after_invalidate);
}

#[test]
fn tool_availability_updates_tool_philosophy() {
    let payload = RepoInstructionPayload::default();
    let without_python = render_system_prompt(&payload, "RUNTIME", None, None, &[], false);
    let with_python = render_system_prompt(&payload, "RUNTIME", None, None, &[], true);
    assert!(without_python.contains("`python_exec` is not currently available"));
    assert!(with_python.contains("`python_exec` is available"));
}

#[test]
fn provider_name_resolves_fragment_from_iron_providers_registry() {
    let config = Config::default()
        .with_provider_name("anthropic")
        .with_model("claude-3-5-sonnet");
    let registry = iron_core::ToolRegistry::new();
    let messages: Vec<Message> = vec![];
    let req =
        iron_core::request_builder::build_inference_request(&config, &messages, None, &registry)
            .unwrap();
    let instr = req.instructions.unwrap();
    assert!(
        instr.contains("## 7. Provider-Specific Guidance"),
        "provider section should exist"
    );
    assert!(
        instr.contains("Anthropic Messages API"),
        "should include resolved anthropic fragment"
    );
}

#[test]
fn provider_name_unknown_falls_back_to_manual_guidance() {
    let prompt_config =
        PromptCompositionConfig::default().with_provider_guidance("manual provider guidance");
    let config = Config::default()
        .with_provider_name("nonexistent-provider")
        .with_prompt_composition(prompt_config);
    let registry = iron_core::ToolRegistry::new();
    let messages: Vec<Message> = vec![];
    let req =
        iron_core::request_builder::build_inference_request(&config, &messages, None, &registry)
            .unwrap();
    let instr = req.instructions.unwrap();
    assert!(instr.contains("manual provider guidance"));
}

#[test]
fn provider_name_overrides_manual_provider_guidance() {
    let prompt_config = PromptCompositionConfig::default()
        .with_provider_guidance("manual guidance that should be ignored");
    let config = Config::default()
        .with_provider_name("anthropic")
        .with_prompt_composition(prompt_config);
    let registry = iron_core::ToolRegistry::new();
    let messages: Vec<Message> = vec![];
    let req =
        iron_core::request_builder::build_inference_request(&config, &messages, None, &registry)
            .unwrap();
    let instr = req.instructions.unwrap();
    assert!(
        !instr.contains("manual guidance that should be ignored"),
        "registry fragment should override manual guidance"
    );
    assert!(
        instr.contains("Anthropic Messages API"),
        "should include resolved anthropic fragment"
    );
}
