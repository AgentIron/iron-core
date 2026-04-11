use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
    pub id: u64,
}

impl JsonRpcRequest {
    pub fn new(method: &str, params: Value, id: u64) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id,
        }
    }
}

/// JSON-RPC 2.0 response
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
    #[serde(default)]
    pub id: Option<u64>,
}

/// JSON-RPC 2.0 error
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

/// MCP protocol messages
pub mod messages {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    /// Initialize request
    #[derive(Debug, Serialize)]
    pub struct InitializeRequest {
        pub protocol_version: String,
        pub capabilities: Value,
        pub client_info: ClientInfo,
    }

    #[derive(Debug, Serialize)]
    pub struct ClientInfo {
        pub name: String,
        pub version: String,
    }

    /// Initialize response
    #[derive(Debug, Deserialize)]
    pub struct InitializeResponse {
        pub protocol_version: String,
        pub capabilities: Value,
        pub server_info: ServerInfo,
    }

    #[derive(Debug, Deserialize)]
    pub struct ServerInfo {
        pub name: String,
        pub version: String,
    }

    /// Tool list request
    #[derive(Debug, Serialize)]
    pub struct ListToolsRequest {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub cursor: Option<String>,
    }

    /// Tool list response
    #[derive(Debug, Deserialize)]
    pub struct ListToolsResponse {
        pub tools: Vec<Tool>,
        #[serde(default)]
        pub next_cursor: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    pub struct Tool {
        pub name: String,
        pub description: String,
        pub input_schema: Value,
    }

    /// Tool call request
    #[derive(Debug, Serialize)]
    pub struct CallToolRequest {
        pub name: String,
        pub arguments: Value,
    }

    /// Tool call response
    #[derive(Debug, Deserialize)]
    pub struct CallToolResponse {
        pub content: Vec<ToolContent>,
        #[serde(default)]
        pub is_error: bool,
    }

    #[derive(Debug, Deserialize)]
    #[serde(tag = "type")]
    pub enum ToolContent {
        #[serde(rename = "text")]
        Text { text: String },
        #[serde(rename = "image")]
        Image { data: String, mime_type: String },
        #[serde(rename = "resource")]
        Resource { resource: Resource },
    }

    #[derive(Debug, Deserialize)]
    pub struct Resource {
        pub uri: String,
        pub mime_type: Option<String>,
        pub text: Option<String>,
        pub blob: Option<String>,
    }
}

/// Convert tool content to JSON value
pub fn tool_content_to_value(content: Vec<messages::ToolContent>) -> Value {
    let mut texts = Vec::new();
    for item in content {
        match item {
            messages::ToolContent::Text { text } => texts.push(text),
            messages::ToolContent::Image { data: _, mime_type } => {
                texts.push(format!("[Image: {}]", mime_type));
            }
            messages::ToolContent::Resource { resource } => {
                if let Some(text) = resource.text {
                    texts.push(text);
                } else if let Some(_blob) = resource.blob {
                    texts.push(format!("[Binary resource: {}]", resource.uri));
                } else {
                    texts.push(format!("[Resource: {}]", resource.uri));
                }
            }
        }
    }

    if texts.len() == 1 {
        serde_json::json!({ "result": texts[0] })
    } else {
        serde_json::json!({ "results": texts })
    }
}

/// Convert tool error content into a useful error string.
pub fn tool_error_to_string(content: Vec<messages::ToolContent>) -> String {
    let value = tool_content_to_value(content);

    if let Some(result) = value.get("result").and_then(Value::as_str) {
        return result.to_string();
    }

    if let Some(results) = value.get("results").and_then(Value::as_array) {
        let parts: Vec<String> = results
            .iter()
            .filter_map(|item| match item {
                Value::String(text) => Some(text.clone()),
                other => serde_json::to_string(other).ok(),
            })
            .collect();
        if !parts.is_empty() {
            return parts.join("\n");
        }
    }

    serde_json::to_string(&value).unwrap_or_else(|_| "Tool execution returned an error".to_string())
}
