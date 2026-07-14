use iron_core::config::{ConfigStore, OpenOptions, ProviderProfileInput};
use iron_core::provider_profile::{
    build_effective_registry, export_provider_profile, import_provider_profile,
    validate_provider_profile,
};
use iron_providers::{ApiFamily, AuthStrategy, ProviderProfile};
use serde_json::json;

/// Helper to create a test cipher for file-based stores.
fn test_cipher() -> std::sync::Arc<dyn iron_core::config::crypto::CredentialCipher> {
    let key = iron_core::config::crypto::XChaCha20Poly1305Cipher::generate_key();
    std::sync::Arc::new(iron_core::config::crypto::XChaCha20Poly1305Cipher::new(
        &key,
    ))
}

/// Helper to open a file-based config store for testing (bypasses OS keyring).
async fn open_test_store(path: impl AsRef<std::path::Path>) -> ConfigStore {
    let options = OpenOptions {
        cipher: Some(test_cipher()),
        busy_timeout: None,
    };
    ConfigStore::open_at_with_options(path, options)
        .await
        .unwrap()
}

// ============================================================================
// Task 1.5: Config-store provider profile CRUD tests
// ============================================================================

#[tokio::test]
async fn provider_profile_roundtrip() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let profile = ProviderProfile::new(
        "my-provider",
        ApiFamily::Completions,
        "https://api.example.com/v1",
    );
    let json = serde_json::to_string(&profile).unwrap();

    store
        .set_provider_profile(&ProviderProfileInput {
            slug: "my-provider".to_string(),
            profile_json: json.clone(),
            source: Some("test".to_string()),
        })
        .await
        .unwrap();

    let retrieved = store.get_provider_profile("my-provider").await.unwrap();
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.slug, "my-provider");
    assert_eq!(retrieved.profile_json, json);
    assert_eq!(retrieved.source.as_deref(), Some("test"));
}

#[tokio::test]
async fn provider_profile_update_replaces_payload() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let profile1 = ProviderProfile::new(
        "test-provider",
        ApiFamily::Completions,
        "https://v1.example.com",
    );
    let profile2 = ProviderProfile::new(
        "test-provider",
        ApiFamily::Messages,
        "https://v2.example.com",
    );

    store
        .set_provider_profile(&ProviderProfileInput {
            slug: "test-provider".to_string(),
            profile_json: serde_json::to_string(&profile1).unwrap(),
            source: None,
        })
        .await
        .unwrap();

    store
        .set_provider_profile(&ProviderProfileInput {
            slug: "test-provider".to_string(),
            profile_json: serde_json::to_string(&profile2).unwrap(),
            source: None,
        })
        .await
        .unwrap();

    let retrieved = store
        .get_provider_profile("test-provider")
        .await
        .unwrap()
        .unwrap();
    let parsed: ProviderProfile = serde_json::from_str(&retrieved.profile_json).unwrap();
    assert_eq!(parsed.base_url, "https://v2.example.com");
    assert_eq!(parsed.family, ApiFamily::Messages);
}

#[tokio::test]
async fn provider_profile_list_is_deterministic() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    for slug in &["zeta", "alpha", "mid"] {
        let profile = ProviderProfile::new(*slug, ApiFamily::Completions, "https://example.com");
        store
            .set_provider_profile(&ProviderProfileInput {
                slug: slug.to_string(),
                profile_json: serde_json::to_string(&profile).unwrap(),
                source: None,
            })
            .await
            .unwrap();
    }

    let list = store.list_provider_profiles().await.unwrap();
    let slugs: Vec<&str> = list.iter().map(|r| r.slug.as_str()).collect();
    assert_eq!(slugs, vec!["alpha", "mid", "zeta"]);
}

#[tokio::test]
async fn provider_profile_delete() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let profile = ProviderProfile::new("to-delete", ApiFamily::Completions, "https://example.com");
    store
        .set_provider_profile(&ProviderProfileInput {
            slug: "to-delete".to_string(),
            profile_json: serde_json::to_string(&profile).unwrap(),
            source: None,
        })
        .await
        .unwrap();

    assert!(store
        .get_provider_profile("to-delete")
        .await
        .unwrap()
        .is_some());

    store.delete_provider_profile("to-delete").await.unwrap();

    assert!(store
        .get_provider_profile("to-delete")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn provider_profile_empty_slug_rejected() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let result = store
        .set_provider_profile(&ProviderProfileInput {
            slug: "".to_string(),
            profile_json: "{}".to_string(),
            source: None,
        })
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn provider_profile_migration_preserves_existing_data() {
    let db_path = std::env::temp_dir().join(format!(
        "iron-core-provider-profile-migration-{}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&db_path);

    // Write an older schema (v1) directly
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true),
        )
        .await
        .unwrap();

    sqlx::raw_sql(
        r#"
        CREATE TABLE schema_version (id INTEGER PRIMARY KEY CHECK (id = 1), version INTEGER NOT NULL);
        CREATE TABLE profiles (id TEXT PRIMARY KEY, schema_version INTEGER NOT NULL, payload TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
        CREATE TABLE prompts (id TEXT PRIMARY KEY, schema_version INTEGER NOT NULL, payload TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
        CREATE TABLE schedule (id TEXT PRIMARY KEY, schema_version INTEGER NOT NULL, payload TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
        CREATE TABLE credentials (provider_slug TEXT PRIMARY KEY, credential_mode TEXT NOT NULL, encrypted_payload BLOB NOT NULL, nonce BLOB NOT NULL, encryption_metadata TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
        INSERT INTO schema_version (id, version) VALUES (1, 1);
        INSERT INTO profiles (id, schema_version, payload, created_at, updated_at)
        VALUES ('legacy', 1, '{"name":"Legacy"}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();
    drop(pool);

    let store = open_test_store(&db_path).await;

    // Legacy profile should be preserved
    let legacy = store.get_profile("legacy").await.unwrap().unwrap();
    assert_eq!(legacy.payload, json!({"name": "Legacy"}));

    // New provider_profiles table should be available
    let profile = ProviderProfile::new(
        "post-migration",
        ApiFamily::Completions,
        "https://example.com",
    );
    store
        .set_provider_profile(&ProviderProfileInput {
            slug: "post-migration".to_string(),
            profile_json: serde_json::to_string(&profile).unwrap(),
            source: None,
        })
        .await
        .unwrap();

    assert!(store
        .get_provider_profile("post-migration")
        .await
        .unwrap()
        .is_some());

    let _ = std::fs::remove_file(&db_path);
}

// ============================================================================
// Task 2.5: Import/export/validation tests
// ============================================================================

#[tokio::test]
async fn import_valid_provider_profile() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let profile = ProviderProfile::new("imported", ApiFamily::Messages, "https://api.imported.com")
        .with_provider_guidance("Custom guidance for imported provider.");
    let json = serde_json::to_string(&profile).unwrap();

    let slug = import_provider_profile(&store, &json, Some("test-import"))
        .await
        .unwrap();
    assert_eq!(slug, "imported");

    let record = store
        .get_provider_profile("imported")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.source.as_deref(), Some("test-import"));
}

#[tokio::test]
async fn import_invalid_json_rejected() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let result = import_provider_profile(&store, "{not valid json}", None).await;
    assert!(result.is_err());
    assert!(store.list_provider_profiles().await.unwrap().is_empty());
}

#[tokio::test]
async fn import_credential_material_rejected() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let json = r#"{
        "slug": "evil",
        "family": "Completions",
        "base_url": "https://evil.example.com",
        "credential_auth": [],
        "default_headers": {},
        "purpose": "General",
        "quirks": {},
        "api_key": "sk-secret-key"
    }"#;

    let result = import_provider_profile(&store, json, None).await;
    assert!(result.is_err());
    assert!(store.list_provider_profiles().await.unwrap().is_empty());
}

#[tokio::test]
async fn import_override_builtin() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let profile = ProviderProfile::new(
        "openai",
        ApiFamily::Messages,
        "https://custom.openai.example",
    )
    .with_provider_guidance("Custom override guidance.");
    let json = serde_json::to_string(&profile).unwrap();

    let slug = import_provider_profile(&store, &json, None).await.unwrap();
    assert_eq!(slug, "openai");

    let registry = build_effective_registry(&store).await.unwrap();
    let fragment = registry.system_prompt_fragment("openai").unwrap();
    assert_eq!(fragment, "Custom override guidance.");
}

#[tokio::test]
async fn import_custom_profile_resolved_by_effective_registry() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let profile = ProviderProfile::new(
        "acme-corp",
        ApiFamily::Completions,
        "https://api.acme.corp/v1",
    )
    .with_provider_guidance("ACME Corp specific instructions.");
    let json = serde_json::to_string(&profile).unwrap();
    import_provider_profile(&store, &json, None).await.unwrap();

    let registry = build_effective_registry(&store).await.unwrap();
    assert!(registry.slugs().contains(&"acme-corp"));
}

#[tokio::test]
async fn export_profile_roundtrip() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let profile = ProviderProfile::new(
        "export-test",
        ApiFamily::Completions,
        "https://export.example.com",
    );
    let json = serde_json::to_string(&profile).unwrap();
    import_provider_profile(&store, &json, None).await.unwrap();

    let exported = export_provider_profile(&store, "export-test")
        .await
        .unwrap();
    assert!(exported.is_some());

    let exported_json = exported.unwrap();
    let parsed: ProviderProfile = serde_json::from_str(&exported_json).unwrap();
    assert_eq!(parsed.slug, "export-test");
    assert_eq!(parsed.base_url, "https://export.example.com");
}

#[tokio::test]
async fn export_nonexistent_returns_none() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let result = export_provider_profile(&store, "nonexistent")
        .await
        .unwrap();
    assert!(result.is_none());
}

#[test]
fn validate_profile_rejects_empty_slug() {
    let json = r#"{"slug":"","family":"Completions","base_url":"https://example.com","credential_auth":[],"default_headers":{},"purpose":"General","quirks":{}}"#;
    assert!(validate_provider_profile(json).is_err());
}

#[test]
fn validate_profile_accepts_credential_auth_metadata() {
    let profile = ProviderProfile::new("test", ApiFamily::Completions, "https://example.com")
        .with_auth(AuthStrategy::BearerToken);
    let json = serde_json::to_string(&profile).unwrap();
    let validated = validate_provider_profile(&json).unwrap();
    assert!(validated.supports_credential(iron_providers::CredentialKind::ApiKey));
}

// ============================================================================
// Task 3.5: Effective registry tests
// ============================================================================

#[tokio::test]
async fn effective_registry_empty_store_preserves_builtins() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let registry = build_effective_registry(&store).await.unwrap();
    assert!(registry.slugs().contains(&"openai"));
    assert!(registry.slugs().contains(&"anthropic"));
}

#[tokio::test]
async fn effective_registry_custom_added() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let profile = ProviderProfile::new(
        "custom-slug",
        ApiFamily::Completions,
        "https://custom.example.com",
    );
    let json = serde_json::to_string(&profile).unwrap();
    import_provider_profile(&store, &json, None).await.unwrap();

    let registry = build_effective_registry(&store).await.unwrap();
    assert!(registry.slugs().contains(&"custom-slug"));
}

#[tokio::test]
async fn effective_registry_override_replaces_builtin_guidance() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let profile = ProviderProfile::new("openai", ApiFamily::Responses, "https://api.openai.com/v1")
        .with_provider_guidance("Override guidance for OpenAI.");
    let json = serde_json::to_string(&profile).unwrap();
    import_provider_profile(&store, &json, None).await.unwrap();

    let registry = build_effective_registry(&store).await.unwrap();
    let fragment = registry.system_prompt_fragment("openai").unwrap();
    assert_eq!(fragment, "Override guidance for OpenAI.");
}

#[tokio::test]
async fn effective_registry_other_builtins_preserved_after_override() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let profile = ProviderProfile::new(
        "openai",
        ApiFamily::Responses,
        "https://custom.openai.example",
    );
    let json = serde_json::to_string(&profile).unwrap();
    import_provider_profile(&store, &json, None).await.unwrap();

    let registry = build_effective_registry(&store).await.unwrap();
    assert!(registry.slugs().contains(&"openai"));
    assert!(registry.slugs().contains(&"anthropic"));
    assert!(registry.slugs().contains(&"kimi"));
}

#[tokio::test]
async fn effective_registry_invalid_profile_skipped() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    // Insert invalid JSON directly via SQL (bypassing set_provider_profile validation)
    sqlx::query(
        "INSERT INTO provider_profiles (slug, profile_json, source, created_at, updated_at) VALUES (?, ?, NULL, ?, ?)",
    )
    .bind("bad")
    .bind("{invalid}")
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(store.pool())
    .await
    .unwrap();

    let registry = build_effective_registry(&store).await.unwrap();
    assert!(!registry.slugs().contains(&"bad"));
    assert!(registry.slugs().contains(&"openai"));
}

// ============================================================================
// Task 4.5: Known provider slug discovery tests
// ============================================================================

#[tokio::test]
async fn known_provider_slugs_includes_persisted_profiles() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let profile = ProviderProfile::new(
        "custom-provider",
        ApiFamily::Completions,
        "https://custom.example.com",
    );
    import_provider_profile(&store, &serde_json::to_string(&profile).unwrap(), None)
        .await
        .unwrap();

    let slugs = store.known_provider_slugs().await.unwrap();
    assert!(slugs.contains("custom-provider"));
    assert!(slugs.contains("openai"));
}

// ============================================================================
// Task 5.4: Credential support from profile metadata tests
// ============================================================================

#[tokio::test]
async fn credential_support_derived_from_custom_profile() {
    use iron_core::provider_credential::domain::{ProviderPromptContext, ProviderSlug};
    use iron_core::provider_credential::store::InMemoryCredentialStore;
    use iron_core::provider_credential::CredentialResolver;

    let store = InMemoryCredentialStore::new();
    let resolver = CredentialResolver::new(
        std::sync::Arc::new(store) as iron_core::provider_credential::DynCredentialStore
    );

    // Custom profile with only API-key support
    let profiles = vec![
        ProviderProfile::new(
            "api-only",
            ApiFamily::Completions,
            "https://api.example.com",
        )
        .with_auth(AuthStrategy::BearerToken),
        ProviderProfile::new(
            "no-auth-provider",
            ApiFamily::Completions,
            "https://local.example.com",
        )
        .with_credential_auth(iron_providers::CredentialKind::NoAuth, AuthStrategy::NoAuth),
    ];

    resolver.merge_support_from_profiles(&profiles);

    // API-key profile: resolve with API key should succeed
    let api_only = resolver
        .resolve(
            &ProviderPromptContext {
                provider_slug: ProviderSlug::new("api-only"),
                model: "test-model".to_string(),
                api_key: Some("sk-test".to_string()),
            },
            Some("sk-test".to_string()),
        )
        .await;
    assert!(api_only.is_ok());
}

#[tokio::test]
async fn credential_support_api_key_profile() {
    use iron_core::provider_credential::domain::ProviderSlug;
    use iron_core::provider_credential::store::InMemoryCredentialStore;
    use iron_core::provider_credential::CredentialResolver;

    let cred_store = std::sync::Arc::new(InMemoryCredentialStore::new())
        as iron_core::provider_credential::DynCredentialStore;
    let resolver = CredentialResolver::new(cred_store.clone());

    let profiles = vec![ProviderProfile::new(
        "custom-key",
        ApiFamily::Completions,
        "https://api.example.com",
    )
    .with_auth(AuthStrategy::BearerToken)];
    resolver.merge_support_from_profiles(&profiles);

    // API key should be accepted for a profile that supports it
    let result = resolver
        .resolve(
            &iron_core::provider_credential::domain::ProviderPromptContext {
                provider_slug: ProviderSlug::new("custom-key"),
                model: "test-model".to_string(),
                api_key: Some("sk-test".to_string()),
            },
            Some("sk-test".to_string()),
        )
        .await;

    assert!(result.is_ok());
}

// ============================================================================
// Task 5.5: Model catalog validation tests
// ============================================================================

#[tokio::test]
async fn custom_model_validates_against_custom_provider_profile() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    // First, import a custom provider profile
    let profile = ProviderProfile::new(
        "custom-ai",
        ApiFamily::Completions,
        "https://api.custom-ai.com",
    );
    import_provider_profile(&store, &serde_json::to_string(&profile).unwrap(), None)
        .await
        .unwrap();

    // Then, try to add a custom model for that provider
    let result = store
        .set_custom_model(&iron_core::config::CustomModelInput {
            provider_slug: "custom-ai".to_string(),
            model_id: "custom-model-v1".to_string(),
            display_name: "Custom Model V1".to_string(),
            context_window: Some(128000),
            output_limit: Some(4096),
            supports_tool_calls: true,
            supports_reasoning: false,
            supports_vision: false,
            supports_streaming: true,
            reasoning_effort_values: vec![],
            cost_input_per_million: Some(0.01),
            cost_output_per_million: Some(0.03),
        })
        .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn custom_model_rejects_unknown_provider_without_profile() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let result = store
        .set_custom_model(&iron_core::config::CustomModelInput {
            provider_slug: "completely-unknown".to_string(),
            model_id: "test-model".to_string(),
            display_name: "Test".to_string(),
            context_window: None,
            output_limit: None,
            supports_tool_calls: true,
            supports_reasoning: false,
            supports_vision: false,
            supports_streaming: true,
            reasoning_effort_values: vec![],
            cost_input_per_million: None,
            cost_output_per_million: None,
        })
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn custom_model_builtin_provider_still_valid() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let result = store
        .set_custom_model(&iron_core::config::CustomModelInput {
            provider_slug: "openai".to_string(),
            model_id: "custom-gpt".to_string(),
            display_name: "Custom GPT".to_string(),
            context_window: None,
            output_limit: None,
            supports_tool_calls: true,
            supports_reasoning: false,
            supports_vision: false,
            supports_streaming: true,
            reasoning_effort_values: vec![],
            cost_input_per_million: None,
            cost_output_per_million: None,
        })
        .await;

    assert!(result.is_ok());
}
