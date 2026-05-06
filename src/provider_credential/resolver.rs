//! Credential resolver: turn app-supplied provider context into a
//! provider-safe `ResolvedCredential`.
//!
//! The resolver looks up stored credentials, checks provider-profile support,
//! refreshes expired/near-expiry OAuth tokens, and prefers API keys over OAuth
//! for dual-mode providers.

use crate::provider_credential::domain::{
    CredentialMode, OAuthTokenSet, ProviderAuthError, ProviderAuthResult, ProviderAuthStatus,
    ProviderPromptContext, ProviderSlug, ResolvedCredential, StoredCredential,
};
use crate::provider_credential::oauth::{refresh_access_token, v1_oauth_metadata};
use crate::provider_credential::store::DynCredentialStore;
use iron_providers::ProviderCredential;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

/// Margin before expiry to trigger proactive refresh.
pub const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);

/// Supported credential modes for a provider.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CredentialSupport {
    pub api_key: bool,
    pub oauth_bearer: bool,
}

/// Resolves provider credentials for a prompt from stored state and optional
/// client-supplied API keys.
pub struct CredentialResolver {
    store: DynCredentialStore,
    support_map: HashMap<String, CredentialSupport>,
    http_client: reqwest::Client,
    status_overrides: Mutex<HashMap<String, ProviderAuthStatus>>,
}

fn build_v1_support_map() -> HashMap<String, CredentialSupport> {
    let mut map = HashMap::new();
    // Hardcoded V1 support based on iron-providers built-in profiles.
    // kimi-code supports both API key (x-api-key) and OAuthBearer.
    map.insert(
        "kimi-code".to_string(),
        CredentialSupport {
            api_key: true,
            oauth_bearer: true,
        },
    );
    // codex supports only OAuthBearer.
    map.insert(
        "codex".to_string(),
        CredentialSupport {
            api_key: false,
            oauth_bearer: true,
        },
    );
    // openai supports only API key (default profile).
    map.insert(
        "openai".to_string(),
        CredentialSupport {
            api_key: true,
            oauth_bearer: false,
        },
    );
    // kimi (general) supports only API key.
    map.insert(
        "kimi".to_string(),
        CredentialSupport {
            api_key: true,
            oauth_bearer: false,
        },
    );
    map
}

impl CredentialResolver {
    /// Create a new resolver with the given store.
    ///
    /// Built-in provider profiles from `iron-providers` are scanned to populate
    /// the credential-support map.
    pub fn new(store: DynCredentialStore) -> Self {
        Self {
            store,
            support_map: build_v1_support_map(),
            http_client: reqwest::Client::new(),
            status_overrides: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new resolver with a custom HTTP client (useful for testing).
    pub fn with_http_client(store: DynCredentialStore, http_client: reqwest::Client) -> Self {
        let mut resolver = Self::new(store);
        resolver.http_client = http_client;
        resolver
    }

    fn support_for(&self, slug: &str) -> CredentialSupport {
        self.support_map
            .get(&slug.to_lowercase())
            .cloned()
            .unwrap_or(CredentialSupport {
                api_key: true,
                oauth_bearer: true,
            })
    }

    /// Resolve a credential for the given prompt context.
    ///
    /// If `api_key` is provided and the provider supports API-key auth, it is
    /// used immediately (API key wins over OAuth for dual-mode providers).
    ///
    /// Otherwise, the credential store is queried for an OAuth credential.
    /// If found and the provider supports OAuth, the token is refreshed if
    /// expired or within `REFRESH_MARGIN` of expiry.
    pub async fn resolve(
        &self,
        context: &ProviderPromptContext,
        api_key: Option<String>,
    ) -> ProviderAuthResult<ResolvedCredential> {
        let slug_str = context.provider_slug.as_str();
        let support = self.support_for(slug_str);

        // Prefer API key if available and supported
        if let Some(key) = api_key {
            if support.api_key {
                self.clear_status_override(slug_str);
                return Ok(ResolvedCredential {
                    provider_credential: ProviderCredential::ApiKey(key),
                    mode: CredentialMode::ApiKey,
                    was_refreshed: false,
                });
            } else {
                return Err(ProviderAuthError::UnsupportedCredential {
                    provider: slug_str.to_string(),
                    mode: CredentialMode::ApiKey,
                });
            }
        }

        // Look up OAuth credential from store
        let stored = self.store.get(&context.provider_slug).await;

        match stored {
            Some(StoredCredential::ApiKey(key)) => {
                if !support.api_key {
                    return Err(ProviderAuthError::UnsupportedCredential {
                        provider: slug_str.to_string(),
                        mode: CredentialMode::ApiKey,
                    });
                }

                // This path should be rare (API keys in the credential store),
                // but handle it gracefully.
                self.clear_status_override(slug_str);
                Ok(ResolvedCredential {
                    provider_credential: ProviderCredential::ApiKey(key),
                    mode: CredentialMode::ApiKey,
                    was_refreshed: false,
                })
            }
            Some(StoredCredential::OAuthBearer(tokens)) => {
                if !support.oauth_bearer {
                    return Err(ProviderAuthError::UnsupportedCredential {
                        provider: slug_str.to_string(),
                        mode: CredentialMode::OAuthBearer,
                    });
                }

                let needs_refresh = Self::token_needs_refresh(&tokens);

                if needs_refresh {
                    let refreshed = self.refresh_oauth(&context.provider_slug, &tokens).await?;
                    Ok(ResolvedCredential {
                        provider_credential: ProviderCredential::OAuthBearer {
                            access_token: refreshed.access_token.clone(),
                            expires_at: refreshed.expires_at,
                            id_token: refreshed.id_token.clone(),
                        },
                        mode: CredentialMode::OAuthBearer,
                        was_refreshed: true,
                    })
                } else {
                    self.clear_status_override(slug_str);
                    Ok(ResolvedCredential {
                        provider_credential: ProviderCredential::OAuthBearer {
                            access_token: tokens.access_token.clone(),
                            expires_at: tokens.expires_at,
                            id_token: tokens.id_token.clone(),
                        },
                        mode: CredentialMode::OAuthBearer,
                        was_refreshed: false,
                    })
                }
            }
            None => Err(ProviderAuthError::NotConfigured(slug_str.to_string())),
        }
    }

    /// Derive the client-visible auth status for a provider.
    ///
    /// This checks both the optional API key and the stored OAuth credential.
    pub async fn status(&self, slug: &ProviderSlug, api_key: Option<&str>) -> ProviderAuthStatus {
        let slug_str = slug.as_str();
        let support = self.support_for(slug_str);

        if api_key.is_some() {
            return if support.api_key {
                ProviderAuthStatus::ConfiguredApiKey
            } else {
                ProviderAuthStatus::UnsupportedCredential
            };
        }

        match self.store.get(slug).await {
            Some(StoredCredential::ApiKey(_)) => {
                if support.api_key {
                    ProviderAuthStatus::ConfiguredApiKey
                } else {
                    ProviderAuthStatus::UnsupportedCredential
                }
            }
            Some(StoredCredential::OAuthBearer(tokens)) => {
                if !support.oauth_bearer {
                    return ProviderAuthStatus::UnsupportedCredential;
                }
                if let Some(status) = self.status_overrides.lock().get(slug_str).cloned() {
                    return status;
                }
                if Self::token_is_expired(&tokens) {
                    ProviderAuthStatus::Expired
                } else if Self::token_needs_refresh(&tokens) {
                    ProviderAuthStatus::Refreshing
                } else {
                    ProviderAuthStatus::ConnectedOAuth {
                        expires_at: tokens.expires_at,
                    }
                }
            }
            None => ProviderAuthStatus::NotConfigured,
        }
    }

    /// Remove OAuth credential for a provider without touching API keys.
    pub async fn disconnect_oauth(&self, slug: &ProviderSlug) {
        // Only remove if the stored credential is OAuth
        if let Some(StoredCredential::OAuthBearer(_)) = self.store.get(slug).await {
            self.store.remove(slug).await;
        }
        self.clear_status_override(slug.as_str());
    }

    fn clear_status_override(&self, slug: &str) {
        self.status_overrides.lock().remove(slug);
    }

    fn record_refresh_error(&self, slug: &ProviderSlug, error: &ProviderAuthError) {
        let status = match error {
            ProviderAuthError::Revoked(_) => ProviderAuthStatus::Revoked,
            ProviderAuthError::RefreshFailed { reason, .. } => ProviderAuthStatus::RefreshFailed {
                reason: reason.clone(),
            },
            _ => return,
        };
        self.status_overrides
            .lock()
            .insert(slug.as_str().to_string(), status);
    }

    fn token_needs_refresh(tokens: &OAuthTokenSet) -> bool {
        match tokens.expires_at {
            Some(expires_at) => {
                let now = SystemTime::now();
                match expires_at.duration_since(now) {
                    Ok(remaining) => remaining < REFRESH_MARGIN,
                    Err(_) => true, // already expired
                }
            }
            None => false, // no expiry known, assume valid
        }
    }

    fn token_is_expired(tokens: &OAuthTokenSet) -> bool {
        match tokens.expires_at {
            Some(expires_at) => SystemTime::now() >= expires_at,
            None => false,
        }
    }

    /// Force-refresh the OAuth token for a provider regardless of expiry.
    ///
    /// This is used after a provider auth failure to obtain a fresh token
    /// before retrying the request.
    pub async fn force_refresh(
        &self,
        slug: &ProviderSlug,
    ) -> ProviderAuthResult<ResolvedCredential> {
        let stored = self.store.get(slug).await;

        match stored {
            Some(StoredCredential::OAuthBearer(tokens)) => {
                let refreshed = self.refresh_oauth(slug, &tokens).await?;
                Ok(ResolvedCredential {
                    provider_credential: ProviderCredential::OAuthBearer {
                        access_token: refreshed.access_token.clone(),
                        expires_at: refreshed.expires_at,
                        id_token: refreshed.id_token.clone(),
                    },
                    mode: CredentialMode::OAuthBearer,
                    was_refreshed: true,
                })
            }
            Some(StoredCredential::ApiKey(key)) => {
                let support = self.support_for(slug.as_str());
                if !support.api_key {
                    return Err(ProviderAuthError::UnsupportedCredential {
                        provider: slug.as_str().to_string(),
                        mode: CredentialMode::ApiKey,
                    });
                }
                self.clear_status_override(slug.as_str());
                Ok(ResolvedCredential {
                    provider_credential: ProviderCredential::ApiKey(key),
                    mode: CredentialMode::ApiKey,
                    was_refreshed: false,
                })
            }
            None => Err(ProviderAuthError::NotConfigured(slug.as_str().to_string())),
        }
    }

    async fn refresh_oauth(
        &self,
        slug: &ProviderSlug,
        tokens: &OAuthTokenSet,
    ) -> ProviderAuthResult<OAuthTokenSet> {
        let metadata = v1_oauth_metadata(slug).ok_or_else(|| ProviderAuthError::RefreshFailed {
            provider: slug.as_str().to_string(),
            reason: "no OAuth metadata for provider".to_string(),
        })?;

        let result =
            match refresh_access_token(&metadata, &tokens.refresh_token, &self.http_client).await {
                Ok(result) => result,
                Err(error) => {
                    self.record_refresh_error(slug, &error);
                    return Err(error);
                }
            };

        let new_set = result.into_token_set(Some(tokens.refresh_token.clone()));

        // Persist the refreshed credential
        self.store
            .set(slug, StoredCredential::OAuthBearer(new_set.clone()))
            .await;

        self.clear_status_override(slug.as_str());
        Ok(new_set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_credential::store::{InMemoryCredentialStore, ProviderCredentialStore};
    use std::sync::Arc;

    fn make_resolver(store: DynCredentialStore) -> CredentialResolver {
        CredentialResolver::new(store)
    }

    fn make_expired_tokens() -> OAuthTokenSet {
        OAuthTokenSet {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at: Some(SystemTime::now() - Duration::from_secs(1)),
            id_token: None,
        }
    }

    fn make_near_expiry_tokens() -> OAuthTokenSet {
        OAuthTokenSet {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at: Some(SystemTime::now() + Duration::from_secs(30)),
            id_token: None,
        }
    }

    fn make_fresh_tokens() -> OAuthTokenSet {
        OAuthTokenSet {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at: Some(SystemTime::now() + Duration::from_secs(3600)),
            id_token: None,
        }
    }

    #[tokio::test]
    async fn resolve_missing_credential() {
        let store: DynCredentialStore = Arc::new(InMemoryCredentialStore::new());
        let resolver = make_resolver(store);
        let ctx = ProviderPromptContext {
            provider_slug: ProviderSlug::new("codex"),
            model: "codex-model".into(),
            api_key: None,
        };

        let result = resolver.resolve(&ctx, None).await;
        assert!(matches!(result, Err(ProviderAuthError::NotConfigured(ref s)) if s == "codex"));
    }

    #[tokio::test]
    async fn resolve_api_key() {
        let store: DynCredentialStore = Arc::new(InMemoryCredentialStore::new());
        let resolver = make_resolver(store);
        let ctx = ProviderPromptContext {
            provider_slug: ProviderSlug::new("openai"),
            model: "gpt-4o".into(),
            api_key: None,
        };

        let result = resolver
            .resolve(&ctx, Some("sk-test".into()))
            .await
            .unwrap();
        assert_eq!(result.mode, CredentialMode::ApiKey);
        assert!(!result.was_refreshed);
        assert_eq!(
            result.provider_credential,
            ProviderCredential::ApiKey("sk-test".into())
        );
    }

    #[tokio::test]
    async fn resolve_api_key_wins_over_oauth() {
        let store = Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("kimi-code"),
                StoredCredential::OAuthBearer(make_fresh_tokens()),
            )
            .await;

        let resolver = make_resolver(store);
        let ctx = ProviderPromptContext {
            provider_slug: ProviderSlug::new("kimi-code"),
            model: "kimi-model".into(),
            api_key: None,
        };

        let result = resolver
            .resolve(&ctx, Some("sk-test".into()))
            .await
            .unwrap();
        assert_eq!(result.mode, CredentialMode::ApiKey);
    }

    #[tokio::test]
    async fn resolve_api_key_unsupported_for_codex() {
        let store: DynCredentialStore = Arc::new(InMemoryCredentialStore::new());
        let resolver = make_resolver(store);
        let ctx = ProviderPromptContext {
            provider_slug: ProviderSlug::new("codex"),
            model: "codex-model".into(),
            api_key: None,
        };

        let result = resolver.resolve(&ctx, Some("sk-test".into())).await;
        assert!(matches!(
            result,
            Err(ProviderAuthError::UnsupportedCredential { ref provider, ref mode })
            if provider == "codex" && *mode == CredentialMode::ApiKey
        ));
    }

    #[tokio::test]
    async fn resolve_oauth_fresh() {
        let store = Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("codex"),
                StoredCredential::OAuthBearer(make_fresh_tokens()),
            )
            .await;

        let resolver = make_resolver(store);
        let ctx = ProviderPromptContext {
            provider_slug: ProviderSlug::new("codex"),
            model: "codex-model".into(),
            api_key: None,
        };

        let result = resolver.resolve(&ctx, None).await.unwrap();
        assert_eq!(result.mode, CredentialMode::OAuthBearer);
        assert!(!result.was_refreshed);
        assert!(matches!(
            result.provider_credential,
            ProviderCredential::OAuthBearer { .. }
        ));
    }

    #[tokio::test]
    async fn resolve_oauth_unsupported_provider() {
        let store = Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("openai"),
                StoredCredential::OAuthBearer(make_fresh_tokens()),
            )
            .await;

        let resolver = make_resolver(store);
        let ctx = ProviderPromptContext {
            provider_slug: ProviderSlug::new("openai"),
            model: "gpt-4o".into(),
            api_key: None,
        };

        // openai built-in profile does not support OAuthBearer
        let result = resolver.resolve(&ctx, None).await;
        assert!(matches!(
            result,
            Err(ProviderAuthError::UnsupportedCredential { ref provider, ref mode })
            if provider == "openai" && *mode == CredentialMode::OAuthBearer
        ));
    }

    #[tokio::test]
    async fn status_api_key() {
        let store: DynCredentialStore = Arc::new(InMemoryCredentialStore::new());
        let resolver = make_resolver(store);

        assert_eq!(
            resolver
                .status(&ProviderSlug::new("kimi-code"), Some("sk"))
                .await,
            ProviderAuthStatus::ConfiguredApiKey
        );
    }

    #[tokio::test]
    async fn status_external_api_key_unsupported() {
        let store: DynCredentialStore = Arc::new(InMemoryCredentialStore::new());
        let resolver = make_resolver(store);

        assert_eq!(
            resolver
                .status(&ProviderSlug::new("codex"), Some("sk"))
                .await,
            ProviderAuthStatus::UnsupportedCredential
        );
    }

    #[tokio::test]
    async fn status_oauth_fresh() {
        let store = Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("codex"),
                StoredCredential::OAuthBearer(make_fresh_tokens()),
            )
            .await;

        let resolver = make_resolver(store);
        let status = resolver.status(&ProviderSlug::new("codex"), None).await;
        assert!(matches!(status, ProviderAuthStatus::ConnectedOAuth { .. }));
    }

    #[tokio::test]
    async fn status_oauth_expired() {
        let store = Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("codex"),
                StoredCredential::OAuthBearer(make_expired_tokens()),
            )
            .await;

        let resolver = make_resolver(store);
        let status = resolver.status(&ProviderSlug::new("codex"), None).await;
        assert_eq!(status, ProviderAuthStatus::Expired);
    }

    #[tokio::test]
    async fn status_oauth_near_expiry_reports_refreshing() {
        let store = Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("codex"),
                StoredCredential::OAuthBearer(make_near_expiry_tokens()),
            )
            .await;

        let resolver = make_resolver(store);
        let status = resolver.status(&ProviderSlug::new("codex"), None).await;
        assert_eq!(status, ProviderAuthStatus::Refreshing);
    }

    #[tokio::test]
    async fn status_unsupported_credential_mode() {
        let store = Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("openai"),
                StoredCredential::OAuthBearer(make_fresh_tokens()),
            )
            .await;

        let resolver = make_resolver(store);
        let status = resolver.status(&ProviderSlug::new("openai"), None).await;
        assert_eq!(status, ProviderAuthStatus::UnsupportedCredential);
    }

    #[tokio::test]
    async fn status_api_key_stored_for_oauth_only_provider() {
        let store = Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("codex"),
                StoredCredential::ApiKey("sk-test".into()),
            )
            .await;

        let resolver = make_resolver(store);
        let status = resolver.status(&ProviderSlug::new("codex"), None).await;
        assert_eq!(status, ProviderAuthStatus::UnsupportedCredential);
    }

    #[tokio::test]
    async fn resolve_stored_api_key_for_oauth_only_provider_is_unsupported() {
        let store = Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("codex"),
                StoredCredential::ApiKey("sk-test".into()),
            )
            .await;

        let resolver = make_resolver(store);
        let ctx = ProviderPromptContext {
            provider_slug: ProviderSlug::new("codex"),
            model: "codex-model".into(),
            api_key: None,
        };

        let result = resolver.resolve(&ctx, None).await;
        assert!(matches!(
            result,
            Err(ProviderAuthError::UnsupportedCredential { ref provider, ref mode })
                if provider == "codex" && *mode == CredentialMode::ApiKey
        ));
    }

    #[tokio::test]
    async fn disconnect_oauth_removes_only_oauth() {
        let store = Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("codex"),
                StoredCredential::OAuthBearer(make_fresh_tokens()),
            )
            .await;

        let resolver = make_resolver(store.clone());
        resolver.disconnect_oauth(&ProviderSlug::new("codex")).await;

        assert!(store.get(&ProviderSlug::new("codex")).await.is_none());
    }

    #[test]
    fn token_needs_refresh_expired() {
        assert!(CredentialResolver::token_needs_refresh(
            &make_expired_tokens()
        ));
    }

    #[test]
    fn token_needs_refresh_near_expiry() {
        assert!(CredentialResolver::token_needs_refresh(
            &make_near_expiry_tokens()
        ));
    }

    #[test]
    fn token_needs_refresh_fresh() {
        assert!(!CredentialResolver::token_needs_refresh(
            &make_fresh_tokens()
        ));
    }

    #[test]
    fn support_map_populated_from_registry() {
        let store: DynCredentialStore = Arc::new(InMemoryCredentialStore::new());
        let resolver = make_resolver(store);

        let kimi_support = resolver.support_for("kimi-code");
        assert!(kimi_support.api_key);
        assert!(kimi_support.oauth_bearer);

        let codex_support = resolver.support_for("codex");
        assert!(!codex_support.api_key);
        assert!(codex_support.oauth_bearer);
    }

    #[tokio::test]
    async fn force_refresh_returns_not_configured_when_missing() {
        let store: DynCredentialStore = Arc::new(InMemoryCredentialStore::new());
        let resolver = make_resolver(store);

        let result = resolver.force_refresh(&ProviderSlug::new("codex")).await;
        assert!(matches!(result, Err(ProviderAuthError::NotConfigured(ref s)) if s == "codex"));
    }

    #[tokio::test]
    async fn force_refresh_returns_api_key_without_refresh() {
        let store = Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("kimi-code"),
                StoredCredential::ApiKey("sk-test".into()),
            )
            .await;

        let resolver = make_resolver(store);
        let result = resolver
            .force_refresh(&ProviderSlug::new("kimi-code"))
            .await
            .unwrap();
        assert_eq!(result.mode, CredentialMode::ApiKey);
        assert!(!result.was_refreshed);
        assert_eq!(
            result.provider_credential,
            ProviderCredential::ApiKey("sk-test".into())
        );
    }

    #[tokio::test]
    async fn force_refresh_oauth_attempts_refresh_and_fails_without_network() {
        let store = Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("codex"),
                StoredCredential::OAuthBearer(make_fresh_tokens()),
            )
            .await;

        let resolver = make_resolver(store);
        let result = resolver.force_refresh(&ProviderSlug::new("codex")).await;
        // Without a mock token server, the refresh request fails.
        assert!(
            matches!(result, Err(ProviderAuthError::RefreshFailed { ref provider, .. }) if provider == "codex"),
            "expected RefreshFailed for codex, got: {:?}",
            result
        );
        assert!(matches!(
            resolver.status(&ProviderSlug::new("codex"), None).await,
            ProviderAuthStatus::RefreshFailed { .. }
        ));
    }

    #[tokio::test]
    async fn resolve_prefers_explicit_api_key_over_stored_oauth() {
        let store = Arc::new(InMemoryCredentialStore::new());
        store
            .set(
                &ProviderSlug::new("kimi-code"),
                StoredCredential::OAuthBearer(make_fresh_tokens()),
            )
            .await;

        let resolver = make_resolver(store);
        let ctx = ProviderPromptContext {
            provider_slug: ProviderSlug::new("kimi-code"),
            model: "kimi-model".into(),
            api_key: None,
        };

        let result = resolver
            .resolve(&ctx, Some("explicit-key".into()))
            .await
            .unwrap();
        assert_eq!(
            result.provider_credential,
            ProviderCredential::ApiKey("explicit-key".into())
        );
    }
}
