//! Filesystem loading for repository-level prompt instructions.

use std::path::{Path, PathBuf};

use crate::error::RuntimeError;
use crate::prompt::config::{
    AdditionalInstructionFile, RepoInstructionConfig, RepoInstructionFamily,
    RepoInstructionPayload, RepoInstructionSource,
};

/// Resolves configured repository instruction files into prompt payloads.
pub struct RepoInstructionLoader;

impl RepoInstructionLoader {
    /// Loads at most one preferred instruction file from each configured scope.
    ///
    /// Scopes are processed in configuration order. Missing files and unreadable
    /// preferred files are skipped, and a disabled configuration returns an
    /// empty payload.
    ///
    /// # Errors
    ///
    /// The current resolver performs best-effort reads and does not return an
    /// error, but the result remains fallible for API consistency.
    pub fn resolve(config: &RepoInstructionConfig) -> Result<RepoInstructionPayload, RuntimeError> {
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

    /// Appends explicitly configured instruction files to an existing payload.
    ///
    /// Files are read and appended in slice order. If a read fails, previously
    /// appended files remain in `payload` and later files are not attempted.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] when a file cannot be read as UTF-8 text.
    pub fn load_additional_files(
        payload: &mut RepoInstructionPayload,
        files: &[PathBuf],
    ) -> Result<(), RuntimeError> {
        for path in files {
            let content = std::fs::read_to_string(path).map_err(|e| {
                RuntimeError::invalid_config(format!(
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
