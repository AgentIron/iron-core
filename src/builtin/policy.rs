use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalScope {
    Command(String),
    FilePath(PathBuf),
    DirectoryPath(PathBuf),
    Domain(String),
    All,
}

impl ApprovalScope {
    pub fn matches(&self, context: &ApprovalScopeMatch) -> bool {
        match (self, context) {
            (ApprovalScope::All, _) => true,
            (ApprovalScope::Command(pattern), ApprovalScopeMatch::Command(cmd)) => pattern == cmd,
            (ApprovalScope::FilePath(pattern), ApprovalScopeMatch::FilePath(path)) => {
                pattern == path
            }
            (ApprovalScope::DirectoryPath(pattern), ApprovalScopeMatch::FilePath(path)) => {
                path.starts_with(pattern)
            }
            (ApprovalScope::DirectoryPath(pattern), ApprovalScopeMatch::DirectoryPath(dir)) => {
                dir.starts_with(pattern) || pattern.starts_with(dir)
            }
            (ApprovalScope::Domain(pattern), ApprovalScopeMatch::Domain(domain)) => {
                domain == pattern || domain.ends_with(&format!(".{}", pattern))
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ApprovalScopeMatch {
    Command(String),
    FilePath(PathBuf),
    DirectoryPath(PathBuf),
    Domain(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDuration {
    Once,
    Session,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPolicy {
    #[default]
    AllowAll,
    DenyAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellAvailability {
    Bash,
    PowerShell,
    None,
}

impl ShellAvailability {
    pub fn detect() -> Self {
        if which_exists("bash") {
            Self::Bash
        } else if which_exists("powershell") || which_exists("pwsh") {
            Self::PowerShell
        } else {
            Self::None
        }
    }

    pub fn tool_name(&self) -> Option<&'static str> {
        match self {
            Self::Bash => Some("bash"),
            Self::PowerShell => Some("powershell"),
            Self::None => None,
        }
    }
}

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
pub struct BuiltinToolPolicy {
    pub approval_duration: ApprovalDuration,
    pub network: NetworkPolicy,
    pub binary_detection_enabled: bool,
}

impl Default for BuiltinToolPolicy {
    fn default() -> Self {
        Self {
            approval_duration: ApprovalDuration::Once,
            network: NetworkPolicy::AllowAll,
            binary_detection_enabled: true,
        }
    }
}

impl BuiltinToolPolicy {
    pub fn validate_path(
        &self,
        path: &Path,
        allowed_roots: &[PathBuf],
    ) -> Result<PathBuf, super::error::BuiltinToolError> {
        let canonical = if path.exists() {
            path.canonicalize().map_err(|e| {
                super::error::BuiltinToolError::io(format!(
                    "failed to canonicalize path {}: {}",
                    path.display(),
                    e
                ))
            })?
        } else {
            let mut resolved = path.to_path_buf();
            if !resolved.is_absolute() {
                resolved = allowed_roots
                    .first()
                    .cloned()
                    .unwrap_or_default()
                    .join(&resolved);
            }
            resolved
        };

        for root in allowed_roots {
            let canonical_root = if root.exists() {
                root.canonicalize().unwrap_or_else(|_| root.clone())
            } else {
                root.clone()
            };
            if canonical.starts_with(&canonical_root) {
                return Ok(canonical);
            }
        }

        Err(super::error::BuiltinToolError::out_of_scope(format!(
            "path '{}' is outside all allowed workspace roots",
            path.display()
        )))
    }
}
