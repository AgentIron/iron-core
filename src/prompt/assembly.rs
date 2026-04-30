use crate::prompt::config::RepoInstructionPayload;

pub struct PromptAssembler;

impl PromptAssembler {
    pub fn assemble(
        baseline: &str,
        repo_payload: &RepoInstructionPayload,
        additional_inline: &[String],
        session_instructions: Option<&str>,
        skill_instructions: Option<&str>,
        runtime_context: &str,
    ) -> String {
        let mut parts: Vec<String> = Vec::new();

        if !baseline.is_empty() {
            parts.push(baseline.to_string());
        }

        let has_repo =
            !repo_payload.sources.is_empty() || !repo_payload.additional_files.is_empty();
        if has_repo {
            parts.push(Self::render_repo_instructions(repo_payload));
        }

        for block in additional_inline {
            if !block.is_empty() {
                parts.push(block.clone());
            }
        }

        if let Some(instr) = session_instructions {
            if !instr.is_empty() {
                parts.push(instr.to_string());
            }
        }

        if let Some(skills) = skill_instructions {
            if !skills.is_empty() {
                parts.push(skills.to_string());
            }
        }

        if !runtime_context.is_empty() {
            parts.push(runtime_context.to_string());
        }

        parts.join("\n\n")
    }

    pub fn render_repo_instructions(payload: &RepoInstructionPayload) -> String {
        if payload.sources.is_empty() && payload.additional_files.is_empty() {
            return String::new();
        }

        let mut section = String::from("<repository_instructions>");
        section.push('\n');

        let source_names: Vec<String> = payload
            .sources
            .iter()
            .map(|s| s.scope.join(&s.filename).display().to_string())
            .chain(
                payload
                    .additional_files
                    .iter()
                    .map(|f| f.path.display().to_string()),
            )
            .collect();

        if !source_names.is_empty() {
            section.push_str("<repository_instruction_sources>\n");
            for name in &source_names {
                section.push_str(&format!("- {}\n", name));
            }
            section.push_str("</repository_instruction_sources>\n");
        }

        for src in &payload.sources {
            section.push_str(&format!(
                "<file_content path=\"{}\">\n",
                src.scope.join(&src.filename).display()
            ));
            section.push_str(&src.content);
            section.push_str("\n</file_content>\n");
        }

        for f in &payload.additional_files {
            section.push_str(&format!("<file_content path=\"{}\">\n", f.path.display()));
            section.push_str(&f.content);
            section.push_str("\n</file_content>\n");
        }

        section.push_str("</repository_instructions>");
        section
    }
}
