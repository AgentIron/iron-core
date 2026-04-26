//! Agent Skills support for iron-core.
//!
//! Skills are declarative instruction sets that models can discover, activate
//! on demand, and apply to tasks. They are lightweight alternatives to plugins
//! — no WASM execution, just behavioral guidance injected into prompts.
//!
//! ## Architecture
//!
//! - [`SkillSource`] trait abstracts discovery (filesystem, client-provided, etc.)
//! - [`SkillCatalog`] unifies discovered skills from all sources
//! - [`Skill`] represents a loaded skill with metadata and body content
//! - [`SkillRegistry`] is runtime-owned and manages the lifecycle
//! - Session-scoped activation is tracked in [`DurableSession`]

pub mod catalog;
pub mod source;

pub use catalog::SkillCatalog;
pub use source::{FilesystemSkillSource, SkillSource, StaticSkillSource};

/// A unique skill name used for identification and activation.
pub type SkillName = String;

/// Metadata extracted from a skill's frontmatter or provided by a client.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkillMetadata {
    /// Unique identifier for the skill (used for activation).
    pub id: SkillName,
    /// Human-readable display name.
    #[serde(alias = "name")]
    pub display_name: String,
    /// Human-readable description shown in the catalog.
    pub description: String,
    /// Where this skill came from.
    #[serde(default)]
    pub origin: SkillOrigin,
    /// Whether this skill should be auto-activated for new sessions.
    #[serde(default)]
    pub auto_activate: bool,
    /// Optional tags for categorization.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional list of tool names this skill recommends having available.
    #[serde(default)]
    pub requires_tools: Vec<String>,
    /// Optional list of capability IDs this skill requires.
    #[serde(default)]
    pub requires_capabilities: Vec<String>,
    /// Whether this skill requires elevated trust to activate.
    #[serde(default)]
    pub requires_trust: bool,
}

/// Where a skill was discovered.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SkillOrigin {
    /// Discovered from the project filesystem (`.agents/skills/`).
    ProjectFilesystem,
    /// Discovered from the user filesystem (`~/.agents/skills/`).
    UserFilesystem,
    /// Provided by the client at runtime (bundled or virtual).
    ClientProvided,
    /// Built into the runtime (future).
    #[default]
    BuiltIn,
}

/// The location of a skill, if it has a filesystem path.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkillLocation {
    /// Absolute path to the skill directory.
    pub directory: std::path::PathBuf,
    /// Path to the SKILL.md file.
    pub skill_file: std::path::PathBuf,
    /// Optional path to a resources directory.
    #[serde(default)]
    pub resources_dir: Option<std::path::PathBuf>,
}

/// A resource bundled with a skill.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkillResourceEntry {
    /// Resource path relative to the skill's virtual root.
    pub path: String,
    /// Human-readable description of the resource.
    pub description: String,
}

/// A fully loaded skill ready for activation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LoadedSkill {
    /// Metadata about the skill.
    pub metadata: SkillMetadata,
    /// Filesystem location, if applicable.
    pub location: Option<SkillLocation>,
    /// The body content (markdown instructions, frontmatter stripped).
    pub body: String,
    /// Bundled resources.
    pub resources: Vec<SkillResourceEntry>,
}

/// Severity level for a skill diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
}

/// A diagnostic message about skill discovery.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillDiagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
    pub skill_name: Option<String>,
}

/// Precedence order for resolving skill collisions.
/// Higher values win over lower values.
pub fn origin_precedence(origin: SkillOrigin) -> u8 {
    match origin {
        SkillOrigin::ProjectFilesystem => 4,
        SkillOrigin::ClientProvided => 3,
        SkillOrigin::UserFilesystem => 2,
        SkillOrigin::BuiltIn => 1,
    }
}

/// A record of an activated skill stored in session state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActivatedSkillRecord {
    /// Name of the activated skill.
    pub name: SkillName,
    /// The body content at time of activation.
    pub body: String,
    /// Resources available at time of activation.
    pub resources: Vec<SkillResourceEntry>,
}

/// Session-scoped skill state.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionSkillState {
    /// Skills currently activated in this session.
    pub active: Vec<ActivatedSkillRecord>,
}

impl SessionSkillState {
    /// Check if a skill is currently active.
    pub fn is_active(&self, name: &str) -> bool {
        self.active.iter().any(|r| r.name == name)
    }

    /// Activate a skill (idempotent).
    pub fn activate(&mut self, record: ActivatedSkillRecord) {
        if !self.is_active(&record.name) {
            self.active.push(record);
        }
    }

    /// Deactivate a skill by name.
    pub fn deactivate(&mut self, name: &str) {
        self.active.retain(|r| r.name != name);
    }

    /// Get the names of all active skills.
    pub fn active_names(&self) -> Vec<&str> {
        self.active.iter().map(|r| r.name.as_str()).collect()
    }

    /// Get the full instruction text for all active skills.
    pub fn active_skill_instructions(&self) -> String {
        let mut output = String::new();
        for skill in &self.active {
            if !output.is_empty() {
                output.push_str("\n\n");
            }
            output.push_str(&render_skill_content(&skill.name, &skill.body));
        }
        output
    }
}

pub(crate) fn render_skill_content(name: &str, body: &str) -> String {
    format!(
        "<skill_content name=\"{}\">\n{}\n</skill_content>",
        name, body
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_skill_state_activation_is_idempotent() {
        let mut state = SessionSkillState::default();

        let record = ActivatedSkillRecord {
            name: "test-skill".to_string(),
            body: "body".to_string(),
            resources: vec![],
        };

        state.activate(record.clone());
        assert_eq!(state.active.len(), 1);

        state.activate(record);
        assert_eq!(
            state.active.len(),
            1,
            "Duplicate activation should be idempotent"
        );
    }

    #[test]
    fn session_skill_state_deactivation() {
        let mut state = SessionSkillState::default();

        state.activate(ActivatedSkillRecord {
            name: "skill-a".to_string(),
            body: "body-a".to_string(),
            resources: vec![],
        });
        state.activate(ActivatedSkillRecord {
            name: "skill-b".to_string(),
            body: "body-b".to_string(),
            resources: vec![],
        });

        assert_eq!(state.active.len(), 2);

        state.deactivate("skill-a");
        assert_eq!(state.active.len(), 1);
        assert!(!state.is_active("skill-a"));
        assert!(state.is_active("skill-b"));
    }

    #[test]
    fn session_skill_state_active_skill_instructions() {
        let mut state = SessionSkillState::default();

        state.activate(ActivatedSkillRecord {
            name: "skill-a".to_string(),
            body: "Do A".to_string(),
            resources: vec![],
        });
        state.activate(ActivatedSkillRecord {
            name: "skill-b".to_string(),
            body: "Do B".to_string(),
            resources: vec![],
        });

        let instructions = state.active_skill_instructions();
        assert!(instructions.contains("<skill_content name=\"skill-a\">"));
        assert!(instructions.contains("Do A"));
        assert!(instructions.contains("<skill_content name=\"skill-b\">"));
        assert!(instructions.contains("Do B"));
    }

    #[test]
    fn skill_metadata_requires_trust_default() {
        // Verify that requires_trust defaults to false when deserializing
        let yaml = r#"
id: test
name: Test
description: A test skill
"#;
        let metadata: SkillMetadata = serde_yaml::from_str(yaml).unwrap();
        assert!(!metadata.requires_trust);
    }

    #[test]
    fn skill_metadata_requires_trust_parsed() {
        let yaml = r#"
id: test
name: Test
description: A test skill
requires_trust: true
"#;
        let metadata: SkillMetadata = serde_yaml::from_str(yaml).unwrap();
        assert!(metadata.requires_trust);
    }
}
