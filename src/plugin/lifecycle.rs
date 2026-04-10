use crate::plugin::config::{PluginConfig, PluginSource, Checksum};
use crate::plugin::manifest::{PluginManifest, ManifestValidationError};
use crate::plugin::registry::PluginRegistry;
use crate::plugin::status::PluginHealth;
use std::path::PathBuf;
use tracing::{info, warn, error};

/// Lifecycle manager for plugin installation and loading
#[derive(Debug, Clone)]
pub struct PluginLifecycle {
    registry: PluginRegistry,
    /// Base directory for downloaded plugin artifacts
    artifact_cache_dir: PathBuf,
}

/// Result of a plugin installation attempt
#[derive(Debug, Clone, PartialEq)]
pub enum InstallResult {
    Success,
    InvalidChecksum { expected: String, computed: String },
    MissingChecksum,
    DownloadFailed(String),
    InvalidManifest(String),
}

impl std::fmt::Display for InstallResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => write!(f, "Plugin installed successfully"),
            Self::InvalidChecksum { expected, computed } => {
                write!(f, "Checksum mismatch: expected {}, computed {}", expected, computed)
            }
            Self::MissingChecksum => write!(f, "Remote plugin requires a checksum"),
            Self::DownloadFailed(e) => write!(f, "Download failed: {}", e),
            Self::InvalidManifest(e) => write!(f, "Invalid manifest: {}", e),
        }
    }
}

impl PluginLifecycle {
    pub fn new(registry: PluginRegistry, artifact_cache_dir: PathBuf) -> Self {
        Self {
            registry,
            artifact_cache_dir,
        }
    }

    /// Install a plugin from the given configuration
    pub async fn install(&self, config: PluginConfig) -> InstallResult {
        info!("Installing plugin: {}", config.id);

        // Register the plugin first
        self.registry.register(config.clone());
        self.registry.update_health(&config.id, PluginHealth::Loading);

        // Fetch and verify the plugin artifact
        let artifact_result = match &config.source {
            PluginSource::LocalPath { path } => {
                info!("Loading local plugin from: {:?}", path);
                self.load_local_artifact(path).await
            }
            PluginSource::Remote { url, checksum } => {
                info!("Downloading remote plugin from: {}", url);
                self.download_and_verify(url, checksum).await
            }
        };

        let artifact_bytes = match artifact_result {
            Ok(bytes) => bytes,
            Err(e) => {
                error!("Failed to load plugin artifact: {}", e);
                self.registry.set_error(&config.id, e.to_string());
                return e;
            }
        };

        // Extract manifest from WASM (placeholder - actual WASM parsing would go here)
        let manifest = match self.extract_manifest(&artifact_bytes).await {
            Ok(m) => m,
            Err(e) => {
                error!("Failed to extract manifest: {}", e);
                self.registry.set_error(&config.id, format!("Manifest extraction failed: {}", e));
                return InstallResult::InvalidManifest(e);
            }
        };

        // Validate manifest
        if let Err(e) = manifest.validate() {
            error!("Manifest validation failed: {}", e);
            self.registry.set_error(&config.id, format!("Invalid manifest: {}", e));
            return InstallResult::InvalidManifest(e.to_string());
        }

        // Store artifact to cache
        let artifact_path = self.artifact_cache_dir.join(format!("{}.wasm", config.id));
        if let Err(e) = tokio::fs::write(&artifact_path, &artifact_bytes).await {
            error!("Failed to write artifact cache: {}", e);
            self.registry.set_error(&config.id, format!("Cache write failed: {}", e));
            return InstallResult::DownloadFailed(e.to_string());
        }

        // Update registry with loaded state
        self.registry.set_manifest(&config.id, manifest);
        self.registry.set_artifact_path(&config.id, artifact_path);
        self.registry.update_health(&config.id, PluginHealth::Healthy);

        info!("Plugin {} installed successfully", config.id);
        InstallResult::Success
    }

    /// Uninstall a plugin
    pub async fn uninstall(&self, plugin_id: &str) -> Result<(), String> {
        info!("Uninstalling plugin: {}", plugin_id);

        // Get artifact path before removing
        let artifact_path = self.registry.get(plugin_id).and_then(|s| s.artifact_path);

        // Remove from registry
        self.registry.unregister(plugin_id);

        // Remove cached artifact if exists
        if let Some(path) = artifact_path {
            if let Err(e) = tokio::fs::remove_file(&path).await {
                warn!("Failed to remove artifact cache: {}", e);
            }
        }

        info!("Plugin {} uninstalled", plugin_id);
        Ok(())
    }

    /// Load a local plugin artifact
    async fn load_local_artifact(&self, path: &PathBuf) -> Result<Vec<u8>, InstallResult> {
        match tokio::fs::read(path).await {
            Ok(bytes) => Ok(bytes),
            Err(e) => Err(InstallResult::DownloadFailed(format!(
                "Failed to read local file: {}",
                e
            ))),
        }
    }

    /// Download and verify a remote plugin
    async fn download_and_verify(
        &self,
        url: &str,
        checksum: &Checksum,
    ) -> Result<Vec<u8>, InstallResult> {
        // Fetch the plugin
        let bytes = match self.fetch_remote(url).await {
            Ok(b) => b,
            Err(e) => return Err(InstallResult::DownloadFailed(e)),
        };

        // Verify checksum
        if let Err(e) = checksum.verify(&bytes) {
            warn!("Checksum verification failed for {}: {}", url, e);
            return Err(InstallResult::InvalidChecksum {
                expected: checksum.value.clone(),
                computed: match e {
                    crate::plugin::config::ChecksumError::Mismatch { computed, .. } => computed,
                },
            });
        }

        info!("Checksum verified for {}", url);
        Ok(bytes)
    }

    /// Fetch remote content (placeholder - actual HTTP client would be used)
    async fn fetch_remote(&self, url: &str) -> Result<Vec<u8>, String> {
        // In production, this would use an HTTP client
        // For now, return an error indicating this is a placeholder
        Err(format!("Remote fetch not implemented for: {}", url))
    }

    /// Extract manifest from WASM binary (placeholder)
    async fn extract_manifest(&self, _artifact: &[u8]) -> Result<PluginManifest, String> {
        // In production, this would parse the WASM binary to extract
        // the embedded manifest (e.g., from a custom section)
        // For now, return a placeholder error
        Err("Manifest extraction not yet implemented".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::config::{Checksum, ChecksumAlgorithm};

    #[test]
    fn test_install_result_display() {
        assert_eq!(
            InstallResult::Success.to_string(),
            "Plugin installed successfully"
        );
        assert_eq!(
            InstallResult::MissingChecksum.to_string(),
            "Remote plugin requires a checksum"
        );
        assert!(
            InstallResult::InvalidChecksum {
                expected: "abc".to_string(),
                computed: "def".to_string(),
            }
            .to_string()
            .contains("Checksum mismatch")
        );
    }
}
