//! Configuration types for `iron-core`.
//!
//! These types define the main runtime configuration surface for `IronAgent`
//! and `IronRuntime`.
//!
//! Applications that keep their own config type can implement [`ConfigSource`]
//! to project iron-core settings into a validated [`Config`] snapshot at
//! construction time.
//!
//! Projection is a snapshot — later mutations to the caller's config do not
//! affect already-constructed runtimes or agents.

use crate::error::RuntimeError;
use iron_providers::{GenerationConfig, ToolPolicy};

pub use crate::context::config::ContextManagementConfig;
pub use crate::prompt::config::PromptCompositionConfig;

/// Projection trait for caller-owned config types.
///
/// Implement this trait on your application config type to project
/// iron-core settings into a validated library-owned `Config` snapshot.
/// The projection occurs at construction time; later mutations to the
/// caller's config object do not affect already-constructed sessions.
pub trait ConfigSource {
    /// Project a validated `Config` snapshot from this source.
    fn to_config(&self) -> Result<Config, RuntimeError>;
}

/// Runtime configuration for `iron-core`.
///
/// This snapshot is validated before use and then owned by the runtime.
/// Builder-style `with_*` helpers return an updated copy for ergonomic setup.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Maximum number of inference/tool iterations before stopping a prompt.
    pub max_iterations: u32,
    /// Default approval strategy for tool execution.
    pub default_approval_strategy: ApprovalStrategy,
    /// Policy for pruning or retaining transcript history.
    pub context_window_policy: ContextWindowPolicy,
    /// Model identifier passed to the provider on each inference request.
    pub model: String,
    /// Default generation settings applied to every inference request.
    pub default_generation: GenerationConfig,
    /// Default tool policy applied when requests include tools.
    pub default_tool_policy: ToolPolicy,
    /// Context management configuration (compaction, telemetry, handoff).
    pub context_management: ContextManagementConfig,
    /// Embedded Python runtime configuration.
    pub embedded_python: EmbeddedPythonConfig,
    /// Prompt composition configuration (baseline, repo instructions, runtime context).
    pub prompt_composition: PromptCompositionConfig,
    /// MCP configuration
    pub mcp: McpConfig,
    /// Plugin configuration
    pub plugins: PluginConfig,
    /// Skill configuration
    pub skills: SkillConfig,
    /// Workspace roots used to derive the runtime context working directory
    /// and workspace root list. When non-empty, the first root becomes the
    /// primary working directory and all roots are surfaced as workspace roots.
    /// When empty, `std::env::current_dir()` is used as a fallback.
    pub workspace_roots: Vec<std::path::PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            default_approval_strategy: ApprovalStrategy::PerTool,
            context_window_policy: ContextWindowPolicy::default(),
            model: "gpt-4o".to_string(),
            default_generation: GenerationConfig::default(),
            default_tool_policy: ToolPolicy::Auto,
            context_management: ContextManagementConfig::default(),
            embedded_python: EmbeddedPythonConfig::default(),
            prompt_composition: PromptCompositionConfig::default(),
            mcp: McpConfig::default(),
            plugins: PluginConfig::default(),
            skills: SkillConfig::default(),
            workspace_roots: Vec::new(),
        }
    }
}

impl Config {
    /// Create a new configuration using the crate defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum iteration limit for each prompt.
    pub fn with_max_iterations(mut self, max: u32) -> Self {
        self.max_iterations = max;
        self
    }

    /// Set the default approval strategy applied to tool calls.
    pub fn with_approval_strategy(mut self, strategy: ApprovalStrategy) -> Self {
        self.default_approval_strategy = strategy;
        self
    }

    /// Set the context window policy used when building provider requests.
    pub fn with_context_window_policy(mut self, policy: ContextWindowPolicy) -> Self {
        self.context_window_policy = policy;
        self
    }

    /// Set the default model identifier used for inference.
    pub fn with_model<S: Into<String>>(mut self, model: S) -> Self {
        self.model = model.into();
        self
    }

    /// Set the default generation settings for future requests.
    pub fn with_default_generation(mut self, generation: GenerationConfig) -> Self {
        self.default_generation = generation;
        self
    }

    /// Set the default tool policy used when tools are present.
    pub fn with_default_tool_policy(mut self, policy: ToolPolicy) -> Self {
        self.default_tool_policy = policy;
        self
    }

    /// Set the context management configuration.
    pub fn with_context_management(mut self, config: ContextManagementConfig) -> Self {
        self.context_management = config;
        self
    }

    /// Set the embedded Python runtime configuration.
    pub fn with_embedded_python(mut self, config: EmbeddedPythonConfig) -> Self {
        self.embedded_python = config;
        self
    }

    /// Set the prompt composition configuration.
    pub fn with_prompt_composition(mut self, config: PromptCompositionConfig) -> Self {
        self.prompt_composition = config;
        self
    }

    /// Enable the embedded Python runtime with its default limits.
    pub fn with_embedded_python_enabled(mut self) -> Self {
        self.embedded_python.enabled = true;
        self
    }

    /// Set the MCP configuration.
    pub fn with_mcp(mut self, mcp: McpConfig) -> Self {
        self.mcp = mcp;
        self
    }

    /// Set the plugin configuration.
    pub fn with_plugins(mut self, plugins: PluginConfig) -> Self {
        self.plugins = plugins;
        self
    }

    /// Set the skill configuration.
    pub fn with_skills(mut self, skills: SkillConfig) -> Self {
        self.skills = skills;
        self
    }

    /// Set the workspace roots used for runtime context rendering.
    pub fn with_workspace_roots(mut self, roots: Vec<std::path::PathBuf>) -> Self {
        self.workspace_roots = roots;
        self
    }

    /// Validate this config, returning an error if required fields are missing
    /// or generation constraints are out of range.
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.model.trim().is_empty() {
            return Err(RuntimeError::invalid_config(
                "Config model is required but was empty",
            ));
        }
        if self.max_iterations == 0 {
            return Err(RuntimeError::invalid_config(
                "Config max_iterations must be greater than 0",
            ));
        }
        if let Some(temp) = self.default_generation.temperature {
            if !(0.0..=2.0).contains(&temp) {
                return Err(RuntimeError::invalid_config(format!(
                    "Config default temperature must be between 0.0 and 2.0, got {}",
                    temp
                )));
            }
        }
        self.context_management
            .validate()
            .map_err(RuntimeError::invalid_config)?;
        if self.embedded_python.enabled {
            self.embedded_python.validate()?;
        }
        Ok(())
    }
}

/// Strategy for deciding whether tool execution requires explicit approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApprovalStrategy {
    /// Always require human approval.
    Always,
    /// Never require human approval.
    Never,
    /// Defer to the tool's `requires_approval` setting.
    #[default]
    PerTool,
}

impl ApprovalStrategy {
    /// Check if approval is required for the given tool setting
    pub fn is_approval_required(self, tool_requires_approval: bool) -> bool {
        match self {
            ApprovalStrategy::Always => true,
            ApprovalStrategy::Never => false,
            ApprovalStrategy::PerTool => tool_requires_approval,
        }
    }
}

/// Policy for retaining transcript history as the conversation grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContextWindowPolicy {
    /// Keep all messages.
    #[default]
    KeepAll,
    /// Keep only the most recent `N` messages.
    KeepRecent(usize),
}

impl ContextWindowPolicy {
    /// Apply this policy to a message list
    pub fn apply<T>(&self, messages: &mut Vec<T>, _summarize_fn: impl FnOnce(&[T]) -> T) {
        match self {
            ContextWindowPolicy::KeepAll => {
                // No action needed
            }
            ContextWindowPolicy::KeepRecent(n) => {
                if messages.len() > *n {
                    let start = messages.len() - *n;
                    *messages = messages.split_off(start);
                }
            }
        }
    }
}

/// Configuration for the embedded Python runtime.
///
/// `iron-core` keeps the Monty-backed `python_exec` runtime in-tree.
/// Publishing this crate to crates.io is deferred until `monty` is
/// available on crates.io.
///
/// These limits control source size, result size, timeout, and child tool fan-out.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddedPythonConfig {
    /// Whether embedded Python execution is enabled.
    pub enabled: bool,
    /// Maximum wall-clock time for a script run in seconds.
    pub max_script_timeout_secs: u64,
    /// Maximum accepted source code size in bytes.
    pub max_source_bytes: usize,
    /// Maximum serialized result payload size in bytes.
    pub max_result_bytes: usize,
    /// Maximum number of child tool calls per script run.
    pub max_child_calls: usize,
    /// Maximum number of items returned in child call outcomes.
    pub max_child_outcome_items: usize,
}

impl Default for EmbeddedPythonConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_script_timeout_secs: 30,
            max_source_bytes: 32 * 1024,
            max_result_bytes: 64 * 1024,
            max_child_calls: 20,
            max_child_outcome_items: 20,
        }
    }
}

impl EmbeddedPythonConfig {
    /// Create a new configuration using the default embedded-Python limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable embedded Python execution.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set the wall-clock timeout for each script run in seconds.
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.max_script_timeout_secs = secs;
        self
    }

    /// Set the maximum accepted source size in bytes.
    pub fn with_max_source_bytes(mut self, bytes: usize) -> Self {
        self.max_source_bytes = bytes;
        self
    }

    /// Set the maximum serialized result size in bytes.
    pub fn with_max_result_bytes(mut self, bytes: usize) -> Self {
        self.max_result_bytes = bytes;
        self
    }

    /// Set the maximum number of child tool calls allowed per script.
    pub fn with_max_child_calls(mut self, n: usize) -> Self {
        self.max_child_calls = n;
        self
    }

    /// Validate the embedded Python configuration.
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.max_script_timeout_secs == 0 {
            return Err(RuntimeError::invalid_config(
                "EmbeddedPythonConfig max_script_timeout_secs must be > 0",
            ));
        }
        if self.max_source_bytes == 0 {
            return Err(RuntimeError::invalid_config(
                "EmbeddedPythonConfig max_source_bytes must be > 0",
            ));
        }
        if self.max_result_bytes == 0 {
            return Err(RuntimeError::invalid_config(
                "EmbeddedPythonConfig max_result_bytes must be > 0",
            ));
        }
        if self.max_child_calls == 0 {
            return Err(RuntimeError::invalid_config(
                "EmbeddedPythonConfig max_child_calls must be > 0",
            ));
        }
        Ok(())
    }
}

/// Configuration for MCP (Model Context Protocol) servers.
///
/// This configuration controls MCP server behavior and defaults for new sessions.
#[derive(Debug, Clone, PartialEq)]
pub struct McpConfig {
    /// Whether MCP is enabled globally for this runtime.
    pub enabled: bool,
    /// Whether configured MCP servers are enabled by default for new sessions.
    /// If false, new sessions start with all MCP servers disabled.
    pub enabled_by_default: bool,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            enabled_by_default: true,
        }
    }
}

impl McpConfig {
    /// Create a new MCP configuration using defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable MCP support globally.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set whether MCP servers are enabled by default for new sessions.
    pub fn with_enabled_by_default(mut self, enabled_by_default: bool) -> Self {
        self.enabled_by_default = enabled_by_default;
        self
    }
}

/// Configuration for WASM integration plugins.
///
/// This configuration controls plugin behavior and defaults for new sessions.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginConfig {
    /// Whether plugins are enabled globally for this runtime.
    pub enabled: bool,
    /// Whether configured plugins are enabled by default for new sessions.
    /// If false, new sessions start with all plugins disabled.
    pub enabled_by_default: bool,
    /// Base directory for caching plugin artifacts
    pub artifact_cache_dir: std::path::PathBuf,
    /// Maximum WASM linear memory per plugin, in bytes. Applied as a ceiling —
    /// if a plugin's manifest declares a smaller limit, the smaller limit wins.
    /// Defaults to 128 MB.
    pub max_memory_bytes: u64,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            enabled_by_default: true,
            artifact_cache_dir: std::path::PathBuf::from("./plugin_cache"),
            max_memory_bytes: crate::plugin::wasm_host::DEFAULT_PLUGIN_MAX_MEMORY_BYTES,
        }
    }
}

impl PluginConfig {
    /// Create a new plugin configuration using defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable plugin support globally.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set whether plugins are enabled by default for new sessions.
    pub fn with_enabled_by_default(mut self, enabled_by_default: bool) -> Self {
        self.enabled_by_default = enabled_by_default;
        self
    }

    /// Set the artifact cache directory.
    pub fn with_artifact_cache_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.artifact_cache_dir = dir;
        self
    }

    /// Set the per-plugin WASM memory ceiling, in bytes.
    pub fn with_max_memory_bytes(mut self, bytes: u64) -> Self {
        self.max_memory_bytes = bytes;
        self
    }
}

/// Configuration for agent skills.
///
/// Skills are declarative instruction sets that models can discover and activate.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillConfig {
    /// Whether skills are enabled globally for this runtime.
    pub enabled: bool,
    /// Whether to trust project-level skills discovered from the workspace.
    /// When false, project-level skills are hidden from the catalog.
    pub trust_project_skills: bool,
    /// Directories to scan for skills in addition to defaults.
    pub additional_skill_dirs: Vec<std::path::PathBuf>,
}

impl Default for SkillConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trust_project_skills: false,
            additional_skill_dirs: Vec::new(),
        }
    }
}

impl SkillConfig {
    /// Create a new skill configuration using defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable skill support globally.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set whether to trust project-level skills.
    pub fn with_trust_project_skills(mut self, trust: bool) -> Self {
        self.trust_project_skills = trust;
        self
    }

    /// Add an additional skill directory to scan.
    pub fn with_additional_skill_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.additional_skill_dirs.push(dir);
        self
    }
}
