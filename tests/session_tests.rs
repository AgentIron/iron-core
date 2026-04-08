//! Tests for iron-core session
#![allow(deprecated)]

use iron_core::session::{Message, Session};

#[test]
fn test_session_new() {
    let session = Session::new();
    assert!(session.is_empty());
    assert_eq!(session.len(), 0);
}

#[test]
fn test_session_with_instructions() {
    let session = Session::with_instructions("You are a helpful assistant");
    assert_eq!(
        session.instructions,
        Some("You are a helpful assistant".to_string())
    );
}

#[test]
fn test_add_user_message() {
    let mut session = Session::new();
    session.add_user_message("Hello");
    assert_eq!(session.len(), 1);
    assert_eq!(session.last_message(), Some(&Message::user("Hello")));
}

#[test]
fn test_add_assistant_message() {
    let mut session = Session::new();
    session.add_assistant_message("Hi there!");
    assert_eq!(session.len(), 1);
    assert_eq!(
        session.last_message(),
        Some(&Message::assistant("Hi there!"))
    );
}

#[test]
fn test_add_tool_result() {
    let mut session = Session::new();
    let result = serde_json::json!({"status": "ok"});
    session.add_tool_result("call_123", "my_tool", result.clone());
    assert_eq!(session.len(), 1);
    assert_eq!(
        session.last_message(),
        Some(&Message::tool("call_123", "my_tool", result))
    );
}

#[test]
fn test_session_to_transcript() {
    let mut session = Session::new();
    session.add_user_message("Hello");
    session.add_assistant_message("Hi!");

    let transcript = session.to_transcript();
    assert_eq!(transcript.messages.len(), 2);
}

#[test]
fn test_session_clear() {
    let mut session = Session::new();
    session.add_user_message("Hello");
    session.clear();
    assert!(session.is_empty());
}
