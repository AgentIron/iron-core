//! Plugin lifecycle manager — install, load, unload, and uninstall.
//!
//! The lifecycle manager owns the pipeline that takes a `PluginConfig` through
//! the following states:
//!
//! ```text
//! Configured → Loading → (cache artifact) → (extract manifest) → (validate) → Healthy
//!                                                        ↘ Error
//! ```
//!
//! Design principles
//! -----------------
//! * **Idempotent installs** — re-installing the same plugin ID replaces the
//!   cached artifact and manifest, resetting the plugin to `Healthy`.
//! * **Atomic state transitions** — the registry is updated in explicit steps
//!   (`Configured` → `Loading` → `Healthy` / `Error`), and stale state is
//!   cleared when a reinstall fails so the plugin is never left in an
//!   inconsistent half-loaded state.
//! * **Checksum enforcement** — remote sources *require* a checksum.  Missing
//!   checksum, checksum mismatch, HTTP failure, and filesystem failure are
//!   each represented as distinct `InstallResult` variants so callers can
//!   surface precise diagnostics.

use crate::plugin::config::{Checksum, PluginConfig, PluginSource};
use crate::plugin::manifest::PluginManifest;
use crate::plugin::registry::{InstallMetadata, PluginRegistry};
use crate::plugin::status::PluginHealth;
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};

/// Result of a plugin installation attempt.
///
/// Each variant maps to a distinct failure mode so that callers (CLI, façade,
/// tests) can render precise diagnostics without string matching.
#[derive(Debug, Clone, PartialEq)]
pub enum InstallResult {
    /// Plugin was installed and loaded successfully.
    Success,
    /// Remote download completed but the computed checksum did not match.
    InvalidChecksum { expected: String, computed: String },
    /// A remote source was provided without a required checksum.
    MissingChecksum,
    /// Network or filesystem failure while fetching the artifact.
    DownloadFailed(String),
    /// Artifact was fetched but the manifest could not be extracted or is
    /// structurally invalid.
    InvalidManifest(String),
    /// The artifact could not be written to the runtime cache directory.
    CacheWriteFailed(String),
    /// WASM host rejected the plugin during the load step.
    LoadFailed(String),
    /// Manifest identity does not match the runtime plugin configuration.
    IdentityMismatch {
        config_id: String,
        manifest_id: String,
    },
}

impl std::fmt::Display for InstallResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => write!(f, "Plugin installed successfully"),
            Self::InvalidChecksum { expected, computed } => {
                write!(
                    f,
                    "Checksum mismatch: expected {}, computed {}",
                    expected, computed
                )
            }
            Self::MissingChecksum => write!(f, "Remote plugin requires a checksum"),
            Self::DownloadFailed(e) => write!(f, "Download failed: {}", e),
            Self::InvalidManifest(e) => write!(f, "Invalid manifest: {}", e),
            Self::CacheWriteFailed(e) => write!(f, "Cache write failed: {}", e),
            Self::LoadFailed(e) => write!(f, "Plugin load failed: {}", e),
            Self::IdentityMismatch {
                config_id,
                manifest_id,
            } => {
                write!(
                    f,
                    "Manifest identity mismatch: config expects '{}', manifest declares '{}'",
                    config_id, manifest_id
                )
            }
        }
    }
}

impl InstallResult {
    /// Returns `true` when the installation succeeded.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }
}

/// Callback used by `PluginLifecycle::install_with_loader` to hand a cached
/// artifact to the WASM host after manifest validation succeeds.
///
/// The loader is invoked *after* the registry has been updated with the
/// manifest and artifact path but *before* health is promoted to `Healthy`.
/// If the loader returns an error, the install is rolled back to `Error`.
pub trait PluginLoader: Send + Sync {
    /// Load a plugin into the WASM runtime given its current registry state.
    ///
    /// Implementations should read the artifact from `artifact_path` and
    /// instantiate a WASM plugin.  On success, return `Ok(())`.  On failure,
    /// return a human-readable error string.
    fn load(&self, plugin_id: &str, artifact_path: &Path) -> Result<(), String>;

    /// Unload a plugin from the WASM runtime.
    ///
    /// Called during uninstall.  A best-effort operation — failures are logged
    /// but not treated as fatal.
    fn unload(&self, plugin_id: &str);
}

/// A no-op loader used when no WASM host is available yet (e.g. tests that
/// only validate the install/uninstall state machine).
pub struct NullPluginLoader;

impl PluginLoader for NullPluginLoader {
    fn load(&self, _plugin_id: &str, _artifact_path: &Path) -> Result<(), String> {
        Ok(())
    }
    fn unload(&self, _plugin_id: &str) {}
}

// ---- Lifecycle manager ---------------------------------------------------

/// Manages the install → load → uninstall pipeline for plugins.
///
/// `PluginLifecycle` is parameterised over a `PluginLoader` so that the
/// lifecycle state machine can be tested independently of Extism.
#[derive(Debug)]
pub struct PluginLifecycle {
    registry: PluginRegistry,
    artifact_cache_dir: PathBuf,
}

impl PluginLifecycle {
    /// Create a new lifecycle manager.
    ///
    /// `artifact_cache_dir` is the runtime-owned directory where cached
    /// artifacts will be stored.  It is created on first use if it does not
    /// exist.
    pub fn new(registry: PluginRegistry, artifact_cache_dir: PathBuf) -> Self {
        Self {
            registry,
            artifact_cache_dir,
        }
    }

    // ------------------------------------------------------------------
    // Install
    // ------------------------------------------------------------------

    /// Install (or re-install) a plugin from the given configuration **without**
    /// loading it into a WASM host.
    ///
    /// This is the full install pipeline minus the host-load step, suitable for
    /// testing or for situations where the WASM runtime is not yet available.
    pub async fn install(&self, config: PluginConfig) -> InstallResult {
        self.install_with_loader(config, &NullPluginLoader).await
    }

    /// Install (or re-install) a plugin, then hand the cached artifact to
    /// `loader` for WASM-runtime instantiation.
    ///
    /// State transitions:
    /// ```text
    /// (not in registry) → Configured → Loading → Healthy
    ///                                           ↘ Error  (on any failure)
    /// ```
    ///
    /// Re-installs are idempotent: the old artifact is replaced and the
    /// manifest/auth state is cleared before the new artifact is processed.
    pub async fn install_with_loader(
        &self,
        config: PluginConfig,
        loader: &dyn PluginLoader,
    ) -> InstallResult {
        let plugin_id = config.id.clone();
        info!(plugin_id = %plugin_id, "Starting plugin install");

        // 1. Register (or replace) the plugin entry in the registry.
        //    Re-installs reset manifest/auth/artifact to a clean state.
        self.registry.register(config.clone());
        self.registry
            .update_health(&plugin_id, PluginHealth::Loading);

        // 2. Ensure the cache directory exists.
        if let Err(e) = self.ensure_cache_dir().await {
            error!(plugin_id = %plugin_id, error = %e, "Failed to create cache directory");
            self.registry
                .set_error(&plugin_id, format!("Cache directory error: {}", e));
            return InstallResult::CacheWriteFailed(e.to_string());
        }

        // 3. Fetch and verify the plugin artifact.
        let artifact_bytes = match &config.source {
            PluginSource::LocalPath { path } => {
                debug!(plugin_id = %plugin_id, path = %path.display(), "Loading local plugin artifact");
                match self.load_local_artifact(path).await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        error!(plugin_id = %plugin_id, error = %e, "Failed to read local artifact");
                        self.rollback_failed_install(&plugin_id);
                        return e;
                    }
                }
            }
            PluginSource::Remote { url, checksum } => {
                debug!(plugin_id = %plugin_id, url = %url, "Downloading remote plugin artifact");
                match self.download_and_verify(url, checksum).await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        error!(plugin_id = %plugin_id, error = %e, "Remote artifact download/verify failed");
                        self.rollback_failed_install(&plugin_id);
                        return e;
                    }
                }
            }
        };

        // 4. Write the artifact to the runtime-owned cache.
        let artifact_path = self.cached_artifact_path(&plugin_id);
        debug!(plugin_id = %plugin_id, path = %artifact_path.display(), "Writing artifact to cache");
        if let Err(e) = tokio::fs::write(&artifact_path, &artifact_bytes).await {
            error!(plugin_id = %plugin_id, error = %e, "Failed to write artifact to cache");
            self.registry
                .set_error(&plugin_id, format!("Cache write failed: {}", e));
            self.rollback_failed_install(&plugin_id);
            return InstallResult::CacheWriteFailed(e.to_string());
        }

        // 5. Extract the manifest from the WASM binary.
        let manifest = match self.extract_manifest(&artifact_bytes).await {
            Ok(m) => m,
            Err(e) => {
                error!(plugin_id = %plugin_id, error = %e, "Manifest extraction failed");
                self.registry
                    .set_error(&plugin_id, format!("Manifest extraction failed: {}", e));
                self.rollback_failed_install(&plugin_id);
                return InstallResult::InvalidManifest(e);
            }
        };

        // 6. Validate the manifest structurally.
        if let Err(e) = manifest.validate() {
            error!(plugin_id = %plugin_id, error = %e, "Manifest validation failed");
            self.registry
                .set_error(&plugin_id, format!("Invalid manifest: {}", e));
            self.rollback_failed_install(&plugin_id);
            return InstallResult::InvalidManifest(e.to_string());
        }

        // 6b. Validate manifest identity against config.
        if manifest.identity.id != config.id {
            error!(
                plugin_id = %plugin_id,
                config_id = %config.id,
                manifest_id = %manifest.identity.id,
                "Manifest identity does not match config"
            );
            self.rollback_failed_install(&plugin_id);
            return InstallResult::IdentityMismatch {
                config_id: config.id,
                manifest_id: manifest.identity.id,
            };
        }

        // 7. Update registry with manifest and artifact path.
        self.registry.set_manifest(&plugin_id, manifest);
        self.registry
            .set_artifact_path(&plugin_id, artifact_path.clone());

        // 7b. Record trusted install metadata.
        let source_description = match &config.source {
            PluginSource::LocalPath { path } => format!("local:{}", path.display()),
            PluginSource::Remote { url, .. } => format!("remote:{}", url),
        };
        let checksum_verified = matches!(&config.source, PluginSource::Remote { .. });
        let install_md = InstallMetadata {
            installed_at: chrono::Utc::now(),
            source_description,
            checksum_verified,
        };
        self.registry.set_install_metadata(&plugin_id, install_md);

        // 8. Hand off to the WASM loader.
        if let Err(e) = loader.load(&plugin_id, &artifact_path) {
            error!(plugin_id = %plugin_id, error = %e, "WASM host rejected plugin load");
            self.registry
                .set_error(&plugin_id, format!("Load failed: {}", e));
            self.rollback_failed_install(&plugin_id);
            return InstallResult::LoadFailed(e);
        }

        // 9. Promote to Healthy.
        self.registry
            .update_health(&plugin_id, PluginHealth::Healthy);
        info!(plugin_id = %plugin_id, "Plugin installed and loaded successfully");
        InstallResult::Success
    }

    // ------------------------------------------------------------------
    // Uninstall
    // ------------------------------------------------------------------

    /// Uninstall a plugin: unload from the WASM host, remove the cached
    /// artifact, and unregister from the registry.
    ///
    /// If the WASM host unload fails, the error is logged but the plugin is
    /// still removed from the registry — a stale WASM instance is less
    /// dangerous than a ghost registry entry.
    pub async fn uninstall(&self, plugin_id: &str) -> Result<(), String> {
        self.uninstall_with_loader(plugin_id, &NullPluginLoader)
            .await
    }

    /// Uninstall a plugin, delegating the WASM-host unload to `loader`.
    pub async fn uninstall_with_loader(
        &self,
        plugin_id: &str,
        loader: &dyn PluginLoader,
    ) -> Result<(), String> {
        info!(plugin_id = %plugin_id, "Uninstalling plugin");

        // Best-effort WASM unload.
        loader.unload(plugin_id);

        // Capture the artifact path before removing from the registry.
        let artifact_path = self.registry.get(plugin_id).and_then(|s| s.artifact_path);

        // Remove from registry (clears manifest, auth, health, etc.).
        self.registry.unregister(plugin_id);

        // Remove cached artifact.
        if let Some(path) = artifact_path {
            if let Err(e) = tokio::fs::remove_file(&path).await {
                warn!(
                    plugin_id = %plugin_id,
                    path = %path.display(),
                    error = %e,
                    "Failed to remove cached artifact (orphaned file)"
                );
            }
        }

        info!(plugin_id = %plugin_id, "Plugin uninstalled");
        Ok(())
    }

    // ------------------------------------------------------------------
    // Artifact helpers
    // ------------------------------------------------------------------

    /// Compute the canonical cache path for a plugin ID.
    ///
    /// The path is deterministic: `<cache_dir>/<plugin_id>.wasm`.
    fn cached_artifact_path(&self, plugin_id: &str) -> PathBuf {
        // Sanitise the plugin ID to produce a safe filename component.
        // Replace path-separator characters and collapse dots/underscores
        // that might confuse the filesystem.
        let safe_id = plugin_id.replace(['/', '\\'], "_").replace("..", "_");
        self.artifact_cache_dir.join(format!("{}.wasm", safe_id))
    }

    /// Ensure the artifact cache directory exists, creating it (and parents)
    /// if necessary.
    async fn ensure_cache_dir(&self) -> Result<(), std::io::Error> {
        tokio::fs::create_dir_all(&self.artifact_cache_dir).await
    }

    // ------------------------------------------------------------------
    // Local artifact loading
    // ------------------------------------------------------------------

    /// Read a local plugin artifact from disk.
    async fn load_local_artifact(&self, path: &Path) -> Result<Vec<u8>, InstallResult> {
        match tokio::fs::read(path).await {
            Ok(bytes) => {
                debug!(path = %path.display(), size = bytes.len(), "Read local artifact");
                Ok(bytes)
            }
            Err(e) => Err(InstallResult::DownloadFailed(format!(
                "Failed to read local file {}: {}",
                path.display(),
                e
            ))),
        }
    }

    // ------------------------------------------------------------------
    // Remote download + checksum
    // ------------------------------------------------------------------

    /// Download a remote plugin and verify its checksum.
    async fn download_and_verify(
        &self,
        url: &str,
        checksum: &Checksum,
    ) -> Result<Vec<u8>, InstallResult> {
        let bytes = match self.fetch_remote(url).await {
            Ok(b) => b,
            Err(e) => return Err(InstallResult::DownloadFailed(e)),
        };

        // Verify checksum
        if let Err(e) = checksum.verify(&bytes) {
            warn!(url = %url, error = %e, "Checksum verification failed");
            return Err(InstallResult::InvalidChecksum {
                expected: checksum.value.clone(),
                computed: match e {
                    crate::plugin::config::ChecksumError::Mismatch { computed, .. } => computed,
                },
            });
        }

        debug!(url = %url, size = bytes.len(), "Remote artifact downloaded and checksum verified");
        Ok(bytes)
    }

    /// Fetch remote content over HTTPS.
    ///
    /// Only `https://` URLs are permitted.  HTTP errors, connection failures,
    /// and body-read errors are all surfaced as descriptive strings.
    async fn fetch_remote(&self, url: &str) -> Result<Vec<u8>, String> {
        if !url.starts_with("https://") {
            return Err(format!(
                "Only HTTPS URLs are supported for remote plugins: {}",
                url
            ));
        }

        let response = reqwest::get(url)
            .await
            .map_err(|e| format!("Failed to fetch {}: {}", url, e))?;

        if !response.status().is_success() {
            return Err(format!("HTTP {} when fetching {}", response.status(), url));
        }

        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("Failed to read response body from {}: {}", url, e))
    }

    // ------------------------------------------------------------------
    // Manifest extraction from WASM custom sections
    //
    // The v1 manifest contract embeds a JSON payload in a WASM custom
    // section named `iron_manifest`.  This implementation reads that
    // section, validates the JSON, and returns a `PluginManifest`.
    // ------------------------------------------------------------------

    /// Extract the plugin manifest from a WASM binary.
    ///
    /// Looks for a WASM custom section named `iron_manifest` containing a
    /// UTF-8 JSON payload that deserializes into a `PluginManifest`.
    /// Returns `Ok(None)` if the section is absent (callers decide how to
    /// handle that case).
    async fn extract_manifest(&self, artifact: &[u8]) -> Result<PluginManifest, String> {
        // Try WASM custom section extraction first.
        if let Some(manifest) = Self::extract_manifest_from_wasm(artifact)? {
            return Ok(manifest);
        }

        Err("No embedded manifest found in WASM binary \
             (custom section 'iron_manifest' absent or invalid)"
            .to_string())
    }

    /// Parse WASM custom sections looking for the plugin manifest.
    ///
    /// The WASM binary format stores custom sections as:
    /// ```text
    /// [0x00] (custom section id)
    /// LEB128  section byte length
    /// LEB128  name byte length
    /// UTF-8   section name
    /// ...     section payload
    /// ```
    fn extract_manifest_from_wasm(artifact: &[u8]) -> Result<Option<PluginManifest>, String> {
        const MAGIC: &[u8; 4] = b"\x00asm";
        const VERSION: &[u8; 4] = &[0x01, 0x00, 0x00, 0x00];
        const MANIFEST_SECTION_NAME: &[u8] = b"iron_manifest";

        if artifact.len() < 8 {
            return Err("WASM binary too short".to_string());
        }

        if &artifact[0..4] != MAGIC {
            return Err("Not a valid WASM binary (magic mismatch)".to_string());
        }
        if &artifact[4..8] != VERSION {
            return Err("Unsupported WASM version (expected 1.0)".to_string());
        }

        let mut offset = 8usize;
        while offset < artifact.len() {
            // Read section id (1 byte)
            let section_id = match artifact.get(offset) {
                Some(&id) => id,
                None => break,
            };
            offset += 1;

            // Read section size (LEB128 u32)
            let (section_size, leb_bytes) = match Self::read_leb128_u32(&artifact[offset..]) {
                Some(result) => result,
                None => break,
            };
            offset += leb_bytes;

            let section_end = offset + section_size as usize;
            if section_end > artifact.len() {
                return Err("WASM section extends past end of binary".to_string());
            }

            // Custom section (id 0) — check the name
            if section_id == 0 {
                let (name_len, name_leb_bytes) = match Self::read_leb128_u32(&artifact[offset..]) {
                    Some(result) => result,
                    None => break,
                };
                let name_start = offset + name_leb_bytes;
                let name_end = name_start + name_len as usize;

                if name_end <= section_end {
                    let name = &artifact[name_start..name_end];
                    if name == MANIFEST_SECTION_NAME {
                        // Found the manifest section — parse the JSON payload.
                        let payload = &artifact[name_end..section_end];
                        let json_str = std::str::from_utf8(payload)
                            .map_err(|e| format!("Manifest section is not valid UTF-8: {}", e))?;
                        let manifest: PluginManifest = serde_json::from_str(json_str)
                            .map_err(|e| format!("Failed to parse manifest JSON: {}", e))?;
                        return Ok(Some(manifest));
                    }
                }
            }

            offset = section_end;
        }

        // No manifest section found — that's not an error here; the caller
        // decides how to handle the absence.
        Ok(None)
    }

    /// Read a LEB128-encoded unsigned 32-bit integer.
    ///
    /// Returns `Some((value, bytes_consumed))` or `None` if the encoding is
    /// incomplete or overflowed.
    fn read_leb128_u32(data: &[u8]) -> Option<(u32, usize)> {
        let mut value: u32 = 0;
        let mut shift: u32 = 0;
        let mut consumed = 0;

        for &byte in data {
            consumed += 1;
            value |= ((byte & 0x7F) as u32) << shift;
            if byte & 0x80 == 0 {
                return Some((value, consumed));
            }
            shift += 7;
            if shift >= 35 {
                // Overflow — more than 5 bytes for a u32
                return None;
            }
        }

        None // Incomplete encoding
    }

    // ------------------------------------------------------------------
    // Reinstall rollback
    // ------------------------------------------------------------------

    /// Roll back registry state after a failed install.
    ///
    /// Clears manifest, artifact path, and credentials so the plugin is not
    /// left in a half-loaded state, then sets health to `Error`.
    fn rollback_failed_install(&self, plugin_id: &str) {
        self.registry.clear_runtime_state(plugin_id);
        self.registry
            .set_error(plugin_id, "Install failed — state rolled back".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::config::PluginSource;
    use crate::plugin::manifest::{
        ExportedTool, PluginIdentity, PluginManifest, PluginPublisher, PresentationMetadata,
    };
    use crate::plugin::network::NetworkPolicy;
    use std::path::PathBuf;

    // ---- Test helpers ----

    fn test_config(id: &str, source: PluginSource) -> PluginConfig {
        PluginConfig {
            id: id.to_string(),
            source,
            enabled_by_default: true,
        }
    }

    fn local_config(id: &str, path: &str) -> PluginConfig {
        test_config(
            id,
            PluginSource::LocalPath {
                path: PathBuf::from(path),
            },
        )
    }

    /// Build a minimal WASM binary with a custom section named
    /// `iron_manifest` containing `payload` (UTF-8).
    fn build_wasm_with_manifest_section(payload: &str) -> Vec<u8> {
        let mut wasm = Vec::new();
        // WASM header
        wasm.extend_from_slice(b"\x00asm");
        wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // version 1.0

        // Custom section (id = 0)
        let section_name = b"iron_manifest";
        let payload_bytes = payload.as_bytes();
        let name_len = encode_leb128(section_name.len() as u32);
        let content_len = name_len.len() + section_name.len() + payload_bytes.len();
        let section_len = encode_leb128(content_len as u32);

        wasm.push(0x00); // custom section id
        wasm.extend_from_slice(&section_len);
        wasm.extend_from_slice(&name_len);
        wasm.extend_from_slice(section_name);
        wasm.extend_from_slice(payload_bytes);

        wasm
    }

    fn encode_leb128(mut value: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        loop {
            let mut byte = (value & 0x7F) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            buf.push(byte);
            if value == 0 {
                break;
            }
        }
        buf
    }

    fn sample_manifest() -> PluginManifest {
        PluginManifest {
            identity: PluginIdentity {
                id: "com.example.test-plugin".to_string(),
                name: "Test Plugin".to_string(),
                version: "1.0.0".to_string(),
            },
            publisher: PluginPublisher {
                name: "Example".to_string(),
                url: None,
                contact: None,
            },
            presentation: PresentationMetadata {
                description: "A test plugin".to_string(),
                long_description: None,
                icon: None,
                category: None,
                keywords: vec![],
            },
            network_policy: NetworkPolicy::Wildcard,
            auth: None,
            tools: vec![ExportedTool {
                name: "greet".to_string(),
                description: "Say hello".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                requires_approval: false,
                auth_requirements: None,
            }],
            api_version: "1.0".to_string(),
        }
    }

    /// Build a minimal manifest with a configurable `identity.id`.
    ///
    /// Use this in integration tests that go through the full install
    /// pipeline so the manifest identity matches the `PluginConfig.id`.
    fn sample_manifest_for_id(id: &str) -> PluginManifest {
        let mut m = sample_manifest();
        m.identity.id = id.to_string();
        m
    }

    // ---- Unit tests: InstallResult ----

    #[test]
    fn install_result_display_success() {
        assert_eq!(
            InstallResult::Success.to_string(),
            "Plugin installed successfully"
        );
    }

    #[test]
    fn install_result_display_missing_checksum() {
        assert_eq!(
            InstallResult::MissingChecksum.to_string(),
            "Remote plugin requires a checksum"
        );
    }

    #[test]
    fn install_result_display_invalid_checksum() {
        let msg = InstallResult::InvalidChecksum {
            expected: "abc".to_string(),
            computed: "def".to_string(),
        }
        .to_string();
        assert!(msg.contains("Checksum mismatch"));
        assert!(msg.contains("abc"));
        assert!(msg.contains("def"));
    }

    #[test]
    fn install_result_display_download_failed() {
        let msg = InstallResult::DownloadFailed("network error".to_string()).to_string();
        assert!(msg.contains("Download failed"));
        assert!(msg.contains("network error"));
    }

    #[test]
    fn install_result_display_cache_write_failed() {
        let msg = InstallResult::CacheWriteFailed("disk full".to_string()).to_string();
        assert!(msg.contains("Cache write failed"));
        assert!(msg.contains("disk full"));
    }

    #[test]
    fn install_result_display_load_failed() {
        let msg = InstallResult::LoadFailed("bad wasm".to_string()).to_string();
        assert!(msg.contains("Plugin load failed"));
        assert!(msg.contains("bad wasm"));
    }

    #[test]
    fn install_result_is_success() {
        assert!(InstallResult::Success.is_success());
        assert!(!InstallResult::MissingChecksum.is_success());
        assert!(!InstallResult::DownloadFailed("x".to_string()).is_success());
    }

    // ---- Unit tests: LEB128 decoder ----

    #[test]
    fn leb128_single_byte() {
        let (value, consumed) = PluginLifecycle::read_leb128_u32(&[0x05]).unwrap();
        assert_eq!(value, 5);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn leb128_multi_byte() {
        // 128 = 0x80 → encoded as [0x80, 0x01]
        let (value, consumed) = PluginLifecycle::read_leb128_u32(&[0x80, 0x01]).unwrap();
        assert_eq!(value, 128);
        assert_eq!(consumed, 2);
    }

    #[test]
    fn leb128_incomplete_returns_none() {
        assert!(PluginLifecycle::read_leb128_u32(&[0x80]).is_none());
    }

    // ---- Unit tests: WASM manifest extraction ----

    #[test]
    fn extract_manifest_from_valid_wasm() {
        let manifest = sample_manifest();
        let wasm = build_wasm_with_manifest_section(&serde_json::to_string(&manifest).unwrap());

        let result = PluginLifecycle::extract_manifest_from_wasm(&wasm)
            .expect("extraction should not error")
            .expect("manifest should be found");

        assert_eq!(result.identity.id, "com.example.test-plugin");
        assert_eq!(result.tools.len(), 1);
        assert_eq!(result.tools[0].name, "greet");
    }

    #[test]
    fn extract_manifest_from_wasm_without_section() {
        // Minimal WASM with no custom sections
        let wasm = b"\x00asm\x01\x00\x00\x00".to_vec();
        let result = PluginLifecycle::extract_manifest_from_wasm(&wasm)
            .expect("extraction should not error");
        assert!(
            result.is_none(),
            "should return None when no manifest section"
        );
    }

    #[test]
    fn extract_manifest_rejects_bad_magic() {
        let bad = b"bad magic bytes here".to_vec();
        let result = PluginLifecycle::extract_manifest_from_wasm(&bad);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("magic mismatch"));
    }

    #[test]
    fn extract_manifest_rejects_bad_json() {
        let wasm = build_wasm_with_manifest_section("{not valid json}");
        let result = PluginLifecycle::extract_manifest_from_wasm(&wasm);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("parse manifest JSON"));
    }

    // ---- Unit tests: cached_artifact_path ----

    #[test]
    fn cached_artifact_path_basic() {
        let lifecycle = PluginLifecycle::new(PluginRegistry::new(), PathBuf::from("/tmp/cache"));
        let path = lifecycle.cached_artifact_path("my-plugin");
        assert_eq!(path, PathBuf::from("/tmp/cache/my-plugin.wasm"));
    }

    #[test]
    fn cached_artifact_path_sanitises_slashes() {
        let lifecycle = PluginLifecycle::new(PluginRegistry::new(), PathBuf::from("/tmp/cache"));
        let path = lifecycle.cached_artifact_path("com/example/plugin");
        assert_eq!(path, PathBuf::from("/tmp/cache/com_example_plugin.wasm"));
    }

    #[test]
    fn cached_artifact_path_sanitises_dotdot() {
        let lifecycle = PluginLifecycle::new(PluginRegistry::new(), PathBuf::from("/tmp/cache"));
        let path = lifecycle.cached_artifact_path("../../etc/passwd");
        // .. replaced by _, / replaced by _
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert!(
            !filename.contains(".."),
            "path traversal must be sanitised, got: {}",
            filename
        );
        assert!(
            filename.starts_with('_'),
            "sanitised filename should start with _: {}",
            filename
        );
        assert!(filename.ends_with(".wasm"));
    }

    // ---- Integration-style tests using temp dirs ----

    #[tokio::test]
    async fn install_local_plugin_success() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let artifact_path = dir.path().join("test.wasm");

        // Write a valid WASM with manifest
        let manifest = sample_manifest_for_id("test-plugin");
        let wasm_bytes =
            build_wasm_with_manifest_section(&serde_json::to_string(&manifest).unwrap());
        tokio::fs::write(&artifact_path, &wasm_bytes).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir.clone());
        let config = local_config("test-plugin", &artifact_path.to_string_lossy());

        let result = lifecycle.install(config).await;
        assert!(result.is_success(), "install should succeed: {}", result);

        // Verify registry state
        let state = registry.get("test-plugin").unwrap();
        assert_eq!(state.health, PluginHealth::Healthy);
        assert!(state.manifest.is_some());
        assert!(state.artifact_path.is_some());

        // Verify install metadata was recorded
        let install_md = state
            .install_metadata
            .as_ref()
            .expect("install_metadata should be set");
        assert!(install_md.source_description.starts_with("local:"));
        assert!(!install_md.checksum_verified);

        // Verify cached artifact exists
        let cached = cache_dir.join("test-plugin.wasm");
        assert!(cached.exists(), "cached artifact should exist");
    }

    #[tokio::test]
    async fn install_local_plugin_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir);
        let config = local_config("missing-plugin", "/nonexistent/path.wasm");

        let result = lifecycle.install(config).await;
        assert!(
            matches!(result, InstallResult::DownloadFailed(_)),
            "missing file should be DownloadFailed, got: {}",
            result
        );

        let state = registry.get("missing-plugin").unwrap();
        assert_eq!(state.health, PluginHealth::Error);
    }

    #[tokio::test]
    async fn install_local_plugin_bad_wasm_magic() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let artifact_path = dir.path().join("bad.wasm");

        tokio::fs::write(&artifact_path, b"not a wasm file")
            .await
            .unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir);
        let config = local_config("bad-plugin", &artifact_path.to_string_lossy());

        let result = lifecycle.install(config).await;
        assert!(
            matches!(result, InstallResult::InvalidManifest(_)),
            "bad WASM should be InvalidManifest, got: {}",
            result
        );

        let state = registry.get("bad-plugin").unwrap();
        assert_eq!(state.health, PluginHealth::Error);
    }

    #[tokio::test]
    async fn install_local_plugin_wasm_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let artifact_path = dir.path().join("empty.wasm");

        // Valid WASM header but no custom sections
        tokio::fs::write(&artifact_path, b"\x00asm\x01\x00\x00\x00")
            .await
            .unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir);
        let config = local_config("empty-plugin", &artifact_path.to_string_lossy());

        let result = lifecycle.install(config).await;
        assert!(
            matches!(result, InstallResult::InvalidManifest(_)),
            "WASM without manifest should be InvalidManifest, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn uninstall_removes_artifact_and_registry_entry() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let artifact_path = dir.path().join("test.wasm");

        let manifest = sample_manifest_for_id("remove-me");
        let wasm_bytes =
            build_wasm_with_manifest_section(&serde_json::to_string(&manifest).unwrap());
        tokio::fs::write(&artifact_path, &wasm_bytes).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir.clone());

        // Install first
        let config = local_config("remove-me", &artifact_path.to_string_lossy());
        let result = lifecycle.install(config).await;
        assert!(result.is_success());

        let cached_path = cache_dir.join("remove-me.wasm");
        assert!(cached_path.exists());

        // Now uninstall
        lifecycle.uninstall("remove-me").await.unwrap();

        assert!(
            registry.get("remove-me").is_none(),
            "should be unregistered"
        );
        assert!(!cached_path.exists(), "cached artifact should be removed");
    }

    #[tokio::test]
    async fn reinstall_replaces_artifact_and_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");

        let mut manifest_v1 = sample_manifest_for_id("reinstall-test");
        manifest_v1.identity.version = "1.0.0".to_string();
        let wasm_v1 =
            build_wasm_with_manifest_section(&serde_json::to_string(&manifest_v1).unwrap());

        let mut manifest_v2 = sample_manifest_for_id("reinstall-test");
        manifest_v2.identity.version = "2.0.0".to_string();
        manifest_v2.tools.push(ExportedTool {
            name: "farewell".to_string(),
            description: "Say goodbye".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            requires_approval: false,
            auth_requirements: None,
        });
        let wasm_v2 =
            build_wasm_with_manifest_section(&serde_json::to_string(&manifest_v2).unwrap());

        // Write v1 artifact
        let artifact_v1_path = dir.path().join("v1.wasm");
        tokio::fs::write(&artifact_v1_path, &wasm_v1).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir.clone());

        // Install v1
        let config = local_config("reinstall-test", &artifact_v1_path.to_string_lossy());
        let result = lifecycle.install(config).await;
        assert!(result.is_success());

        let state_v1 = registry.get("reinstall-test").unwrap();
        assert_eq!(
            state_v1.manifest.as_ref().unwrap().identity.version,
            "1.0.0"
        );
        assert_eq!(state_v1.manifest.as_ref().unwrap().tools.len(), 1);

        // Write v2 artifact
        let artifact_v2_path = dir.path().join("v2.wasm");
        tokio::fs::write(&artifact_v2_path, &wasm_v2).await.unwrap();

        // Reinstall with v2
        let config_v2 = local_config("reinstall-test", &artifact_v2_path.to_string_lossy());
        let result_v2 = lifecycle.install(config_v2).await;
        assert!(result_v2.is_success());

        let state_v2 = registry.get("reinstall-test").unwrap();
        assert_eq!(state_v2.health, PluginHealth::Healthy);
        assert_eq!(
            state_v2.manifest.as_ref().unwrap().identity.version,
            "2.0.0"
        );
        assert_eq!(state_v2.manifest.as_ref().unwrap().tools.len(), 2);
    }

    #[tokio::test]
    async fn install_with_loader_called_on_success() {
        struct RecordingLoader {
            loaded: std::sync::Mutex<Vec<String>>,
            unloaded: std::sync::Mutex<Vec<String>>,
        }
        impl PluginLoader for RecordingLoader {
            fn load(&self, plugin_id: &str, _artifact_path: &Path) -> Result<(), String> {
                self.loaded.lock().unwrap().push(plugin_id.to_string());
                Ok(())
            }
            fn unload(&self, plugin_id: &str) {
                self.unloaded.lock().unwrap().push(plugin_id.to_string());
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let artifact_path = dir.path().join("test.wasm");

        let manifest = sample_manifest_for_id("loader-test");
        let wasm_bytes =
            build_wasm_with_manifest_section(&serde_json::to_string(&manifest).unwrap());
        tokio::fs::write(&artifact_path, &wasm_bytes).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir);

        let loader = RecordingLoader {
            loaded: std::sync::Mutex::new(Vec::new()),
            unloaded: std::sync::Mutex::new(Vec::new()),
        };

        let config = local_config("loader-test", &artifact_path.to_string_lossy());
        let result = lifecycle.install_with_loader(config, &loader).await;
        assert!(result.is_success());

        assert_eq!(loader.loaded.lock().unwrap().len(), 1);
        assert_eq!(loader.loaded.lock().unwrap()[0], "loader-test");
        assert!(loader.unloaded.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn install_with_loader_failure_rolls_back() {
        struct FailingLoader;
        impl PluginLoader for FailingLoader {
            fn load(&self, _plugin_id: &str, _artifact_path: &Path) -> Result<(), String> {
                Err("WASM host rejected the plugin".to_string())
            }
            fn unload(&self, _plugin_id: &str) {}
        }

        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let artifact_path = dir.path().join("test.wasm");

        let manifest = sample_manifest_for_id("fail-load");
        let wasm_bytes =
            build_wasm_with_manifest_section(&serde_json::to_string(&manifest).unwrap());
        tokio::fs::write(&artifact_path, &wasm_bytes).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir);

        let config = local_config("fail-load", &artifact_path.to_string_lossy());
        let result = lifecycle.install_with_loader(config, &FailingLoader).await;
        assert!(
            matches!(result, InstallResult::LoadFailed(ref e) if e.contains("rejected")),
            "expected LoadFailed, got: {}",
            result
        );

        let state = registry.get("fail-load").unwrap();
        assert_eq!(state.health, PluginHealth::Error);
    }

    #[tokio::test]
    async fn uninstall_with_loader_unloads_from_host() {
        struct RecordingLoader {
            unloaded: std::sync::Mutex<Vec<String>>,
        }
        impl PluginLoader for RecordingLoader {
            fn load(&self, _plugin_id: &str, _artifact_path: &Path) -> Result<(), String> {
                Ok(())
            }
            fn unload(&self, plugin_id: &str) {
                self.unloaded.lock().unwrap().push(plugin_id.to_string());
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let artifact_path = dir.path().join("test.wasm");

        let manifest = sample_manifest_for_id("uninstall-test");
        let wasm_bytes =
            build_wasm_with_manifest_section(&serde_json::to_string(&manifest).unwrap());
        tokio::fs::write(&artifact_path, &wasm_bytes).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir);

        let loader = RecordingLoader {
            unloaded: std::sync::Mutex::new(Vec::new()),
        };

        // Install first
        let config = local_config("uninstall-test", &artifact_path.to_string_lossy());
        lifecycle.install_with_loader(config, &loader).await;

        // Uninstall
        lifecycle
            .uninstall_with_loader("uninstall-test", &loader)
            .await
            .unwrap();

        assert!(registry.get("uninstall-test").is_none());
        assert_eq!(loader.unloaded.lock().unwrap()[0], "uninstall-test");
    }

    #[test]
    fn install_result_display_identity_mismatch() {
        let msg = InstallResult::IdentityMismatch {
            config_id: "test-plugin".to_string(),
            manifest_id: "com.example.other".to_string(),
        }
        .to_string();
        assert!(msg.contains("Manifest identity mismatch"));
        assert!(msg.contains("test-plugin"));
        assert!(msg.contains("com.example.other"));
    }

    #[test]
    fn install_result_is_success_rejects_identity_mismatch() {
        assert!(!InstallResult::IdentityMismatch {
            config_id: "a".to_string(),
            manifest_id: "b".to_string(),
        }
        .is_success());
    }

    #[tokio::test]
    async fn install_rejects_manifest_identity_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let artifact_path = dir.path().join("test.wasm");

        // Manifest declares id = "com.example.test-plugin" (the default)
        let manifest = sample_manifest();
        let wasm_bytes =
            build_wasm_with_manifest_section(&serde_json::to_string(&manifest).unwrap());
        tokio::fs::write(&artifact_path, &wasm_bytes).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir);

        // Config expects id = "wrong-plugin" — mismatch!
        let config = local_config("wrong-plugin", &artifact_path.to_string_lossy());
        let result = lifecycle.install(config).await;

        assert!(
            matches!(
                result,
                InstallResult::IdentityMismatch {
                    ref config_id,
                    ref manifest_id,
                } if config_id == "wrong-plugin" && manifest_id == "com.example.test-plugin"
            ),
            "expected IdentityMismatch, got: {}",
            result
        );

        // Plugin should be in Error state
        let state = registry.get("wrong-plugin").unwrap();
        assert_eq!(state.health, PluginHealth::Error);
    }

    // ---- NullPluginLoader tests ----

    #[test]
    fn null_loader_always_succeeds() {
        let loader = NullPluginLoader;
        assert!(loader.load("test", Path::new("/dev/null")).is_ok());
        // unload is no-op — just verify it doesn't panic
        loader.unload("test");
    }

    // ---- Phase 9.1: Additional lifecycle edge case tests ----

    /// Verifies that the full local install pipeline leaves the registry with
    /// an artifact path, a manifest, and the correct install metadata.
    #[tokio::test]
    async fn install_local_plugin_sets_all_registry_fields() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let artifact_path = dir.path().join("full.wasm");

        let manifest = sample_manifest_for_id("full-test");
        let wasm = build_wasm_with_manifest_section(&serde_json::to_string(&manifest).unwrap());
        tokio::fs::write(&artifact_path, &wasm).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir.clone());
        let config = local_config("full-test", &artifact_path.to_string_lossy());

        let result = lifecycle.install(config).await;
        assert!(result.is_success(), "{}", result);

        let state = registry.get("full-test").unwrap();
        assert_eq!(state.health, PluginHealth::Healthy);
        assert!(state.manifest.is_some());
        assert!(state.artifact_path.is_some());
        assert!(state.install_metadata.is_some());
        assert!(state.last_error.is_none());

        // Artifact points to the cached copy, not the source.
        let cached = state.artifact_path.unwrap();
        assert_eq!(cached, cache_dir.join("full-test.wasm"));
        assert!(cached.exists());
    }

    /// Verify that rollback on a failed install clears the manifest and
    /// credentials from a previous successful install of the same plugin ID.
    #[tokio::test]
    async fn failed_reinstall_clears_prior_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");

        // Install v1 successfully.
        let manifest_v1 = sample_manifest_for_id("rollback-test");
        let wasm_v1 =
            build_wasm_with_manifest_section(&serde_json::to_string(&manifest_v1).unwrap());
        let artifact_v1 = dir.path().join("v1.wasm");
        tokio::fs::write(&artifact_v1, &wasm_v1).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir);

        lifecycle
            .install(local_config(
                "rollback-test",
                &artifact_v1.to_string_lossy(),
            ))
            .await;
        assert!(registry.get("rollback-test").unwrap().manifest.is_some());

        // Attempt v2 with invalid WASM — should fail and roll back.
        let artifact_v2 = dir.path().join("v2.wasm");
        tokio::fs::write(&artifact_v2, b"not wasm").await.unwrap();

        let result = lifecycle
            .install(local_config(
                "rollback-test",
                &artifact_v2.to_string_lossy(),
            ))
            .await;
        assert!(matches!(result, InstallResult::InvalidManifest(_)));

        let state = registry.get("rollback-test").unwrap();
        assert_eq!(state.health, PluginHealth::Error);
        assert!(
            state.manifest.is_none(),
            "manifest from prior install should be cleared after rollback"
        );
        assert!(
            state.install_metadata.is_none(),
            "install_metadata should be cleared after rollback"
        );
    }

    /// Verify that duplicate tool names in the manifest cause an
    /// `InstallResult::InvalidManifest`.
    #[tokio::test]
    async fn install_rejects_duplicate_tool_names() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let artifact_path = dir.path().join("dup.wasm");

        let mut manifest = sample_manifest_for_id("dup-tools");
        manifest.tools.push(ExportedTool {
            name: "greet".to_string(), // duplicate
            description: "Another greet".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            requires_approval: false,
            auth_requirements: None,
        });
        let wasm = build_wasm_with_manifest_section(&serde_json::to_string(&manifest).unwrap());
        tokio::fs::write(&artifact_path, &wasm).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir);
        let result = lifecycle
            .install(local_config("dup-tools", &artifact_path.to_string_lossy()))
            .await;

        assert!(
            matches!(result, InstallResult::InvalidManifest(ref e) if e.contains("Duplicate")),
            "expected InvalidManifest with 'Duplicate', got: {}",
            result
        );
        assert_eq!(
            registry.get("dup-tools").unwrap().health,
            PluginHealth::Error
        );
    }

    /// Verify that an unsupported API version in the manifest causes
    /// `InstallResult::InvalidManifest`.
    #[tokio::test]
    async fn install_rejects_unsupported_api_version() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let artifact_path = dir.path().join("badver.wasm");

        let mut manifest = sample_manifest_for_id("badver-plugin");
        manifest.api_version = "99.0".to_string();
        let wasm = build_wasm_with_manifest_section(&serde_json::to_string(&manifest).unwrap());
        tokio::fs::write(&artifact_path, &wasm).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir);
        let result = lifecycle
            .install(local_config(
                "badver-plugin",
                &artifact_path.to_string_lossy(),
            ))
            .await;

        assert!(
            matches!(result, InstallResult::InvalidManifest(ref e) if e.contains("Unsupported API version")),
            "expected InvalidManifest for unsupported version, got: {}",
            result
        );
    }

    /// Verify that a manifest with a missing `identity.id` field is rejected.
    #[tokio::test]
    async fn install_rejects_manifest_with_empty_identity_id() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let artifact_path = dir.path().join("noid.wasm");

        let mut manifest = sample_manifest_for_id("");
        manifest.identity.id = "".to_string();
        let wasm = build_wasm_with_manifest_section(&serde_json::to_string(&manifest).unwrap());
        tokio::fs::write(&artifact_path, &wasm).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir);
        let result = lifecycle
            .install(local_config("", &artifact_path.to_string_lossy()))
            .await;

        assert!(
            matches!(result, InstallResult::InvalidManifest(ref e) if e.contains("Missing required field")),
            "expected InvalidManifest for empty identity.id, got: {}",
            result
        );
    }

    /// Verify that uninstalling a non-existent plugin returns Ok.
    #[tokio::test]
    async fn uninstall_nonexistent_plugin_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir);

        let result = lifecycle.uninstall("no-such-plugin").await;
        assert!(
            result.is_ok(),
            "uninstalling a non-existent plugin should be Ok"
        );
    }

    /// Verify that a local install from a path containing a WASM version 1
    /// header but with corrupt data past the manifest section still succeeds
    /// if the manifest itself is valid (Extism is forgiving with trailing data).
    #[tokio::test]
    async fn install_succeeds_with_extra_wasm_data_after_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let artifact_path = dir.path().join("extra.wasm");

        let manifest = sample_manifest_for_id("extra-data");
        let mut wasm = build_wasm_with_manifest_section(&serde_json::to_string(&manifest).unwrap());
        // Append some garbage data — still valid WASM as far as the
        // lifecycle is concerned (manifest extraction only reads sections).
        wasm.extend_from_slice(b"\x00\x01\x02\x03");

        tokio::fs::write(&artifact_path, &wasm).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir);
        let result = lifecycle
            .install(local_config("extra-data", &artifact_path.to_string_lossy()))
            .await;

        assert!(
            result.is_success(),
            "install with extra data after manifest should succeed: {}",
            result
        );
    }

    /// Verify that a local install that has already been cached replaces the
    /// cached file on reinstall.
    #[tokio::test]
    async fn reinstall_overwrites_cached_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");

        // Write v1
        let manifest_v1 = sample_manifest_for_id("overwrite-test");
        let wasm_v1 =
            build_wasm_with_manifest_section(&serde_json::to_string(&manifest_v1).unwrap());
        let src_v1 = dir.path().join("v1.wasm");
        tokio::fs::write(&src_v1, &wasm_v1).await.unwrap();

        let registry = PluginRegistry::new();
        let lifecycle = PluginLifecycle::new(registry.clone(), cache_dir.clone());
        lifecycle
            .install(local_config("overwrite-test", &src_v1.to_string_lossy()))
            .await;

        let cached = cache_dir.join("overwrite-test.wasm");
        let size_v1 = std::fs::metadata(&cached).unwrap().len();

        // Write v2 (larger)
        let mut manifest_v2 = sample_manifest_for_id("overwrite-test");
        manifest_v2.tools.push(ExportedTool {
            name: "extra_tool".to_string(),
            description: "Extra".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            requires_approval: false,
            auth_requirements: None,
        });
        let wasm_v2 =
            build_wasm_with_manifest_section(&serde_json::to_string(&manifest_v2).unwrap());
        let src_v2 = dir.path().join("v2.wasm");
        tokio::fs::write(&src_v2, &wasm_v2).await.unwrap();

        lifecycle
            .install(local_config("overwrite-test", &src_v2.to_string_lossy()))
            .await;

        let size_v2 = std::fs::metadata(&cached).unwrap().len();
        assert!(
            size_v2 > size_v1,
            "reinstall should overwrite cached artifact (v2 size {} > v1 size {})",
            size_v2,
            size_v1
        );
    }
}
