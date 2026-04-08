use std::path::PathBuf;
use std::sync::Arc;

use crate::builtin::config::BuiltinToolConfig;
use crate::builtin::error::BuiltinToolError;
use crate::builtin::helpers::BuiltinMeta;
use crate::error::LoopResult;
use crate::tool::{Tool, ToolDefinition, ToolFuture};
use serde_json::Value;

pub struct GlobTool {
    config: Arc<BuiltinToolConfig>,
}

impl GlobTool {
    pub fn new(config: BuiltinToolConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

impl Tool for GlobTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "glob",
            "Find filesystem paths matching a pattern within allowed roots. Use '*' as a directory listing primitive.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern to match (e.g. '**/*.rs', 'src/*', '*')"
                    },
                    "path": {
                        "type": "string",
                        "description": "Base directory for the pattern (must be within allowed roots, defaults to first allowed root)"
                    }
                },
                "required": ["pattern"]
            }),
        )
    }

    fn execute(&self, _call_id: &str, arguments: Value) -> ToolFuture {
        let config = self.config.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || execute_glob(&config, arguments))
                .await
                .map_err(|e| crate::error::LoopError::tool_execution(e.to_string()))?
        })
    }

    fn requires_approval(&self) -> bool {
        false
    }
}

fn execute_glob(config: &BuiltinToolConfig, args: Value) -> LoopResult<Value> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'pattern' argument"))
        .map_err(crate::error::LoopError::from)?;

    let base_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| config.allowed_roots.first().cloned().unwrap_or_default());

    let canonical_base = config
        .policy
        .validate_path(&base_path, &config.allowed_roots)
        .map_err(crate::error::LoopError::from)?;

    if !canonical_base.is_dir() {
        return Err(crate::error::LoopError::from(
            BuiltinToolError::invalid_input(format!(
                "'{}' is not a directory",
                base_path.display()
            )),
        ));
    }

    let mut matches: Vec<String> = Vec::new();
    let mut truncated = false;

    let glob_pattern = if pattern.contains('/') || pattern.contains('\\') {
        pattern.to_string()
    } else {
        format!("**/{}", pattern)
    };

    visit_dirs(
        &canonical_base,
        &glob_pattern,
        &mut matches,
        config.max_glob_results,
        &mut truncated,
    )
    .map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::io(format!("glob traversal error: {}", e)))
    })?;

    let total_count = matches.len();
    let meta = if truncated {
        BuiltinMeta::with_truncation(total_count)
    } else {
        BuiltinMeta::empty()
    };

    Ok(serde_json::json!({
        "paths": matches,
        "count": total_count,
        "truncated": truncated,
        "meta": meta,
    }))
}

fn visit_dirs(
    dir: &std::path::Path,
    pattern: &str,
    results: &mut Vec<String>,
    max_results: usize,
    truncated: &mut bool,
) -> Result<(), std::io::Error> {
    if results.len() >= max_results {
        *truncated = true;
        return Ok(());
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        if file_name_str.starts_with('.') {
            continue;
        }

        let relative = path
            .strip_prefix(
                path.parent()
                    .and_then(|p| p.parent())
                    .unwrap_or(path.parent().unwrap_or(&path)),
            )
            .unwrap_or(&path);

        if glob_match(pattern, &path.to_string_lossy())
            || glob_match(pattern, &relative.to_string_lossy())
        {
            if results.len() >= max_results {
                *truncated = true;
                return Ok(());
            }
            results.push(path.to_string_lossy().to_string());
        }

        if path.is_dir() {
            visit_dirs(&path, pattern, results, max_results, truncated)?;
        }
    }

    Ok(())
}

fn glob_match(pattern: &str, path: &str) -> bool {
    let simple = simple_glob_pattern(pattern);
    if simple {
        return simple_match(pattern, path);
    }
    path.contains(pattern.trim_start_matches("**/"))
}

fn simple_glob_pattern(pattern: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    chars.iter().all(|c| {
        c.is_alphanumeric() || *c == '.' || *c == '*' || *c == '/' || *c == '-' || *c == '_'
    })
}

fn simple_match(pattern: &str, path: &str) -> bool {
    let p = pattern.trim_start_matches("**/");
    if p == "*" {
        return !path.contains(std::path::MAIN_SEPARATOR);
    }
    path.ends_with(p.trim_start_matches('*')) || path.contains(p)
}

pub struct GrepTool {
    config: Arc<BuiltinToolConfig>,
}

impl GrepTool {
    pub fn new(config: BuiltinToolConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

impl Tool for GrepTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "grep",
            "Search file content with a query within allowed roots. Returns matching file locations with line context.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The search pattern (regex supported)"
                    },
                    "path": {
                        "type": "string",
                        "description": "Base directory to search in (must be within allowed roots, defaults to first allowed root)"
                    },
                    "include": {
                        "type": "string",
                        "description": "File pattern to include (e.g. '*.rs', '*.ts')"
                    }
                },
                "required": ["pattern"]
            }),
        )
    }

    fn execute(&self, _call_id: &str, arguments: Value) -> ToolFuture {
        let config = self.config.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || execute_grep(&config, arguments))
                .await
                .map_err(|e| crate::error::LoopError::tool_execution(e.to_string()))?
        })
    }

    fn requires_approval(&self) -> bool {
        false
    }
}

#[derive(Debug)]
struct GrepMatch {
    path: String,
    line_number: usize,
    line: String,
}

fn execute_grep(config: &BuiltinToolConfig, args: Value) -> LoopResult<Value> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'pattern' argument"))
        .map_err(crate::error::LoopError::from)?;

    let base_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| config.allowed_roots.first().cloned().unwrap_or_default());

    let include_filter = args.get("include").and_then(|v| v.as_str());

    let canonical_base = config
        .policy
        .validate_path(&base_path, &config.allowed_roots)
        .map_err(crate::error::LoopError::from)?;

    if !canonical_base.is_dir() {
        return Err(crate::error::LoopError::from(
            BuiltinToolError::invalid_input(format!(
                "'{}' is not a directory",
                base_path.display()
            )),
        ));
    }

    let regex = regex::Regex::new(pattern).map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::invalid_input(format!(
            "invalid regex pattern: {}",
            e
        )))
    })?;

    let mut matches: Vec<GrepMatch> = Vec::new();
    let mut truncated = false;

    grep_visit_dirs(
        &canonical_base,
        &regex,
        include_filter,
        &mut matches,
        config.max_grep_results,
        &mut truncated,
    )
    .map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::io(format!("grep traversal error: {}", e)))
    })?;

    let match_results: Vec<serde_json::Value> = matches
        .iter()
        .map(|m| {
            serde_json::json!({
                "path": m.path,
                "line_number": m.line_number,
                "line": m.line,
            })
        })
        .collect();

    let total_count = match_results.len();
    let meta = if truncated {
        BuiltinMeta::with_truncation(total_count)
    } else {
        BuiltinMeta::empty()
    };

    Ok(serde_json::json!({
        "matches": match_results,
        "count": total_count,
        "truncated": truncated,
        "meta": meta,
    }))
}

fn grep_visit_dirs(
    dir: &std::path::Path,
    regex: &regex::Regex,
    include_filter: Option<&str>,
    results: &mut Vec<GrepMatch>,
    max_results: usize,
    truncated: &mut bool,
) -> Result<(), std::io::Error> {
    if results.len() >= max_results {
        *truncated = true;
        return Ok(());
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        if file_name_str.starts_with('.') {
            continue;
        }

        if path.is_dir() {
            grep_visit_dirs(
                &path,
                regex,
                include_filter,
                results,
                max_results,
                truncated,
            )?;
        } else {
            if !file_matches_include(&file_name_str, include_filter) {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(&path) {
                for (i, line) in content.lines().enumerate() {
                    if results.len() >= max_results {
                        *truncated = true;
                        return Ok(());
                    }
                    if regex.is_match(line) {
                        results.push(GrepMatch {
                            path: path.to_string_lossy().to_string(),
                            line_number: i + 1,
                            line: line.to_string(),
                        });
                    }
                }
            }
        }
    }

    Ok(())
}

fn file_matches_include(file_name: &str, include: Option<&str>) -> bool {
    match include {
        None => true,
        Some(pattern) => {
            let pat = pattern.trim_start_matches('*');
            file_name.ends_with(pat)
        }
    }
}
