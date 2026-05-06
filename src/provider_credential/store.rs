//! Provider credential store boundary.
//!
//! Clients provide a store implementation for OAuth credential persistence.
//! `iron-core` interacts only through this boundary and has no dependency on
//! concrete storage backends such as SQLite or OS keyrings.

use crate::provider_credential::domain::{ProviderAuthStatus, ProviderSlug, StoredCredential};
use std::collections::HashMap;
use std::sync::Arc;

/// Async boundary for provider credential storage.
///
/// Implementations are provided by clients (e.g. AgentIron with SQLite,
/// `iron-tui` with an in-memory store, etc.). `iron-core` uses this trait
/// to look up, update, remove, and list provider credentials.
#[async_trait::async_trait]
pub trait ProviderCredentialStore: Send + Sync {
    /// Look up the stored credential for a provider, if any.
    async fn get(&self, slug: &ProviderSlug) -> Option<StoredCredential>;

    /// Store or replace the credential for a provider.
    async fn set(&self, slug: &ProviderSlug, credential: StoredCredential);

    /// Remove the stored credential for a provider.
    async fn remove(&self, slug: &ProviderSlug);

    /// List all providers that have stored credentials.
    async fn list_slugs(&self) -> Vec<ProviderSlug>;
}

/// An in-memory credential store for testing and lightweight clients.
///
/// All operations are synchronous under the hood but exposed via the async
/// trait for compatibility.
pub struct InMemoryCredentialStore {
    inner: parking_lot::Mutex<HashMap<String, StoredCredential>>,
}

impl InMemoryCredentialStore {
    /// Create a new empty in-memory store.
    pub fn new() -> Self {
        Self {
            inner: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    /// Create a new store pre-populated with credentials.
    pub fn from_map(map: HashMap<String, StoredCredential>) -> Self {
        Self {
            inner: parking_lot::Mutex::new(map),
        }
    }

    /// Get a clone of the entire contents for inspection in tests.
    pub fn snapshot(&self) -> HashMap<String, StoredCredential> {
        self.inner.lock().clone()
    }

    /// Derive a client-visible auth status from a stored credential.
    ///
    /// This helper is available on the in-memory store for test convenience.
    /// Production store implementations may compute status differently.
    pub fn status_from_credential(cred: Option<&StoredCredential>) -> ProviderAuthStatus {
        match cred {
            None => ProviderAuthStatus::NotConfigured,
            Some(StoredCredential::ApiKey(_)) => ProviderAuthStatus::ConfiguredApiKey,
            Some(StoredCredential::OAuthBearer(tokens)) => {
                let now = std::time::SystemTime::now();
                match tokens.expires_at {
                    Some(expires_at) if now >= expires_at => ProviderAuthStatus::Expired,
                    _ => ProviderAuthStatus::ConnectedOAuth {
                        expires_at: tokens.expires_at,
                    },
                }
            }
        }
    }
}

impl Default for InMemoryCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ProviderCredentialStore for InMemoryCredentialStore {
    async fn get(&self, slug: &ProviderSlug) -> Option<StoredCredential> {
        self.inner.lock().get(slug.as_str()).cloned()
    }

    async fn set(&self, slug: &ProviderSlug, credential: StoredCredential) {
        self.inner
            .lock()
            .insert(slug.as_str().to_string(), credential);
    }

    async fn remove(&self, slug: &ProviderSlug) {
        self.inner.lock().remove(slug.as_str());
    }

    async fn list_slugs(&self) -> Vec<ProviderSlug> {
        self.inner
            .lock()
            .keys()
            .cloned()
            .map(ProviderSlug::new)
            .collect()
    }
}

/// A no-op store that never persists anything.
///
/// Useful when the managed provider path is enabled but the client does not
/// wish to persist credentials.
pub struct NullCredentialStore;

#[async_trait::async_trait]
impl ProviderCredentialStore for NullCredentialStore {
    async fn get(&self, _slug: &ProviderSlug) -> Option<StoredCredential> {
        None
    }

    async fn set(&self, _slug: &ProviderSlug, _credential: StoredCredential) {
        // no-op
    }

    async fn remove(&self, _slug: &ProviderSlug) {
        // no-op
    }

    async fn list_slugs(&self) -> Vec<ProviderSlug> {
        Vec::new()
    }
}

/// Type-erased handle to a credential store.
pub type DynCredentialStore = Arc<dyn ProviderCredentialStore>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_credential::domain::{OAuthTokenSet, ProviderAuthStatus};

    #[tokio::test]
    async fn in_memory_store_roundtrip() {
        let store = InMemoryCredentialStore::new();
        let slug = ProviderSlug::new("kimi-code");
        let cred = StoredCredential::ApiKey("sk-test".into());

        assert!(store.get(&slug).await.is_none());

        store.set(&slug, cred.clone()).await;
        let got = store.get(&slug).await;
        assert_eq!(got, Some(cred));

        store.remove(&slug).await;
        assert!(store.get(&slug).await.is_none());
    }

    #[tokio::test]
    async fn in_memory_store_list_slugs() {
        let store = InMemoryCredentialStore::new();
        let slug_a = ProviderSlug::new("kimi-code");
        let slug_b = ProviderSlug::new("codex");

        store
            .set(&slug_a, StoredCredential::ApiKey("sk-a".into()))
            .await;
        store
            .set(&slug_b, StoredCredential::ApiKey("sk-b".into()))
            .await;

        let mut slugs = store.list_slugs().await;
        slugs.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        assert_eq!(slugs, vec![slug_b, slug_a]);
    }

    #[tokio::test]
    async fn null_store_always_empty() {
        let store = NullCredentialStore;
        let slug = ProviderSlug::new("codex");
        assert!(store.get(&slug).await.is_none());
        assert!(store.list_slugs().await.is_empty());
    }

    #[test]
    fn status_from_credential_api_key() {
        let cred = StoredCredential::ApiKey("sk-test".into());
        assert_eq!(
            InMemoryCredentialStore::status_from_credential(Some(&cred)),
            ProviderAuthStatus::ConfiguredApiKey
        );
    }

    #[test]
    fn status_from_credential_oauth_valid() {
        let cred = StoredCredential::OAuthBearer(OAuthTokenSet {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at: Some(std::time::SystemTime::now() + std::time::Duration::from_secs(3600)),
            id_token: None,
        });
        assert!(matches!(
            InMemoryCredentialStore::status_from_credential(Some(&cred)),
            ProviderAuthStatus::ConnectedOAuth { .. }
        ));
    }

    #[test]
    fn status_from_credential_oauth_expired() {
        let cred = StoredCredential::OAuthBearer(OAuthTokenSet {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at: Some(std::time::SystemTime::now() - std::time::Duration::from_secs(1)),
            id_token: None,
        });
        assert_eq!(
            InMemoryCredentialStore::status_from_credential(Some(&cred)),
            ProviderAuthStatus::Expired
        );
    }

    #[test]
    fn status_from_credential_none() {
        assert_eq!(
            InMemoryCredentialStore::status_from_credential(None),
            ProviderAuthStatus::NotConfigured
        );
    }
}
