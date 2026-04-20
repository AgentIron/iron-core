//! Handoff export/import for session continuity.
//!
//! The handoff bundle transfers conversation state (messages, instructions,
//! compacted context) but explicitly EXCLUDES:
//! - Runtime-local tool capabilities (including MCP server inventory)
//! - Runtime-local capability configurations
//! - Session MCP enablement state
//! - Session plugin enablement state and plugin auth bindings
//! - Other runtime-local state that may differ between environments
//!
//! This preserves portability: the destination runtime determines its own
//! available integrations and tools.

use crate::context::config::ContextManagementConfig;
use crate::context::models::{CompactedContext, PortabilityNote};
use crate::durable::{DurableSession, SessionId, StructuredMessage};
use crate::skill::SessionSkillState;
use serde::{Deserialize, Serialize};

pub const HANDOFF_BUNDLE_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandoffBundleMetadata {
    pub version: String,
    pub source_model: String,
    pub source_provider: Option<String>,
    pub source_session_id: String,
    pub size_estimate_tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandoffBundle {
    pub version: String,
    pub instructions: Option<String>,
    pub handoff_note: String,
    pub compacted_context: CompactedContext,
    pub recent_tail: Vec<StructuredMessage>,
    pub skill_state: SessionSkillState,
    pub metadata: HandoffBundleMetadata,
}

pub struct HandoffExporter;

impl HandoffExporter {
    pub fn can_export(session: &DurableSession) -> bool {
        !session.tool_records.iter().any(|r| {
            matches!(
                r.status,
                crate::durable::ToolRecordStatus::PendingApproval
                    | crate::durable::ToolRecordStatus::Running
            )
        })
    }

    pub fn export(
        session: &DurableSession,
        model: &str,
        compacted: Option<&CompactedContext>,
        tail: Vec<StructuredMessage>,
        config: &ContextManagementConfig,
        provider_name: Option<&str>,
    ) -> Result<HandoffBundle, String> {
        if !Self::can_export(session) {
            return Err("Cannot export handoff bundle: session has active tool calls".into());
        }

        let compacted_context = compacted.cloned().unwrap_or_default();

        let target_tokens = config.handoff_export.default_target_tokens;
        let size_estimate =
            estimate_bundle_size(&compacted_context, &tail, session.instructions.as_deref(), &session.skill_state);

        let mut handoff_note = format!(
            "Context transferred from session {} on model {}.",
            session.id, model
        );

        let mut loss_notes = Vec::new();
        for msg in &tail {
            for block in msg.content_blocks().iter() {
                if let crate::durable::ContentBlock::Resource { ref uri, .. } = block {
                    if is_local_resource(uri.as_str()) {
                        loss_notes.push(format!(
                            "Local resource '{}' may not be accessible at destination",
                            uri
                        ));
                    }
                }
            }
        }

        if !loss_notes.is_empty() {
            handoff_note.push_str(&format!("\nPortability notes: {}", loss_notes.join("; ")));
        }

        if size_estimate > target_tokens {
            handoff_note.push_str(&format!(
                "\nBundle size (~{} tokens) exceeds target (~{} tokens).",
                size_estimate, target_tokens
            ));
        }

        let mut bundle = HandoffBundle {
            version: HANDOFF_BUNDLE_VERSION.to_string(),
            instructions: session.instructions.clone(),
            handoff_note,
            compacted_context,
            recent_tail: tail,
            skill_state: session.skill_state.clone(),
            metadata: HandoffBundleMetadata {
                version: HANDOFF_BUNDLE_VERSION.to_string(),
                source_model: model.to_string(),
                source_provider: provider_name.map(|s| s.to_string()),
                source_session_id: session.id.to_string(),
                size_estimate_tokens: size_estimate,
            },
        };

        if config.handoff_export.include_portability_notes && !loss_notes.is_empty() {
            for note in loss_notes {
                bundle
                    .compacted_context
                    .portability_notes
                    .get_or_insert_with(Vec::new)
                    .push(PortabilityNote::non_portable(&note, "local-only resource"));
            }
        }

        Ok(bundle)
    }
}

pub struct HandoffImporter;

impl HandoffImporter {
    pub fn hydrate(target: &mut DurableSession, bundle: HandoffBundle) -> Result<(), String> {
        if target.id == SessionId::new() {
            return Err(
                "Cannot hydrate into a freshly-constructed session with mismatched identity".into(),
            );
        }

        if !bundle.handoff_note.is_empty() {
            target.add_agent_text(format!("[Handoff] {}", bundle.handoff_note));
        }

        if let Some(instr) = bundle.instructions {
            target.set_instructions(instr);
        }

        if !bundle.compacted_context.is_empty() {
            let rendered = bundle.compacted_context.render_to_text();
            if !rendered.is_empty() {
                target.add_agent_text(format!("[Compacted context]\n{}", rendered));
            }
        }

        for msg in bundle.recent_tail {
            target.messages.push(msg);
        }

        // Restore skill state so activated skills survive handoff
        target.skill_state = bundle.skill_state;

        // Note: MCP server and plugin enablement are NOT imported as part of handoff.
        // The destination runtime determines its own tool availability.

        Ok(())
    }

    pub fn hydrate_into_new(bundle: HandoffBundle) -> DurableSession {
        let mut session = DurableSession::new(SessionId::new());

        if let Some(instr) = bundle.instructions {
            session.set_instructions(instr);
        }

        if !bundle.handoff_note.is_empty() {
            session.add_agent_text(format!("[Handoff] {}", bundle.handoff_note));
        }

        if !bundle.compacted_context.is_empty() {
            let rendered = bundle.compacted_context.render_to_text();
            if !rendered.is_empty() {
                session.add_agent_text(format!("[Compacted context]\n{}", rendered));
            }
        }

        for msg in bundle.recent_tail {
            session.messages.push(msg);
        }

        // Restore skill state so activated skills survive handoff
        session.skill_state = bundle.skill_state;

        // Note: MCP server and plugin enablement are NOT imported as part of handoff.
        // The destination runtime determines its own tool availability.

        session
    }
}

fn estimate_bundle_size(
    compacted: &CompactedContext,
    tail: &[StructuredMessage],
    instructions: Option<&str>,
    skill_state: &SessionSkillState,
) -> usize {
    let mut total = 0usize;
    if let Some(instr) = instructions {
        total += (instr.len() as f64 * 0.25).ceil() as usize;
    }
    total += (compacted.render_to_text().len() as f64 * 0.25).ceil() as usize;
    for msg in tail {
        total += (msg.text_content().len() as f64 * 0.25).ceil() as usize;
    }
    // Include activated skill instructions in size estimate
    let skill_instructions = skill_state.active_skill_instructions();
    if !skill_instructions.is_empty() {
        total += (skill_instructions.len() as f64 * 0.25).ceil() as usize;
    }
    total
}

fn is_local_resource(uri: &str) -> bool {
    uri.starts_with("file://")
        || uri.starts_with("localhost")
        || uri.starts_with("127.0.0.1")
        || uri.starts_with("unix://")
        || uri.starts_with("/dev/")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that `hydrate_into_new` does **not** carry over plugin
    /// enablement state from the source session.  The destination runtime
    /// must determine its own tool availability.
    #[test]
    fn hydrate_into_new_produces_empty_plugin_enablement() {
        // Create a source session with plugin enablement state.
        let mut source = DurableSession::new(SessionId::new());
        source.set_plugin_enabled("plugin-a", true);
        source.set_plugin_enabled("plugin-b", false);
        assert_eq!(
            source.list_enabled_plugins(),
            vec!["plugin-a".to_string()],
            "sanity check: source should have plugin-a enabled"
        );

        // Export a handoff bundle (minimal — no messages needed).
        let config = ContextManagementConfig::default();
        let bundle = HandoffExporter::export(&source, "test-model", None, vec![], &config, None)
            .expect("export should succeed for idle session");

        // Hydrate into a new session.
        let hydrated = HandoffImporter::hydrate_into_new(bundle);

        // The hydrated session must have empty plugin enablement.
        assert!(
            hydrated.list_enabled_plugins().is_empty(),
            "plugin enablement must not survive handoff; got {:?}",
            hydrated.list_enabled_plugins()
        );
        assert_eq!(
            hydrated.is_plugin_enabled("plugin-a"),
            None,
            "plugin-a must have no explicit enablement after hydration"
        );
        assert_eq!(
            hydrated.is_plugin_enabled("plugin-b"),
            None,
            "plugin-b must have no explicit enablement after hydration"
        );
    }

    /// Verifies that the `hydrate` (in-place) path also excludes plugin
    /// enablement — the import path never touches plugin_enablement.
    #[test]
    fn hydrate_in_place_does_not_modify_plugin_enablement() {
        let mut source = DurableSession::new(SessionId::new());
        source.set_plugin_enabled("plugin-x", true);
        source.add_agent_text("hello");

        let config = ContextManagementConfig::default();
        let bundle = HandoffExporter::export(&source, "test-model", None, vec![], &config, None)
            .expect("export should succeed");

        // Create a target with its own plugin enablement.
        let mut target = DurableSession::new(SessionId::new());
        target.set_plugin_enabled("plugin-y", true);

        HandoffImporter::hydrate(&mut target, bundle).expect("hydrate should succeed");

        // Target must retain its own enablement; nothing from source imported.
        assert_eq!(
            target.is_plugin_enabled("plugin-x"),
            None,
            "source plugin enablement must not leak into target"
        );
        assert_eq!(
            target.is_plugin_enabled("plugin-y"),
            Some(true),
            "target's own plugin enablement must be preserved"
        );
    }

    #[test]
    fn handoff_preserves_activated_skills() {
        let mut source = DurableSession::new(SessionId::new());
        source.activate_skill(
            "test-skill",
            "# Test Skill\nDo something useful.",
            vec![],
        );
        source.add_agent_text("hello");

        assert!(source.skill_state.is_active("test-skill"));

        let config = ContextManagementConfig::default();
        let bundle = HandoffExporter::export(&source, "test-model", None, vec![], &config, None)
            .expect("export should succeed");

        // Verify skill state is in the bundle
        assert!(bundle.skill_state.is_active("test-skill"));
        assert_eq!(bundle.skill_state.active.len(), 1);
        assert_eq!(bundle.skill_state.active[0].name, "test-skill");
        assert_eq!(bundle.skill_state.active[0].body, "# Test Skill\nDo something useful.");

        // Hydrate into a new session
        let hydrated = HandoffImporter::hydrate_into_new(bundle);

        // Skill state should survive handoff
        assert!(hydrated.skill_state.is_active("test-skill"));
        assert_eq!(hydrated.skill_state.active.len(), 1);
        assert_eq!(
            hydrated.active_skill_instructions(),
            "<skill_content name=\"test-skill\">\n# Test Skill\nDo something useful.\n</skill_content>"
        );
    }

    #[test]
    fn handoff_includes_skill_state_in_size_estimate() {
        let mut source = DurableSession::new(SessionId::new());
        source.activate_skill("big-skill", &"x".repeat(1000), vec![]);

        let config = ContextManagementConfig::default();
        let bundle = HandoffExporter::export(&source, "test-model", None, vec![], &config, None)
            .expect("export should succeed");

        // Size estimate should include skill instructions (~250 tokens for 1000 chars)
        assert!(bundle.metadata.size_estimate_tokens >= 250,
            "Size estimate should include skill instructions, got {} tokens", bundle.metadata.size_estimate_tokens);
    }
}
