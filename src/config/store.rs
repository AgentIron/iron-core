use super::{
    crypto::{DynCredentialCipher, XChaCha20Poly1305Cipher},
    db,
    error::ConfigError,
    key_source::{EnvVarKeySource, KeySource, OsKeyringKeySource, StaticKeySource},
    migrations,
    records::*,
};
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Durable configuration store for AgentIron.
pub struct ConfigStore {
    pool: SqlitePool,
    cipher: Option<DynCredentialCipher>,
}

/// Options for opening a ConfigStore.
#[derive(Default)]
pub struct OpenOptions {
    /// Optional cipher to use instead of resolving one from key sources.
    pub cipher: Option<DynCredentialCipher>,
    /// Optional busy timeout for SQLite write-lock contention.
    ///
    /// When omitted the default is 5 seconds.
    pub busy_timeout: Option<Duration>,
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

    /// Access the underlying SQLite pool (for tests and direct queries).
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
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

    // ============================================================================
    // Provider Config APIs
    // ============================================================================

    /// Store or replace a provider runtime configuration.
    pub async fn set_provider_config(
        &self,
        input: &ProviderConfigInput,
    ) -> Result<ProviderConfigRecord, ConfigError> {
        if input.provider_slug.trim().is_empty() {
            return Err(ConfigError::Validation(
                "Provider slug must not be empty".to_string(),
            ));
        }
        if input.display_name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "Provider display name must not be empty".to_string(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO provider_configs (provider_slug, display_name, enabled, base_url, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(provider_slug) DO UPDATE SET
                display_name = excluded.display_name,
                enabled = excluded.enabled,
                base_url = excluded.base_url,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&input.provider_slug)
        .bind(&input.display_name)
        .bind(input.enabled)
        .bind(&input.base_url)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        Ok(ProviderConfigRecord {
            provider_slug: input.provider_slug.clone(),
            display_name: input.display_name.clone(),
            enabled: input.enabled,
            base_url: input.base_url.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    /// Get a provider runtime configuration by slug.
    pub async fn get_provider_config(
        &self,
        provider_slug: &str,
    ) -> Result<Option<ProviderConfigRecord>, ConfigError> {
        let row = sqlx::query(
            "SELECT provider_slug, display_name, enabled, base_url, created_at, updated_at FROM provider_configs WHERE provider_slug = ?",
        )
        .bind(provider_slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        match row {
            Some(row) => Ok(Some(ProviderConfigRecord {
                provider_slug: row.get("provider_slug"),
                display_name: row.get("display_name"),
                enabled: row.get::<i64, _>("enabled") != 0,
                base_url: row.get("base_url"),
                created_at: parse_datetime(row.get::<String, _>("created_at"))?,
                updated_at: parse_datetime(row.get::<String, _>("updated_at"))?,
            })),
            None => Ok(None),
        }
    }

    /// List all provider runtime configurations.
    pub async fn list_provider_configs(&self) -> Result<Vec<ProviderConfigRecord>, ConfigError> {
        let rows = sqlx::query(
            "SELECT provider_slug, display_name, enabled, base_url, created_at, updated_at FROM provider_configs ORDER BY updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        rows.into_iter()
            .map(|row| {
                Ok(ProviderConfigRecord {
                    provider_slug: row.get("provider_slug"),
                    display_name: row.get("display_name"),
                    enabled: row.get::<i64, _>("enabled") != 0,
                    base_url: row.get("base_url"),
                    created_at: parse_datetime(row.get::<String, _>("created_at"))?,
                    updated_at: parse_datetime(row.get::<String, _>("updated_at"))?,
                })
            })
            .collect()
    }

    /// Remove a provider runtime configuration by slug.
    pub async fn remove_provider_config(&self, provider_slug: &str) -> Result<(), ConfigError> {
        sqlx::query("DELETE FROM provider_configs WHERE provider_slug = ?")
            .bind(provider_slug)
            .execute(&self.pool)
            .await
            .map_err(ConfigError::from)?;
        Ok(())
    }

    /// Return the union of built-in provider slugs and persisted provider config slugs.
    pub async fn known_provider_slugs(
        &self,
    ) -> Result<std::collections::HashSet<String>, ConfigError> {
        let mut slugs = std::collections::HashSet::new();

        // Built-in providers from iron-providers
        let registry = iron_providers::ProviderRegistry::default();
        for slug in registry.slugs() {
            slugs.insert(slug.to_string());
        }

        // Persisted provider configs
        for config in self.list_provider_configs().await? {
            slugs.insert(config.provider_slug);
        }

        Ok(slugs)
    }

    // ============================================================================
    // Custom Model APIs
    // ============================================================================

    /// Store or replace a custom model record.
    pub async fn set_custom_model(
        &self,
        input: &CustomModelInput,
    ) -> Result<CustomModelRecord, ConfigError> {
        if input.provider_slug.trim().is_empty() {
            return Err(ConfigError::Validation(
                "Provider slug must not be empty".to_string(),
            ));
        }
        if input.model_id.trim().is_empty() {
            return Err(ConfigError::Validation(
                "Model ID must not be empty".to_string(),
            ));
        }
        if input.display_name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "Display name must not be empty".to_string(),
            ));
        }
        if matches!(input.context_window, Some(0)) {
            return Err(ConfigError::Validation(
                "Context window must be greater than 0 when set".to_string(),
            ));
        }
        if matches!(input.output_limit, Some(0)) {
            return Err(ConfigError::Validation(
                "Output limit must be greater than 0 when set".to_string(),
            ));
        }
        validate_optional_non_negative_f64(input.cost_input_per_million, "Input cost per million")?;
        validate_optional_non_negative_f64(
            input.cost_output_per_million,
            "Output cost per million",
        )?;

        // Validate provider slug is known (built-in or persisted provider config)
        let known_slugs = self.known_provider_slugs().await?;
        if !known_slugs.contains(&input.provider_slug) {
            return Err(ConfigError::Validation(format!(
                "Provider slug '{}' is not recognized. Add a provider config first or use a built-in provider slug.",
                input.provider_slug
            )));
        }

        // Enforce extend-only semantics: custom models must not shadow built-ins.
        let empty_custom_models: Vec<CustomModelRecord> = Vec::new();
        let builtin_catalog = super::effective_catalog::build_effective_catalog(
            &super::builtin_models::builtin_model_catalog(),
            &empty_custom_models,
        )?;
        if builtin_catalog.contains(&input.provider_slug, &input.model_id) {
            return Err(ConfigError::Validation(format!(
                "Custom model ({} / {}) conflicts with a built-in model id",
                input.provider_slug, input.model_id
            )));
        }

        let reasoning_json = serde_json::to_string(&input.reasoning_effort_values)?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO custom_models (provider_slug, model_id, display_name, context_window, output_limit, supports_tool_calls, supports_reasoning, supports_vision, supports_streaming, reasoning_effort_values_json, cost_input_per_million, cost_output_per_million, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(provider_slug, model_id) DO UPDATE SET
                display_name = excluded.display_name,
                context_window = excluded.context_window,
                output_limit = excluded.output_limit,
                supports_tool_calls = excluded.supports_tool_calls,
                supports_reasoning = excluded.supports_reasoning,
                supports_vision = excluded.supports_vision,
                supports_streaming = excluded.supports_streaming,
                reasoning_effort_values_json = excluded.reasoning_effort_values_json,
                cost_input_per_million = excluded.cost_input_per_million,
                cost_output_per_million = excluded.cost_output_per_million,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&input.provider_slug)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(input.context_window.map(|v| v as i64))
        .bind(input.output_limit.map(|v| v as i64))
        .bind(input.supports_tool_calls)
        .bind(input.supports_reasoning)
        .bind(input.supports_vision)
        .bind(input.supports_streaming)
        .bind(reasoning_json)
        .bind(input.cost_input_per_million)
        .bind(input.cost_output_per_million)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        Ok(CustomModelRecord {
            provider_slug: input.provider_slug.clone(),
            model_id: input.model_id.clone(),
            display_name: input.display_name.clone(),
            context_window: input.context_window,
            output_limit: input.output_limit,
            supports_tool_calls: input.supports_tool_calls,
            supports_reasoning: input.supports_reasoning,
            supports_vision: input.supports_vision,
            supports_streaming: input.supports_streaming,
            reasoning_effort_values: input.reasoning_effort_values.clone(),
            cost_input_per_million: input.cost_input_per_million,
            cost_output_per_million: input.cost_output_per_million,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    /// Get a custom model by provider slug and model ID.
    pub async fn get_custom_model(
        &self,
        provider_slug: &str,
        model_id: &str,
    ) -> Result<Option<CustomModelRecord>, ConfigError> {
        let row = sqlx::query(
            "SELECT provider_slug, model_id, display_name, context_window, output_limit, supports_tool_calls, supports_reasoning, supports_vision, supports_streaming, reasoning_effort_values_json, cost_input_per_million, cost_output_per_million, created_at, updated_at FROM custom_models WHERE provider_slug = ? AND model_id = ?",
        )
        .bind(provider_slug)
        .bind(model_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        match row {
            Some(row) => Ok(Some(CustomModelRecord {
                provider_slug: row.get("provider_slug"),
                model_id: row.get("model_id"),
                display_name: row.get("display_name"),
                context_window: normalize_optional_u32(row.get::<Option<i64>, _>("context_window")),
                output_limit: normalize_optional_u32(row.get::<Option<i64>, _>("output_limit")),
                supports_tool_calls: row.get::<i64, _>("supports_tool_calls") != 0,
                supports_reasoning: row.get::<i64, _>("supports_reasoning") != 0,
                supports_vision: row.get::<i64, _>("supports_vision") != 0,
                supports_streaming: row.get::<i64, _>("supports_streaming") != 0,
                reasoning_effort_values: parse_reasoning_effort_values(
                    row.get::<Option<String>, _>("reasoning_effort_values_json"),
                )?,
                cost_input_per_million: row.get("cost_input_per_million"),
                cost_output_per_million: row.get("cost_output_per_million"),
                created_at: parse_datetime(row.get::<String, _>("created_at"))?,
                updated_at: parse_datetime(row.get::<String, _>("updated_at"))?,
            })),
            None => Ok(None),
        }
    }

    /// List custom models, optionally filtered by provider slug.
    pub async fn list_custom_models(
        &self,
        provider_slug: Option<&str>,
    ) -> Result<Vec<CustomModelRecord>, ConfigError> {
        let rows = if let Some(slug) = provider_slug {
            sqlx::query(
                "SELECT provider_slug, model_id, display_name, context_window, output_limit, supports_tool_calls, supports_reasoning, supports_vision, supports_streaming, reasoning_effort_values_json, cost_input_per_million, cost_output_per_million, created_at, updated_at FROM custom_models WHERE provider_slug = ? ORDER BY updated_at DESC",
            )
            .bind(slug)
            .fetch_all(&self.pool)
            .await
            .map_err(ConfigError::from)?
        } else {
            sqlx::query(
                "SELECT provider_slug, model_id, display_name, context_window, output_limit, supports_tool_calls, supports_reasoning, supports_vision, supports_streaming, reasoning_effort_values_json, cost_input_per_million, cost_output_per_million, created_at, updated_at FROM custom_models ORDER BY updated_at DESC",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(ConfigError::from)?
        };

        rows.into_iter()
            .map(|row| {
                Ok(CustomModelRecord {
                    provider_slug: row.get("provider_slug"),
                    model_id: row.get("model_id"),
                    display_name: row.get("display_name"),
                    context_window: normalize_optional_u32(
                        row.get::<Option<i64>, _>("context_window"),
                    ),
                    output_limit: normalize_optional_u32(row.get::<Option<i64>, _>("output_limit")),
                    supports_tool_calls: row.get::<i64, _>("supports_tool_calls") != 0,
                    supports_reasoning: row.get::<i64, _>("supports_reasoning") != 0,
                    supports_vision: row.get::<i64, _>("supports_vision") != 0,
                    supports_streaming: row.get::<i64, _>("supports_streaming") != 0,
                    reasoning_effort_values: parse_reasoning_effort_values(
                        row.get::<Option<String>, _>("reasoning_effort_values_json"),
                    )?,
                    cost_input_per_million: row.get("cost_input_per_million"),
                    cost_output_per_million: row.get("cost_output_per_million"),
                    created_at: parse_datetime(row.get::<String, _>("created_at"))?,
                    updated_at: parse_datetime(row.get::<String, _>("updated_at"))?,
                })
            })
            .collect()
    }

    /// Remove a custom model by provider slug and model ID.
    pub async fn remove_custom_model(
        &self,
        provider_slug: &str,
        model_id: &str,
    ) -> Result<(), ConfigError> {
        sqlx::query("DELETE FROM custom_models WHERE provider_slug = ? AND model_id = ?")
            .bind(provider_slug)
            .bind(model_id)
            .execute(&self.pool)
            .await
            .map_err(ConfigError::from)?;
        Ok(())
    }

    // ============================================================================
    // Default Model APIs
    // ============================================================================

    /// Set the default model selection.
    pub async fn set_default_model(
        &self,
        input: &DefaultModelInput,
    ) -> Result<DefaultModelRecord, ConfigError> {
        if input.provider_slug.trim().is_empty() {
            return Err(ConfigError::Validation(
                "Provider slug must not be empty".to_string(),
            ));
        }
        if input.model_id.trim().is_empty() {
            return Err(ConfigError::Validation(
                "Model ID must not be empty".to_string(),
            ));
        }

        // Validate that the requested default model exists in the effective catalog.
        let custom_models = self.list_custom_models(None).await?;
        let catalog = super::effective_catalog::build_effective_catalog(
            &super::builtin_models::builtin_model_catalog(),
            &custom_models,
        )?;
        if !catalog.contains(&input.provider_slug, &input.model_id) {
            return Err(ConfigError::Validation(format!(
                "Default model ({} / {}) is not present in the model catalog",
                input.provider_slug, input.model_id
            )));
        }

        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO runtime_defaults (id, provider_slug, model_id, updated_at)
            VALUES (1, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                provider_slug = excluded.provider_slug,
                model_id = excluded.model_id,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&input.provider_slug)
        .bind(&input.model_id)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        Ok(DefaultModelRecord {
            provider_slug: input.provider_slug.clone(),
            model_id: input.model_id.clone(),
            updated_at: Utc::now(),
        })
    }

    /// Get the default model selection.
    pub async fn get_default_model(&self) -> Result<Option<DefaultModelRecord>, ConfigError> {
        let row = sqlx::query(
            "SELECT provider_slug, model_id, updated_at FROM runtime_defaults WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        match row {
            Some(row) => Ok(Some(DefaultModelRecord {
                provider_slug: row.get("provider_slug"),
                model_id: row.get("model_id"),
                updated_at: parse_datetime(row.get::<String, _>("updated_at"))?,
            })),
            None => Ok(None),
        }
    }

    /// Clear the default model selection.
    pub async fn clear_default_model(&self) -> Result<(), ConfigError> {
        sqlx::query("DELETE FROM runtime_defaults WHERE id = 1")
            .execute(&self.pool)
            .await
            .map_err(ConfigError::from)?;
        Ok(())
    }

    // ============================================================================
    // MCP Server APIs
    // ============================================================================

    /// Store or replace an MCP server configuration.
    pub async fn set_mcp_server(
        &self,
        input: &McpServerConfigInput,
    ) -> Result<McpServerConfigRecord, ConfigError> {
        if input.id.trim().is_empty() {
            return Err(ConfigError::Validation(
                "MCP server ID must not be empty".to_string(),
            ));
        }
        if input.label.trim().is_empty() {
            return Err(ConfigError::Validation(
                "MCP server label must not be empty".to_string(),
            ));
        }
        validate_mcp_server_input(input)?;
        let (transport_kind, command, args_json, env_json, url, headers_json) =
            serialize_mcp_transport(&input.transport)?;
        let inherited_env_vars_json = serde_json::to_string(&input.inherited_env_vars)?;
        let working_dir_str = input
            .working_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO mcp_servers (id, label, description, transport_kind, command, args_json, env_json, inherited_env_vars_json, url, headers_json, working_dir, enabled_by_default, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                label = excluded.label,
                description = excluded.description,
                transport_kind = excluded.transport_kind,
                command = excluded.command,
                args_json = excluded.args_json,
                env_json = excluded.env_json,
                inherited_env_vars_json = excluded.inherited_env_vars_json,
                url = excluded.url,
                headers_json = excluded.headers_json,
                working_dir = excluded.working_dir,
                enabled_by_default = excluded.enabled_by_default,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&input.id)
        .bind(&input.label)
        .bind(&input.description)
        .bind(&transport_kind)
        .bind(&command)
        .bind(&args_json)
        .bind(&env_json)
        .bind(&inherited_env_vars_json)
        .bind(&url)
        .bind(&headers_json)
        .bind(&working_dir_str)
        .bind(input.enabled_by_default)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        Ok(McpServerConfigRecord {
            id: input.id.clone(),
            label: input.label.clone(),
            description: input.description.clone(),
            transport: input.transport.clone(),
            working_dir: input.working_dir.clone(),
            enabled_by_default: input.enabled_by_default,
            inherited_env_vars: input.inherited_env_vars.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    /// Get an MCP server configuration by ID.
    pub async fn get_mcp_server(
        &self,
        id: &str,
    ) -> Result<Option<McpServerConfigRecord>, ConfigError> {
        let row = sqlx::query(
            "SELECT id, label, description, transport_kind, command, args_json, env_json, inherited_env_vars_json, url, headers_json, working_dir, enabled_by_default, created_at, updated_at FROM mcp_servers WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        match row {
            Some(row) => {
                let transport = deserialize_mcp_transport(
                    row.get("transport_kind"),
                    row.get("command"),
                    row.get("args_json"),
                    row.get("env_json"),
                    row.get("url"),
                    row.get("headers_json"),
                )?;
                let inherited_env_vars: Vec<String> =
                    serde_json::from_str(&row.get::<String, _>("inherited_env_vars_json"))?;
                let working_dir: Option<String> = row.get("working_dir");
                Ok(Some(McpServerConfigRecord {
                    id: row.get("id"),
                    label: row.get("label"),
                    description: row.get("description"),
                    transport,
                    working_dir: working_dir.map(PathBuf::from),
                    enabled_by_default: row.get::<i64, _>("enabled_by_default") != 0,
                    inherited_env_vars,
                    created_at: parse_datetime(row.get::<String, _>("created_at"))?,
                    updated_at: parse_datetime(row.get::<String, _>("updated_at"))?,
                }))
            }
            None => Ok(None),
        }
    }

    /// List all MCP server configurations.
    pub async fn list_mcp_servers(&self) -> Result<Vec<McpServerConfigRecord>, ConfigError> {
        let rows = sqlx::query(
            "SELECT id, label, description, transport_kind, command, args_json, env_json, inherited_env_vars_json, url, headers_json, working_dir, enabled_by_default, created_at, updated_at FROM mcp_servers ORDER BY updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        let mut servers = Vec::new();
        for row in rows {
            let transport = deserialize_mcp_transport(
                row.get("transport_kind"),
                row.get("command"),
                row.get("args_json"),
                row.get("env_json"),
                row.get("url"),
                row.get("headers_json"),
            )?;
            let inherited_env_vars: Vec<String> =
                serde_json::from_str(&row.get::<String, _>("inherited_env_vars_json"))?;
            let working_dir: Option<String> = row.get("working_dir");
            servers.push(McpServerConfigRecord {
                id: row.get("id"),
                label: row.get("label"),
                description: row.get("description"),
                transport,
                working_dir: working_dir.map(PathBuf::from),
                enabled_by_default: row.get::<i64, _>("enabled_by_default") != 0,
                inherited_env_vars,
                created_at: parse_datetime(row.get::<String, _>("created_at"))?,
                updated_at: parse_datetime(row.get::<String, _>("updated_at"))?,
            });
        }
        Ok(servers)
    }

    /// Remove an MCP server configuration by ID.
    pub async fn remove_mcp_server(&self, id: &str) -> Result<(), ConfigError> {
        sqlx::query("DELETE FROM mcp_servers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(ConfigError::from)?;
        Ok(())
    }

    // ============================================================================
    // Skill Settings APIs
    // ============================================================================

    /// Store or replace skill settings.
    pub async fn set_skill_settings(
        &self,
        input: &SkillSettingsInput,
    ) -> Result<SkillSettingsRecord, ConfigError> {
        validate_skill_settings_input(input)?;
        let additional_skill_dirs_json = serde_json::to_string(&input.additional_skill_dirs)?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO skill_settings (id, trust_project_skills, additional_skill_dirs_json, updated_at)
            VALUES (1, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                trust_project_skills = excluded.trust_project_skills,
                additional_skill_dirs_json = excluded.additional_skill_dirs_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(input.trust_project_skills)
        .bind(&additional_skill_dirs_json)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        Ok(SkillSettingsRecord {
            trust_project_skills: input.trust_project_skills,
            additional_skill_dirs: input.additional_skill_dirs.clone(),
            updated_at: Utc::now(),
        })
    }

    /// Get skill settings, returning defaults if not set.
    pub async fn get_skill_settings(&self) -> Result<SkillSettingsRecord, ConfigError> {
        let row = sqlx::query(
            "SELECT trust_project_skills, additional_skill_dirs_json, updated_at FROM skill_settings WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        match row {
            Some(row) => {
                let additional_skill_dirs_json: String = row.get("additional_skill_dirs_json");
                let additional_skill_dirs: Vec<PathBuf> =
                    serde_json::from_str(&additional_skill_dirs_json)?;
                Ok(SkillSettingsRecord {
                    trust_project_skills: row.get::<i64, _>("trust_project_skills") != 0,
                    additional_skill_dirs,
                    updated_at: parse_datetime(row.get::<String, _>("updated_at"))?,
                })
            }
            None => Ok(SkillSettingsRecord {
                trust_project_skills: false,
                additional_skill_dirs: Vec::new(),
                updated_at: Utc::now(),
            }),
        }
    }

    // ============================================================================
    // Runtime Settings Snapshot
    // ============================================================================

    /// Load a validated runtime settings snapshot from the config store.
    pub async fn load_runtime_settings(&self) -> Result<RuntimeSettingsSnapshot, ConfigError> {
        let provider_configs = self.list_provider_configs().await?;
        let custom_models = self.list_custom_models(None).await?;
        let default_model = self.get_default_model().await?;
        let mcp_servers = self.list_mcp_servers().await?;
        let skill_settings = self.get_skill_settings().await?;

        // Validate all persisted custom models reference known provider slugs.
        let known_provider_slugs = self.known_provider_slugs().await?;
        if let Some(model) = custom_models
            .iter()
            .find(|model| !known_provider_slugs.contains(&model.provider_slug))
        {
            return Err(ConfigError::Validation(format!(
                "Custom model ({} / {}) references an unknown provider slug",
                model.provider_slug, model.model_id
            )));
        }

        // Validate cross-record consistency
        if let Some(ref default) = default_model {
            if default.provider_slug.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "Default model provider slug is empty".to_string(),
                ));
            }
            if default.model_id.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "Default model ID is empty".to_string(),
                ));
            }

            // Validate default model exists in effective catalog (built-in + custom)
            let catalog = super::effective_catalog::build_effective_catalog(
                &super::builtin_models::builtin_model_catalog(),
                &custom_models,
            )?;
            if !catalog.contains(&default.provider_slug, &default.model_id) {
                return Err(ConfigError::Validation(format!(
                    "Default model ({} / {}) is not present in the model catalog",
                    default.provider_slug, default.model_id
                )));
            }
        }

        // Validate MCP server IDs are unique (defensive; DB has PK)
        let mut seen_ids = std::collections::HashSet::new();
        for server in &mcp_servers {
            if !seen_ids.insert(&server.id) {
                return Err(ConfigError::Validation(format!(
                    "Duplicate MCP server ID: {}",
                    server.id
                )));
            }
        }

        // Validate inherited_env_vars entries are names only
        for server in &mcp_servers {
            for var_name in &server.inherited_env_vars {
                validate_env_var_name(var_name)?;
            }
        }

        validate_skill_settings_input(&SkillSettingsInput {
            trust_project_skills: skill_settings.trust_project_skills,
            additional_skill_dirs: skill_settings.additional_skill_dirs.clone(),
        })?;

        Ok(RuntimeSettingsSnapshot {
            provider_configs,
            custom_models,
            default_model,
            mcp_servers,
            skill_settings,
        })
    }
}

/// Parse an RFC3339 datetime string into a `DateTime<Utc>`.
fn parse_datetime(s: String) -> Result<DateTime<Utc>, ConfigError> {
    Ok(chrono::DateTime::parse_from_rfc3339(&s)
        .map_err(|e| ConfigError::Deserialization(e.to_string()))?
        .with_timezone(&Utc))
}

/// Normalize an optional i64 value from the database to Option<u32>,
/// treating negative values as None to avoid wraparound.
fn normalize_optional_u32(value: Option<i64>) -> Option<u32> {
    value.and_then(|v| if v < 0 { None } else { Some(v as u32) })
}

/// Parse reasoning effort values JSON. NULL or empty string defaults to empty Vec.
fn parse_reasoning_effort_values(json: Option<String>) -> Result<Vec<String>, ConfigError> {
    match json {
        None => Ok(Vec::new()),
        Some(s) if s.trim().is_empty() => Ok(Vec::new()),
        Some(s) => serde_json::from_str(&s).map_err(|e| {
            ConfigError::Deserialization(format!("Invalid reasoning_effort_values JSON: {}", e))
        }),
    }
}

fn validate_optional_non_negative_f64(value: Option<f64>, label: &str) -> Result<(), ConfigError> {
    if let Some(value) = value {
        if !value.is_finite() || value < 0.0 {
            return Err(ConfigError::Validation(format!(
                "{} must be finite and non-negative when set",
                label
            )));
        }
    }
    Ok(())
}

fn validate_env_var_name(name: &str) -> Result<(), ConfigError> {
    if name.trim().is_empty() {
        return Err(ConfigError::Validation(
            "Inherited environment variable name must not be empty".to_string(),
        ));
    }
    if name.contains('=') {
        return Err(ConfigError::Validation(format!(
            "Inherited environment variable '{}' contains '=' and is not a valid variable name",
            name
        )));
    }
    Ok(())
}

fn validate_mcp_server_input(input: &McpServerConfigInput) -> Result<(), ConfigError> {
    use crate::mcp::server::McpTransport;

    match &input.transport {
        McpTransport::Stdio { command, .. } => {
            if command.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "MCP stdio command must not be empty".to_string(),
                ));
            }
        }
        McpTransport::Http { config } | McpTransport::HttpSse { config } => {
            if config.url.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "MCP HTTP URL must not be empty".to_string(),
                ));
            }
        }
    }

    for name in &input.inherited_env_vars {
        validate_env_var_name(name)?;
    }

    Ok(())
}

fn validate_skill_settings_input(input: &SkillSettingsInput) -> Result<(), ConfigError> {
    for dir in &input.additional_skill_dirs {
        if dir.as_os_str().is_empty() {
            return Err(ConfigError::Validation(
                "Additional skill directory must not be empty".to_string(),
            ));
        }
    }
    Ok(())
}

type SerializedMcpTransport = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Serialize an MCP transport into database columns.
fn serialize_mcp_transport(
    transport: &crate::mcp::server::McpTransport,
) -> Result<SerializedMcpTransport, ConfigError> {
    use crate::mcp::server::McpTransport;
    match transport {
        McpTransport::Stdio { command, args, env } => {
            let args_json = serde_json::to_string(args)?;
            let env_json = serde_json::to_string(env)?;
            Ok((
                "stdio".to_string(),
                Some(command.clone()),
                Some(args_json),
                Some(env_json),
                None,
                None,
            ))
        }
        McpTransport::Http { config } => {
            let headers_json = config
                .headers
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            Ok((
                "http".to_string(),
                None,
                None,
                None,
                Some(config.url.clone()),
                headers_json,
            ))
        }
        McpTransport::HttpSse { config } => {
            let headers_json = config
                .headers
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            Ok((
                "http_sse".to_string(),
                None,
                None,
                None,
                Some(config.url.clone()),
                headers_json,
            ))
        }
    }
}

/// Deserialize MCP transport from database columns.
fn deserialize_mcp_transport(
    transport_kind: String,
    command: Option<String>,
    args_json: Option<String>,
    env_json: Option<String>,
    url: Option<String>,
    headers_json: Option<String>,
) -> Result<crate::mcp::server::McpTransport, ConfigError> {
    use crate::mcp::server::{HttpConfig, McpTransport};
    match transport_kind.as_str() {
        "stdio" => {
            let command = command
                .ok_or_else(|| ConfigError::Deserialization("Missing stdio command".to_string()))?;
            let args: Vec<String> = args_json
                .map(|s| serde_json::from_str(&s))
                .transpose()?
                .unwrap_or_default();
            let env: HashMap<String, String> = env_json
                .map(|s| serde_json::from_str(&s))
                .transpose()?
                .unwrap_or_default();
            Ok(McpTransport::Stdio { command, args, env })
        }
        "http" => {
            let url =
                url.ok_or_else(|| ConfigError::Deserialization("Missing HTTP URL".to_string()))?;
            let headers: Option<HashMap<String, String>> =
                headers_json.map(|s| serde_json::from_str(&s)).transpose()?;
            Ok(McpTransport::Http {
                config: HttpConfig { url, headers },
            })
        }
        "http_sse" => {
            let url = url
                .ok_or_else(|| ConfigError::Deserialization("Missing HTTP+SSE URL".to_string()))?;
            let headers: Option<HashMap<String, String>> =
                headers_json.map(|s| serde_json::from_str(&s)).transpose()?;
            Ok(McpTransport::HttpSse {
                config: HttpConfig { url, headers },
            })
        }
        other => Err(ConfigError::Deserialization(format!(
            "Unknown MCP transport kind: {}",
            other
        ))),
    }
}

// ============================================================================
// Saved Handoff APIs (Issue #69)
// ============================================================================

/// Convert a usize token count to an i64 for database storage.
fn to_db_token_count(value: usize) -> Result<i64, ConfigError> {
    i64::try_from(value).map_err(|_| {
        ConfigError::Validation("size_estimate_tokens exceeds SQLite INTEGER range".to_string())
    })
}

/// Convert an i64 token count from database storage to usize.
fn from_db_token_count(value: i64) -> Result<usize, ConfigError> {
    usize::try_from(value).map_err(|_| {
        ConfigError::Deserialization(format!(
            "Invalid persisted size_estimate_tokens value: {}",
            value
        ))
    })
}

impl ConfigStore {
    /// Save or replace a handoff bundle.
    ///
    /// Returns `ConfigError::Validation` if the ID or name is empty, or if the
    /// bundle version or metadata version is not supported.
    pub async fn save_handoff(&self, input: &SavedHandoffInput) -> Result<(), ConfigError> {
        // Validate ID
        if input.id.is_empty() {
            return Err(ConfigError::Validation(
                "Handoff ID must not be empty".to_string(),
            ));
        }

        // Validate name
        if input.name.is_empty() {
            return Err(ConfigError::Validation(
                "Handoff name must not be empty".to_string(),
            ));
        }

        // Validate bundle version
        if input.bundle.version != crate::context::handoff::HANDOFF_BUNDLE_VERSION {
            return Err(ConfigError::Validation(format!(
                "Unsupported handoff bundle version: {} (expected {})",
                input.bundle.version,
                crate::context::handoff::HANDOFF_BUNDLE_VERSION
            )));
        }

        // Validate metadata version
        if input.bundle.metadata.version != crate::context::handoff::HANDOFF_BUNDLE_VERSION {
            return Err(ConfigError::Validation(format!(
                "Unsupported handoff metadata version: {} (expected {})",
                input.bundle.metadata.version,
                crate::context::handoff::HANDOFF_BUNDLE_VERSION
            )));
        }

        // Serialize bundle
        let bundle_json = serde_json::to_string(&input.bundle)
            .map_err(|e| ConfigError::Serialization(e.to_string()))?;

        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO saved_handoffs (id, name, bundle_json, bundle_version, source_session_id, source_model, source_provider, size_estimate_tokens, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                bundle_json = excluded.bundle_json,
                bundle_version = excluded.bundle_version,
                source_session_id = excluded.source_session_id,
                source_model = excluded.source_model,
                source_provider = excluded.source_provider,
                size_estimate_tokens = excluded.size_estimate_tokens,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&input.id)
        .bind(&input.name)
        .bind(&bundle_json)
        .bind(&input.bundle.version)
        .bind(&input.bundle.metadata.source_session_id)
        .bind(&input.bundle.metadata.source_model)
        .bind(&input.bundle.metadata.source_provider)
        .bind(to_db_token_count(input.bundle.metadata.size_estimate_tokens)?)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        Ok(())
    }

    /// Load a saved handoff by ID.
    ///
    /// Returns `Ok(None)` if the handoff does not exist.
    /// Returns a typed error if the stored bundle is malformed or has an
    /// unsupported version.
    pub async fn load_handoff(&self, id: &str) -> Result<Option<SavedHandoffRecord>, ConfigError> {
        let row = sqlx::query(
            r#"
            SELECT id, name, bundle_json, bundle_version, source_session_id,
                   source_model, source_provider, size_estimate_tokens,
                   created_at, updated_at
            FROM saved_handoffs
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        match row {
            Some(row) => {
                let bundle_json: String = row.get("bundle_json");
                let bundle: crate::context::handoff::HandoffBundle =
                    serde_json::from_str(&bundle_json)
                        .map_err(|e| ConfigError::Deserialization(e.to_string()))?;

                // Validate loaded bundle version
                if bundle.version != crate::context::handoff::HANDOFF_BUNDLE_VERSION {
                    return Err(ConfigError::Validation(format!(
                        "Unsupported stored handoff bundle version: {} (expected {})",
                        bundle.version,
                        crate::context::handoff::HANDOFF_BUNDLE_VERSION
                    )));
                }

                // Validate loaded metadata version
                if bundle.metadata.version != crate::context::handoff::HANDOFF_BUNDLE_VERSION {
                    return Err(ConfigError::Validation(format!(
                        "Unsupported stored handoff metadata version: {} (expected {})",
                        bundle.metadata.version,
                        crate::context::handoff::HANDOFF_BUNDLE_VERSION
                    )));
                }

                Ok(Some(SavedHandoffRecord {
                    metadata: SavedHandoffMetadata {
                        id: row.get("id"),
                        name: row.get("name"),
                        bundle_version: row.get("bundle_version"),
                        source_session_id: row.get("source_session_id"),
                        source_model: row.get("source_model"),
                        source_provider: row.get("source_provider"),
                        size_estimate_tokens: from_db_token_count(
                            row.get::<i64, _>("size_estimate_tokens"),
                        )?,
                        created_at: parse_datetime(row.get::<String, _>("created_at"))?,
                        updated_at: parse_datetime(row.get::<String, _>("updated_at"))?,
                    },
                    bundle,
                }))
            }
            None => Ok(None),
        }
    }

    /// List all saved handoff metadata.
    ///
    /// Returns metadata only; full bundles are not deserialized.
    pub async fn list_handoffs(&self) -> Result<Vec<SavedHandoffMetadata>, ConfigError> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, bundle_version, source_session_id,
                   source_model, source_provider, size_estimate_tokens,
                   created_at, updated_at
            FROM saved_handoffs
            ORDER BY updated_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(ConfigError::from)?;

        rows.into_iter()
            .map(|row| {
                Ok(SavedHandoffMetadata {
                    id: row.get("id"),
                    name: row.get("name"),
                    bundle_version: row.get("bundle_version"),
                    source_session_id: row.get("source_session_id"),
                    source_model: row.get("source_model"),
                    source_provider: row.get("source_provider"),
                    size_estimate_tokens: from_db_token_count(
                        row.get::<i64, _>("size_estimate_tokens"),
                    )?,
                    created_at: parse_datetime(row.get::<String, _>("created_at"))?,
                    updated_at: parse_datetime(row.get::<String, _>("updated_at"))?,
                })
            })
            .collect()
    }

    /// Delete a saved handoff by ID.
    pub async fn delete_handoff(&self, id: &str) -> Result<(), ConfigError> {
        sqlx::query("DELETE FROM saved_handoffs WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(ConfigError::from)?;

        Ok(())
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
