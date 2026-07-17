//! Effective provider registry construction.
//!
//! Builds a [`ProviderRegistry`] by starting with built-in profiles from
//! `iron-providers` and applying persisted custom/override provider profiles
//! from the config store.

use crate::config::{ConfigError, ConfigStore};
use iron_providers::{ProviderProfile, ProviderRegistry};

/// Build an effective provider registry from built-ins plus persisted profiles.
///
/// Starts with [`ProviderRegistry::default`] (which loads all built-in profiles)
/// and applies each persisted provider profile. Persisted profiles
/// with built-in slugs override the built-in; persisted profiles with new slugs
/// are added alongside built-ins.
///
/// Invalid stored profiles are skipped with a warning rather than failing the
/// entire operation.
///
/// # Errors
///
/// Returns [`ConfigError`] if persisted provider-profile rows cannot be listed.
pub async fn build_effective_registry(
    store: &ConfigStore,
) -> Result<ProviderRegistry, ConfigError> {
    let mut registry = ProviderRegistry::default();

    let records = store.list_provider_profiles().await?;
    for record in records {
        match serde_json::from_str::<ProviderProfile>(&record.profile_json) {
            Ok(profile) => {
                if profile.slug != record.slug {
                    tracing::warn!(
                        row_slug = %record.slug,
                        payload_slug = %profile.slug,
                        "Stored provider profile slug mismatch; skipping"
                    );
                    continue;
                }
                let slug = profile.slug.clone();
                registry.register(profile);
                tracing::debug!(slug = %slug, "Applied persisted provider profile to effective registry");
            }
            Err(e) => {
                tracing::warn!(
                    slug = %record.slug,
                    error = %e,
                    "Failed to deserialize stored provider profile; skipping"
                );
            }
        }
    }

    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iron_providers::{ApiFamily, ProviderProfile};

    #[tokio::test]
    async fn empty_store_preserves_builtins() {
        let store = ConfigStore::open_in_memory().await.unwrap();
        let registry = build_effective_registry(&store).await.unwrap();

        // Built-in providers should be available
        assert!(registry.slugs().contains(&"openai"));
        assert!(registry.slugs().contains(&"anthropic"));
    }

    #[tokio::test]
    async fn custom_profile_is_added() {
        let store = ConfigStore::open_in_memory().await.unwrap();

        let profile = ProviderProfile::new(
            "my-custom",
            ApiFamily::Completions,
            "https://custom.example.com",
        );
        let json = serde_json::to_string(&profile).unwrap();
        import_test_profile(&store, "my-custom", &json).await;

        let registry = build_effective_registry(&store).await.unwrap();
        assert!(registry.slugs().contains(&"my-custom"));
    }

    #[tokio::test]
    async fn override_replaces_builtin() {
        let store = ConfigStore::open_in_memory().await.unwrap();

        let profile = ProviderProfile::new(
            "openai",
            ApiFamily::Messages,
            "https://custom.openai.example",
        )
        .with_provider_guidance("Custom OpenAI guidance.");
        let json = serde_json::to_string(&profile).unwrap();
        import_test_profile(&store, "openai", &json).await;

        let registry = build_effective_registry(&store).await.unwrap();
        let fragment = registry.system_prompt_fragment("openai").unwrap();
        assert_eq!(fragment, "Custom OpenAI guidance.");
    }

    #[tokio::test]
    async fn invalid_profile_skipped() {
        let store = ConfigStore::open_in_memory().await.unwrap();

        // Insert invalid JSON directly via SQL (bypassing set_provider_profile validation)
        sqlx::query(
            "INSERT INTO provider_profiles (slug, profile_json, source, created_at, updated_at) VALUES (?, ?, NULL, ?, ?)",
        )
        .bind("bad-profile")
        .bind("{not valid}")
        .bind("2026-01-01T00:00:00Z")
        .bind("2026-01-01T00:00:00Z")
        .execute(store.pool())
        .await
        .unwrap();

        let registry = build_effective_registry(&store).await.unwrap();
        // Built-ins should still be present
        assert!(registry.slugs().contains(&"openai"));
        // Invalid profile should not be present
        assert!(!registry.slugs().contains(&"bad-profile"));
    }

    async fn import_test_profile(store: &ConfigStore, slug: &str, json: &str) {
        let input = crate::config::ProviderProfileInput {
            slug: slug.to_string(),
            profile_json: json.to_string(),
            source: None,
        };
        store.set_provider_profile(&input).await.unwrap();
    }
}
