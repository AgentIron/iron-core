//! Encryption primitives for provider credentials stored in [`super::ConfigStore`].
//!
//! Production ciphertext uses XChaCha20-Poly1305 with a fresh random nonce per
//! write. Callers supply associated data, which the store derives from provider
//! slug and credential mode so encrypted rows cannot be moved between identities.

use chacha20poly1305::{
    aead::{Aead, Generate, Key, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use std::sync::Arc;

/// Key material for credential encryption.
pub struct CredentialKey {
    /// Raw 256-bit key material; callers should avoid logging or serializing it.
    pub key: [u8; 32],
}

/// Encryption result containing ciphertext and metadata.
#[derive(Debug, Clone)]
pub struct EncryptedPayload {
    /// Authenticated ciphertext, including the Poly1305 authentication tag.
    pub ciphertext: Vec<u8>,
    /// Per-encryption nonce; XChaCha20-Poly1305 payloads require 24 bytes.
    pub nonce: Vec<u8>,
    /// Versioned algorithm identifier needed to select a compatible decoder.
    pub metadata: String,
}

/// Abstraction for credential encryption/decryption.
pub trait CredentialCipher: Send + Sync {
    /// Encrypt a serialized credential payload and authenticate `associated_data`.
    ///
    /// Returns [`ConfigError::Encryption`](super::error::ConfigError::Encryption)
    /// if authenticated encryption fails.
    fn encrypt(
        &self,
        plaintext: &[u8],
        associated_data: &[u8],
    ) -> Result<EncryptedPayload, super::error::ConfigError>;

    /// Decrypt and authenticate an encrypted credential payload.
    ///
    /// The associated data must exactly match the value used for encryption.
    /// Unsupported metadata, malformed nonces, authentication failures, and
    /// mismatched associated data return
    /// [`ConfigError::Decryption`](super::error::ConfigError::Decryption).
    fn decrypt(
        &self,
        encrypted: &EncryptedPayload,
        associated_data: &[u8],
    ) -> Result<Vec<u8>, super::error::ConfigError>;
}

/// XChaCha20-Poly1305 cipher implementation.
pub struct XChaCha20Poly1305Cipher {
    cipher: XChaCha20Poly1305,
}

impl XChaCha20Poly1305Cipher {
    /// Create a new cipher from 32-byte key material.
    pub fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: XChaCha20Poly1305::new_from_slice(key).expect("valid 32-byte key"),
        }
    }

    /// Generate a new random 32-byte key.
    pub fn generate_key() -> [u8; 32] {
        Key::<XChaCha20Poly1305>::generate().into()
    }
}

impl CredentialCipher for XChaCha20Poly1305Cipher {
    fn encrypt(
        &self,
        plaintext: &[u8],
        associated_data: &[u8],
    ) -> Result<EncryptedPayload, super::error::ConfigError> {
        let nonce = XNonce::generate();
        let payload = chacha20poly1305::aead::Payload {
            msg: plaintext,
            aad: associated_data,
        };
        let ciphertext = self
            .cipher
            .encrypt(&nonce, payload)
            .map_err(|e| super::error::ConfigError::Encryption(e.to_string()))?;

        Ok(EncryptedPayload {
            ciphertext,
            nonce: nonce.to_vec(),
            metadata: "xchacha20poly1305:v1".to_string(),
        })
    }

    fn decrypt(
        &self,
        encrypted: &EncryptedPayload,
        associated_data: &[u8],
    ) -> Result<Vec<u8>, super::error::ConfigError> {
        if encrypted.metadata != "xchacha20poly1305:v1" {
            return Err(super::error::ConfigError::Decryption(format!(
                "Unsupported encryption metadata: {}",
                encrypted.metadata
            )));
        }

        if encrypted.nonce.len() != 24 {
            return Err(super::error::ConfigError::Decryption(format!(
                "Invalid nonce length: expected 24, got {}",
                encrypted.nonce.len()
            )));
        }
        let nonce = XNonce::try_from(encrypted.nonce.as_slice()).map_err(|_| {
            super::error::ConfigError::Decryption("Invalid nonce bytes".to_string())
        })?;
        let payload = chacha20poly1305::aead::Payload {
            msg: encrypted.ciphertext.as_ref(),
            aad: associated_data,
        };
        let plaintext = self
            .cipher
            .decrypt(&nonce, payload)
            .map_err(|e| super::error::ConfigError::Decryption(e.to_string()))?;

        Ok(plaintext)
    }
}

/// A cipher that stores credentials in plaintext (for testing only).
pub struct PlaintextCipher;

impl CredentialCipher for PlaintextCipher {
    fn encrypt(
        &self,
        plaintext: &[u8],
        _associated_data: &[u8],
    ) -> Result<EncryptedPayload, super::error::ConfigError> {
        Ok(EncryptedPayload {
            ciphertext: plaintext.to_vec(),
            nonce: vec![],
            metadata: "plaintext:test".to_string(),
        })
    }

    fn decrypt(
        &self,
        encrypted: &EncryptedPayload,
        _associated_data: &[u8],
    ) -> Result<Vec<u8>, super::error::ConfigError> {
        Ok(encrypted.ciphertext.clone())
    }
}

/// Thread-safe, type-erased credential cipher used by the configuration store.
pub type DynCredentialCipher = Arc<dyn CredentialCipher>;
