//! JSON-RPC wire types used by the MCP transport clients.
//!
//! The types in [`messages`] model the initialize, paginated tool discovery,
//! and tool execution exchanges. Conversion helpers reduce MCP content blocks
//! to the JSON values and error strings returned by the runtime tool layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An outbound JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC protocol version, always `"2.0"` for requests built here.
    pub jsonrpc: String,
    /// MCP method to invoke.
    pub method: String,
    /// Method-specific parameters.
    pub params: Value,
    /// Request identifier used to correlate the response.
    pub id: u64,
}

impl JsonRpcRequest {
    /// Builds a JSON-RPC 2.0 request with the supplied method, parameters, and ID.
    pub fn new(method: &str, params: Value, id: u64) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id,
        }
    }
}

/// An inbound JSON-RPC 2.0 response.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC protocol version reported by the server.
    pub jsonrpc: String,
    /// Successful result payload, when present.
    pub result: Option<Value>,
    /// Structured RPC error, when the request failed.
    pub error: Option<JsonRpcError>,
    /// Correlation ID; some servers omit it from bootstrap responses.
    #[serde(default)]
    pub id: Option<u64>,
}

/// A JSON-RPC 2.0 error object returned by an MCP server.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    /// Numeric JSON-RPC error code.
    pub code: i32,
    /// Human-readable error message.
    pub message: String,
    /// Optional server-defined diagnostic payload.
    pub data: Option<Value>,
}

/// Method-specific MCP request, response, tool, and content types.
pub mod messages {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    /// Client capabilities sent during MCP connection initialization.
    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct InitializeRequest {
        /// MCP protocol version requested by the client.
        pub protocol_version: String,
        /// Client capability object.
        pub capabilities: Value,
        /// Name and version identifying the client implementation.
        pub client_info: ClientInfo,
    }

    /// Name and version identifying an MCP client.
    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ClientInfo {
        /// Client implementation name.
        pub name: String,
        /// Client implementation version.
        pub version: String,
    }

    /// Server capabilities returned after successful initialization.
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct InitializeResponse {
        /// MCP protocol version selected by the server.
        pub protocol_version: String,
        /// Server capability object.
        pub capabilities: Value,
        /// Name and version identifying the server implementation.
        pub server_info: ServerInfo,
    }

    /// Name and version identifying an MCP server.
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ServerInfo {
        /// Server implementation name.
        pub name: String,
        /// Server implementation version.
        pub version: String,
    }

    /// A request for one page of the server's tool catalog.
    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ListToolsRequest {
        /// Opaque pagination cursor returned by the previous page.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub cursor: Option<String>,
    }

    /// One page of tools discovered from an MCP server.
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ListToolsResponse {
        /// Tool definitions in this page.
        pub tools: Vec<Tool>,
        /// Cursor for the next page, or `None` when discovery is complete.
        #[serde(default)]
        pub next_cursor: Option<String>,
    }

    /// A tool definition advertised by an MCP server.
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Tool {
        /// Server-local tool name.
        pub name: String,
        /// Description presented to the model.
        pub description: String,
        /// JSON Schema describing accepted arguments.
        pub input_schema: Value,
    }

    /// A request to execute a named MCP tool.
    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct CallToolRequest {
        /// Server-local name of the tool to invoke.
        pub name: String,
        /// JSON arguments supplied to the tool.
        pub arguments: Value,
    }

    /// Content blocks returned by an MCP tool invocation.
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct CallToolResponse {
        /// Ordered content emitted by the tool.
        pub content: Vec<ToolContent>,
        /// Whether the content describes a tool-level failure.
        #[serde(default)]
        pub is_error: bool,
    }

    /// A content block returned by an MCP tool.
    #[derive(Debug, Deserialize)]
    #[serde(tag = "type")]
    pub enum ToolContent {
        /// Plain text content.
        #[serde(rename = "text")]
        Text {
            /// Text emitted by the tool.
            text: String,
        },
        /// Base64-encoded image content.
        #[serde(rename = "image")]
        Image {
            /// Base64-encoded image bytes.
            data: String,
            /// Media type of the encoded image.
            #[serde(rename = "mimeType")]
            mime_type: String,
        },
        /// Text or binary resource content.
        #[serde(rename = "resource")]
        Resource {
            /// Resource descriptor returned by the server.
            resource: Resource,
        },
    }

    /// Resource content referenced or embedded by a tool response.
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Resource {
        /// URI identifying the resource.
        pub uri: String,
        /// Optional media type of the resource.
        pub mime_type: Option<String>,
        /// Optional inline UTF-8 text content.
        pub text: Option<String>,
        /// Optional inline base64-encoded binary content.
        pub blob: Option<String>,
    }
}

/// Converts MCP content blocks into the runtime's compact JSON result shape.
///
/// A single block becomes `{"result": ...}` and multiple blocks become
/// `{"results": [...]}`. Images and non-text resources are represented by
/// descriptive placeholders rather than exposing binary data.
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

/// Converts MCP error content into a useful error string.
///
/// Text blocks are joined with newlines; if no textual representation is
/// available, the normalized JSON value is serialized instead.
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
