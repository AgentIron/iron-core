use super::error::ConfigError;

/// Source for credential encryption keys.
pub trait KeySource: Send + Sync {
    /// Get the encryption key material.
    async fn get_key(&self) -> Result<[u8; 32], ConfigError>;
}

/// OS keyring-backed key source.
pub struct OsKeyringKeySource {
    service: String,
    account: String,
}

impl OsKeyringKeySource {
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }
}

impl KeySource for OsKeyringKeySource {
    async fn get_key(&self) -> Result<[u8; 32], ConfigError> {
        let entry = keyring::Entry::new(&self.service, &self.account)
            .map_err(|e| ConfigError::KeyUnavailable(format!("Failed to access keyring: {}", e)))?;

        match entry.get_password() {
            Ok(password) => {
                let decoded =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &password)
                        .map_err(|e| {
                        ConfigError::KeyUnavailable(format!("Invalid key encoding: {}", e))
                    })?;
                if decoded.len() != 32 {
                    return Err(ConfigError::KeyUnavailable(
                        "Key must be 32 bytes".to_string(),
                    ));
                }
                let mut key = [0u8; 32];
                key.copy_from_slice(&decoded);
                Ok(key)
            }
            Err(keyring::Error::NoEntry) => {
                // Generate a new key
                let key = super::crypto::XChaCha20Poly1305Cipher::generate_key();
                let encoded =
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key);
                entry.set_password(&encoded).map_err(|e| {
                    ConfigError::KeyUnavailable(format!("Failed to store key: {}", e))
                })?;
                Ok(key)
            }
            Err(e) => Err(ConfigError::KeyUnavailable(format!("Keyring error: {}", e))),
        }
    }
}

/// Environment variable key source.
pub struct EnvVarKeySource {
    var_name: String,
}

impl EnvVarKeySource {
    pub fn new(var_name: impl Into<String>) -> Self {
        Self {
            var_name: var_name.into(),
        }
    }
}

impl KeySource for EnvVarKeySource {
    async fn get_key(&self) -> Result<[u8; 32], ConfigError> {
        let value = std::env::var(&self.var_name).map_err(|_| {
            ConfigError::KeyUnavailable(format!("Environment variable {} not set", self.var_name))
        })?;

        let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &value)
            .map_err(|e| ConfigError::KeyUnavailable(format!("Invalid base64: {}", e)))?;

        if decoded.len() != 32 {
            return Err(ConfigError::KeyUnavailable(
                "Key must be 32 bytes".to_string(),
            ));
        }

        let mut key = [0u8; 32];
        key.copy_from_slice(&decoded);
        Ok(key)
    }
}

/// Static key source for testing.
pub struct StaticKeySource {
    key: [u8; 32],
}

impl StaticKeySource {
    pub fn new(key: [u8; 32]) -> Self {
        Self { key }
    }
}

impl KeySource for StaticKeySource {
    async fn get_key(&self) -> Result<[u8; 32], ConfigError> {
        Ok(self.key)
    }
}
