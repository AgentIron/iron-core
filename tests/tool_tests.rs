//! Tests for iron-core tools

use iron_core::tool::{FunctionTool, Tool, ToolDefinition, ToolRegistry};
use serde_json::json;

#[test]
fn test_tool_definition_new() {
    let schema = json!({"type": "object"});
    let def = ToolDefinition::new("test_tool", "A test tool", schema.clone());
    assert_eq!(def.name, "test_tool");
    assert_eq!(def.description, "A test tool");
    assert_eq!(def.input_schema, schema);
    assert!(!def.requires_approval);
}

#[test]
fn test_tool_definition_with_approval() {
    let def = ToolDefinition::new("test", "desc", json!({})).with_approval(true);
    assert!(def.requires_approval);
}

#[test]
fn test_tool_registry_new() {
    let registry = ToolRegistry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
}

#[test]
fn test_tool_registry_register() {
    let mut registry = ToolRegistry::new();
    let tool = FunctionTool::simple("my_tool", "Does something", |_| Ok(json!("result")));
    registry.register(tool);
    assert_eq!(registry.len(), 1);
    assert!(registry.contains("my_tool"));
}

#[test]
fn test_tool_registry_get() {
    let mut registry = ToolRegistry::new();
    let tool = FunctionTool::simple("my_tool", "Does something", |_| Ok(json!("result")));
    registry.register(tool);

    let retrieved = registry.get("my_tool");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().definition().name, "my_tool");
}

#[test]
fn test_tool_registry_get_missing() {
    let registry = ToolRegistry::new();
    assert!(registry.get("missing").is_none());
}

#[test]
fn test_tool_registry_definitions() {
    let mut registry = ToolRegistry::new();
    let tool = FunctionTool::simple("tool1", "First tool", |_| Ok(json!(1)));
    registry.register(tool);

    let defs = registry.definitions();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name, "tool1");
}

#[test]
fn test_tool_registry_clear() {
    let mut registry = ToolRegistry::new();
    let tool = FunctionTool::simple("tool1", "First tool", |_| Ok(json!(1)));
    registry.register(tool);
    registry.clear();
    assert!(registry.is_empty());
}

#[test]
fn test_function_tool_simple() {
    let tool = FunctionTool::simple("adder", "Adds numbers", |_| Ok(json!(42)));
    let def = tool.definition();
    assert_eq!(def.name, "adder");
    assert_eq!(def.description, "Adds numbers");
    assert!(!def.requires_approval);
}

#[tokio::test]
async fn test_function_tool_execute() {
    let tool = FunctionTool::simple("test", "desc", |_| Ok(json!("result")));
    let result: Result<serde_json::Value, _> = tool.execute("call_1", json!({})).await;
    assert_eq!(result.unwrap(), json!("result"));
}
