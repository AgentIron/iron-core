//! JSON metadata and result helpers shared by built-in tools.

use serde_json::Value;

#[derive(Debug, Clone, Default)]
/// Typed representation of truncation and continuation metadata.
pub struct BuiltinMeta {
    /// Whether the corresponding output omits remaining content.
    pub truncated: bool,
    /// Total untruncated content size in bytes, when known.
    pub total_bytes: Option<usize>,
    /// Offset from which a caller can request the next segment.
    pub continuation_offset: Option<usize>,
}

impl BuiltinMeta {
    /// Return an empty JSON metadata object.
    pub fn empty() -> Value {
        serde_json::json!({})
    }

    /// Return JSON metadata for truncated content of `total_bytes` bytes.
    pub fn with_truncation(total_bytes: usize) -> Value {
        serde_json::json!({
            "truncated": true,
            "total_bytes": total_bytes,
        })
    }

    /// Return truncation metadata with the next continuation `offset`.
    pub fn with_continuation(offset: usize, total_bytes: usize) -> Value {
        serde_json::json!({
            "truncated": true,
            "total_bytes": total_bytes,
            "continuation_offset": offset,
        })
    }
}

/// Result type used by built-in tool implementation helpers.
pub type BuiltinResult<T = Value> = Result<T, super::error::BuiltinToolError>;

/// Attach `meta` to object-shaped `data`, or wrap non-object data with it.
pub fn success_result(data: Value, meta: Value) -> Value {
    let mut result = data;
    if let Some(obj) = result.as_object_mut() {
        obj.insert("meta".to_string(), meta);
    } else {
        return serde_json::json!({
            "data": result,
            "meta": meta,
        });
    }
    result
}

/// Build a JSON error object from a stable code and human-readable message.
pub fn error_result(code: super::error::BuiltinErrorCode, message: impl Into<String>) -> Value {
    serde_json::json!({
        "error": {
            "code": code.as_str(),
            "message": message.into(),
        }
    })
}
