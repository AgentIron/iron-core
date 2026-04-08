use std::sync::Arc;

use crate::builtin::config::BuiltinToolConfig;
use crate::builtin::error::BuiltinToolError;
use crate::builtin::helpers::BuiltinMeta;
use crate::builtin::policy::NetworkPolicy;
use crate::error::LoopResult;
use crate::tool::{Tool, ToolDefinition, ToolFuture};
use serde_json::Value;

pub struct WebFetchTool {
    config: Arc<BuiltinToolConfig>,
}

impl WebFetchTool {
    pub fn new(config: BuiltinToolConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

impl Tool for WebFetchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "webfetch",
            "Fetch content from a URL and return it in a normalized text format. Subject to network policy and size limits.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch"
                    },
                    "format": {
                        "type": "string",
                        "description": "Output format: 'text' (default) or 'markdown'",
                        "enum": ["text", "markdown"]
                    }
                },
                "required": ["url"]
            }),
        )
        .with_approval(true)
    }

    fn execute(&self, _call_id: &str, arguments: Value) -> ToolFuture {
        let config = self.config.clone();
        Box::pin(async move { execute_webfetch(&config, arguments).await })
    }

    fn requires_approval(&self) -> bool {
        true
    }
}

async fn execute_webfetch(config: &BuiltinToolConfig, args: Value) -> LoopResult<Value> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'url' argument"))
        .map_err(crate::error::LoopError::from)?;

    let _format = args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("text");

    validate_url(url)?;

    if config.policy.network == NetworkPolicy::DenyAll {
        return Err(crate::error::LoopError::from(
            BuiltinToolError::network_denied("network access is denied by policy"),
        ));
    }

    let response = reqwest::get(url).await.map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::fetch_failed(format!(
            "fetch failed: {}",
            e
        )))
    })?;

    if !response.status().is_success() {
        return Err(crate::error::LoopError::from(
            BuiltinToolError::fetch_failed(format!("HTTP {}", response.status())),
        ));
    }

    let body = response.text().await.map_err(|e| {
        crate::error::LoopError::from(BuiltinToolError::fetch_failed(format!(
            "failed to read response: {}",
            e
        )))
    })?;

    let total_bytes = body.len();
    let (content, truncated) = if body.len() > config.max_fetch_bytes {
        let truncated_at = body.floor_char_boundary(config.max_fetch_bytes);
        (body[..truncated_at].to_string(), true)
    } else {
        (body, false)
    };

    let meta = if truncated {
        BuiltinMeta::with_truncation(total_bytes)
    } else {
        BuiltinMeta::empty()
    };

    Ok(serde_json::json!({
        "content": content,
        "url": url,
        "size": total_bytes,
        "truncated": truncated,
        "meta": meta,
    }))
}

fn validate_url(url: &str) -> Result<(), BuiltinToolError> {
    let parsed = url::Url::parse(url)
        .map_err(|_| BuiltinToolError::invalid_url(format!("invalid URL: {}", url)))?;

    match parsed.scheme() {
        "http" | "https" => Ok(()),
        scheme => Err(BuiltinToolError::invalid_url(format!(
            "unsupported URL scheme: {} (only http and https are supported)",
            scheme
        ))),
    }
}
