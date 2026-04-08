use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::builtin::config::BuiltinToolConfig;
use crate::builtin::error::BuiltinToolError;
use crate::builtin::helpers::BuiltinMeta;
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
            "Read content from a text file. Supports offset and limit for bounded reads. Returns content with line numbers and continuation metadata.",
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
        return Err(crate::error::LoopError::from(
            BuiltinToolError::invalid_input(format!(
                "'{}' is a directory, not a file; use glob for directory listing",
                path.display()
            )),
        ));
    }

    let bytes = std::fs::read(&canonical).map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::io(format!("failed to read file: {}", e)))
    })?;

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

    Ok(serde_json::json!({
        "content": output,
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
            "Create or replace a text file. Creates missing parent directories for the target file path. The file path must be within allowed roots.",
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

    Ok(serde_json::json!({
        "path": resolved.to_string_lossy(),
        "bytes_written": content.len(),
        "created": !existed,
        "meta": BuiltinMeta::empty(),
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

impl Tool for EditTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "edit",
            "Apply an exact-match text replacement to an existing file. The old_string must match exactly one location in the file.",
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

    let content = std::fs::read_to_string(&canonical).map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::io(format!("failed to read file: {}", e)))
    })?;

    let match_count = content.matches(old_string).count();
    if match_count == 0 {
        return Err(crate::error::LoopError::from(
            BuiltinToolError::edit_mismatch("old_string not found in file"),
        ));
    }
    if match_count > 1 {
        return Err(crate::error::LoopError::from(
            BuiltinToolError::edit_ambiguous(format!(
                "old_string matched {} locations; provide more context to make it unique",
                match_count
            )),
        ));
    }

    let new_content = content.replacen(old_string, new_string, 1);
    std::fs::write(&canonical, &new_content).map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::io(format!("failed to write file: {}", e)))
    })?;

    Ok(serde_json::json!({
        "path": canonical.to_string_lossy(),
        "replacements": 1,
        "meta": BuiltinMeta::empty(),
    }))
}
