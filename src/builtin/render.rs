//! Shared tool-result rendering for model-facing compact output.
//!
//! Built-in tools produce structured internal results (JSON) for runtime
//! correctness and testability. This module converts those structured results
//! into compact text that minimizes token consumption when sent to the model.
//!
//! Design decisions:
//! - Internal paths stay absolute; rendered paths are root-relative when possible.
//! - Truncation uses a standardized bracketed footer with tool-specific guidance.
//! - Ordering is deterministic (lexicographic by rendered path, ascending line number).

use std::path::{Path, PathBuf};

/// Render an absolute path as root-relative when it falls under a configured root.
/// Falls back to the absolute path when no root matches.
/// When multiple roots match, chooses the most specific (longest) matching root.
pub fn render_path(absolute: &Path, roots: &[PathBuf]) -> String {
    let abs_str = absolute.to_string_lossy();

    let best_match = roots
        .iter()
        .filter(|root| absolute.starts_with(root))
        .max_by_key(|root| root.as_os_str().len());

    match best_match {
        Some(root) => {
            let relative = absolute.strip_prefix(root).unwrap_or(absolute);
            let rel_str = relative.to_string_lossy();
            if rel_str.is_empty() {
                // Path is exactly the root itself
                ".".to_string()
            } else {
                rel_str.to_string()
            }
        }
        None => abs_str.to_string(),
    }
}

/// Render a truncation footer with tool-specific recovery guidance.
pub fn render_truncation_footer(guidance: &str) -> String {
    format!("\n[truncated: {}]", guidance)
}

/// Render a warning about skipped paths during traversal.
pub fn render_skip_warning(count: usize) -> String {
    format!(
        "\n[warning: skipped {} unreadable paths during search]",
        count
    )
}

pub fn render_mutation_summary(
    operation: &str,
    path: &Path,
    roots: &[PathBuf],
    detail: Option<&str>,
) -> String {
    let rendered_path = render_path(path, roots);
    match detail {
        Some(detail) if !detail.is_empty() => {
            format!("{} {} ({})", operation, rendered_path, detail)
        }
        _ => format!("{} {}", operation, rendered_path),
    }
}

pub fn render_directory_entries(entries: &[String]) -> String {
    entries.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_path_under_root() {
        let roots = vec![PathBuf::from("/home/user/project")];
        let path = Path::new("/home/user/project/src/main.rs");
        assert_eq!(render_path(path, &roots), "src/main.rs");
    }

    #[test]
    fn render_path_outside_roots() {
        let roots = vec![PathBuf::from("/home/user/project")];
        let path = Path::new("/tmp/other/file.rs");
        assert_eq!(render_path(path, &roots), "/tmp/other/file.rs");
    }

    #[test]
    fn render_path_picks_most_specific_root() {
        let roots = vec![PathBuf::from("/repo"), PathBuf::from("/repo/subproject")];
        let path = Path::new("/repo/subproject/src/main.rs");
        assert_eq!(render_path(path, &roots), "src/main.rs");
    }

    #[test]
    fn render_path_exact_root() {
        let roots = vec![PathBuf::from("/home/user/project")];
        let path = Path::new("/home/user/project");
        assert_eq!(render_path(path, &roots), ".");
    }

    #[test]
    fn render_path_no_roots() {
        let roots: Vec<PathBuf> = vec![];
        let path = Path::new("/home/user/project/src/main.rs");
        assert_eq!(render_path(path, &roots), "/home/user/project/src/main.rs");
    }
}
