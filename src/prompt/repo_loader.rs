use std::path::{Path, PathBuf};

use crate::error::LoopError;
use crate::prompt::config::{
    AdditionalInstructionFile, RepoInstructionConfig, RepoInstructionFamily,
    RepoInstructionPayload, RepoInstructionSource,
};

pub struct RepoInstructionLoader;

impl RepoInstructionLoader {
    pub fn resolve(config: &RepoInstructionConfig) -> Result<RepoInstructionPayload, LoopError> {
        if !config.enabled {
            return Ok(RepoInstructionPayload::default());
        }

        let mut sources = Vec::new();
        for scope in &config.scopes {
            if let Some(src) = resolve_scope(scope, config.family) {
                sources.push(src);
            }
        }

        Ok(RepoInstructionPayload {
            sources,
            additional_files: Vec::new(),
        })
    }

    pub fn load_additional_files(
        payload: &mut RepoInstructionPayload,
        files: &[PathBuf],
    ) -> Result<(), LoopError> {
        for path in files {
            let content = std::fs::read_to_string(path).map_err(|e| {
                LoopError::invalid_config(format!(
                    "failed to read additional instruction file {}: {}",
                    path.display(),
                    e
                ))
            })?;
            payload.additional_files.push(AdditionalInstructionFile {
                path: path.clone(),
                content,
            });
        }
        Ok(())
    }
}

fn resolve_scope(scope: &Path, family: RepoInstructionFamily) -> Option<RepoInstructionSource> {
    let candidates = family.candidates();
    for &filename in candidates {
        let file_path = scope.join(filename);
        if file_path.is_file() {
            let content = std::fs::read_to_string(&file_path).ok()?;
            return Some(RepoInstructionSource {
                scope: scope.to_path_buf(),
                filename: filename.to_string(),
                content,
            });
        }
    }
    None
}
