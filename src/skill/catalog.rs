use crate::skill::source::SkillSource;
use crate::skill::{
    origin_precedence, DiagnosticLevel, LoadedSkill, SkillDiagnostic, SkillMetadata,
};
use std::collections::HashMap;
use tracing::debug;

/// A unified catalog of skills from all sources.
///
/// The catalog handles collision resolution using origin precedence:
/// project > client > user > built-in.
pub struct SkillCatalog {
    /// Skills indexed by name, after collision resolution.
    skills: HashMap<String, CatalogEntry>,
    /// Diagnostics from discovery (skipped skills, collisions, etc.).
    diagnostics: Vec<SkillDiagnostic>,
}

/// An entry in the skill catalog.
#[derive(Debug, Clone)]
struct CatalogEntry {
    skill: LoadedSkill,
}

impl SkillCatalog {
    /// Create a new empty catalog.
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Discover skills from all provided sources and build the catalog.
    ///
    /// Skills with the same name are resolved using origin precedence.
    pub fn discover(sources: &[Box<dyn SkillSource>]) -> Self {
        let mut catalog = Self::new();

        for source in sources.iter() {
            let discovered = source.discover();
            debug!(count = discovered.len(), "Discovered skills from source");

            for (name, _metadata) in discovered {
                if let Some(skill) = source.load(&name) {
                    catalog.insert(name, skill);
                } else {
                    catalog.diagnostics.push(SkillDiagnostic {
                        level: DiagnosticLevel::Warning,
                        message: format!("Skill '{}' discovered but failed to load", name),
                        skill_name: Some(name),
                    });
                }
            }

            // Collect any diagnostics from the source itself
            let source_diagnostics = source.diagnostics();
            catalog.diagnostics.extend(source_diagnostics);
        }

        catalog
    }

    /// Load a skill by name.
    ///
    /// Returns a clone of the skill if found.
    pub fn load(&self, name: &str) -> Option<LoadedSkill> {
        self.skills.get(name).map(|e| e.skill.clone())
    }

    /// Get metadata for all skills in the catalog.
    pub fn list(&self) -> Vec<&SkillMetadata> {
        self.skills.values().map(|e| &e.skill.metadata).collect()
    }

    /// Get all loaded skills in the catalog.
    pub fn list_all(&self) -> Vec<&LoadedSkill> {
        self.skills.values().map(|e| &e.skill).collect()
    }

    /// Get metadata for a specific skill.
    pub fn get(&self, name: &str) -> Option<&SkillMetadata> {
        self.skills.get(name).map(|e| &e.skill.metadata)
    }

    /// Check if a skill exists in the catalog.
    pub fn contains(&self, name: &str) -> bool {
        self.skills.contains_key(name)
    }

    /// Get the number of skills in the catalog.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Check if the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Get all diagnostics from discovery.
    pub fn diagnostics(&self) -> &[SkillDiagnostic] {
        &self.diagnostics
    }

    /// Extend discovery diagnostics with externally-generated entries.
    pub fn extend_diagnostics(&mut self, diagnostics: Vec<SkillDiagnostic>) {
        self.diagnostics.extend(diagnostics);
    }

    /// Register a skill directly into the catalog.
    pub fn register(&mut self, skill: LoadedSkill) {
        self.insert(skill.metadata.id.clone(), skill);
    }

    /// Insert or update a skill in the catalog.
    ///
    /// If a skill with the same name already exists, the one with higher
    /// origin precedence wins.
    fn insert(&mut self, name: String, skill: LoadedSkill) {
        if let Some(existing) = self.skills.get(&name) {
            let existing_prec = origin_precedence(existing.skill.metadata.origin);
            let new_prec = origin_precedence(skill.metadata.origin);

            if existing_prec >= new_prec {
                self.diagnostics.push(SkillDiagnostic {
                    level: DiagnosticLevel::Info,
                    message: format!(
                        "Skill '{}' from {:?} was shadowed by {:?} (lower precedence)",
                        name, skill.metadata.origin, existing.skill.metadata.origin
                    ),
                    skill_name: Some(name.clone()),
                });
                return;
            } else {
                self.diagnostics.push(SkillDiagnostic {
                    level: DiagnosticLevel::Info,
                    message: format!(
                        "Skill '{}' from {:?} replaced {:?} (higher precedence)",
                        name, skill.metadata.origin, existing.skill.metadata.origin
                    ),
                    skill_name: Some(name.clone()),
                });
            }
        }

        self.skills.insert(name, CatalogEntry { skill });
    }
}

impl Default for SkillCatalog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::source::StaticSkillSource;
    use crate::skill::SkillOrigin;

    fn make_skill(name: &str, origin: SkillOrigin) -> LoadedSkill {
        LoadedSkill {
            metadata: SkillMetadata {
                id: name.to_string(),
                display_name: name.to_string(),
                description: format!("{} skill", name),
                origin,
                auto_activate: false,
                tags: Vec::new(),
                requires_tools: Vec::new(),
                requires_capabilities: Vec::new(),
                requires_trust: false,
            },
            location: None,
            body: format!("# {}\nInstructions", name),
            resources: Vec::new(),
        }
    }

    #[test]
    fn catalog_resolves_collisions_by_precedence() {
        let mut user_source = StaticSkillSource::new();
        user_source.register(make_skill("test", SkillOrigin::UserFilesystem));

        let mut project_source = StaticSkillSource::new();
        project_source.register(make_skill("test", SkillOrigin::ProjectFilesystem));

        let sources: Vec<Box<dyn SkillSource>> =
            vec![Box::new(user_source), Box::new(project_source)];

        let catalog = SkillCatalog::discover(&sources);

        assert_eq!(catalog.len(), 1);
        let skill = catalog.get("test").unwrap();
        assert_eq!(skill.origin, SkillOrigin::ProjectFilesystem);
    }

    #[test]
    fn catalog_preserves_both_when_no_collision() {
        let mut source1 = StaticSkillSource::new();
        source1.register(make_skill("skill-a", SkillOrigin::UserFilesystem));

        let mut source2 = StaticSkillSource::new();
        source2.register(make_skill("skill-b", SkillOrigin::ProjectFilesystem));

        let sources: Vec<Box<dyn SkillSource>> = vec![Box::new(source1), Box::new(source2)];

        let catalog = SkillCatalog::discover(&sources);

        assert_eq!(catalog.len(), 2);
        assert!(catalog.contains("skill-a"));
        assert!(catalog.contains("skill-b"));
    }

    #[test]
    fn catalog_loads_from_correct_source() {
        let mut source = StaticSkillSource::new();
        source.register(make_skill("loadable", SkillOrigin::ClientProvided));

        let sources: Vec<Box<dyn SkillSource>> = vec![Box::new(source)];
        let catalog = SkillCatalog::discover(&sources);

        let loaded = catalog.load("loadable").unwrap();
        assert_eq!(loaded.metadata.display_name, "loadable");
    }

    #[test]
    fn catalog_records_collision_diagnostics() {
        let mut user_source = StaticSkillSource::new();
        user_source.register(make_skill("test", SkillOrigin::UserFilesystem));

        let mut project_source = StaticSkillSource::new();
        project_source.register(make_skill("test", SkillOrigin::ProjectFilesystem));

        let sources: Vec<Box<dyn SkillSource>> =
            vec![Box::new(user_source), Box::new(project_source)];

        let catalog = SkillCatalog::discover(&sources);

        let diagnostics = catalog.diagnostics();
        assert!(
            !diagnostics.is_empty(),
            "Should have recorded collision diagnostics"
        );
        let has_shadow_diagnostic = diagnostics
            .iter()
            .any(|d| d.message.contains("shadowed") || d.message.contains("replaced"));
        assert!(
            has_shadow_diagnostic,
            "Should have a shadow/replace diagnostic: {:?}",
            diagnostics
        );
    }

    #[test]
    fn catalog_list_returns_all_metadata() {
        let mut source = StaticSkillSource::new();
        source.register(make_skill("skill-a", SkillOrigin::ClientProvided));
        source.register(make_skill("skill-b", SkillOrigin::ClientProvided));

        let sources: Vec<Box<dyn SkillSource>> = vec![Box::new(source)];
        let catalog = SkillCatalog::discover(&sources);

        let list = catalog.list();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|m| m.id == "skill-a"));
        assert!(list.iter().any(|m| m.id == "skill-b"));
    }
}
