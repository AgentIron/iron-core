//! Configuration, limits, and shared state for built-in tools.

use parking_lot::Mutex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::policy::{BuiltinToolPolicy, ShellAvailability};

#[derive(Debug, Clone)]
/// Runtime configuration shared by all built-in tool instances.
///
/// Clones share read-tracking state, which enforces the read-before-edit
/// invariant across tools created from the same configuration.
pub struct BuiltinToolConfig {
    /// Workspace roots within which filesystem operations are permitted.
    pub allowed_roots: Vec<PathBuf>,
    /// Security and approval policy applied by built-in tools.
    pub policy: BuiltinToolPolicy,
    /// Shell backend that may be registered and instantiated.
    pub shell_availability: ShellAvailability,
    /// Maximum bytes retained independently from shell stdout and stderr.
    pub max_output_bytes: usize,
    /// Default maximum number of bytes returned by the read tool.
    pub max_read_bytes: usize,
    /// Default timeout available to built-in operations.
    pub default_timeout: Duration,
    /// Default maximum duration of a shell command.
    pub shell_timeout: Duration,
    /// Maximum number of paths collected by a glob search.
    pub max_glob_results: usize,
    /// Maximum number of content matches collected by a grep search.
    pub max_grep_results: usize,
    /// Maximum response-body bytes returned by web fetch.
    pub max_fetch_bytes: usize,
    /// Tool names excluded from registration and instantiation.
    pub disabled_tools: Vec<String>,
    /// Canonical paths successfully read and therefore eligible for editing.
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
    /// Create a configuration with the given allowed roots and default limits.
    pub fn new(allowed_roots: Vec<PathBuf>) -> Self {
        Self {
            allowed_roots,
            ..Self::default()
        }
    }

    /// Select the shell backend exposed by this configuration.
    pub fn with_shell_availability(mut self, avail: ShellAvailability) -> Self {
        self.shell_availability = avail;
        self
    }

    /// Disable the tools whose exact names occur in `tools`.
    pub fn with_disabled_tools(mut self, tools: Vec<String>) -> Self {
        self.disabled_tools = tools;
        self
    }

    /// Replace the security and approval policy.
    pub fn with_policy(mut self, policy: BuiltinToolPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Set the maximum retained bytes for each shell output stream.
    pub fn with_max_output_bytes(mut self, bytes: usize) -> Self {
        self.max_output_bytes = bytes;
        self
    }

    /// Set the read tool's default byte limit.
    pub fn with_max_read_bytes(mut self, bytes: usize) -> Self {
        self.max_read_bytes = bytes;
        self
    }

    /// Set the default shell-command timeout.
    pub fn with_shell_timeout(mut self, timeout: Duration) -> Self {
        self.shell_timeout = timeout;
        self
    }

    /// Set the general default operation timeout.
    pub fn with_default_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Return whether a tool name is absent from [`Self::disabled_tools`].
    pub fn is_tool_enabled(&self, tool_name: &str) -> bool {
        !self.disabled_tools.iter().any(|d| d == tool_name)
    }

    /// Validate invariants required before using this configuration.
    ///
    /// # Errors
    ///
    /// Returns a configuration error when no roots are configured or any root
    /// is relative.
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

    /// Record that `path` has been read for subsequent edit authorization.
    ///
    /// Callers must provide the canonical path used by the corresponding edit
    /// operation because tracking stores paths verbatim.
    pub fn record_read(&self, path: &Path) {
        self.read_tracking.lock().insert(path.to_path_buf());
    }

    /// Return whether `path` has previously been recorded as read.
    ///
    /// Callers must provide the same canonical path that was passed to
    /// [`Self::record_read`].
    pub fn has_read(&self, path: &Path) -> bool {
        self.read_tracking.lock().contains(path)
    }
}
