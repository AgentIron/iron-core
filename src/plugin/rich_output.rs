//! Normalized transcript and constrained rich-view output from plugin tools.
//!
//! Plain string results become transcript-only envelopes. JSON objects are
//! interpreted as rich output only when they explicitly use the
//! `plugin_tool_result` kind, preserving arbitrary legacy JSON unchanged.
//! Rich payloads are limited to typed todo, status, and progress views; unknown
//! fields and executable presentation content are rejected during normalization.

use crate::plugin::wasm_host::{WasmError, WasmResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const NORMALIZED_RESULT_KIND: &str = "plugin_tool_result";

/// Canonical result shape returned by normalized plugin tool execution.
///
/// The transcript is suitable for model history, while the optional view is a
/// constrained client-rendered projection. Runtime metadata identifies the
/// source and cannot be supplied by the plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginToolResultEnvelope {
    /// Envelope discriminator, always `"plugin_tool_result"` after normalization.
    pub kind: String,
    /// Text persisted in the model-visible transcript.
    pub transcript: PluginToolTranscript,
    /// Optional structured view for a supporting client.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<PluginToolView>,
    /// Runtime-injected provenance for this result.
    pub metadata: PluginToolResultMetadata,
}

/// Model-visible text associated with a plugin tool result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginToolTranscript {
    /// Non-empty text to append to the conversation transcript.
    pub text: String,
}

/// A constrained, client-rendered projection of plugin output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginToolView {
    /// Stable logical view ID used to correlate updates.
    pub id: String,
    /// How the client should combine this view with prior output.
    pub mode: PluginToolViewMode,
    /// Typed presentation payload rendered by the client.
    pub payload: PluginToolViewPayload,
}

/// Update behavior for a rich plugin view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginToolViewMode {
    /// Replace the prior view with the same logical ID.
    Replace,
    /// Append this view to existing output.
    Append,
    /// Display the view temporarily rather than retaining it as durable output.
    Transient,
}

/// Supported safe presentation payloads for plugin rich output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginToolViewPayload {
    /// A checklist-style collection of todo items.
    TodoList(TodoListView),
    /// A textual status callout.
    Status(StatusView),
    /// Fractional progress for an ongoing operation.
    Progress(ProgressView),
}

/// A checklist view containing one or more todo items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoListView {
    /// Optional heading displayed above the list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Non-empty list of todo items.
    pub items: Vec<TodoListItem>,
}

/// One item in a [`TodoListView`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoListItem {
    /// Non-empty stable item ID used to correlate updates.
    pub id: String,
    /// Non-empty user-facing item text.
    pub label: String,
    /// Whether the item is complete.
    pub done: bool,
}

/// A textual status view with optional severity and title.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusView {
    /// Non-empty status message.
    pub text: String,
    /// Optional semantic level used for client styling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<StatusLevel>,
    /// Optional heading displayed with the status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Semantic level of a plugin status view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusLevel {
    /// Neutral informational status.
    Info,
    /// Successful completion or healthy status.
    Success,
    /// Warning that may require attention.
    Warning,
    /// Failed or unhealthy status.
    Error,
}

/// Fractional progress view for an ongoing operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressView {
    /// Optional description of the operation being measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Completion fraction in the inclusive range `0.0..=1.0`.
    pub value: f64,
}

/// Runtime-injected provenance for a normalized plugin tool result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginToolResultMetadata {
    /// Registry ID of the plugin that produced the result.
    pub plugin_id: String,
    /// Plugin-local name of the executed tool.
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

/// Normalizes and validates a raw result from a plugin tool.
///
/// String values become transcript-only envelopes. Objects explicitly tagged
/// `plugin_tool_result` are parsed with unknown fields denied, validated, and
/// rewritten with runtime-owned metadata. Other JSON values pass through
/// unchanged.
///
/// # Errors
///
/// Returns [`WasmError::ExecutionFailed`] when a declared rich envelope is
/// malformed, has an unsupported kind, contains empty required text or IDs,
/// has an empty todo list, or reports progress outside `0.0..=1.0`.
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

/// Returns transcript text from a normalized result envelope.
///
/// Returns `None` for plain JSON, malformed envelopes, or a different kind.
pub fn transcript_text(result: &Value) -> Option<&str> {
    result
        .get("kind")
        .and_then(Value::as_str)
        .filter(|kind| *kind == NORMALIZED_RESULT_KIND)
        .and_then(|_| result.get("transcript"))
        .and_then(|transcript| transcript.get("text"))
        .and_then(Value::as_str)
}

/// Returns the raw structured view from a normalized result envelope.
///
/// Returns `None` when no view is present or the value is not a normalized
/// `plugin_tool_result` envelope.
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
