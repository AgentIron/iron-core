//! Sandbox allowlists and resource limits for embedded-Python execution.

use crate::config::EmbeddedPythonConfig;

/// Builtin/module allowlists and per-run resource limits.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Python builtin names available to scripts.
    pub allowed_builtins: Vec<&'static str>,
    /// Python module names scripts may import.
    pub allowed_modules: Vec<&'static str>,
    /// Maximum accepted Python source length in bytes.
    pub max_source_bytes: usize,
    /// Maximum serialized JSON result length in bytes.
    pub max_result_bytes: usize,
    /// Maximum number of host tool calls initiated by one script.
    pub max_child_calls: usize,
    /// Maximum script execution duration in seconds.
    pub timeout_secs: u64,
}

impl SandboxConfig {
    /// Builds sandbox limits from runtime configuration and the default allowlists.
    pub fn from_config(config: &EmbeddedPythonConfig) -> Self {
        Self {
            allowed_builtins: DEFAULT_ALLOWED_BUILTINS.to_vec(),
            allowed_modules: DEFAULT_ALLOWED_MODULES.to_vec(),
            max_source_bytes: config.max_source_bytes,
            max_result_bytes: config.max_result_bytes,
            max_child_calls: config.max_child_calls,
            timeout_secs: config.max_script_timeout_secs,
        }
    }
}

static DEFAULT_ALLOWED_BUILTINS: &[&str] = &[
    "abs",
    "all",
    "any",
    "bool",
    "dict",
    "enumerate",
    "filter",
    "float",
    "int",
    "isinstance",
    "len",
    "list",
    "map",
    "max",
    "min",
    "print",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "sorted",
    "str",
    "sum",
    "tuple",
    "type",
    "zip",
    "None",
    "True",
    "False",
];

static DEFAULT_ALLOWED_MODULES: &[&str] = &["math", "json", "re"];

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            allowed_builtins: DEFAULT_ALLOWED_BUILTINS.to_vec(),
            allowed_modules: DEFAULT_ALLOWED_MODULES.to_vec(),
            max_source_bytes: 32 * 1024,
            max_result_bytes: 64 * 1024,
            max_child_calls: 20,
            timeout_secs: 30,
        }
    }
}
