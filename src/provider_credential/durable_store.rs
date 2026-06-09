use crate::config::{ConfigError, ConfigStore};
use crate::provider_credential::domain::{ProviderSlug, StoredCredential};
use crate::provider_credential::store::ProviderCredentialStore;
use async_trait::async_trait;
use tracing::warn;

/// A durable provider credential store backed by ConfigStore.
pub struct DurableCredentialStore {
    store: ConfigStore,
}

impl DurableCredentialStore {
    /// Create a new durable credential store from a ConfigStore.
    pub fn new(store: ConfigStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ProviderCredentialStore for DurableCredentialStore {
    async fn get(&self, slug: &ProviderSlug) -> Option<StoredCredential> {
        match self.store.get_credential(slug.as_str()).await {
            Ok(Some(bytes)) => match serde_json::from_slice::<StoredCredential>(&bytes) {
                Ok(cred) => Some(cred),
                Err(e) => {
                    warn!(provider = %slug.as_str(), error = %e, "Failed to deserialize stored credential");
                    None
                }
            },
            Ok(None) => None,
            Err(e) => {
                warn!(provider = %slug.as_str(), error = %e, "Durable credential get failed");
                None
            }
        }
    }

    async fn set(&self, slug: &ProviderSlug, credential: StoredCredential) {
        let mode = match &credential {
            StoredCredential::ApiKey(_) => "api_key",
            StoredCredential::OAuthBearer(_) => "oauth_bearer",
        };

        let payload = match serde_json::to_vec(&credential) {
            Ok(p) => p,
            Err(e) => {
                warn!(provider = %slug.as_str(), error = %e, "Failed to serialize credential for storage");
                return;
            }
        };

        if let Err(e) = self
            .store
            .set_credential(slug.as_str(), mode, &payload)
            .await
        {
            warn!(provider = %slug.as_str(), error = %e, "Durable credential set failed");
        }
    }

    async fn remove(&self, slug: &ProviderSlug) {
        if let Err(e) = self.store.remove_credential(slug.as_str()).await {
            warn!(provider = %slug.as_str(), error = %e, "Durable credential remove failed");
        }
    }

    async fn list_slugs(&self) -> Vec<ProviderSlug> {
        match self.store.list_credential_slugs().await {
            Ok(slugs) => slugs.into_iter().map(ProviderSlug::new).collect(),
            Err(e) => {
                warn!(error = %e, "Durable credential list_slugs failed");
                Vec::new()
            }
        }
    }
}

/// A fallible version of the credential store boundary.
///
/// This trait allows durable implementations to surface errors
/// while preserving the existing infallible trait for compatibility.
#[async_trait]
pub trait FallibleCredentialStore: Send + Sync {
    /// Look up the stored credential for a provider.
    async fn get(&self, slug: &ProviderSlug) -> Result<Option<StoredCredential>, ConfigError>;

    /// Store or replace the credential for a provider.
    async fn set(
        &self,
        slug: &ProviderSlug,
        credential: StoredCredential,
    ) -> Result<(), ConfigError>;

    /// Remove the stored credential for a provider.
    async fn remove(&self, slug: &ProviderSlug) -> Result<(), ConfigError>;

    /// List all providers that have stored credentials.
    async fn list_slugs(&self) -> Result<Vec<ProviderSlug>, ConfigError>;

    /// Get credential metadata without decrypting secrets.
    async fn get_metadata(
        &self,
        slug: &ProviderSlug,
    ) -> Result<Option<CredentialMetadata>, ConfigError>;
}

/// Credential metadata without secret material.
#[derive(Debug, Clone)]
pub struct CredentialMetadata {
    pub provider_slug: String,
    pub credential_mode: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[async_trait]
impl FallibleCredentialStore for DurableCredentialStore {
    async fn get(&self, slug: &ProviderSlug) -> Result<Option<StoredCredential>, ConfigError> {
        match self.store.get_credential(slug.as_str()).await? {
            Some(bytes) => {
                let cred = serde_json::from_slice(&bytes)
                    .map_err(|e| ConfigError::Deserialization(e.to_string()))?;
                Ok(Some(cred))
            }
            None => Ok(None),
        }
    }

    async fn set(
        &self,
        slug: &ProviderSlug,
        credential: StoredCredential,
    ) -> Result<(), ConfigError> {
        let mode = match &credential {
            StoredCredential::ApiKey(_) => "api_key",
            StoredCredential::OAuthBearer(_) => "oauth_bearer",
        };

        let payload = serde_json::to_vec(&credential)
            .map_err(|e| ConfigError::Serialization(e.to_string()))?;

        self.store
            .set_credential(slug.as_str(), mode, &payload)
            .await
    }

    async fn remove(&self, slug: &ProviderSlug) -> Result<(), ConfigError> {
        self.store.remove_credential(slug.as_str()).await
    }

    async fn list_slugs(&self) -> Result<Vec<ProviderSlug>, ConfigError> {
        let slugs = self.store.list_credential_slugs().await?;
        Ok(slugs.into_iter().map(ProviderSlug::new).collect())
    }

    async fn get_metadata(
        &self,
        slug: &ProviderSlug,
    ) -> Result<Option<CredentialMetadata>, ConfigError> {
        match self.store.get_credential_metadata(slug.as_str()).await? {
            Some(record) => Ok(Some(CredentialMetadata {
                provider_slug: record.provider_slug,
                credential_mode: record.credential_mode,
                created_at: record.created_at,
                updated_at: record.updated_at,
            })),
            None => Ok(None),
        }
    }
}
