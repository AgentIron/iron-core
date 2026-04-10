use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for a WASM plugin installation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginConfig {
    /// Stable runtime identity for this plugin
    pub id: String,
    /// Source of the plugin artifact
    pub source: PluginSource,
    /// Whether this plugin should be enabled by default for new sessions
    pub enabled_by_default: bool,
}

/// Source location for a plugin artifact
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginSource {
    /// Load from a local file path
    LocalPath {
        /// Absolute or relative path to the WASM file
        path: PathBuf,
    },
    /// Load from an HTTPS URL with checksum verification
    Remote {
        /// HTTPS URL to fetch the plugin
        url: String,
        /// Required checksum for verification
        checksum: Checksum,
    },
}

/// Checksum for plugin artifact verification
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checksum {
    /// Checksum algorithm
    pub algorithm: ChecksumAlgorithm,
    /// Hex-encoded checksum value
    pub value: String,
}

/// Supported checksum algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumAlgorithm {
    /// SHA-256 hash
    Sha256,
    /// SHA-512 hash
    Sha512,
}

impl Checksum {
    /// Verify that the given bytes match this checksum
    pub fn verify(&self, data: &[u8]) -> Result<(), ChecksumError> {
        match self.algorithm {
            ChecksumAlgorithm::Sha256 => {
                use sha2::{Digest, Sha256};
                let hash = Sha256::digest(data);
                let computed = hex::encode(hash);
                if computed.eq_ignore_ascii_case(&self.value) {
                    Ok(())
                } else {
                    Err(ChecksumError::Mismatch {
                        expected: self.value.clone(),
                        computed,
                    })
                }
            }
            ChecksumAlgorithm::Sha512 => {
                use sha2::{Digest, Sha512};
                let hash = Sha512::digest(data);
                let computed = hex::encode(hash);
                if computed.eq_ignore_ascii_case(&self.value) {
                    Ok(())
                } else {
                    Err(ChecksumError::Mismatch {
                        expected: self.value.clone(),
                        computed,
                    })
                }
            }
        }
    }
}

/// Errors that can occur during checksum verification
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChecksumError {
    /// Checksum does not match the computed hash
    Mismatch { expected: String, computed: String },
}

impl std::fmt::Display for ChecksumError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mismatch { expected, computed } => {
                write!(
                    f,
                    "Checksum mismatch: expected {}, computed {}",
                    expected, computed
                )
            }
        }
    }
}

impl std::error::Error for ChecksumError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_checksum_verification() {
        let data = b"hello world";
        let checksum = Checksum {
            algorithm: ChecksumAlgorithm::Sha256,
            value: "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9".to_string(),
        };

        assert!(checksum.verify(data).is_ok());
    }

    #[test]
    fn test_sha256_checksum_mismatch() {
        let data = b"hello world";
        let checksum = Checksum {
            algorithm: ChecksumAlgorithm::Sha256,
            value: "invalid".to_string(),
        };

        let result = checksum.verify(data);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ChecksumError::Mismatch { .. }
        ));
    }

    #[test]
    fn test_sha512_checksum_verification() {
        let data = b"hello world";
        let checksum = Checksum {
            algorithm: ChecksumAlgorithm::Sha512,
            value: "309ecc489c12d6eb4cc40f50c902f2b4d0ed77ee511a7c7a9bcd3ca86d4cd86f989dd35bc5ff499670da34255b45b0cfd830e81f605dcf7dc5542e93ae9cd76f".to_string(),
        };

        assert!(checksum.verify(data).is_ok());
    }
}
