use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepoInstructionFamily {
    #[default]
    PreferAgentsFallbackClaude,
    AgentsOnly,
    ClaudeOnly,
}

impl RepoInstructionFamily {
    pub fn candidates(&self) -> &[&str] {
        match self {
            Self::PreferAgentsFallbackClaude => &["AGENTS.md", "CLAUDE.md"],
            Self::AgentsOnly => &["AGENTS.md"],
            Self::ClaudeOnly => &["CLAUDE.md"],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepoInstructionConfig {
    pub enabled: bool,
    pub family: RepoInstructionFamily,
    pub scopes: Vec<PathBuf>,
}

impl Default for RepoInstructionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            family: RepoInstructionFamily::default(),
            scopes: vec![std::env::current_dir().unwrap_or_default()],
        }
    }
}

impl RepoInstructionConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn with_family(mut self, family: RepoInstructionFamily) -> Self {
        self.family = family;
        self
    }

    pub fn with_scopes(mut self, scopes: Vec<PathBuf>) -> Self {
        self.scopes = scopes;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptCompositionConfig {
    pub repo_instructions: RepoInstructionConfig,
    pub additional_files: Vec<PathBuf>,
    pub additional_inline: Vec<String>,
    pub protected_resources: Vec<String>,
}

impl Default for PromptCompositionConfig {
    fn default() -> Self {
        Self {
            repo_instructions: RepoInstructionConfig::default(),
            additional_files: Vec::new(),
            additional_inline: Vec::new(),
            protected_resources: vec![
                ".git".to_string(),
                ".ssh".to_string(),
                ".env".to_string(),
                ".envrc".to_string(),
            ],
        }
    }
}

impl PromptCompositionConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_repo_instructions(mut self, config: RepoInstructionConfig) -> Self {
        self.repo_instructions = config;
        self
    }

    pub fn with_additional_files(mut self, files: Vec<PathBuf>) -> Self {
        self.additional_files = files;
        self
    }

    pub fn with_additional_inline(mut self, blocks: Vec<String>) -> Self {
        self.additional_inline = blocks;
        self
    }

    pub fn with_protected_resources(mut self, resources: Vec<String>) -> Self {
        self.protected_resources = resources;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoInstructionSource {
    pub scope: PathBuf,
    pub filename: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdditionalInstructionFile {
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RepoInstructionPayload {
    pub sources: Vec<RepoInstructionSource>,
    pub additional_files: Vec<AdditionalInstructionFile>,
}
