use crate::tool::{ToolDefinition, ToolRegistry};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const RESERVED_METHODS: &[&str] = &["call", "available", "describe", "python_exec"];
const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

#[derive(Debug, Clone)]
pub(crate) struct ToolCatalogEntry {
    definition: ToolDefinition,
    alias: Option<String>,
}

impl ToolCatalogEntry {
    pub(crate) fn name(&self) -> &str {
        &self.definition.name
    }

    pub(crate) fn summary_json(&self) -> Value {
        json!({
            "name": self.definition.name,
            "alias": self.alias,
            "description": self.definition.description,
            "requires_approval": self.definition.requires_approval,
        })
    }

    pub(crate) fn describe_json(&self) -> Value {
        json!({
            "name": self.definition.name,
            "alias": self.alias,
            "description": self.definition.description,
            "requires_approval": self.definition.requires_approval,
            "input_schema": self.definition.input_schema,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ToolCatalog {
    entries: Vec<ToolCatalogEntry>,
    by_name: HashMap<String, usize>,
    by_alias: HashMap<String, usize>,
}

impl ToolCatalog {
    pub(crate) fn from_registry(registry: &ToolRegistry) -> Self {
        let mut definitions = registry.definitions();
        definitions.sort_by(|a, b| a.name.cmp(&b.name));
        Self::from_definitions(definitions)
    }

    pub(crate) fn from_definitions(definitions: Vec<ToolDefinition>) -> Self {
        let mut entries = Vec::with_capacity(definitions.len());
        let mut by_name = HashMap::with_capacity(definitions.len());
        let mut by_alias = HashMap::with_capacity(definitions.len());
        let mut assigned_aliases = HashSet::with_capacity(definitions.len());

        for definition in definitions {
            let alias = alias_for_tool(&definition.name, &assigned_aliases);
            if let Some(alias_name) = alias.as_ref() {
                assigned_aliases.insert(alias_name.clone());
            }

            let index = entries.len();
            by_name.insert(definition.name.clone(), index);
            if let Some(alias_name) = alias.as_ref() {
                by_alias.insert(alias_name.clone(), index);
            }
            entries.push(ToolCatalogEntry { definition, alias });
        }

        Self {
            entries,
            by_name,
            by_alias,
        }
    }

    pub(crate) fn namespace_object(&self) -> monty::MontyObject {
        monty::MontyObject::Dataclass {
            name: "IronTools".to_string(),
            type_id: 0,
            field_names: Vec::new(),
            attrs: monty::DictPairs::from(Vec::<(monty::MontyObject, monty::MontyObject)>::new()),
            frozen: true,
        }
    }

    pub(crate) fn available_json(&self) -> Value {
        Value::Array(
            self.entries
                .iter()
                .map(ToolCatalogEntry::summary_json)
                .collect(),
        )
    }

    pub(crate) fn describe_json(&self, name: &str) -> Option<Value> {
        self.entry_by_name(name)
            .map(ToolCatalogEntry::describe_json)
    }

    pub(crate) fn entry_by_name(&self, name: &str) -> Option<&ToolCatalogEntry> {
        self.by_name.get(name).map(|idx| &self.entries[*idx])
    }

    pub(crate) fn entry_by_alias(&self, alias: &str) -> Option<&ToolCatalogEntry> {
        self.by_alias.get(alias).map(|idx| &self.entries[*idx])
    }
}

fn alias_for_tool(name: &str, assigned_aliases: &HashSet<String>) -> Option<String> {
    if RESERVED_METHODS.contains(&name) {
        return None;
    }

    let mut alias = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            alias.push(ch);
        } else {
            alias.push('_');
        }
    }

    if alias.is_empty() {
        return None;
    }

    let first = alias.chars().next().unwrap();
    if !matches!(first, '_' | 'a'..='z' | 'A'..='Z') {
        alias.insert_str(0, "tool_");
    }

    if RESERVED_METHODS.contains(&alias.as_str()) || PYTHON_KEYWORDS.contains(&alias.as_str()) {
        alias.insert_str(0, "tool_");
    }

    if assigned_aliases.contains(&alias) {
        return None;
    }

    Some(alias)
}

pub(crate) fn is_tools_namespace(obj: &monty::MontyObject) -> bool {
    matches!(obj, monty::MontyObject::Dataclass { name, .. } if name == "IronTools")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn alias_generation_sanitizes_names() {
        let catalog = ToolCatalog::from_definitions(vec![
            ToolDefinition::new("read-file", "desc", json!({})),
            ToolDefinition::new("123tool", "desc", json!({})),
        ]);

        assert_eq!(
            catalog.entry_by_name("read-file").unwrap().alias.as_deref(),
            Some("read_file")
        );
        assert_eq!(
            catalog.entry_by_name("123tool").unwrap().alias.as_deref(),
            Some("tool_123tool")
        );
    }

    #[test]
    fn alias_generation_omits_reserved_and_colliding_names() {
        let catalog = ToolCatalog::from_definitions(vec![
            ToolDefinition::new("call", "desc", json!({})),
            ToolDefinition::new("foo-bar", "desc", json!({})),
            ToolDefinition::new("foo_bar", "desc", json!({})),
            ToolDefinition::new("python_exec", "desc", json!({})),
        ]);

        assert_eq!(
            catalog.entry_by_name("call").unwrap().alias.as_deref(),
            None
        );
        assert_eq!(
            catalog.entry_by_name("foo-bar").unwrap().alias.as_deref(),
            Some("foo_bar")
        );
        assert_eq!(
            catalog.entry_by_name("foo_bar").unwrap().alias.as_deref(),
            None
        );
        assert_eq!(
            catalog
                .entry_by_name("python_exec")
                .unwrap()
                .alias
                .as_deref(),
            None
        );
    }
}
