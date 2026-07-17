//! System-prompt composition, repository instruction loading, and runtime-context rendering.
//!
//! Prompt material is assembled in the stable order defined by
//! [`PROMPT_SECTION_ORDER`]. Repository and client-provided text is treated as
//! prompt content; loading it does not grant additional runtime capabilities.

pub mod assembly;
pub mod baseline;
pub mod config;
pub mod repo_loader;
pub mod runtime_context;
pub mod system;

pub use assembly::PromptAssembler;
pub use baseline::BASELINE_PROMPT;
pub use config::{
    AdditionalInstructionFile, ClientPromptFragment, PromptCompositionConfig,
    RepoInstructionConfig, RepoInstructionFamily, RepoInstructionPayload, RepoInstructionSource,
};
pub use repo_loader::RepoInstructionLoader;
pub use runtime_context::RuntimeContextRenderer;
pub use system::{
    PromptSection, PromptSectionMetadata, PromptSectionOwner, PromptSectionTemperature,
    SystemPromptCache, SystemPromptFingerprint, SystemPromptInputs, SystemPromptRenderer,
    PROMPT_SECTION_ORDER,
};
