use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct BuiltinMeta {
    pub truncated: bool,
    pub total_bytes: Option<usize>,
    pub continuation_offset: Option<usize>,
}

impl BuiltinMeta {
    pub fn empty() -> Value {
        serde_json::json!({})
    }

    pub fn with_truncation(total_bytes: usize) -> Value {
        serde_json::json!({
            "truncated": true,
            "total_bytes": total_bytes,
        })
    }

    pub fn with_continuation(offset: usize, total_bytes: usize) -> Value {
        serde_json::json!({
            "truncated": true,
            "total_bytes": total_bytes,
            "continuation_offset": offset,
        })
    }
}

pub type BuiltinResult<T = Value> = Result<T, super::error::BuiltinToolError>;

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

pub fn error_result(code: super::error::BuiltinErrorCode, message: impl Into<String>) -> Value {
    serde_json::json!({
        "error": {
            "code": code.as_str(),
            "message": message.into(),
        }
    })
}
