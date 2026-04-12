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
    #[serde(rename_all = "camelCase")]
    pub struct InitializeRequest {
        pub protocol_version: String,
        pub capabilities: Value,
        pub client_info: ClientInfo,
    }

    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ClientInfo {
        pub name: String,
        pub version: String,
    }

    /// Initialize response
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct InitializeResponse {
        pub protocol_version: String,
        pub capabilities: Value,
        pub server_info: ServerInfo,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ServerInfo {
        pub name: String,
        pub version: String,
    }

    /// Tool list request
    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ListToolsRequest {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub cursor: Option<String>,
    }

    /// Tool list response
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ListToolsResponse {
        pub tools: Vec<Tool>,
        #[serde(default)]
        pub next_cursor: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Tool {
        pub name: String,
        pub description: String,
        pub input_schema: Value,
    }

    /// Tool call request
    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct CallToolRequest {
        pub name: String,
        pub arguments: Value,
    }

    /// Tool call response
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
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
        Image {
            data: String,
            #[serde(rename = "mimeType")]
            mime_type: String,
        },
        #[serde(rename = "resource")]
        Resource { resource: Resource },
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
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

#[cfg(test)]
mod tests {
    use super::messages::*;

    // ── Serialization: outbound requests use camelCase ────────────────

    #[test]
    fn initialize_request_serializes_camel_case() {
        let req = InitializeRequest {
            protocol_version: "2024-11-05".to_string(),
            capabilities: serde_json::json!({}),
            client_info: ClientInfo {
                name: "iron-core".to_string(),
                version: "0.1.0".to_string(),
            },
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(
            json.get("protocolVersion").is_some(),
            "should use protocolVersion"
        );
        assert!(json.get("clientInfo").is_some(), "should use clientInfo");
        assert!(
            json.get("protocol_version").is_none(),
            "should NOT use protocol_version"
        );
        assert!(
            json.get("client_info").is_none(),
            "should NOT use client_info"
        );
    }

    #[test]
    fn call_tool_request_serializes_camel_case() {
        let req = CallToolRequest {
            name: "test".to_string(),
            arguments: serde_json::json!({}),
        };
        let json = serde_json::to_value(&req).unwrap();
        // name and arguments are single words — no camelCase difference,
        // but rename_all should not break them.
        assert_eq!(json["name"], "test");
    }

    // ── Deserialization: inbound responses use camelCase ───────────────

    #[test]
    fn initialize_response_deserializes_camel_case() {
        let json = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "serverInfo": { "name": "test-server", "version": "1.0.0" }
        });
        let resp: InitializeResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.protocol_version, "2024-11-05");
        assert_eq!(resp.server_info.name, "test-server");
    }

    #[test]
    fn list_tools_response_deserializes_camel_case() {
        let json = serde_json::json!({
            "tools": [{
                "name": "my_tool",
                "description": "A tool",
                "inputSchema": { "type": "object" }
            }],
            "nextCursor": "page2"
        });
        let resp: ListToolsResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.tools.len(), 1);
        assert_eq!(resp.tools[0].input_schema["type"], "object");
        assert_eq!(resp.next_cursor, Some("page2".to_string()));
    }

    #[test]
    fn call_tool_response_deserializes_camel_case() {
        let json = serde_json::json!({
            "content": [{ "type": "text", "text": "hello" }],
            "isError": true
        });
        let resp: CallToolResponse = serde_json::from_value(json).unwrap();
        assert!(resp.is_error);
        assert_eq!(resp.content.len(), 1);
    }

    #[test]
    fn tool_content_image_deserializes_camel_case() {
        let json = serde_json::json!({
            "type": "image",
            "data": "base64data",
            "mimeType": "image/png"
        });
        let content: ToolContent = serde_json::from_value(json).unwrap();
        match content {
            ToolContent::Image { mime_type, .. } => {
                assert_eq!(mime_type, "image/png");
            }
            _ => panic!("expected Image variant"),
        }
    }

    #[test]
    fn resource_deserializes_camel_case() {
        let json = serde_json::json!({
            "uri": "file:///test.txt",
            "mimeType": "text/plain",
            "text": "hello"
        });
        let resource: Resource = serde_json::from_value(json).unwrap();
        assert_eq!(resource.uri, "file:///test.txt");
        assert_eq!(resource.mime_type, Some("text/plain".to_string()));
    }
}
