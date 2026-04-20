//! Tool registry and definitions
//!
//! Tool registration, execution, and approval management.

use crate::error::RuntimeResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Type alias for a tool execution future.
pub type ToolFuture = Pin<Box<dyn Future<Output = RuntimeResult<Value>> + Send>>;

/// Executable tool trait.
///
/// # Contract: do not block the orchestration runtime
///
/// The `execute` method returns a future that runs on `iron-core`'s async
/// orchestration runtime (whether private or shared). Implementations **must
/// not** perform blocking I/O, CPU-heavy computation, or any operation that
/// could stall the executor inside the returned future. Blocking work should
/// be offloaded to [`tokio::task::spawn_blocking`] or a dedicated thread pool.
///
/// Built-in helpers like [`FunctionTool`] already route sync handlers through
/// `spawn_blocking`. Custom async `Tool` implementations that need to call
/// blocking APIs should do the same.
pub trait Tool: Send + Sync {
    /// Get the tool definition (metadata for the model)
    fn definition(&self) -> ToolDefinition;

    /// Execute the tool with the given arguments.
    ///
    /// The returned future runs on the orchestration runtime.
    /// See the [trait-level contract](Tool#contract-do-not-block-the-orchestration-runtime)
    /// for blocking requirements.
    fn execute(&self, call_id: &str, arguments: Value) -> ToolFuture;

    /// Check if this tool requires human approval
    fn requires_approval(&self) -> bool;
}

impl std::fmt::Debug for dyn Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let def = self.definition();
        f.debug_struct("Tool")
            .field("name", &def.name)
            .field("requires_approval", &def.requires_approval)
            .finish()
    }
}

/// Model-facing tool metadata and schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Unique tool name exposed to the model.
    pub name: String,
    /// Natural-language description shown to the model.
    pub description: String,
    /// JSON Schema describing accepted arguments.
    pub input_schema: Value,
    /// Whether this tool requires explicit user approval before execution.
    pub requires_approval: bool,
}

impl ToolDefinition {
    /// Create a new tool definition.
    pub fn new<S1: Into<String>, S2: Into<String>>(
        name: S1,
        description: S2,
        input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            requires_approval: false,
        }
    }

    /// Set whether this tool requires approval.
    pub fn with_approval(mut self, requires: bool) -> Self {
        self.requires_approval = requires;
        self
    }

    /// Convert this definition into the provider-facing schema type.
    pub fn to_provider_definition(&self) -> iron_providers::ToolDefinition {
        iron_providers::ToolDefinition::new(
            self.name.clone(),
            self.description.clone(),
            self.input_schema.clone(),
        )
    }
}

/// Registry of tools available to the agent runtime.
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    version: u64,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tool_count", &self.tools.len())
            .field("tool_names", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        Self {
            tools: self.tools.clone(),
            version: self.version,
        }
    }
}

impl ToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or replace a tool by its definition name.
    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        let definition = tool.definition();
        self.tools.insert(definition.name, Arc::new(tool));
        self.version += 1;
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    /// Check whether a tool with the given name exists.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Return all tool definitions registered in this registry.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    /// Return all tool definitions converted for provider requests.
    pub fn provider_definitions(&self) -> Vec<iron_providers::ToolDefinition> {
        self.definitions()
            .into_iter()
            .map(|d| d.to_provider_definition())
            .collect()
    }

    /// Get the number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Remove all registered tools.
    pub fn clear(&mut self) {
        self.tools.clear();
        self.version += 1;
    }

    /// Return the current mutation version for cache invalidation.
    pub fn version(&self) -> u64 {
        self.version
    }
}

/// A simple function-based tool wrapper.
///
/// Sync handlers are automatically routed through [`tokio::task::spawn_blocking`]
/// so they do not block the async orchestration runtime.
pub struct FunctionTool {
    definition: ToolDefinition,
    handler: Arc<dyn Fn(Value) -> RuntimeResult<Value> + Send + Sync>,
}

impl std::fmt::Debug for FunctionTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FunctionTool")
            .field("definition", &self.definition)
            .finish()
    }
}

impl FunctionTool {
    /// Create a new function-backed tool.
    pub fn new<F>(definition: ToolDefinition, handler: F) -> Self
    where
        F: Fn(Value) -> RuntimeResult<Value> + Send + Sync + 'static,
    {
        Self {
            definition,
            handler: Arc::new(handler),
        }
    }

    /// Create a simple schemaless object tool with a sync handler.
    pub fn simple<S1, S2, F>(name: S1, description: S2, handler: F) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
        F: Fn(Value) -> RuntimeResult<Value> + Send + Sync + 'static,
    {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {},
        });

        let definition = ToolDefinition::new(name, description, schema);
        Self::new(definition, handler)
    }
}

impl Tool for FunctionTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    fn execute(&self, _call_id: &str, arguments: Value) -> ToolFuture {
        let handler = self.handler.clone();
        Box::pin(async move {
            match tokio::task::spawn_blocking(move || handler(arguments)).await {
                Ok(result) => result,
                Err(e) => Err(crate::error::RuntimeError::tool_execution(e.to_string())),
            }
        })
    }

    fn requires_approval(&self) -> bool {
        self.definition.requires_approval
    }
}
