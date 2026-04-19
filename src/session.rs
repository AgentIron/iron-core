#![allow(deprecated)]
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use iron_providers::Message;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub messages: Vec<Message>,
    pub instructions: Option<String>,
    pub metadata: serde_json::Map<String, Value>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            messages: vec![],
            instructions: None,
            metadata: serde_json::Map::new(),
        }
    }

    pub fn with_instructions<S: Into<String>>(instructions: S) -> Self {
        Self {
            messages: vec![],
            instructions: Some(instructions.into()),
            metadata: serde_json::Map::new(),
        }
    }

    pub fn add_user_message<S: Into<String>>(&mut self, content: S) {
        self.messages.push(Message::User {
            content: content.into(),
        });
    }

    pub fn add_assistant_message<S: Into<String>>(&mut self, content: S) {
        self.messages.push(Message::Assistant {
            content: content.into(),
        });
    }

    pub fn add_tool_call<S1: Into<String>, S2: Into<String>>(
        &mut self,
        call_id: S1,
        tool_name: S2,
        arguments: Value,
    ) {
        self.messages.push(Message::AssistantToolCall {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            arguments,
        });
    }

    pub fn add_tool_result<S1: Into<String>, S2: Into<String>>(
        &mut self,
        call_id: S1,
        tool_name: S2,
        result: Value,
    ) {
        self.messages.push(Message::Tool {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            result,
        });
    }

    pub fn add_system_structured_message<S: Into<String>>(&mut self, kind: S, payload: Value) {
        self.messages
            .push(Message::system_structured(kind, payload));
    }

    pub fn last_message(&self) -> Option<&Message> {
        self.messages.last()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    pub fn to_transcript(&self) -> iron_providers::Transcript {
        iron_providers::Transcript::with_messages(self.messages.clone())
    }

    pub fn set_instructions<S: Into<String>>(&mut self, instructions: S) {
        self.instructions = Some(instructions.into());
    }
}
