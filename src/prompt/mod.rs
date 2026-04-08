pub mod assembly;
pub mod baseline;
pub mod config;
pub mod repo_loader;
pub mod runtime_context;

pub use assembly::PromptAssembler;
pub use baseline::BASELINE_PROMPT;
pub use config::{
    AdditionalInstructionFile, PromptCompositionConfig, RepoInstructionConfig,
    RepoInstructionFamily, RepoInstructionPayload, RepoInstructionSource,
};
pub use repo_loader::RepoInstructionLoader;
pub use runtime_context::RuntimeContextRenderer;
