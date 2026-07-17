//! Security and approval policies enforced by built-in tools.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Resource scope to which a user approval applies.
pub enum ApprovalScope {
    /// Exactly one command string.
    Command(
        /// Exact command text.
        String,
    ),
    /// Exactly one filesystem path.
    FilePath(
        /// Exact file path.
        PathBuf,
    ),
    /// A directory and paths beneath it.
    DirectoryPath(
        /// Directory path that anchors the scope.
        PathBuf,
    ),
    /// A domain and all of its subdomains.
    Domain(
        /// Domain suffix without a leading wildcard.
        String,
    ),
    /// Every approval context.
    All,
}

impl ApprovalScope {
    /// Return whether this scope covers `context`.
    ///
    /// Directory-to-directory matching is symmetric for ancestor and
    /// descendant contexts; domain matching includes exact and subdomains.
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
/// Concrete resource context tested against an [`ApprovalScope`].
pub enum ApprovalScopeMatch {
    /// Command being considered for approval.
    Command(
        /// Command text being checked.
        String,
    ),
    /// File path being considered for approval.
    FilePath(
        /// File path being checked.
        PathBuf,
    ),
    /// Directory path being considered for approval.
    DirectoryPath(
        /// Directory path being checked.
        PathBuf,
    ),
    /// Domain being considered for approval.
    Domain(
        /// Domain name being checked.
        String,
    ),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Lifetime of a granted approval.
pub enum ApprovalDuration {
    /// Consume the approval for one operation.
    Once,
    /// Retain the approval for the current session.
    Session,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
/// Coarse network-access policy for built-in tools.
pub enum NetworkPolicy {
    /// Permit network operations, subject to private-address restrictions.
    #[default]
    AllowAll,
    /// Deny all network operations.
    DenyAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Command shell available for built-in shell execution.
pub enum ShellAvailability {
    /// Bash is available.
    Bash,
    /// PowerShell or PowerShell Core is available.
    PowerShell,
    /// No supported shell was detected.
    None,
}

impl ShellAvailability {
    /// Detect the first supported shell available on the process search path.
    ///
    /// Bash is preferred over PowerShell when both are present.
    pub fn detect() -> Self {
        if which_exists("bash") {
            Self::Bash
        } else if which_exists("powershell") || which_exists("pwsh") {
            Self::PowerShell
        } else {
            Self::None
        }
    }

    /// Return the built-in tool name corresponding to this availability.
    pub fn tool_name(&self) -> Option<&'static str> {
        match self {
            Self::Bash => Some("bash"),
            Self::PowerShell => Some("powershell"),
            Self::None => None,
        }
    }
}

fn which_exists(cmd: &str) -> bool {
    let lookup_cmd = if cfg!(windows) { "where" } else { "which" };
    std::process::Command::new(lookup_cmd)
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
/// Security and approval settings applied to built-in tool operations.
pub struct BuiltinToolPolicy {
    /// How long a granted tool approval remains valid.
    pub approval_duration: ApprovalDuration,
    /// Whether built-in network access is permitted.
    pub network: NetworkPolicy,
    /// Whether file reads identify and summarize likely binary content.
    pub binary_detection_enabled: bool,
    /// When `false` (default), `webfetch` refuses requests that resolve to
    /// loopback, link-local, or private IP ranges. Opt in to reach internal
    /// services or cloud metadata endpoints deliberately.
    pub allow_private_network: bool,
}

impl Default for BuiltinToolPolicy {
    fn default() -> Self {
        Self {
            approval_duration: ApprovalDuration::Once,
            network: NetworkPolicy::AllowAll,
            binary_detection_enabled: true,
            allow_private_network: false,
        }
    }
}

impl BuiltinToolPolicy {
    /// Resolve `path` and verify that it lies within an allowed root.
    ///
    /// Relative paths are resolved against the first root. Existing ancestors
    /// are canonicalized so symlinks cannot escape a root, while nonexistent
    /// final components remain usable for file creation.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when canonicalization fails, or an out-of-scope
    /// error when the resolved path is outside all roots or retains `..`.
    pub fn validate_path(
        &self,
        path: &Path,
        allowed_roots: &[PathBuf],
    ) -> Result<PathBuf, super::error::BuiltinToolError> {
        let mut absolute = path.to_path_buf();
        if !absolute.is_absolute() {
            let root = allowed_roots.first().cloned().unwrap_or_default();
            absolute = root.join(&absolute);
        }

        let canonical = canonicalize_by_ancestor(&absolute).map_err(|e| {
            super::error::BuiltinToolError::io(format!(
                "failed to canonicalize path {}: {}",
                path.display(),
                e
            ))
        })?;

        if canonical
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(super::error::BuiltinToolError::out_of_scope(format!(
                "path '{}' is outside all allowed workspace roots (unresolved parent-directory components)",
                path.display()
            )));
        }

        for root in allowed_roots {
            let canonical_root = canonicalize_by_ancestor(root).unwrap_or_else(|_| root.clone());
            if path_contains(&canonical_root, &canonical) {
                return Ok(canonical);
            }
        }

        Err(super::error::BuiltinToolError::out_of_scope(format!(
            "path '{}' is outside all allowed workspace roots",
            path.display()
        )))
    }
}

/// Canonicalize `path` by walking up to the nearest existing ancestor,
/// canonicalizing that ancestor, and rejoining the remaining tail components.
///
/// Why: `Path::canonicalize` fails if the target doesn't exist, which previously
/// forced a fallback that skipped symlink resolution. Resolving via an existing
/// ancestor preserves symlink safety while still letting us build a canonical
/// path for a file that does not yet exist.
fn canonicalize_by_ancestor(path: &Path) -> std::io::Result<PathBuf> {
    if path.exists() {
        return path.canonicalize();
    }
    let components: Vec<Component> = path.components().collect();
    for split in (0..=components.len()).rev() {
        let prefix: PathBuf = components[..split].iter().collect();
        if prefix.as_os_str().is_empty() {
            continue;
        }
        if prefix.exists() {
            let canonical_prefix = prefix.canonicalize()?;
            let tail: PathBuf = components[split..].iter().collect();
            return Ok(canonical_prefix.join(tail));
        }
    }
    Ok(path.to_path_buf())
}

/// Returns true iff `candidate` is `root` or a descendant of `root` under
/// component-wise containment (avoiding the `/foo` vs `/foobar` prefix bug of
/// naive string comparison).
fn path_contains(root: &Path, candidate: &Path) -> bool {
    let mut root_components = root.components();
    let mut candidate_components = candidate.components();
    loop {
        match (root_components.next(), candidate_components.next()) {
            (Some(r), Some(c)) if r == c => continue,
            (None, _) => return true,
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn rejects_relative_parent_traversal() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let policy = BuiltinToolPolicy::default();
        let target = root.join("subdir/../../escape.txt");
        let err = policy
            .validate_path(&target, std::slice::from_ref(&root))
            .unwrap_err();
        assert!(err.to_string().contains("outside all allowed"));
    }

    #[test]
    fn rejects_absolute_escape() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let policy = BuiltinToolPolicy::default();
        let outside = std::env::temp_dir().join("iron-core-escape-test.txt");
        let err = policy
            .validate_path(&outside, std::slice::from_ref(&root))
            .unwrap_err();
        assert!(err.to_string().contains("outside all allowed"));
    }

    #[test]
    fn accepts_nonexistent_file_inside_root() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let policy = BuiltinToolPolicy::default();
        let target = root.join("new-file.txt");
        let resolved = policy
            .validate_path(&target, std::slice::from_ref(&root))
            .unwrap();
        assert_eq!(resolved, target);
    }

    #[test]
    fn accepts_nested_nonexistent_path_inside_root() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("a")).unwrap();
        let policy = BuiltinToolPolicy::default();
        let target = root.join("a/b/c.txt");
        let resolved = policy
            .validate_path(&target, std::slice::from_ref(&root))
            .unwrap();
        assert_eq!(resolved, target);
    }

    #[test]
    fn accepts_existing_path_inside_root() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let existing = root.join("exists.txt");
        fs::write(&existing, b"hi").unwrap();
        let policy = BuiltinToolPolicy::default();
        let resolved = policy
            .validate_path(&existing, std::slice::from_ref(&root))
            .unwrap();
        assert_eq!(resolved, existing);
    }

    #[test]
    fn rejects_prefix_collision() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap().join("foo");
        let sibling = tmp.path().canonicalize().unwrap().join("foobar");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        let inside_sibling = sibling.join("leak.txt");
        fs::write(&inside_sibling, b"").unwrap();
        let policy = BuiltinToolPolicy::default();
        let err = policy
            .validate_path(&inside_sibling, std::slice::from_ref(&root))
            .unwrap_err();
        assert!(err.to_string().contains("outside all allowed"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape_from_allowed_root() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let outside = tempdir().unwrap();
        let outside_path = outside.path().canonicalize().unwrap();
        let link = root.join("escape-link");
        std::os::unix::fs::symlink(&outside_path, &link).unwrap();
        let target = link.join("leak.txt");
        let policy = BuiltinToolPolicy::default();
        let err = policy
            .validate_path(&target, std::slice::from_ref(&root))
            .unwrap_err();
        assert!(err.to_string().contains("outside all allowed"));
    }
}
