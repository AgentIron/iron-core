//! Built-in `glob` and `grep` tools backed by the ripgrep ecosystem.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::builtin::config::BuiltinToolConfig;
use crate::builtin::error::BuiltinToolError;
use crate::builtin::render::{render_path, render_skip_warning, render_truncation_footer};
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
            "Fast file pattern matching tool that works with any codebase size.\n\
             \n\
             - Supports glob patterns like \"**/*.js\" or \"src/**/*.ts\"\n\
             - Returns matching file paths sorted lexicographically, directories get a / suffix\n\
             - Use this tool when you need to find files by name patterns\n\
             - You can call multiple tools in a single response. It is always better to speculatively perform multiple searches as a batch.\n\
             - For reading directory contents directly, use the read tool on a directory path.",
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
            "Fast content search tool that works with any codebase size.\n\
             \n\
             - Searches file contents using regular expressions\n\
             - Supports full regex syntax (e.g. \"log.*Error\", \"function\\s+\\w+\")\n\
             - Filter files with the include parameter (e.g. \"*.js\", \"*.{ts,tsx}\")\n\
             - Output modes: \"content\" shows matching lines (default), \"files_with_matches\" shows file paths, \"count\" shows total and per-file counts\n\
             - Use case_insensitive for case-insensitive matching, multiline for patterns spanning newlines\n\
             - Use head_limit and offset for paginating large result sets\n\
             - You can call multiple tools in a single response. Batch independent searches for optimal performance.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "The regex pattern to search for in file contents"},
                    "path": {"type": "string", "description": "Base directory to search in (must be within allowed roots, defaults to first allowed root)"},
                    "include": {"type": "string", "description": "File glob pattern to include (e.g. '*.rs', '*.{ts,tsx}')"},
                    "mode": {"type": "string", "enum": ["content", "files_with_matches", "count"], "description": "Output mode"},
                    "case_insensitive": {"type": "boolean", "description": "Match case-insensitively"},
                    "multiline": {"type": "boolean", "description": "Allow regex matches to span newlines"},
                    "head_limit": {"type": "integer", "description": "Maximum number of result entries to return"},
                    "offset": {"type": "integer", "description": "Skip the first N result entries"}
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
struct GlobResult {
    paths: Vec<PathBuf>,
    truncated: bool,
    skipped: usize,
}

#[derive(Debug, Clone)]
struct GrepMatch {
    path: PathBuf,
    line_number: usize,
    line: String,
}

#[derive(Debug)]
struct GrepSearchResult {
    matches: Vec<GrepMatch>,
    file_counts: BTreeMap<PathBuf, usize>,
    total_count: usize,
    truncated: bool,
    skipped: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum GrepMode {
    Content,
    Files,
    Count,
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
    let explicit_path = args.get("path").is_some();

    let canonical_base = config
        .policy
        .validate_path(&base_path, &config.allowed_roots)
        .map_err(crate::error::LoopError::from)?;
    if !canonical_base.is_dir() {
        return Err(crate::error::LoopError::from(BuiltinToolError::invalid_input(format!(
            "'{}' is not a directory",
            base_path.display()
        ))));
    }

    let result = glob_search(
        &canonical_base,
        &config.allowed_roots,
        pattern,
        explicit_path,
        config.max_glob_results,
    )?;

    Ok(serde_json::json!({
        "content": render_glob_output(&result, &config.allowed_roots),
        "internal": {
            "paths": result.paths.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>(),
            "count": result.paths.len(),
            "truncated": result.truncated,
            "skipped": result.skipped,
        }
    }))
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
    let explicit_path = args.get("path").is_some();
    let include_filter = args.get("include").and_then(|v| v.as_str());
    let case_insensitive = args.get("case_insensitive").and_then(|v| v.as_bool()).unwrap_or(false);
    let multiline = args.get("multiline").and_then(|v| v.as_bool()).unwrap_or(false);
    let head_limit = args.get("head_limit").and_then(|v| v.as_u64()).map(|v| v as usize);
    let offset = args.get("offset").and_then(|v| v.as_u64()).map(|v| v as usize);
    let mode = match args.get("mode").and_then(|v| v.as_str()).unwrap_or("content") {
        "files_with_matches" => GrepMode::Files,
        "count" => GrepMode::Count,
        _ => GrepMode::Content,
    };

    let canonical_base = config
        .policy
        .validate_path(&base_path, &config.allowed_roots)
        .map_err(crate::error::LoopError::from)?;
    if !canonical_base.is_dir() {
        return Err(crate::error::LoopError::from(BuiltinToolError::invalid_input(format!(
            "'{}' is not a directory",
            base_path.display()
        ))));
    }

    let result = grep_search(
        &canonical_base,
        &config.allowed_roots,
        pattern,
        include_filter,
        explicit_path,
        case_insensitive,
        multiline,
        config.max_grep_results,
    )?;

    Ok(serde_json::json!({
        "content": render_grep_output(&result, &mode, &config.allowed_roots, offset, head_limit),
        "internal": {
            "total_count": result.total_count,
            "file_count": result.file_counts.len(),
            "truncated": result.truncated,
            "skipped": result.skipped,
        }
    }))
}

fn glob_search(
    base: &Path,
    allowed_roots: &[PathBuf],
    pattern: &str,
    explicit_path: bool,
    max_results: usize,
) -> Result<GlobResult, BuiltinToolError> {
    let glob_pattern = if pattern.contains('/') || pattern.contains('\\') {
        pattern.to_string()
    } else {
        format!("**/{}", pattern)
    };
    let matcher = globset::GlobBuilder::new(&glob_pattern)
        .literal_separator(false)
        .build()
        .map_err(|e| BuiltinToolError::invalid_input(format!("invalid glob pattern: {}", e)))?
        .compile_matcher();

    let disable_filters = explicit_path || pattern_targets_hidden(pattern);
    let mut walker = ignore::WalkBuilder::new(base);
    walker
        .standard_filters(!disable_filters)
        .require_git(false)
        .follow_links(true)
        .hidden(!disable_filters)
        .git_ignore(!disable_filters)
        .git_global(!disable_filters)
        .git_exclude(!disable_filters);

    let mut paths = Vec::new();
    let mut truncated = false;
    let mut skipped = 0usize;
    let mut seen = HashSet::new();

    for entry in walker.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let path = entry.path().to_path_buf();
        let canonical = match path.canonicalize() {
            Ok(c) => c,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if !canonical_within_roots(&canonical, allowed_roots) || !seen.insert(canonical) {
            continue;
        }

        let relative = path.strip_prefix(base).unwrap_or(&path);
        if matcher.is_match(&path) || matcher.is_match(relative) {
            if paths.len() >= max_results {
                truncated = true;
                break;
            }
            paths.push(path);
        }
    }

    Ok(GlobResult { paths, truncated, skipped })
}

fn grep_search(
    base: &Path,
    allowed_roots: &[PathBuf],
    pattern: &str,
    include_filter: Option<&str>,
    explicit_path: bool,
    case_insensitive: bool,
    multiline: bool,
    max_results: usize,
) -> Result<GrepSearchResult, BuiltinToolError> {
    let mut matcher_builder = grep_regex::RegexMatcherBuilder::new();
    matcher_builder
        .case_insensitive(case_insensitive)
        .multi_line(multiline)
        .dot_matches_new_line(multiline);
    let matcher = matcher_builder
        .build(pattern)
        .map_err(|e| BuiltinToolError::invalid_input(format!("invalid regex pattern: {}", e)))?;

    let include_matcher = include_filter
        .map(|pat| {
            globset::GlobBuilder::new(pat)
                .literal_separator(false)
                .build()
                .map_err(|e| BuiltinToolError::invalid_input(format!("invalid include pattern: {}", e)))
                .map(|g| g.compile_matcher())
        })
        .transpose()?;

    let disable_filters = explicit_path
        || include_filter.is_some()
        || pattern_targets_hidden(pattern)
        || include_filter.map(pattern_targets_hidden).unwrap_or(false);
    let mut walker = ignore::WalkBuilder::new(base);
    walker
        .standard_filters(!disable_filters)
        .require_git(false)
        .follow_links(true)
        .hidden(!disable_filters)
        .git_ignore(!disable_filters)
        .git_global(!disable_filters)
        .git_exclude(!disable_filters);

    let mut matches = Vec::new();
    let mut file_counts = BTreeMap::new();
    let mut truncated = false;
    let mut skipped = 0usize;
    let mut seen = HashSet::new();

    for entry in walker.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let path = entry.path().to_path_buf();
        if path.is_dir() {
            continue;
        }
        let canonical = match path.canonicalize() {
            Ok(c) => c,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if !canonical_within_roots(&canonical, allowed_roots) || !seen.insert(canonical) {
            continue;
        }

        if let Some(ref include) = include_matcher {
            let relative = path.strip_prefix(base).unwrap_or(&path);
            if !include.is_match(&path) && !include.is_match(relative) {
                continue;
            }
        }

        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if is_binary(&bytes) {
            continue;
        }

        let content = String::from_utf8_lossy(&bytes);
        let mut sink = CollectSink::default();
        if grep_searcher::Searcher::new()
            .search_slice(&matcher, &bytes, &mut sink)
            .is_err()
        {
            skipped += 1;
            continue;
        }
        if sink.matches.is_empty() {
            continue;
        }

        file_counts.insert(path.clone(), sink.matches.len());
        for m in sink.matches {
            if matches.len() >= max_results {
                truncated = true;
                break;
            }
            matches.push(GrepMatch {
                path: path.clone(),
                line_number: m.line_number,
                line: content
                    .lines()
                    .nth(m.line_number.saturating_sub(1))
                    .unwrap_or("")
                    .to_string(),
            });
        }
        if truncated {
            break;
        }
    }

    let total_count = file_counts.values().sum();
    Ok(GrepSearchResult { matches, file_counts, total_count, truncated, skipped })
}

#[derive(Default)]
struct CollectSink {
    matches: Vec<LineMatch>,
}

struct LineMatch {
    line_number: usize,
}

impl grep_searcher::Sink for CollectSink {
    type Error = std::io::Error;

    fn matched(
        &mut self,
        _searcher: &grep_searcher::Searcher,
        mat: &grep_searcher::SinkMatch<'_>,
    ) -> Result<bool, Self::Error> {
        self.matches.push(LineMatch {
            line_number: mat.line_number().unwrap_or(0) as usize,
        });
        Ok(true)
    }
}

fn render_glob_output(result: &GlobResult, roots: &[PathBuf]) -> String {
    let mut rendered: Vec<(String, bool)> = result
        .paths
        .iter()
        .map(|path| (render_path(path, roots), path.is_dir()))
        .collect();
    rendered.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::new();
    for (path, is_dir) in rendered {
        if is_dir {
            out.push_str(&format!("{}/\n", path));
        } else {
            out.push_str(&format!("{}\n", path));
        }
    }
    if result.truncated {
        out.push_str(&render_truncation_footer("more entries not shown; refine pattern or use offset"));
    }
    if result.skipped > 5 {
        out.push_str(&render_skip_warning(result.skipped));
    }
    out
}

fn render_grep_output(
    result: &GrepSearchResult,
    mode: &GrepMode,
    roots: &[PathBuf],
    offset: Option<usize>,
    head_limit: Option<usize>,
) -> String {
    let mut files: Vec<(String, &PathBuf, usize)> = result
        .file_counts
        .iter()
        .map(|(path, count)| (render_path(path, roots), path, *count))
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::new();
    match mode {
        GrepMode::Files => {
            let start = offset.unwrap_or(0);
            let end = head_limit.map(|l| (start + l).min(files.len())).unwrap_or(files.len());
            for (rendered, _, _) in files.iter().take(end).skip(start) {
                out.push_str(&format!("{}\n", rendered));
            }
            if result.truncated {
                out.push_str(&render_truncation_footer("more files not shown; refine pattern or use offset"));
            }
        }
        GrepMode::Count => {
            out.push_str(&format!("{} matches total\n\n", result.total_count));
            let start = offset.unwrap_or(0);
            let end = head_limit.map(|l| (start + l).min(files.len())).unwrap_or(files.len());
            for (rendered, _, count) in files.iter().take(end).skip(start) {
                out.push_str(&format!("{}: {}\n", rendered, count));
            }
            if result.truncated {
                out.push_str(&render_truncation_footer("more files not shown; refine pattern or use offset"));
            }
        }
        GrepMode::Content => {
            let mut matches = result.matches.clone();
            matches.sort_by(|a, b| {
                render_path(&a.path, roots)
                    .cmp(&render_path(&b.path, roots))
                    .then_with(|| a.line_number.cmp(&b.line_number))
            });
            let start = offset.unwrap_or(0);
            let end = (start + head_limit.unwrap_or(matches.len())).min(matches.len());
            let visible = &matches[start..end];
            let mut current_file: Option<String> = None;
            for m in visible {
                let rendered = render_path(&m.path, roots);
                if current_file.as_ref() != Some(&rendered) {
                    current_file = Some(rendered.clone());
                    out.push_str(&format!("{}:\n", rendered));
                }
                out.push_str(&format!("  {}: {}\n", m.line_number, truncate_line(&m.line)));
            }
            if result.truncated {
                out.push_str(&render_truncation_footer(
                    "more matches not shown; refine pattern, increase offset, or read matching files directly",
                ));
            }
        }
    }
    if result.skipped > 5 {
        out.push_str(&render_skip_warning(result.skipped));
    }
    out
}

fn canonical_within_roots(canonical: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| {
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.clone());
        canonical.starts_with(&canonical_root)
    })
}

fn pattern_targets_hidden(pattern: &str) -> bool {
    pattern
        .split(['/', '\\'])
        .any(|segment| segment.starts_with('.') && segment != "." && segment != "..")
}

fn truncate_line(line: &str) -> &str {
    if line.len() > 2000 {
        &line[..line.floor_char_boundary(2000)]
    } else {
        line
    }
}

fn is_binary(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let check_len = bytes.len().min(8192);
    let sample = &bytes[..check_len];
    let null_count = sample.iter().filter(|&&b| b == 0).count();
    null_count > 0 && (null_count as f64 / check_len as f64) > 0.01
}
