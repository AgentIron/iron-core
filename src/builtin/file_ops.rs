use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::builtin::config::BuiltinToolConfig;
use crate::builtin::error::BuiltinToolError;
use crate::builtin::helpers::BuiltinMeta;
use crate::builtin::render::{
    render_directory_entries, render_mutation_summary, render_truncation_footer,
};
use crate::error::LoopResult;
use crate::tool::{Tool, ToolDefinition, ToolFuture};
use serde_json::Value;

pub struct ReadTool {
    config: Arc<BuiltinToolConfig>,
}

impl ReadTool {
    pub fn new(config: BuiltinToolConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

impl Tool for ReadTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "read",
            "Read a file or directory from the local filesystem.\n\
             \n\
             Usage:\n\
             - The path parameter should be an absolute path within allowed roots.\n\
             - By default, returns up to the configured line limit starting from the beginning of the file.\n\
             - Use offset to start from a later line (1-indexed) and limit to control how many lines.\n\
             - Avoid tiny repeated slices. If you need more context, read a larger window.\n\
             - When reading a directory, returns entries sorted lexicographically with / suffix for subdirectories.\n\
             - You can call multiple tools in a single response. Read multiple files in parallel when possible.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to read (must be within allowed roots)"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Line number to start reading from (1-indexed, default 1)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of lines to read"
                    }
                },
                "required": ["path"]
            }),
        )
    }

    fn execute(&self, _call_id: &str, arguments: Value) -> ToolFuture {
        let config = self.config.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || execute_read(&config, arguments))
                .await
                .map_err(|e| crate::error::LoopError::tool_execution(e.to_string()))?
        })
    }

    fn requires_approval(&self) -> bool {
        false
    }
}

fn execute_read(config: &BuiltinToolConfig, args: Value) -> LoopResult<Value> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'path' argument"))
        .map_err(crate::error::LoopError::from)?;

    let path = PathBuf::from(path_str);
    let canonical = config
        .policy
        .validate_path(&path, &config.allowed_roots)
        .map_err(crate::error::LoopError::from)?;

    if !canonical.exists() {
        return Err(crate::error::LoopError::from(
            BuiltinToolError::path_not_found(format!("file not found: {}", path.display())),
        ));
    }

    let metadata = std::fs::metadata(&canonical).map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::io(format!("cannot read metadata: {}", e)))
    })?;

    if metadata.is_dir() {
        config.record_read(&canonical);
        return render_directory_listing(&canonical, &config.allowed_roots);
    }

    let bytes = std::fs::read(&canonical).map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::io(format!("failed to read file: {}", e)))
    })?;

    config.record_read(&canonical);

    if config.policy.binary_detection_enabled && is_binary(&bytes) {
        return Ok(serde_json::json!({
            "content": format!("[binary file, {} bytes]", bytes.len()),
            "is_binary": true,
            "size": bytes.len(),
            "meta": BuiltinMeta::empty(),
        }));
    }

    let content = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let total_bytes = content.len();

    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|l| l as usize)
        .unwrap_or(config.max_read_bytes);

    let start = offset.saturating_sub(1);
    if start >= lines.len() {
        return Ok(serde_json::json!({
            "content": "",
            "line_count": 0,
            "total_lines": total_lines,
            "meta": BuiltinMeta::empty(),
        }));
    }

    let end = (start + limit).min(lines.len());
    let selected_lines = &lines[start..end];

    let mut output = String::new();
    for (i, line) in selected_lines.iter().enumerate() {
        let line_no = start + i + 1;
        output.push_str(&format!("{}: {}\n", line_no, truncate_line(line)));
    }

    let has_more = end < lines.len();
    let meta = if has_more {
        BuiltinMeta::with_continuation(end + 1, total_bytes)
    } else {
        BuiltinMeta::empty()
    };

    if has_more {
        output.push_str(&render_truncation_footer(&format!(
            "continue with offset={}",
            end + 1
        )));
    }

    Ok(serde_json::json!({
        "content": output,
        "internal": {
            "path": canonical.to_string_lossy(),
            "line_count": selected_lines.len(),
            "total_lines": total_lines,
            "is_directory": false,
        },
        "line_count": selected_lines.len(),
        "total_lines": total_lines,
        "meta": meta,
    }))
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

pub struct WriteTool {
    config: Arc<BuiltinToolConfig>,
}

impl WriteTool {
    pub fn new(config: BuiltinToolConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

impl Tool for WriteTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "write",
            "Write a file to the local filesystem.\n\
             \n\
             Usage:\n\
             - This tool will overwrite the existing file if one exists at the provided path.\n\
             - Creates missing parent directories automatically.\n\
             - ALWAYS prefer editing existing files using edit or multiedit. NEVER write new files unless explicitly required.\n\
             - NEVER proactively create documentation files (*.md) or README files unless explicitly requested.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to write (must be within allowed roots)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Text content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        )
        .with_approval(true)
    }

    fn execute(&self, _call_id: &str, arguments: Value) -> ToolFuture {
        let config = self.config.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || execute_write(&config, arguments))
                .await
                .map_err(|e| crate::error::LoopError::tool_execution(e.to_string()))?
        })
    }

    fn requires_approval(&self) -> bool {
        true
    }
}

fn execute_write(config: &BuiltinToolConfig, args: Value) -> LoopResult<Value> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'path' argument"))
        .map_err(crate::error::LoopError::from)?;

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'content' argument"))
        .map_err(crate::error::LoopError::from)?;

    let path = PathBuf::from(path_str);
    let resolved =
        resolve_write_path(&path, &config.allowed_roots).map_err(crate::error::LoopError::from)?;

    if let Some(parent) = resolved.parent() {
        if !parent.exists() {
            validate_no_path_conflict(parent, &config.allowed_roots)?;
            std::fs::create_dir_all(parent).map_err(|e| {
                crate::error::LoopError::from(BuiltinToolError::io(format!(
                    "failed to create parent directories: {}",
                    e
                )))
            })?;
        }
    }

    let existed = resolved.exists();
    std::fs::write(&resolved, content).map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::io(format!("failed to write file: {}", e)))
    })?;

    let summary = if existed {
        render_mutation_summary("Overwrote", &resolved, &config.allowed_roots, None)
    } else {
        render_mutation_summary("Created", &resolved, &config.allowed_roots, None)
    };

    Ok(serde_json::json!({
        "content": summary,
        "internal": {
            "path": resolved.to_string_lossy(),
            "bytes_written": content.len(),
            "created": !existed,
        },
    }))
}

fn resolve_write_path(path: &Path, allowed_roots: &[PathBuf]) -> Result<PathBuf, BuiltinToolError> {
    if path.is_absolute() {
        for root in allowed_roots {
            if path.starts_with(root) {
                return Ok(path.to_path_buf());
            }
            let canonical_root = root.canonicalize().unwrap_or_else(|_| root.clone());
            if path.starts_with(&canonical_root) {
                return Ok(path.to_path_buf());
            }
        }
        return Err(BuiltinToolError::out_of_scope(format!(
            "path '{}' is outside all allowed workspace roots",
            path.display()
        )));
    }

    let root = allowed_roots
        .first()
        .ok_or_else(|| BuiltinToolError::config("no allowed roots configured"))?;
    Ok(root.join(path))
}

fn validate_no_path_conflict(
    parent: &Path,
    _allowed_roots: &[PathBuf],
) -> Result<(), BuiltinToolError> {
    let mut current = parent;
    loop {
        if current.exists() && !current.is_dir() {
            return Err(BuiltinToolError::path_conflict(format!(
                "path segment '{}' exists and is not a directory",
                current.display()
            )));
        }
        match current.parent() {
            Some(p) if p != current => current = p,
            _ => break,
        }
    }
    Ok(())
}

pub struct EditTool {
    config: Arc<BuiltinToolConfig>,
}

impl EditTool {
    pub fn new(config: BuiltinToolConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

pub struct MultieditTool {
    config: Arc<BuiltinToolConfig>,
}

impl MultieditTool {
    pub fn new(config: BuiltinToolConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

impl Tool for EditTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "edit",
            "Perform exact string replacements in files.\n\
             \n\
             Usage:\n\
             - The old_string must match exactly, including all whitespace and indentation.\n\
             - By default, old_string must match exactly one location. Use replace_all to replace all occurrences.\n\
             - ALWAYS prefer editing existing files over creating new ones.\n\
             - For multiple edits to the same file, use multiedit instead of multiple edit calls.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to edit (must be within allowed roots)"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The exact text to find and replace"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The replacement text"
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Replace all occurrences of old_string (default false). When false, old_string must match exactly one location."
                    }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        )
        .with_approval(true)
    }

    fn execute(&self, _call_id: &str, arguments: Value) -> ToolFuture {
        let config = self.config.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || execute_edit(&config, arguments))
                .await
                .map_err(|e| crate::error::LoopError::tool_execution(e.to_string()))?
        })
    }

    fn requires_approval(&self) -> bool {
        true
    }
}

fn execute_edit(config: &BuiltinToolConfig, args: Value) -> LoopResult<Value> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'path' argument"))
        .map_err(crate::error::LoopError::from)?;

    let old_string = args
        .get("old_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'old_string' argument"))
        .map_err(crate::error::LoopError::from)?;

    let new_string = args
        .get("new_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'new_string' argument"))
        .map_err(crate::error::LoopError::from)?;

    let replace_all = args
        .get("replace_all")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if old_string == new_string {
        return Err(crate::error::LoopError::from(
            BuiltinToolError::invalid_input("old_string and new_string are identical"),
        ));
    }

    let path = PathBuf::from(path_str);
    let canonical = config
        .policy
        .validate_path(&path, &config.allowed_roots)
        .map_err(crate::error::LoopError::from)?;

    if !canonical.exists() {
        return Err(crate::error::LoopError::from(
            BuiltinToolError::path_not_found(format!("file not found: {}", path.display())),
        ));
    }

    if !config.has_read(&canonical) {
        return Err(crate::error::LoopError::from(BuiltinToolError::invalid_input(
            "file must be read before editing",
        )));
    }

    let content = std::fs::read_to_string(&canonical).map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::io(format!("failed to read file: {}", e)))
    })?;

    let match_count = content.matches(old_string).count();
    if match_count == 0 {
        return Err(crate::error::LoopError::from(
            BuiltinToolError::edit_mismatch("old_string not found in file"),
        ));
    }
    if !replace_all && match_count > 1 {
        return Err(crate::error::LoopError::from(
            BuiltinToolError::edit_ambiguous(format!(
                "old_string matched {} locations; provide more context to make it unique or use replace_all",
                match_count
            )),
        ));
    }

    let new_content = if replace_all {
        content.replace(old_string, new_string)
    } else {
        content.replacen(old_string, new_string, 1)
    };
    let replacements = if replace_all { match_count } else { 1 };
    std::fs::write(&canonical, &new_content).map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::io(format!("failed to write file: {}", e)))
    })?;

    Ok(serde_json::json!({
        "content": render_mutation_summary(
            "Updated",
            &canonical,
            &config.allowed_roots,
            Some(&format!("{} replacements", replacements)),
        ),
        "internal": {
            "path": canonical.to_string_lossy(),
            "replacements": replacements,
        },
        "meta": BuiltinMeta::empty(),
    }))
}

// ---------------------------------------------------------------------------
// MultieditTool implementation
// ---------------------------------------------------------------------------

impl Tool for MultieditTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "multiedit",
            "Apply multiple exact-match text replacements to a single file in one atomic operation. All edits are applied sequentially; if any edit fails, no changes are written.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to edit (must be within allowed roots)"
                    },
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_string": {
                                    "type": "string",
                                    "description": "The exact text to find and replace"
                                },
                                "new_string": {
                                    "type": "string",
                                    "description": "The replacement text"
                                },
                                "replace_all": {
                                    "type": "boolean",
                                    "description": "Replace all occurrences of old_string in this edit item (default false)"
                                }
                            },
                            "required": ["old_string", "new_string"]
                        },
                        "description": "Array of edit operations to apply sequentially"
                    }
                },
                "required": ["path", "edits"]
            }),
        )
        .with_approval(true)
    }

    fn execute(&self, _call_id: &str, arguments: Value) -> ToolFuture {
        let config = self.config.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || execute_multiedit(&config, arguments))
                .await
                .map_err(|e| crate::error::LoopError::tool_execution(e.to_string()))?
        })
    }

    fn requires_approval(&self) -> bool {
        true
    }
}

fn execute_multiedit(config: &BuiltinToolConfig, args: Value) -> LoopResult<Value> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'path' argument"))
        .map_err(crate::error::LoopError::from)?;

    let edits = args
        .get("edits")
        .and_then(|v| v.as_array())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'edits' argument"))
        .map_err(crate::error::LoopError::from)?;

    if edits.is_empty() {
        return Err(crate::error::LoopError::from(
            BuiltinToolError::invalid_input("edits array must not be empty"),
        ));
    }

    let path = PathBuf::from(path_str);
    let canonical = config
        .policy
        .validate_path(&path, &config.allowed_roots)
        .map_err(crate::error::LoopError::from)?;

    if !canonical.exists() {
        return Err(crate::error::LoopError::from(
            BuiltinToolError::path_not_found(format!("file not found: {}", path.display())),
        ));
    }

    if !config.has_read(&canonical) {
        return Err(crate::error::LoopError::from(BuiltinToolError::invalid_input(
            "file must be read before editing",
        )));
    }

    let mut content = std::fs::read_to_string(&canonical).map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::io(format!("failed to read file: {}", e)))
    })?;

    let mut total_replacements = 0usize;

    // Apply edits sequentially against the evolving buffer.
    // If any edit fails, we abort without writing.
    for (i, edit) in edits.iter().enumerate() {
        let old_string = edit
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::error::LoopError::from(BuiltinToolError::invalid_input(format!(
                    "edit {}: missing 'old_string'",
                    i + 1
                )))
            })?;

        let new_string = edit
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::error::LoopError::from(BuiltinToolError::invalid_input(format!(
                    "edit {}: missing 'new_string'",
                    i + 1
                )))
            })?;

        let replace_all = edit
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if old_string == new_string {
            return Err(crate::error::LoopError::from(
                BuiltinToolError::invalid_input(format!(
                    "edit {}: old_string and new_string are identical",
                    i + 1
                )),
            ));
        }

        let match_count = content.matches(old_string).count();
        if match_count == 0 {
            return Err(crate::error::LoopError::from(
                BuiltinToolError::edit_mismatch(format!(
                    "edit {}: old_string not found in file; no changes were applied",
                    i + 1
                )),
            ));
        }
        if !replace_all && match_count > 1 {
            return Err(crate::error::LoopError::from(
                BuiltinToolError::edit_ambiguous(format!(
                    "edit {}: old_string matched {} locations; provide more context or use replace_all; no changes were applied",
                    i + 1, match_count
                )),
            ));
        }

        if replace_all {
            content = content.replace(old_string, new_string);
            total_replacements += match_count;
        } else {
            content = content.replacen(old_string, new_string, 1);
            total_replacements += 1;
        }
    }

    // All edits validated and applied in memory; write the result.
    std::fs::write(&canonical, &content).map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::io(format!("failed to write file: {}", e)))
    })?;

    Ok(serde_json::json!({
        "content": render_mutation_summary(
            "Applied edits to",
            &canonical,
            &config.allowed_roots,
            Some(&format!("{} edits, {} replacements", edits.len(), total_replacements)),
        ),
        "internal": {
            "path": canonical.to_string_lossy(),
            "edits_applied": edits.len(),
            "replacements": total_replacements,
        },
        "meta": BuiltinMeta::empty(),
    }))
}

/// Render a directory listing as compact model-facing text.
/// Entries are sorted lexicographically, rendered relative to the directory,
/// with `/` suffix for subdirectories. Hidden entries are included.
/// `.` and `..` are excluded.
fn render_directory_listing(
    dir: &std::path::Path,
    _roots: &[std::path::PathBuf],
) -> LoopResult<Value> {
    let mut entries: Vec<String> = Vec::new();

    let read_dir = std::fs::read_dir(dir).map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::io(format!(
            "failed to read directory: {}",
            e
        )))
    })?;

    for entry in read_dir {
        let entry = entry.map_err(|e| {
            crate::error::LoopError::from(BuiltinToolError::io(format!(
                "directory entry error: {}",
                e
            )))
        })?;

        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        // Exclude . and ..
        if name == "." || name == ".." {
            continue;
        }

        let is_dir = entry
            .file_type()
            .map(|ft| ft.is_dir())
            .unwrap_or(false);

        if is_dir {
            entries.push(format!("{}/", name));
        } else {
            entries.push(name.to_string());
        }
    }

    // Sort lexicographically.
    entries.sort();

    let rendered = render_directory_entries(&entries);

    Ok(serde_json::json!({
        "content": rendered,
        "entry_count": entries.len(),
        "is_directory": true,
    }))
}
