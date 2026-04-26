use crate::skill::{
    LoadedSkill, SkillDiagnostic, SkillLocation, SkillMetadata, SkillOrigin, SkillResourceEntry,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// A source that can discover and load skills.
pub trait SkillSource: Send + Sync {
    /// Discover available skills from this source.
    ///
    /// Returns a map of skill names to metadata. The source may not load
    /// full skill bodies at this stage.
    fn discover(&self) -> HashMap<String, SkillMetadata>;

    /// Load a full skill by name.
    ///
    /// Returns `None` if the skill is not found or cannot be loaded.
    fn load(&self, name: &str) -> Option<LoadedSkill>;

    /// Return any diagnostics accumulated during discovery/load.
    ///
    /// Default implementation returns an empty vector.
    fn diagnostics(&self) -> Vec<SkillDiagnostic> {
        Vec::new()
    }
}

/// A skill source that scans filesystem directories for skill directories.
///
/// Each skill directory must contain a `SKILL.md` file with YAML frontmatter.
pub struct FilesystemSkillSource {
    root: PathBuf,
    origin: SkillOrigin,
    diagnostics: std::sync::Mutex<Vec<SkillDiagnostic>>,
}

impl FilesystemSkillSource {
    /// Create a new filesystem skill source.
    pub fn new(root: impl Into<PathBuf>, origin: SkillOrigin) -> Self {
        Self {
            root: root.into(),
            origin,
            diagnostics: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Get the root directory being scanned.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Parse a SKILL.md file into metadata and body.
    fn parse_skill_file(
        &self,
        path: &Path,
    ) -> Option<(SkillMetadata, String, Vec<SkillResourceEntry>)> {
        let content = std::fs::read_to_string(path).ok()?;

        // Split frontmatter from body
        let (frontmatter, body) = split_frontmatter(&content)?;

        // Parse YAML frontmatter
        let metadata: SkillMetadata = match serde_yaml::from_str(frontmatter) {
            Ok(m) => m,
            Err(e) => {
                warn!(path = %path.display(), error = %e, "Failed to parse skill frontmatter, using fallback metadata");
                // Extract skill name from parent directory or filename
                let skill_name = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                self.diagnostics.lock().unwrap().push(SkillDiagnostic {
                    level: crate::skill::DiagnosticLevel::Warning,
                    message: format!(
                        "Failed to parse frontmatter for '{}': {}. Using fallback metadata.",
                        skill_name, e
                    ),
                    skill_name: Some(skill_name.clone()),
                });
                SkillMetadata {
                    id: skill_name.clone(),
                    display_name: skill_name.clone(),
                    description: format!("Skill '{}' with unparsable frontmatter", skill_name),
                    origin: self.origin,
                    auto_activate: false,
                    tags: Vec::new(),
                    requires_tools: Vec::new(),
                    requires_capabilities: Vec::new(),
                    requires_trust: false,
                }
            }
        };

        // Discover resources
        let resources = self.discover_resources(path.parent()?);

        Some((metadata, body.to_string(), resources))
    }

    /// Discover resource files in the skill directory.
    fn discover_resources(&self, skill_dir: &Path) -> Vec<SkillResourceEntry> {
        let resources_dir = skill_dir.join("resources");
        if !resources_dir.exists() || !resources_dir.is_dir() {
            return Vec::new();
        }

        let mut resources = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&resources_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let rel_path = path
                        .strip_prefix(&resources_dir)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string();
                    resources.push(SkillResourceEntry {
                        path: rel_path,
                        description: String::new(), // Could be enhanced with .md sidecars
                    });
                }
            }
        }
        resources
    }
}

impl SkillSource for FilesystemSkillSource {
    fn discover(&self) -> HashMap<String, SkillMetadata> {
        let mut skills = HashMap::new();

        if !self.root.exists() || !self.root.is_dir() {
            return skills;
        }

        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) => {
                warn!(root = %self.root.display(), error = %e, "Failed to read skills directory");
                return skills;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let skill_file = path.join("SKILL.md");
            if !skill_file.exists() {
                continue;
            }

            if let Some((mut metadata, _, _)) = self.parse_skill_file(&skill_file) {
                // Ensure origin matches the source
                metadata.origin = self.origin;
                skills.insert(metadata.id.clone(), metadata);
            }
        }

        debug!(count = skills.len(), root = %self.root.display(), "Discovered skills");
        skills
    }

    fn load(&self, name: &str) -> Option<LoadedSkill> {
        let skill_dir = self.root.join(name);
        let skill_file = skill_dir.join("SKILL.md");

        if !skill_file.exists() {
            return None;
        }

        let (mut metadata, body, resources) = self.parse_skill_file(&skill_file)?;
        metadata.origin = self.origin;

        Some(LoadedSkill {
            metadata,
            location: Some(SkillLocation {
                directory: skill_dir.clone(),
                skill_file,
                resources_dir: skill_dir
                    .join("resources")
                    .exists()
                    .then(|| skill_dir.join("resources")),
            }),
            body,
            resources,
        })
    }

    fn diagnostics(&self) -> Vec<SkillDiagnostic> {
        self.diagnostics.lock().unwrap().drain(..).collect()
    }
}

/// A skill source backed by statically-provided skills.
///
/// Useful for client-provided or bundled skills that don't exist on disk.
pub struct StaticSkillSource {
    skills: HashMap<String, LoadedSkill>,
}

impl StaticSkillSource {
    /// Create a new empty static skill source.
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    /// Register a skill with this source.
    pub fn register(&mut self, skill: LoadedSkill) {
        self.skills.insert(skill.metadata.id.clone(), skill);
    }

    /// Register a skill from raw components.
    pub fn register_raw(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        body: impl Into<String>,
    ) {
        let name = name.into();
        let skill = LoadedSkill {
            metadata: SkillMetadata {
                id: name.clone(),
                display_name: name.clone(),
                description: description.into(),
                origin: SkillOrigin::ClientProvided,
                auto_activate: false,
                tags: Vec::new(),
                requires_tools: Vec::new(),
                requires_capabilities: Vec::new(),
                requires_trust: false,
            },
            location: None,
            body: body.into(),
            resources: Vec::new(),
        };
        self.skills.insert(name, skill);
    }
}

impl Default for StaticSkillSource {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillSource for StaticSkillSource {
    fn discover(&self) -> HashMap<String, SkillMetadata> {
        self.skills
            .values()
            .map(|s| (s.metadata.id.clone(), s.metadata.clone()))
            .collect()
    }

    fn load(&self, name: &str) -> Option<LoadedSkill> {
        self.skills.get(name).cloned()
    }
}

/// Split markdown content into YAML frontmatter and body.
///
/// Expects frontmatter delimited by `---` at the start of the file.
fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let content = content.trim_start();

    // Check for --- delimiter
    if !content.starts_with("---") {
        // No frontmatter - entire content is body
        return Some(("", content));
    }

    // Find the closing ---
    let after_open = &content[3..];
    let close_idx = after_open.find("---")?;

    let frontmatter = after_open[..close_idx].trim();
    let body = after_open[close_idx + 3..].trim_start();

    Some((frontmatter, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_frontmatter_with_valid_delimiters() {
        let content = "---\nname: test\n---\n# Body\nContent here.";
        let (frontmatter, body) = split_frontmatter(content).unwrap();
        assert_eq!(frontmatter.trim(), "name: test");
        assert_eq!(body, "# Body\nContent here.");
    }

    #[test]
    fn split_frontmatter_without_delimiters() {
        let content = "# Just body\nNo frontmatter.";
        let (frontmatter, body) = split_frontmatter(content).unwrap();
        assert_eq!(frontmatter, "");
        assert_eq!(body, "# Just body\nNo frontmatter.");
    }

    #[test]
    fn static_source_register_and_load() {
        let mut source = StaticSkillSource::new();
        source.register_raw(
            "test-skill",
            "A test skill",
            "# Instructions\nDo something.",
        );

        let skills = source.discover();
        assert_eq!(skills.len(), 1);
        assert!(skills.contains_key("test-skill"));

        let loaded = source.load("test-skill").unwrap();
        assert_eq!(loaded.metadata.display_name, "test-skill");
        assert_eq!(loaded.body, "# Instructions\nDo something.");
    }

    #[test]
    fn filesystem_source_discovers_skills_from_directory() {
        let temp_dir =
            std::env::temp_dir().join(format!("iron-core-test-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);

        // Create a skill directory with a valid SKILL.md
        let skill_dir = temp_dir.join("test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nid: test-skill\nname: Test Skill\ndescription: A test skill\n---\n# Instructions\nDo something."
        ).unwrap();

        let source = FilesystemSkillSource::new(&temp_dir, SkillOrigin::ProjectFilesystem);
        let skills = source.discover();

        assert_eq!(skills.len(), 1);
        assert!(skills.contains_key("test-skill"));
        let metadata = skills.get("test-skill").unwrap();
        assert_eq!(metadata.display_name, "Test Skill");
        assert_eq!(metadata.description, "A test skill");

        let loaded = source.load("test-skill").unwrap();
        assert_eq!(loaded.body, "# Instructions\nDo something.");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn filesystem_source_handles_malformed_yaml_gracefully() {
        let temp_dir =
            std::env::temp_dir().join(format!("iron-core-test-malformed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);

        // Create a skill directory with malformed YAML frontmatter
        let skill_dir = temp_dir.join("bad-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nthis is not: valid: yaml: [\n---\n# Instructions\nDo something.",
        )
        .unwrap();

        let source = FilesystemSkillSource::new(&temp_dir, SkillOrigin::ProjectFilesystem);
        let skills = source.discover();

        // Should still discover the skill with fallback metadata
        assert_eq!(skills.len(), 1);
        assert!(skills.contains_key("bad-skill"));
        let metadata = skills.get("bad-skill").unwrap();
        assert_eq!(metadata.display_name, "bad-skill");
        assert!(metadata.description.contains("unparsable"));

        // Should have recorded a diagnostic
        let diagnostics = source.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].level, crate::skill::DiagnosticLevel::Warning);
        assert!(diagnostics[0]
            .message
            .contains("Failed to parse frontmatter"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn filesystem_source_skips_directories_without_skill_md() {
        let temp_dir =
            std::env::temp_dir().join(format!("iron-core-test-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);

        // Create a directory without SKILL.md
        let empty_dir = temp_dir.join("empty-dir");
        std::fs::create_dir_all(&empty_dir).unwrap();
        std::fs::write(empty_dir.join("README.md"), "# Not a skill").unwrap();

        let source = FilesystemSkillSource::new(&temp_dir, SkillOrigin::ProjectFilesystem);
        let skills = source.discover();

        assert_eq!(skills.len(), 0);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
