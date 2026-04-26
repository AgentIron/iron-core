use crate::plugin::wasm_host::{WasmError, WasmResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const NORMALIZED_RESULT_KIND: &str = "plugin_tool_result";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginToolResultEnvelope {
    pub kind: String,
    pub transcript: PluginToolTranscript,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<PluginToolView>,
    pub metadata: PluginToolResultMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginToolTranscript {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginToolView {
    pub id: String,
    pub mode: PluginToolViewMode,
    pub payload: PluginToolViewPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginToolViewMode {
    Replace,
    Append,
    Transient,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginToolViewPayload {
    TodoList(TodoListView),
    Status(StatusView),
    Progress(ProgressView),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoListView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub items: Vec<TodoListItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoListItem {
    pub id: String,
    pub label: String,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusView {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<StatusLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginToolResultMetadata {
    pub plugin_id: String,
    pub tool_name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginToolResultInput {
    #[serde(default)]
    kind: Option<String>,
    transcript: PluginToolTranscript,
    #[serde(default)]
    view: Option<PluginToolView>,
    #[serde(default)]
    metadata: Option<Value>,
}

pub fn normalize_plugin_tool_result(
    plugin_id: &str,
    tool_name: &str,
    result: Value,
) -> WasmResult<Value> {
    if let Value::String(text) = result {
        return normalized_value(plugin_id, tool_name, PluginToolTranscript { text }, None);
    }

    if !looks_like_rich_result_candidate(&result) {
        return Ok(result);
    }

    let input: PluginToolResultInput = serde_json::from_value(result).map_err(|e| {
        WasmError::ExecutionFailed(format!(
            "Plugin returned invalid rich result envelope: {}",
            e
        ))
    })?;

    if let Some(kind) = input.kind.as_deref() {
        if kind != NORMALIZED_RESULT_KIND {
            return Err(WasmError::ExecutionFailed(format!(
                "Plugin returned unsupported rich result kind '{}'. Expected '{}'.",
                kind, NORMALIZED_RESULT_KIND
            )));
        }
    }

    validate_transcript(&input.transcript)?;
    if let Some(view) = input.view.as_ref() {
        validate_view(view)?;
    }

    normalized_value(plugin_id, tool_name, input.transcript, input.view)
}

pub fn transcript_text(result: &Value) -> Option<&str> {
    result
        .get("kind")
        .and_then(Value::as_str)
        .filter(|kind| *kind == NORMALIZED_RESULT_KIND)
        .and_then(|_| result.get("transcript"))
        .and_then(|transcript| transcript.get("text"))
        .and_then(Value::as_str)
}

pub fn view(result: &Value) -> Option<&Value> {
    result
        .get("kind")
        .and_then(Value::as_str)
        .filter(|kind| *kind == NORMALIZED_RESULT_KIND)
        .and_then(|_| result.get("view"))
}

fn looks_like_rich_result_candidate(result: &Value) -> bool {
    let Value::Object(object) = result else {
        return false;
    };

    // Only trigger normalization when the plugin explicitly signals the
    // rich-result kind.  This avoids breaking backwards compatibility for
    // plugins that happen to use "transcript" or "view" as unrelated keys.
    object
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == NORMALIZED_RESULT_KIND)
}

fn validate_transcript(transcript: &PluginToolTranscript) -> WasmResult<()> {
    if transcript.text.trim().is_empty() {
        return Err(WasmError::ExecutionFailed(
            "Plugin rich result transcript.text must be a non-empty string".to_string(),
        ));
    }

    Ok(())
}

fn validate_view(view: &PluginToolView) -> WasmResult<()> {
    if view.id.trim().is_empty() {
        return Err(WasmError::ExecutionFailed(
            "Plugin rich result view.id must be a non-empty string".to_string(),
        ));
    }

    match &view.payload {
        PluginToolViewPayload::TodoList(todo_list) => {
            if todo_list.items.is_empty() {
                return Err(WasmError::ExecutionFailed(
                    "Plugin rich result todo_list payload must contain at least one item"
                        .to_string(),
                ));
            }
            for item in &todo_list.items {
                if item.id.trim().is_empty() || item.label.trim().is_empty() {
                    return Err(WasmError::ExecutionFailed(
                        "Plugin rich result todo_list items must have non-empty id and label"
                            .to_string(),
                    ));
                }
            }
        }
        PluginToolViewPayload::Status(status) => {
            if status.text.trim().is_empty() {
                return Err(WasmError::ExecutionFailed(
                    "Plugin rich result status payload must have non-empty text".to_string(),
                ));
            }
        }
        PluginToolViewPayload::Progress(progress) => {
            if !(0.0..=1.0).contains(&progress.value) {
                return Err(WasmError::ExecutionFailed(
                    "Plugin rich result progress payload value must be between 0.0 and 1.0"
                        .to_string(),
                ));
            }
        }
    }

    Ok(())
}

fn normalized_value(
    plugin_id: &str,
    tool_name: &str,
    transcript: PluginToolTranscript,
    view: Option<PluginToolView>,
) -> WasmResult<Value> {
    serde_json::to_value(PluginToolResultEnvelope {
        kind: NORMALIZED_RESULT_KIND.to_string(),
        transcript,
        view,
        metadata: PluginToolResultMetadata {
            plugin_id: plugin_id.to_string(),
            tool_name: tool_name.to_string(),
        },
    })
    .map_err(|e| WasmError::ExecutionFailed(format!("Failed to serialize plugin result: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn string_result_normalizes_to_text_only_envelope() {
        let normalized = normalize_plugin_tool_result("todo-plugin", "update", json!("Done"))
            .expect("string result should normalize");

        assert_eq!(transcript_text(&normalized), Some("Done"));
        assert!(view(&normalized).is_none());
        assert_eq!(
            normalized,
            json!({
                "kind": "plugin_tool_result",
                "transcript": {"text": "Done"},
                "metadata": {
                    "plugin_id": "todo-plugin",
                    "tool_name": "update"
                }
            })
        );
    }

    #[test]
    fn rich_result_normalizes_and_preserves_view() {
        let normalized = normalize_plugin_tool_result(
            "todo-plugin",
            "update",
            json!({
                "kind": "plugin_tool_result",
                "transcript": {"text": "Updated tasks"},
                "view": {
                    "id": "todo:1",
                    "mode": "replace",
                    "payload": {
                        "kind": "todo_list",
                        "title": "Current Tasks",
                        "items": [
                            {"id": "a", "label": "Review", "done": true}
                        ]
                    }
                }
            }),
        )
        .expect("rich result should normalize");

        assert_eq!(transcript_text(&normalized), Some("Updated tasks"));
        assert_eq!(
            view(&normalized),
            Some(&json!({
                "id": "todo:1",
                "mode": "replace",
                "payload": {
                    "kind": "todo_list",
                    "title": "Current Tasks",
                    "items": [
                        {"id": "a", "label": "Review", "done": true}
                    ]
                }
            }))
        );
    }

    #[test]
    fn plain_json_result_is_left_untouched() {
        let raw = json!({"ok": true, "count": 2});
        let normalized =
            normalize_plugin_tool_result("todo-plugin", "update", raw.clone()).unwrap();
        assert_eq!(normalized, raw);
        assert!(transcript_text(&normalized).is_none());
    }

    #[test]
    fn unknown_view_kind_is_rejected() {
        let error = normalize_plugin_tool_result(
            "todo-plugin",
            "update",
            json!({
                "kind": "plugin_tool_result",
                "transcript": {"text": "Updated tasks"},
                "view": {
                    "id": "todo:1",
                    "mode": "replace",
                    "payload": {
                        "kind": "table",
                        "rows": []
                    }
                }
            }),
        )
        .unwrap_err();

        assert!(error.to_string().contains("invalid rich result envelope"));
    }

    #[test]
    fn arbitrary_code_bearing_payload_is_rejected() {
        let error = normalize_plugin_tool_result(
            "todo-plugin",
            "update",
            json!({
                "kind": "plugin_tool_result",
                "transcript": {"text": "Updated status"},
                "view": {
                    "id": "status:1",
                    "mode": "replace",
                    "payload": {
                        "kind": "status",
                        "text": "Ready",
                        "script": "alert('xss')"
                    }
                }
            }),
        )
        .unwrap_err();

        assert!(error.to_string().contains("invalid rich result envelope"));
    }

    #[test]
    fn non_rich_json_with_transcript_key_is_left_untouched() {
        // Backwards compatibility: a plugin that uses "transcript" for its
        // own purposes (without the rich-result kind) must not be mangled.
        let raw = json!({
            "transcript": "hello",
            "data": 42
        });
        let normalized =
            normalize_plugin_tool_result("todo-plugin", "update", raw.clone()).unwrap();
        assert_eq!(normalized, raw);
        assert!(transcript_text(&normalized).is_none());
    }

    #[test]
    fn status_view_kind_normalizes() {
        let normalized = normalize_plugin_tool_result(
            "status-plugin",
            "check",
            json!({
                "kind": "plugin_tool_result",
                "transcript": {"text": "Service is healthy"},
                "view": {
                    "id": "health:1",
                    "mode": "replace",
                    "payload": {
                        "kind": "status",
                        "text": "All systems operational",
                        "level": "success"
                    }
                }
            }),
        )
        .expect("status view should normalize");

        assert_eq!(transcript_text(&normalized), Some("Service is healthy"));
        let view_val = view(&normalized).unwrap();
        assert_eq!(view_val.get("id").unwrap().as_str(), Some("health:1"));
        assert_eq!(view_val.get("mode").unwrap().as_str(), Some("replace"));
    }

    #[test]
    fn progress_view_kind_normalizes() {
        let normalized = normalize_plugin_tool_result(
            "progress-plugin",
            "upload",
            json!({
                "kind": "plugin_tool_result",
                "transcript": {"text": "Upload at 50%"},
                "view": {
                    "id": "upload:1",
                    "mode": "append",
                    "payload": {
                        "kind": "progress",
                        "label": "Uploading file",
                        "value": 0.5
                    }
                }
            }),
        )
        .expect("progress view should normalize");

        assert_eq!(transcript_text(&normalized), Some("Upload at 50%"));
        let view_val = view(&normalized).unwrap();
        assert_eq!(view_val.get("mode").unwrap().as_str(), Some("append"));
    }

    #[test]
    fn transient_mode_is_preserved() {
        let normalized = normalize_plugin_tool_result(
            "progress-plugin",
            "poll",
            json!({
                "kind": "plugin_tool_result",
                "transcript": {"text": "Still working"},
                "view": {
                    "id": "poll:1",
                    "mode": "transient",
                    "payload": {
                        "kind": "progress",
                        "value": 0.75
                    }
                }
            }),
        )
        .expect("transient mode should normalize");

        let view_val = view(&normalized).unwrap();
        assert_eq!(view_val.get("mode").unwrap().as_str(), Some("transient"));
    }

    #[test]
    fn empty_transcript_text_is_rejected() {
        let error = normalize_plugin_tool_result(
            "todo-plugin",
            "update",
            json!({
                "kind": "plugin_tool_result",
                "transcript": {"text": "   "},
                "view": {
                    "id": "todo:1",
                    "mode": "replace",
                    "payload": {
                        "kind": "todo_list",
                        "items": [
                            {"id": "a", "label": "Review", "done": true}
                        ]
                    }
                }
            }),
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("transcript.text must be a non-empty string"));
    }

    #[test]
    fn empty_view_id_is_rejected() {
        let error = normalize_plugin_tool_result(
            "todo-plugin",
            "update",
            json!({
                "kind": "plugin_tool_result",
                "transcript": {"text": "Updated tasks"},
                "view": {
                    "id": "  ",
                    "mode": "replace",
                    "payload": {
                        "kind": "todo_list",
                        "items": [
                            {"id": "a", "label": "Review", "done": true}
                        ]
                    }
                }
            }),
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("view.id must be a non-empty string"));
    }

    #[test]
    fn progress_value_out_of_bounds_is_rejected() {
        let error = normalize_plugin_tool_result(
            "progress-plugin",
            "upload",
            json!({
                "kind": "plugin_tool_result",
                "transcript": {"text": "Upload at 150%"},
                "view": {
                    "id": "upload:1",
                    "mode": "replace",
                    "payload": {
                        "kind": "progress",
                        "value": 1.5
                    }
                }
            }),
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("value must be between 0.0 and 1.0"));
    }
}
