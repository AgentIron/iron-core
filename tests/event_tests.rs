//! Tests for iron-core events
#![allow(deprecated)]

use iron_core::events::StreamEvent;
use serde_json::json;

#[test]
fn test_stream_event_status() {
    let event = StreamEvent::status("Thinking...");
    assert_eq!(
        event,
        StreamEvent::Status {
            message: "Thinking...".to_string()
        }
    );
}

#[test]
fn test_stream_event_output() {
    let event = StreamEvent::output("Hello!");
    assert_eq!(
        event,
        StreamEvent::Output {
            content: "Hello!".to_string()
        }
    );
}

#[test]
fn test_stream_event_tool_call() {
    let args = json!({"name": "test"});
    let event = StreamEvent::tool_call("call_1", "my_tool", args.clone());
    assert_eq!(
        event,
        StreamEvent::ToolCall {
            call_id: "call_1".to_string(),
            tool_name: "my_tool".to_string(),
            arguments: args
        }
    );
}

#[test]
fn test_stream_event_is_terminal() {
    assert!(StreamEvent::Complete.is_terminal());
    assert!(StreamEvent::error("test").is_terminal());
    assert!(StreamEvent::max_iterations(5).is_terminal());
    assert!(!StreamEvent::output("test").is_terminal());
    assert!(!StreamEvent::status("thinking").is_terminal());
}
