//! Configuration and data types for composing system prompts.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Selects repository instruction filenames and their precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepoInstructionFamily {
    /// Prefer `AGENTS.md`, falling back to `CLAUDE.md` in each scope.
    #[default]
    PreferAgentsFallbackClaude,
    /// Load only `AGENTS.md` files.
    AgentsOnly,
    /// Load only `CLAUDE.md` files.
    ClaudeOnly,
}

impl RepoInstructionFamily {
    /// Returns candidate filenames in lookup precedence order.
    pub fn candidates(&self) -> &[&str] {
        match self {
            Self::PreferAgentsFallbackClaude => &["AGENTS.md", "CLAUDE.md"],
            Self::AgentsOnly => &["AGENTS.md"],
            Self::ClaudeOnly => &["CLAUDE.md"],
        }
    }
}

/// Controls discovery of repository instruction files.
#[derive(Debug, Clone, PartialEq)]
pub struct RepoInstructionConfig {
    /// Whether repository instruction discovery is enabled.
    pub enabled: bool,
    /// Filename family and precedence used within each scope.
    pub family: RepoInstructionFamily,
    /// Directories searched in order for instruction files.
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
    /// Creates the default enabled repository instruction configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets whether repository instruction discovery is enabled.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Sets the repository instruction filename family.
    pub fn with_family(mut self, family: RepoInstructionFamily) -> Self {
        self.family = family;
        self
    }

    /// Replaces the ordered directories searched for instructions.
    pub fn with_scopes(mut self, scopes: Vec<PathBuf>) -> Self {
        self.scopes = scopes;
        self
    }
}

/// Controls all non-conversational text composed into the system prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptCompositionConfig {
    /// Repository instruction discovery settings.
    pub repo_instructions: RepoInstructionConfig,
    /// Explicit files appended after discovered repository instructions.
    pub additional_files: Vec<PathBuf>,
    /// Inline instruction blocks rendered in their stored order.
    pub additional_inline: Vec<String>,
    /// Resource names or paths the rendered prompt marks as protected.
    pub protected_resources: Vec<String>,
    /// Manual provider guidance used when registry guidance is unavailable.
    pub provider_guidance: Option<String>,
    /// Client override for the editing-guidelines section.
    pub client_editing_guidance: Option<String>,
    /// Client-owned fragments rendered in the client-injection section.
    pub client_injections: Vec<ClientPromptFragment>,
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
            provider_guidance: None,
            client_editing_guidance: None,
            client_injections: Vec::new(),
        }
    }
}

impl PromptCompositionConfig {
    /// Creates the default prompt-composition configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces repository instruction discovery settings.
    pub fn with_repo_instructions(mut self, config: RepoInstructionConfig) -> Self {
        self.repo_instructions = config;
        self
    }

    /// Replaces the explicit additional instruction files.
    pub fn with_additional_files(mut self, files: Vec<PathBuf>) -> Self {
        self.additional_files = files;
        self
    }

    /// Replaces inline instruction blocks, preserving the supplied order.
    pub fn with_additional_inline(mut self, blocks: Vec<String>) -> Self {
        self.additional_inline = blocks;
        self
    }

    /// Replaces the protected-resource labels rendered into runtime context.
    pub fn with_protected_resources(mut self, resources: Vec<String>) -> Self {
        self.protected_resources = resources;
        self
    }

    /// Sets fallback provider-specific guidance.
    pub fn with_provider_guidance<S: Into<String>>(mut self, guidance: S) -> Self {
        self.provider_guidance = Some(guidance.into());
        self
    }

    /// Sets the client-owned editing guidance section.
    pub fn with_client_editing_guidance<S: Into<String>>(mut self, guidance: S) -> Self {
        self.client_editing_guidance = Some(guidance.into());
        self
    }

    /// Replaces client prompt fragments, preserving the supplied order.
    pub fn with_client_injections(mut self, fragments: Vec<ClientPromptFragment>) -> Self {
        self.client_injections = fragments;
        self
    }
}

/// A client-owned Markdown fragment injected into the system prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientPromptFragment {
    /// Optional heading rendered before the fragment body.
    pub title: Option<String>,
    /// Markdown body rendered verbatim when non-empty.
    pub markdown: String,
}

impl ClientPromptFragment {
    /// Creates an untitled fragment from Markdown text.
    pub fn new<S: Into<String>>(markdown: S) -> Self {
        Self {
            title: None,
            markdown: markdown.into(),
        }
    }

    /// Creates a fragment with a heading and Markdown body.
    pub fn titled<T: Into<String>, M: Into<String>>(title: T, markdown: M) -> Self {
        Self {
            title: Some(title.into()),
            markdown: markdown.into(),
        }
    }
}

/// A discovered repository instruction file and its loaded contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoInstructionSource {
    /// Directory in which the instruction file was discovered.
    pub scope: PathBuf,
    /// Discovered filename, such as `AGENTS.md`.
    pub filename: String,
    /// UTF-8 file contents.
    pub content: String,
}

/// An explicitly configured instruction file and its loaded contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdditionalInstructionFile {
    /// Configured file path.
    pub path: PathBuf,
    /// UTF-8 file contents.
    pub content: String,
}

/// Loaded repository and additional instruction material.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RepoInstructionPayload {
    /// Discovered repository files in scope order.
    pub sources: Vec<RepoInstructionSource>,
    /// Explicitly loaded files in configuration order.
    pub additional_files: Vec<AdditionalInstructionFile>,
}
