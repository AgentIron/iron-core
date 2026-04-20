use parking_lot::Mutex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::policy::{BuiltinToolPolicy, ShellAvailability};

#[derive(Debug, Clone)]
pub struct BuiltinToolConfig {
    pub allowed_roots: Vec<PathBuf>,
    pub policy: BuiltinToolPolicy,
    pub shell_availability: ShellAvailability,
    pub max_output_bytes: usize,
    pub max_read_bytes: usize,
    pub default_timeout: Duration,
    pub shell_timeout: Duration,
    pub max_glob_results: usize,
    pub max_grep_results: usize,
    pub max_fetch_bytes: usize,
    pub disabled_tools: Vec<String>,
    pub read_tracking: Arc<Mutex<HashSet<PathBuf>>>,
}

impl Default for BuiltinToolConfig {
    fn default() -> Self {
        Self {
            allowed_roots: vec![std::env::current_dir().unwrap_or_default()],
            policy: BuiltinToolPolicy::default(),
            shell_availability: ShellAvailability::detect(),
            max_output_bytes: 256 * 1024,
            max_read_bytes: 256 * 1024,
            default_timeout: Duration::from_secs(120),
            shell_timeout: Duration::from_secs(120),
            max_glob_results: 1000,
            max_grep_results: 500,
            max_fetch_bytes: 512 * 1024,
            disabled_tools: Vec::new(),
            read_tracking: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

impl BuiltinToolConfig {
    pub fn new(allowed_roots: Vec<PathBuf>) -> Self {
        Self {
            allowed_roots,
            ..Self::default()
        }
    }

    pub fn with_shell_availability(mut self, avail: ShellAvailability) -> Self {
        self.shell_availability = avail;
        self
    }

    pub fn with_disabled_tools(mut self, tools: Vec<String>) -> Self {
        self.disabled_tools = tools;
        self
    }

    pub fn with_policy(mut self, policy: BuiltinToolPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_max_output_bytes(mut self, bytes: usize) -> Self {
        self.max_output_bytes = bytes;
        self
    }

    pub fn with_max_read_bytes(mut self, bytes: usize) -> Self {
        self.max_read_bytes = bytes;
        self
    }

    pub fn with_shell_timeout(mut self, timeout: Duration) -> Self {
        self.shell_timeout = timeout;
        self
    }

    pub fn with_default_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    pub fn is_tool_enabled(&self, tool_name: &str) -> bool {
        !self.disabled_tools.iter().any(|d| d == tool_name)
    }

    pub fn validate(&self) -> Result<(), super::error::BuiltinToolError> {
        if self.allowed_roots.is_empty() {
            return Err(super::error::BuiltinToolError::config(
                "at least one allowed root is required",
            ));
        }
        for root in &self.allowed_roots {
            if !root.is_absolute() {
                return Err(super::error::BuiltinToolError::config(format!(
                    "allowed root must be an absolute path: {}",
                    root.display()
                )));
            }
        }
        Ok(())
    }

    pub fn record_read(&self, path: &Path) {
        self.read_tracking.lock().insert(path.to_path_buf());
    }

    pub fn has_read(&self, path: &Path) -> bool {
        self.read_tracking.lock().contains(path)
    }
}
