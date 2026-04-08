use iron_core::{
    config::Config,
    prompt::config::PromptCompositionConfig,
    prompt::{
        AdditionalInstructionFile, PromptAssembler, RepoInstructionConfig, RepoInstructionFamily,
        RepoInstructionLoader, RepoInstructionPayload, RepoInstructionSource,
        RuntimeContextRenderer,
    },
};
use std::path::PathBuf;

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
fn prompt_layer_ordering_baseline_first() {
    let payload = RepoInstructionPayload::default();
    let result = PromptAssembler::assemble("BASELINE", &payload, &[], Some("SESSION"), "RUNTIME");
    let baseline_pos = result.find("BASELINE").unwrap();
    let session_pos = result.find("SESSION").unwrap();
    let runtime_pos = result.find("RUNTIME").unwrap();
    assert!(baseline_pos < session_pos);
    assert!(session_pos < runtime_pos);
}

#[test]
fn prompt_layer_ordering_repo_between_baseline_and_session() {
    let payload = RepoInstructionPayload {
        sources: vec![make_source("AGENTS.md", "repo content")],
        additional_files: vec![],
    };
    let result = PromptAssembler::assemble("BASELINE", &payload, &[], Some("SESSION"), "RUNTIME");
    let baseline_pos = result.find("BASELINE").unwrap();
    let repo_pos = result.find("repo content").unwrap();
    let session_pos = result.find("SESSION").unwrap();
    let runtime_pos = result.find("RUNTIME").unwrap();
    assert!(baseline_pos < repo_pos);
    assert!(repo_pos < session_pos);
    assert!(session_pos < runtime_pos);
}

#[test]
fn prompt_layer_ordering_inline_between_repo_and_session() {
    let payload = RepoInstructionPayload::default();
    let result = PromptAssembler::assemble(
        "BASELINE",
        &payload,
        &["INLINE".to_string()],
        Some("SESSION"),
        "RUNTIME",
    );
    let baseline_pos = result.find("BASELINE").unwrap();
    let inline_pos = result.find("INLINE").unwrap();
    let session_pos = result.find("SESSION").unwrap();
    assert!(baseline_pos < inline_pos);
    assert!(inline_pos < session_pos);
}

#[test]
fn absent_layers_omitted() {
    let payload = RepoInstructionPayload::default();
    let result = PromptAssembler::assemble("BASELINE", &payload, &[], None, "");
    assert!(result.contains("BASELINE"));
    assert!(!result.contains("<repository_instructions>"));
    assert!(!result.contains("<runtime_context>"));
}

#[test]
fn repo_instructions_renders_sources_metadata() {
    let payload = RepoInstructionPayload {
        sources: vec![make_source("AGENTS.md", "do good things")],
        additional_files: vec![make_additional("/extra.md", "extra stuff")],
    };
    let result = PromptAssembler::assemble("", &payload, &[], None, "");
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
fn baseline_prompt_mentions_protected_resources() {
    let prompt = iron_core::prompt::BASELINE_PROMPT;
    assert!(prompt.contains("Protected Resources"));
    assert!(prompt.contains("protected resource"));
    assert!(prompt.contains("python_exec"));
}

#[test]
fn request_builder_composes_instructions() {
    let config = Config::default();
    let registry = iron_core::ToolRegistry::new();
    let messages: Vec<iron_providers::Message> = vec![];
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
        .with_protected_resources(vec![".secret".to_string()]);

    assert!(!config.repo_instructions.enabled);
    assert_eq!(config.additional_inline, vec!["inline block"]);
    assert_eq!(config.protected_resources, vec![".secret"]);
}
