use super::{
    crypto::{DynCredentialCipher, XChaCha20Poly1305Cipher},
    db,
    error::ConfigError,
    key_source::{EnvVarKeySource, KeySource, OsKeyringKeySource, StaticKeySource},
    migrations,
    records::*,
};
use chrono::Utc;
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Durable configuration store for AgentIron.
pub struct ConfigStore {
    pool: SqlitePool,
    cipher: Option<DynCredentialCipher>,
}

/// Options for opening a ConfigStore.
pub struct OpenOptions {
    /// Optional cipher to use instead of resolving one from key sources.
    pub cipher: Option<DynCredentialCipher>,
    /// Optional busy timeout for SQLite write-lock contention.
    ///
    /// When omitted the default is 5 seconds.
    pub busy_timeout: Option<Duration>,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            cipher: None,
            busy_timeout: None,
        }
    }
}

impl ConfigStore {
    /// Open the platform-default config store.
    pub async fn open() -> Result<Self, ConfigError> {
        let path = default_config_path()?;
        Self::open_at(path).await
    }

    /// Open a config store at an explicit path.
    pub async fn open_at(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        Self::open_at_with_options(path, OpenOptions::default()).await
    }

    /// Open a config store with explicit options.
    pub async fn open_at_with_options(
        path: impl AsRef<Path>,
        options: OpenOptions,
    ) -> Result<Self, ConfigError> {
        let path = path.as_ref();

        // Create parent directories
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let pool = if let Some(timeout) = options.busy_timeout {
            db::create_pool_with_timeout(path, timeout).await?
        } else {
            db::create_pool(path).await?
        };
        migrations::apply_migrations(&pool).await?;

        let cipher = if let Some(cipher) = options.cipher {
            Some(cipher)
        } else {
            // Try env var first (for headless/testing), then OS keyring
            let key_source = if let Ok(key) =
                EnvVarKeySource::new("AGENTIRON_CONFIG_ENCRYPTION_KEY")
                    .get_key()
                    .await
            {
                Some(StaticKeySource::new(key))
            } else if let Ok(key) = OsKeyringKeySource::new("agentiron", "config-encryption")
                .get_key()
                .await
            {
                Some(StaticKeySource::new(key))
            } else {
                None
            };

            match key_source {
                Some(ks) => {
                    let key = ks.get_key().await?;
                    Some(Arc::new(XChaCha20Poly1305Cipher::new(&key)) as DynCredentialCipher)
                }
                None => None,
            }
        };

        Ok(Self { pool, cipher })
    }

    /// Create an in-memory config store for testing.
    pub async fn open_in_memory() -> Result<Self, ConfigError> {
        let pool = db::create_memory_pool().await?;
        migrations::apply_migrations(&pool).await?;

        // Use a test cipher
        let key = XChaCha20Poly1305Cipher::generate_key();
        let cipher = Arc::new(XChaCha20Poly1305Cipher::new(&key)) as DynCredentialCipher;

        Ok(Self {
            pool,
            cipher: Some(cipher),
        })
    }

    /// Create an in-memory store with a specific cipher for testing.
    pub async fn open_in_memory_with_cipher(
        cipher: DynCredentialCipher,
    ) -> Result<Self, ConfigError> {
        let pool = db::create_memory_pool().await?;
        migrations::apply_migrations(&pool).await?;

        Ok(Self {
            pool,
            cipher: Some(cipher),
        })
    }

    // Profile APIs

    /// Store or replace a profile record.
    ///
    /// Returns `ConfigError::Validation` if the ID is empty.
    pub async fn set_profile(&self, input: &ProfileInput) -> Result<(), ConfigError> {
        if input.id.is_empty() {
            return Err(ConfigError::Validation(
                "Profile ID must not be empty".to_string(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        let payload = serde_json::to_string(&input.payload)?;

        sqlx::query(
            r#"
            INSERT INTO profiles (id, schema_version, payload, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                schema_version = excluded.schema_version,
                payload = excluded.payload,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&input.id)
        .bind(input.schema_version)
        .bind(&payload)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        Ok(())
    }

    /// Get a profile by ID.
    pub async fn get_profile(&self, id: &str) -> Result<Option<ProfileRecord>, ConfigError> {
        let row = sqlx::query(
            "SELECT id, schema_version, payload, created_at, updated_at FROM profiles WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        match row {
            Some(row) => {
                let payload: String = row.get("payload");
                Ok(Some(ProfileRecord {
                    id: row.get("id"),
                    schema_version: row.get("schema_version"),
                    payload: serde_json::from_str(&payload)?,
                    created_at: chrono::DateTime::parse_from_rfc3339(
                        &row.get::<String, _>("created_at"),
                    )
                    .map_err(|e| ConfigError::Deserialization(e.to_string()))?
                    .with_timezone(&Utc),
                    updated_at: chrono::DateTime::parse_from_rfc3339(
                        &row.get::<String, _>("updated_at"),
                    )
                    .map_err(|e| ConfigError::Deserialization(e.to_string()))?
                    .with_timezone(&Utc),
                }))
            }
            None => Ok(None),
        }
    }

    /// Delete a profile by ID.
    pub async fn delete_profile(&self, id: &str) -> Result<(), ConfigError> {
        sqlx::query("DELETE FROM profiles WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(ConfigError::from)?;

        Ok(())
    }

    /// List all profile IDs.
    pub async fn list_profile_ids(&self) -> Result<Vec<String>, ConfigError> {
        let rows = sqlx::query("SELECT id FROM profiles ORDER BY updated_at DESC")
            .fetch_all(&self.pool)
            .await
            .map_err(ConfigError::from)?;

        Ok(rows.into_iter().map(|r| r.get::<String, _>("id")).collect())
    }

    // Prompt APIs

    /// Store or replace a prompt record.
    ///
    /// Returns `ConfigError::Validation` if the ID is empty.
    pub async fn set_prompt(&self, input: &PromptInput) -> Result<(), ConfigError> {
        if input.id.is_empty() {
            return Err(ConfigError::Validation(
                "Prompt ID must not be empty".to_string(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        let payload = serde_json::to_string(&input.payload)?;

        sqlx::query(
            r#"
            INSERT INTO prompts (id, schema_version, payload, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                schema_version = excluded.schema_version,
                payload = excluded.payload,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&input.id)
        .bind(input.schema_version)
        .bind(&payload)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        Ok(())
    }

    /// Get a prompt by ID.
    pub async fn get_prompt(&self, id: &str) -> Result<Option<PromptRecord>, ConfigError> {
        let row = sqlx::query(
            "SELECT id, schema_version, payload, created_at, updated_at FROM prompts WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        match row {
            Some(row) => {
                let payload: String = row.get("payload");
                Ok(Some(PromptRecord {
                    id: row.get("id"),
                    schema_version: row.get("schema_version"),
                    payload: serde_json::from_str(&payload)?,
                    created_at: chrono::DateTime::parse_from_rfc3339(
                        &row.get::<String, _>("created_at"),
                    )
                    .map_err(|e| ConfigError::Deserialization(e.to_string()))?
                    .with_timezone(&Utc),
                    updated_at: chrono::DateTime::parse_from_rfc3339(
                        &row.get::<String, _>("updated_at"),
                    )
                    .map_err(|e| ConfigError::Deserialization(e.to_string()))?
                    .with_timezone(&Utc),
                }))
            }
            None => Ok(None),
        }
    }

    /// Delete a prompt by ID.
    pub async fn delete_prompt(&self, id: &str) -> Result<(), ConfigError> {
        sqlx::query("DELETE FROM prompts WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(ConfigError::from)?;

        Ok(())
    }

    /// List all prompt IDs.
    pub async fn list_prompt_ids(&self) -> Result<Vec<String>, ConfigError> {
        let rows = sqlx::query("SELECT id FROM prompts ORDER BY updated_at DESC")
            .fetch_all(&self.pool)
            .await
            .map_err(ConfigError::from)?;

        Ok(rows.into_iter().map(|r| r.get::<String, _>("id")).collect())
    }

    // Schedule APIs

    /// Store or replace a schedule record.
    ///
    /// Returns `ConfigError::Validation` if the ID is empty.
    pub async fn set_schedule(&self, input: &ScheduleInput) -> Result<(), ConfigError> {
        if input.id.is_empty() {
            return Err(ConfigError::Validation(
                "Schedule ID must not be empty".to_string(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        let payload = serde_json::to_string(&input.payload)?;

        sqlx::query(
            r#"
            INSERT INTO schedule (id, schema_version, payload, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                schema_version = excluded.schema_version,
                payload = excluded.payload,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&input.id)
        .bind(input.schema_version)
        .bind(&payload)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        Ok(())
    }

    /// Get a schedule entry by ID.
    pub async fn get_schedule(&self, id: &str) -> Result<Option<ScheduleRecord>, ConfigError> {
        let row = sqlx::query(
            "SELECT id, schema_version, payload, created_at, updated_at FROM schedule WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        match row {
            Some(row) => {
                let payload: String = row.get("payload");
                Ok(Some(ScheduleRecord {
                    id: row.get("id"),
                    schema_version: row.get("schema_version"),
                    payload: serde_json::from_str(&payload)?,
                    created_at: chrono::DateTime::parse_from_rfc3339(
                        &row.get::<String, _>("created_at"),
                    )
                    .map_err(|e| ConfigError::Deserialization(e.to_string()))?
                    .with_timezone(&Utc),
                    updated_at: chrono::DateTime::parse_from_rfc3339(
                        &row.get::<String, _>("updated_at"),
                    )
                    .map_err(|e| ConfigError::Deserialization(e.to_string()))?
                    .with_timezone(&Utc),
                }))
            }
            None => Ok(None),
        }
    }

    /// Delete a schedule entry by ID.
    pub async fn delete_schedule(&self, id: &str) -> Result<(), ConfigError> {
        sqlx::query("DELETE FROM schedule WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(ConfigError::from)?;

        Ok(())
    }

    /// List all schedule IDs.
    pub async fn list_schedule_ids(&self) -> Result<Vec<String>, ConfigError> {
        let rows = sqlx::query("SELECT id FROM schedule ORDER BY updated_at DESC")
            .fetch_all(&self.pool)
            .await
            .map_err(ConfigError::from)?;

        Ok(rows.into_iter().map(|r| r.get::<String, _>("id")).collect())
    }

    // Credential APIs

    /// Store or replace a provider credential.
    pub async fn set_credential(
        &self,
        provider_slug: &str,
        credential_mode: &str,
        payload: &[u8],
    ) -> Result<(), ConfigError> {
        let cipher = self.cipher.as_ref().ok_or_else(|| {
            ConfigError::KeyUnavailable("No encryption key available".to_string())
        })?;

        let ad = format!("{}:{}", provider_slug, credential_mode);
        let encrypted = cipher.encrypt(payload, ad.as_bytes())?;
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO credentials (provider_slug, credential_mode, encrypted_payload, nonce, encryption_metadata, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(provider_slug) DO UPDATE SET
                credential_mode = excluded.credential_mode,
                encrypted_payload = excluded.encrypted_payload,
                nonce = excluded.nonce,
                encryption_metadata = excluded.encryption_metadata,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(provider_slug)
        .bind(credential_mode)
        .bind(&encrypted.ciphertext)
        .bind(&encrypted.nonce)
        .bind(&encrypted.metadata)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        Ok(())
    }

    /// Get a decrypted credential by provider slug.
    pub async fn get_credential(
        &self,
        provider_slug: &str,
    ) -> Result<Option<Vec<u8>>, ConfigError> {
        let row = sqlx::query(
            "SELECT provider_slug, credential_mode, encrypted_payload, nonce, encryption_metadata FROM credentials WHERE provider_slug = ?",
        )
        .bind(provider_slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        match row {
            Some(row) => {
                let cipher = self.cipher.as_ref().ok_or_else(|| {
                    ConfigError::KeyUnavailable("No encryption key available".to_string())
                })?;

                let credential_mode: String = row.get("credential_mode");
                let ad = format!("{}:{}", provider_slug, credential_mode);

                let encrypted = super::crypto::EncryptedPayload {
                    ciphertext: row.get("encrypted_payload"),
                    nonce: row.get("nonce"),
                    metadata: row.get("encryption_metadata"),
                };

                let decrypted = cipher.decrypt(&encrypted, ad.as_bytes())?;
                Ok(Some(decrypted))
            }
            None => Ok(None),
        }
    }

    /// Remove a credential by provider slug.
    pub async fn remove_credential(&self, provider_slug: &str) -> Result<(), ConfigError> {
        sqlx::query("DELETE FROM credentials WHERE provider_slug = ?")
            .bind(provider_slug)
            .execute(&self.pool)
            .await
            .map_err(ConfigError::from)?;

        Ok(())
    }

    /// List all provider slugs with credentials.
    pub async fn list_credential_slugs(&self) -> Result<Vec<String>, ConfigError> {
        let rows = sqlx::query("SELECT provider_slug FROM credentials ORDER BY updated_at DESC")
            .fetch_all(&self.pool)
            .await
            .map_err(ConfigError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| r.get::<String, _>("provider_slug"))
            .collect())
    }

    /// Get credential metadata without decrypting.
    pub async fn get_credential_metadata(
        &self,
        provider_slug: &str,
    ) -> Result<Option<CredentialRecord>, ConfigError> {
        let row = sqlx::query(
            "SELECT provider_slug, credential_mode, created_at, updated_at FROM credentials WHERE provider_slug = ?",
        )
        .bind(provider_slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        match row {
            Some(row) => Ok(Some(CredentialRecord {
                provider_slug: row.get("provider_slug"),
                credential_mode: row.get("credential_mode"),
                created_at: chrono::DateTime::parse_from_rfc3339(
                    &row.get::<String, _>("created_at"),
                )
                .map_err(|e| ConfigError::Deserialization(e.to_string()))?
                .with_timezone(&Utc),
                updated_at: chrono::DateTime::parse_from_rfc3339(
                    &row.get::<String, _>("updated_at"),
                )
                .map_err(|e| ConfigError::Deserialization(e.to_string()))?
                .with_timezone(&Utc),
            })),
            None => Ok(None),
        }
    }

    /// Acquire a connection from the pool.
    ///
    /// This is exposed for integration tests that need to hold transactions
    /// to simulate write-lock contention.
    #[doc(hidden)]
    pub async fn acquire(&self) -> Result<sqlx::pool::PoolConnection<sqlx::Sqlite>, ConfigError> {
        self.pool
            .acquire()
            .await
            .map_err(|e| ConfigError::Query(e.to_string()))
    }
}

/// Resolve the platform-default config path.
///
/// Spec-required paths:
/// - Linux (`XDG_CONFIG_HOME` set): `$XDG_CONFIG_HOME/agentiron/config.db`
/// - Linux (no XDG): `~/.config/agentiron/config.db`
/// - macOS: `~/Library/Application Support/com.agentiron/iron-core/config.db`
/// - Windows: `%APPDATA%/AgentIron/config.db`
pub fn default_config_path() -> Result<std::path::PathBuf, ConfigError> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "linux")] {
            let config_dir = match std::env::var("XDG_CONFIG_HOME") {
                Ok(v) if !v.is_empty() => std::path::PathBuf::from(v),
                _ => {
                    let home = std::env::var("HOME")
                        .map_err(|_| ConfigError::Path("HOME not set".to_string()))?;
                    std::path::PathBuf::from(home).join(".config")
                }
            };
            Ok(config_dir.join("agentiron").join("config.db"))
        } else if #[cfg(target_os = "macos")] {
            let home = std::env::var("HOME")
                .map_err(|_| ConfigError::Path("HOME not set".to_string()))?;
            Ok(std::path::PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("com.agentiron")
                .join("iron-core")
                .join("config.db"))
        } else if #[cfg(target_os = "windows")] {
            let app_data = std::env::var("APPDATA")
                .map_err(|_| ConfigError::Path("APPDATA not set".to_string()))?;
            Ok(std::path::PathBuf::from(app_data)
                .join("AgentIron")
                .join("config.db"))
        } else {
            compile_error!("Unsupported platform for default_config_path");
        }
    }
}
