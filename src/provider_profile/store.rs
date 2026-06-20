//! Provider profile store boundary and ConfigStore-backed implementation.
//!
//! Provides import/export of provider profile JSON through the core config
//! store. The trait mirrors the pattern used by `ProviderCredentialStore`:
//! `iron-core` defines the boundary, clients provide concrete implementations.

use crate::config::{ConfigError, ConfigStore, ProviderProfileInput};
use async_trait::async_trait;

/// Async boundary for provider profile storage.
///
/// Implementations are provided by the core config store. Clients that do not
/// configure durable storage can use built-in-only provider behavior without
/// implementing this trait.
#[async_trait]
pub trait ProfileStore: Send + Sync {
    /// Get a stored provider profile by slug, if any.
    async fn get(&self, slug: &str) -> Option<String>;

    /// Store or replace a provider profile payload by slug.
    async fn set(&self, slug: &str, profile_json: &str, source: Option<&str>);

    /// Remove a stored provider profile by slug.
    async fn remove(&self, slug: &str);

    /// List all stored provider profile slugs.
    async fn list_slugs(&self) -> Vec<String>;
}

/// ConfigStore-backed provider profile store.
///
/// Wraps a [`ConfigStore`] to provide the [`ProfileStore`] boundary. Profile
/// payloads are validated before persistence.
pub struct DurableProfileStore {
    store: ConfigStore,
}

impl DurableProfileStore {
    /// Create a new durable profile store from a [`ConfigStore`].
    pub fn new(store: ConfigStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ProfileStore for DurableProfileStore {
    async fn get(&self, slug: &str) -> Option<String> {
        match self.store.get_provider_profile(slug).await {
            Ok(Some(record)) => Some(record.profile_json),
            Ok(None) => None,
            Err(e) => {
                tracing::warn!(slug = %slug, error = %e, "Durable profile store get failed");
                None
            }
        }
    }

    async fn set(&self, slug: &str, profile_json: &str, source: Option<&str>) {
        let input = ProviderProfileInput {
            slug: slug.to_string(),
            profile_json: profile_json.to_string(),
            source: source.map(|s| s.to_string()),
        };
        if let Err(e) = self.store.set_provider_profile(&input).await {
            tracing::warn!(slug = %slug, error = %e, "Durable profile store set failed");
        }
    }

    async fn remove(&self, slug: &str) {
        if let Err(e) = self.store.delete_provider_profile(slug).await {
            tracing::warn!(slug = %slug, error = %e, "Durable profile store remove failed");
        }
    }

    async fn list_slugs(&self) -> Vec<String> {
        match self.store.list_provider_profiles().await {
            Ok(records) => records.into_iter().map(|r| r.slug).collect(),
            Err(e) => {
                tracing::warn!(error = %e, "Durable profile store list_slugs failed");
                Vec::new()
            }
        }
    }
}

/// Import a provider profile from JSON into the config store.
///
/// Validates the payload (including rejecting credential secret material),
/// deserializes it to extract the slug, and persists the validated JSON.
///
/// Returns the slug of the imported profile on success.
pub async fn import_provider_profile(
    store: &ConfigStore,
    json: &str,
    source: Option<&str>,
) -> Result<String, ConfigError> {
    let profile = super::validation::validate_provider_profile(json)?;
    let slug = profile.slug.clone();
    let input = ProviderProfileInput {
        slug: slug.clone(),
        profile_json: serde_json::to_string(&profile)
            .map_err(|e| ConfigError::Serialization(e.to_string()))?,
        source: source.map(|s| s.to_string()),
    };
    store.set_provider_profile(&input).await?;
    Ok(slug)
}

/// Export a persisted provider profile as JSON by slug.
///
/// Returns `None` if no profile is stored for the slug. The exported JSON
/// contains only non-secret provider profile metadata.
pub async fn export_provider_profile(
    store: &ConfigStore,
    slug: &str,
) -> Result<Option<String>, ConfigError> {
    match store.get_provider_profile(slug).await? {
        Some(record) => Ok(Some(record.profile_json)),
        None => Ok(None),
    }
}
