use crate::plugin::auth::OAuthRequirements;
use crate::plugin::network::NetworkPolicy;
use serde::{Deserialize, Serialize};

/// Plugin manifest containing metadata, capabilities, and exported tools
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Plugin identity and versioning
    #[serde(flatten)]
    pub identity: PluginIdentity,
    /// Publisher information
    pub publisher: PluginPublisher,
    /// User-facing presentation metadata
    pub presentation: PresentationMetadata,
    /// Network access policy
    pub network_policy: NetworkPolicy,
    /// Authentication requirements (optional)
    #[serde(default)]
    pub auth: Option<OAuthRequirements>,
    /// Exported tools provided by this plugin
    pub tools: Vec<ExportedTool>,
    /// Optional maximum WASM linear memory the plugin is willing to consume,
    /// in bytes. When set, the host uses the smaller of this value and the
    /// runtime ceiling; when unset, the runtime ceiling alone applies.
    #[serde(default)]
    pub max_memory_bytes: Option<u64>,
    /// Plugin API version
    #[serde(rename = "api_version")]
    pub api_version: String,
}

/// Plugin identity and versioning
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginIdentity {
    /// Unique plugin identifier (reverse domain format recommended)
    pub id: String,
    /// Human-readable plugin name
    pub name: String,
    /// Semantic version
    pub version: String,
}

/// Plugin publisher information
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginPublisher {
    /// Publisher name
    pub name: String,
    /// Optional publisher URL
    #[serde(default)]
    pub url: Option<String>,
    /// Optional contact email
    #[serde(default)]
    pub contact: Option<String>,
}

/// User-facing presentation metadata
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresentationMetadata {
    /// Short description (one line)
    pub description: String,
    /// Longer description with usage information
    #[serde(default)]
    pub long_description: Option<String>,
    /// Optional icon URL or data URI
    #[serde(default)]
    pub icon: Option<String>,
    /// Optional category for grouping
    #[serde(default)]
    pub category: Option<String>,
    /// Keywords for search/discovery
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// Metadata for a tool exported by a plugin
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportedTool {
    /// Tool name (unique within the plugin)
    pub name: String,
    /// Tool description for the model
    pub description: String,
    /// JSON schema for tool input
    pub input_schema: serde_json::Value,
    /// Whether this tool requires human approval
    #[serde(default)]
    pub requires_approval: bool,
    /// Per-tool authentication requirements
    #[serde(default)]
    pub auth_requirements: Option<ToolAuthRequirements>,
}

/// Authentication requirements for a specific tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolAuthRequirements {
    /// Required OAuth scopes for this tool
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Whether this tool is available without authentication
    #[serde(default)]
    pub available_unauthenticated: bool,
}

impl PluginManifest {
    /// Validate the manifest structure
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        // Validate identity
        if self.identity.id.is_empty() {
            return Err(ManifestValidationError::MissingField("identity.id".into()));
        }
        if self.identity.name.is_empty() {
            return Err(ManifestValidationError::MissingField(
                "identity.name".into(),
            ));
        }
        if self.identity.version.is_empty() {
            return Err(ManifestValidationError::MissingField(
                "identity.version".into(),
            ));
        }

        // Validate publisher
        if self.publisher.name.is_empty() {
            return Err(ManifestValidationError::MissingField(
                "publisher.name".into(),
            ));
        }

        // Validate presentation
        if self.presentation.description.is_empty() {
            return Err(ManifestValidationError::MissingField(
                "presentation.description".into(),
            ));
        }

        // Validate API version
        if self.api_version != "1.0" {
            return Err(ManifestValidationError::UnsupportedApiVersion(
                self.api_version.clone(),
            ));
        }

        // Validate tools have unique names
        let mut names = std::collections::HashSet::new();
        for tool in &self.tools {
            if !names.insert(&tool.name) {
                return Err(ManifestValidationError::DuplicateToolName(
                    tool.name.clone(),
                ));
            }
        }

        Ok(())
    }

    /// Get tool metadata by name
    pub fn get_tool(&self, name: &str) -> Option<&ExportedTool> {
        self.tools.iter().find(|t| t.name == name)
    }
}

/// Errors that can occur during manifest validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestValidationError {
    MissingField(String),
    UnsupportedApiVersion(String),
    DuplicateToolName(String),
}

impl std::fmt::Display for ManifestValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "Missing required field: {}", field),
            Self::UnsupportedApiVersion(version) => {
                write!(f, "Unsupported API version: {}", version)
            }
            Self::DuplicateToolName(name) => write!(f, "Duplicate tool name: {}", name),
        }
    }
}

impl std::error::Error for ManifestValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_valid_manifest() -> PluginManifest {
        PluginManifest {
            identity: PluginIdentity {
                id: "com.example.my-plugin".to_string(),
                name: "My Plugin".to_string(),
                version: "1.0.0".to_string(),
            },
            publisher: PluginPublisher {
                name: "Example Corp".to_string(),
                url: Some("https://example.com".to_string()),
                contact: None,
            },
            presentation: PresentationMetadata {
                description: "A test plugin".to_string(),
                long_description: None,
                icon: None,
                category: Some("integrations".to_string()),
                keywords: vec!["test".to_string()],
            },
            network_policy: NetworkPolicy::Allowlist(vec!["api.example.com".to_string()]),
            auth: None,
            tools: vec![ExportedTool {
                name: "fetch_data".to_string(),
                description: "Fetch data from the API".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
                requires_approval: false,
                auth_requirements: None,
            }],
            max_memory_bytes: None,
            api_version: "1.0".to_string(),
        }
    }

    #[test]
    fn test_valid_manifest() {
        let manifest = create_valid_manifest();
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn test_missing_identity_id() {
        let mut manifest = create_valid_manifest();
        manifest.identity.id = "".to_string();
        assert!(matches!(
            manifest.validate().unwrap_err(),
            ManifestValidationError::MissingField(field) if field == "identity.id"
        ));
    }

    #[test]
    fn test_duplicate_tool_names() {
        let mut manifest = create_valid_manifest();
        manifest.tools.push(manifest.tools[0].clone());
        assert!(matches!(
            manifest.validate().unwrap_err(),
            ManifestValidationError::DuplicateToolName(name) if name == "fetch_data"
        ));
    }

    #[test]
    fn test_unsupported_api_version() {
        let mut manifest = create_valid_manifest();
        manifest.api_version = "2.0".to_string();
        assert!(matches!(
            manifest.validate().unwrap_err(),
            ManifestValidationError::UnsupportedApiVersion(version) if version == "2.0"
        ));
    }
}
