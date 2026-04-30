use crate::prompt::{ClientPromptFragment, RepoInstructionPayload};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptSection {
    Identity,
    StaticContext,
    CoreGuidelines,
    ToolPhilosophy,
    EditingGuidelines,
    Safety,
    ProviderSpecificGuidance,
    CommunicationFormatting,
    ClientInjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptSectionOwner {
    Core,
    Provider,
    Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptSectionTemperature {
    Cold,
    Warm,
    Hot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptSectionMetadata {
    pub section: PromptSection,
    pub title: &'static str,
    pub owner: PromptSectionOwner,
    pub temperature: PromptSectionTemperature,
}

pub const PROMPT_SECTION_ORDER: [PromptSection; 9] = [
    PromptSection::Identity,
    PromptSection::StaticContext,
    PromptSection::CoreGuidelines,
    PromptSection::ToolPhilosophy,
    PromptSection::EditingGuidelines,
    PromptSection::Safety,
    PromptSection::ProviderSpecificGuidance,
    PromptSection::CommunicationFormatting,
    PromptSection::ClientInjection,
];

impl PromptSection {
    pub fn metadata(self) -> PromptSectionMetadata {
        match self {
            PromptSection::Identity => PromptSectionMetadata {
                section: self,
                title: "Identity",
                owner: PromptSectionOwner::Core,
                temperature: PromptSectionTemperature::Cold,
            },
            PromptSection::StaticContext => PromptSectionMetadata {
                section: self,
                title: "Static Context",
                owner: PromptSectionOwner::Core,
                temperature: PromptSectionTemperature::Warm,
            },
            PromptSection::CoreGuidelines => PromptSectionMetadata {
                section: self,
                title: "Core Guidelines",
                owner: PromptSectionOwner::Core,
                temperature: PromptSectionTemperature::Cold,
            },
            PromptSection::ToolPhilosophy => PromptSectionMetadata {
                section: self,
                title: "Tool Philosophy",
                owner: PromptSectionOwner::Core,
                temperature: PromptSectionTemperature::Hot,
            },
            PromptSection::EditingGuidelines => PromptSectionMetadata {
                section: self,
                title: "Editing Guidelines",
                owner: PromptSectionOwner::Client,
                temperature: PromptSectionTemperature::Warm,
            },
            PromptSection::Safety => PromptSectionMetadata {
                section: self,
                title: "Safety & Destructive Actions",
                owner: PromptSectionOwner::Core,
                temperature: PromptSectionTemperature::Cold,
            },
            PromptSection::ProviderSpecificGuidance => PromptSectionMetadata {
                section: self,
                title: "Provider-Specific Guidance",
                owner: PromptSectionOwner::Provider,
                temperature: PromptSectionTemperature::Warm,
            },
            PromptSection::CommunicationFormatting => PromptSectionMetadata {
                section: self,
                title: "Communication & Formatting",
                owner: PromptSectionOwner::Core,
                temperature: PromptSectionTemperature::Cold,
            },
            PromptSection::ClientInjection => PromptSectionMetadata {
                section: self,
                title: "Client Injection",
                owner: PromptSectionOwner::Client,
                temperature: PromptSectionTemperature::Warm,
            },
        }
    }
}

pub struct SystemPromptInputs<'a> {
    pub baseline: &'a str,
    pub runtime_context: &'a str,
    pub repo_payload: &'a RepoInstructionPayload,
    pub additional_inline: &'a [String],
    pub session_instructions: Option<&'a str>,
    pub skill_instructions: Option<&'a str>,
    pub provider_guidance: Option<&'a str>,
    pub client_editing_guidance: Option<&'a str>,
    pub client_injections: &'a [ClientPromptFragment],
    pub python_exec_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemPromptFingerprint(String);

impl SystemPromptFingerprint {
    pub fn from_inputs(inputs: &SystemPromptInputs<'_>) -> Self {
        let mut value = String::new();
        push_part(&mut value, inputs.baseline);
        push_part(&mut value, inputs.runtime_context);
        push_part(&mut value, &format!("{:?}", inputs.repo_payload));
        push_part(&mut value, &format!("{:?}", inputs.additional_inline));
        push_part(&mut value, inputs.session_instructions.unwrap_or_default());
        push_part(&mut value, inputs.skill_instructions.unwrap_or_default());
        push_part(&mut value, inputs.provider_guidance.unwrap_or_default());
        push_part(
            &mut value,
            inputs.client_editing_guidance.unwrap_or_default(),
        );
        push_part(&mut value, &format!("{:?}", inputs.client_injections));
        push_part(
            &mut value,
            if inputs.python_exec_available {
                "1"
            } else {
                "0"
            },
        );
        Self(value)
    }
}

#[derive(Debug, Default, Clone)]
pub struct SystemPromptCache {
    rendered: Option<String>,
    fingerprint: Option<SystemPromptFingerprint>,
}

impl SystemPromptCache {
    pub fn render(&mut self, inputs: &SystemPromptInputs<'_>) -> &str {
        let fingerprint = SystemPromptFingerprint::from_inputs(inputs);
        if self.fingerprint.as_ref() != Some(&fingerprint) {
            self.rendered = Some(SystemPromptRenderer::render(inputs));
            self.fingerprint = Some(fingerprint);
        }

        self.rendered.as_deref().unwrap_or_default()
    }

    pub fn invalidate_working_directory(&mut self) {
        self.invalidate();
    }

    pub fn invalidate_provider_guidance(&mut self) {
        self.invalidate();
    }

    pub fn invalidate_tool_availability(&mut self) {
        self.invalidate();
    }

    pub fn invalidate(&mut self) {
        self.rendered = None;
        self.fingerprint = None;
    }
}

pub struct SystemPromptRenderer;

impl SystemPromptRenderer {
    pub fn render(inputs: &SystemPromptInputs<'_>) -> String {
        PROMPT_SECTION_ORDER
            .iter()
            .enumerate()
            .map(|(idx, section)| Self::render_section(idx + 1, *section, inputs))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn render_section(
        index: usize,
        section: PromptSection,
        inputs: &SystemPromptInputs<'_>,
    ) -> String {
        let meta = section.metadata();
        let mut rendered = format!("## {}. {}\n", index, meta.title);
        let body = match section {
            PromptSection::Identity => render_identity(),
            PromptSection::StaticContext => inputs.runtime_context.to_string(),
            PromptSection::CoreGuidelines => render_core_guidelines(inputs.baseline),
            PromptSection::ToolPhilosophy => render_tool_philosophy(inputs.python_exec_available),
            PromptSection::EditingGuidelines => inputs
                .client_editing_guidance
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(render_default_editing_guidelines),
            PromptSection::Safety => render_safety(),
            PromptSection::ProviderSpecificGuidance => inputs
                .provider_guidance
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("No provider-specific guidance supplied.")
                .to_string(),
            PromptSection::CommunicationFormatting => render_communication_formatting(),
            PromptSection::ClientInjection => render_client_injection(inputs),
        };
        rendered.push_str(body.trim());
        rendered
    }
}

fn render_identity() -> String {
    "You are an AI coding agent powered by iron-core. Follow the core-owned instructions in this prompt and preserve the authority boundaries between core, provider, and client sections.".to_string()
}

fn render_core_guidelines(baseline: &str) -> String {
    if baseline.trim().is_empty() {
        "Work efficiently, verify changes where practical, and keep reasoning grounded in the available repository context.".to_string()
    } else {
        baseline.to_string()
    }
}

fn render_tool_philosophy(python_exec_available: bool) -> String {
    let mut text = String::from(
        "Use tools deliberately. Prefer direct file/search tools for repository inspection and terminal commands for build, test, and package-manager operations.",
    );
    if python_exec_available {
        text.push_str("\n\n`python_exec` is available for deterministic computation and safe orchestration of independent tool calls; use the exposed tools namespace rather than direct host filesystem or network access.");
    } else {
        text.push_str("\n\n`python_exec` is not currently available in the visible tool catalog.");
    }
    text
}

fn render_default_editing_guidelines() -> String {
    "Make the smallest correct change, preserve existing style, and avoid modifying unrelated user work.".to_string()
}

fn render_safety() -> String {
    "Do not perform destructive or irreversible actions unless explicitly requested. Never read or modify protected resources, credentials, or secrets unless the user has specifically authorized that exact action.".to_string()
}

fn render_communication_formatting() -> String {
    "Communicate concisely. Report what changed, how it was verified, and any remaining risks or blockers.".to_string()
}

fn render_client_injection(inputs: &SystemPromptInputs<'_>) -> String {
    let mut parts = Vec::new();
    let repo = crate::prompt::PromptAssembler::render_repo_instructions(inputs.repo_payload);
    if !repo.is_empty() {
        parts.push(repo);
    }

    parts.extend(
        inputs
            .additional_inline
            .iter()
            .filter(|s| !s.is_empty())
            .cloned(),
    );

    if let Some(instructions) = inputs.session_instructions.filter(|s| !s.is_empty()) {
        parts.push(instructions.to_string());
    }

    if let Some(skills) = inputs.skill_instructions.filter(|s| !s.is_empty()) {
        parts.push(skills.to_string());
    }

    for fragment in inputs.client_injections {
        if fragment.markdown.trim().is_empty() {
            continue;
        }
        let mut rendered = String::new();
        if let Some(title) = fragment.title.as_ref().filter(|s| !s.trim().is_empty()) {
            rendered.push_str(&format!("### {}\n", title));
        }
        rendered.push_str(&fragment.markdown);
        parts.push(rendered);
    }

    if parts.is_empty() {
        "No client injection supplied.".to_string()
    } else {
        parts.join("\n\n")
    }
}

fn push_part(target: &mut String, value: &str) {
    target.push_str(&value.len().to_string());
    target.push(':');
    target.push_str(value);
    target.push('|');
}
