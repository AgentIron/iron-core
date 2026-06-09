use iron_core::config::crypto::XChaCha20Poly1305Cipher;
use iron_core::config::{
    default_config_path, ConfigStore, OpenOptions, ProfileInput, PromptInput, ScheduleInput,
};
use iron_core::provider_credential::{
    domain::{OAuthTokenSet, ProviderPromptContext, ProviderSlug, StoredCredential},
    CredentialResolver, DurableCredentialStore, ProviderCredentialStore,
};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Helper to create a test cipher for file-based stores.
fn test_cipher() -> std::sync::Arc<dyn iron_core::config::crypto::CredentialCipher> {
    let key = XChaCha20Poly1305Cipher::generate_key();
    std::sync::Arc::new(XChaCha20Poly1305Cipher::new(&key))
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

#[tokio::test]
async fn test_config_store_in_memory() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    // Test profile CRUD
    let profile = ProfileInput {
        id: "test-profile".to_string(),
        schema_version: 1,
        payload: json!({"name": "Test Profile"}),
    };

    store.set_profile(&profile).await.unwrap();

    let retrieved = store.get_profile("test-profile").await.unwrap();
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.id, "test-profile");
    assert_eq!(retrieved.schema_version, 1);
    assert_eq!(retrieved.payload, json!({"name": "Test Profile"}));

    // Test profile update
    let updated_profile = ProfileInput {
        id: "test-profile".to_string(),
        schema_version: 2,
        payload: json!({"name": "Updated Profile"}),
    };
    store.set_profile(&updated_profile).await.unwrap();

    let retrieved = store.get_profile("test-profile").await.unwrap().unwrap();
    assert_eq!(retrieved.schema_version, 2);
    assert_eq!(retrieved.payload, json!({"name": "Updated Profile"}));

    // Test profile list
    let ids = store.list_profile_ids().await.unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0], "test-profile");

    // Test profile delete
    store.delete_profile("test-profile").await.unwrap();
    let retrieved = store.get_profile("test-profile").await.unwrap();
    assert!(retrieved.is_none());
}

#[tokio::test]
async fn test_prompt_crud() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let prompt = PromptInput {
        id: "test-prompt".to_string(),
        schema_version: 1,
        payload: json!({"template": "Hello {{name}}"}),
    };

    store.set_prompt(&prompt).await.unwrap();

    let retrieved = store.get_prompt("test-prompt").await.unwrap().unwrap();
    assert_eq!(retrieved.id, "test-prompt");
    assert_eq!(retrieved.payload, json!({"template": "Hello {{name}}"}));

    store.delete_prompt("test-prompt").await.unwrap();
    assert!(store.get_prompt("test-prompt").await.unwrap().is_none());
}

#[tokio::test]
async fn test_schedule_crud() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let schedule = ScheduleInput {
        id: "test-schedule".to_string(),
        schema_version: 1,
        payload: json!({"cron": "0 0 * * *"}),
    };

    store.set_schedule(&schedule).await.unwrap();

    let retrieved = store.get_schedule("test-schedule").await.unwrap().unwrap();
    assert_eq!(retrieved.id, "test-schedule");
    assert_eq!(retrieved.payload, json!({"cron": "0 0 * * *"}));

    store.delete_schedule("test-schedule").await.unwrap();
    assert!(store.get_schedule("test-schedule").await.unwrap().is_none());
}

#[tokio::test]
async fn test_credential_storage() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    // Store a credential
    let payload = b"test-api-key";
    store
        .set_credential("openai", "api_key", payload)
        .await
        .unwrap();

    // Retrieve and verify
    let retrieved = store.get_credential("openai").await.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap(), payload);

    // Replace with OAuth
    let oauth_payload = b"oauth-token-data";
    store
        .set_credential("openai", "oauth_bearer", oauth_payload)
        .await
        .unwrap();

    let retrieved = store.get_credential("openai").await.unwrap().unwrap();
    assert_eq!(retrieved, oauth_payload);

    // Check metadata
    let metadata = store
        .get_credential_metadata("openai")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(metadata.provider_slug, "openai");
    assert_eq!(metadata.credential_mode, "oauth_bearer");

    // List credentials
    let slugs = store.list_credential_slugs().await.unwrap();
    assert_eq!(slugs.len(), 1);
    assert_eq!(slugs[0], "openai");

    // Remove credential
    store.remove_credential("openai").await.unwrap();
    assert!(store.get_credential("openai").await.unwrap().is_none());
}

#[tokio::test]
async fn test_not_found_returns_none() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    assert!(store.get_profile("nonexistent").await.unwrap().is_none());
    assert!(store.get_prompt("nonexistent").await.unwrap().is_none());
    assert!(store.get_schedule("nonexistent").await.unwrap().is_none());
    assert!(store.get_credential("nonexistent").await.unwrap().is_none());
}

#[tokio::test]
async fn test_migrations_applied() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    // Verify tables exist by performing operations
    let profile = ProfileInput {
        id: "migration-test".to_string(),
        schema_version: 1,
        payload: json!(null),
    };
    store.set_profile(&profile).await.unwrap();
    assert!(store.get_profile("migration-test").await.unwrap().is_some());
}

// ============================================================================
// ID Validation Tests (Blocker 2)
// ============================================================================

#[tokio::test]
async fn test_empty_profile_id_rejected() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let profile = ProfileInput {
        id: "".to_string(),
        schema_version: 1,
        payload: json!({"name": "test"}),
    };
    let err = store.set_profile(&profile).await.unwrap_err();
    assert!(
        matches!(err, iron_core::config::ConfigError::Validation(ref msg) if msg.contains("Profile ID"))
    );
}

#[tokio::test]
async fn test_empty_prompt_id_rejected() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let prompt = PromptInput {
        id: "".to_string(),
        schema_version: 1,
        payload: json!({"template": "test"}),
    };
    let err = store.set_prompt(&prompt).await.unwrap_err();
    assert!(
        matches!(err, iron_core::config::ConfigError::Validation(ref msg) if msg.contains("Prompt ID"))
    );
}

#[tokio::test]
async fn test_empty_schedule_id_rejected() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let schedule = ScheduleInput {
        id: "".to_string(),
        schema_version: 1,
        payload: json!({"cron": "0 0 * * *"}),
    };
    let err = store.set_schedule(&schedule).await.unwrap_err();
    assert!(
        matches!(err, iron_core::config::ConfigError::Validation(ref msg) if msg.contains("Schedule ID"))
    );
}

// ============================================================================
// Concurrency and Process Safety Tests (Tasks 6.1-6.4)
// ============================================================================

#[tokio::test]
async fn test_two_instances_same_database_path() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_concurrent.db");

    // Open two independent ConfigStore instances pointing to the same database
    let store1 = open_test_store(&db_path).await;
    let store2 = open_test_store(&db_path).await;

    // Write from store1
    let profile = ProfileInput {
        id: "concurrent-profile".to_string(),
        schema_version: 1,
        payload: json!({"name": "From Store 1"}),
    };
    store1.set_profile(&profile).await.unwrap();

    // Read from store2
    let retrieved = store2.get_profile("concurrent-profile").await.unwrap();
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.payload, json!({"name": "From Store 1"}));

    // Update from store2
    let updated = ProfileInput {
        id: "concurrent-profile".to_string(),
        schema_version: 2,
        payload: json!({"name": "From Store 2"}),
    };
    store2.set_profile(&updated).await.unwrap();

    // Read updated from store1
    let retrieved = store1
        .get_profile("concurrent-profile")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retrieved.schema_version, 2);
    assert_eq!(retrieved.payload, json!({"name": "From Store 2"}));
}

#[tokio::test]
async fn test_concurrent_reads_arc_shared() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    // Seed data
    for i in 0..10 {
        let profile = ProfileInput {
            id: format!("profile-{}", i),
            schema_version: 1,
            payload: json!({"index": i}),
        };
        store.set_profile(&profile).await.unwrap();
    }

    // Run multiple concurrent reads
    let store = Arc::new(store);
    let futures: Vec<_> = (0..10)
        .map(|i| {
            let store = Arc::clone(&store);
            async move {
                let id = format!("profile-{}", i);
                let result = store.get_profile(&id).await.unwrap();
                assert!(result.is_some());
                let result = result.unwrap();
                assert_eq!(result.payload, json!({"index": i}));
            }
        })
        .collect();

    futures::future::join_all(futures).await;
}

#[tokio::test]
async fn test_concurrent_writes_serialized() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_serial_writes.db");

    let store1 = Arc::new(open_test_store(&db_path).await);
    let store2 = Arc::new(open_test_store(&db_path).await);

    // Run concurrent writes from store1
    let futures1: Vec<_> = (0..5)
        .map(|i| {
            let store = Arc::clone(&store1);
            async move {
                let profile = ProfileInput {
                    id: format!("write-test-{}", i),
                    schema_version: 1,
                    payload: json!({"source": "store1", "index": i}),
                };
                store.set_profile(&profile).await.unwrap();
            }
        })
        .collect();

    // Run concurrent writes from store2
    let futures2: Vec<_> = (0..5)
        .map(|i| {
            let store = Arc::clone(&store2);
            async move {
                let profile = ProfileInput {
                    id: format!("write-test-{}", i + 5),
                    schema_version: 1,
                    payload: json!({"source": "store2", "index": i + 5}),
                };
                store.set_profile(&profile).await.unwrap();
            }
        })
        .collect();

    // Run both groups concurrently
    let (_results1, _results2) = tokio::join!(
        futures::future::join_all(futures1),
        futures::future::join_all(futures2),
    );

    // Verify all writes succeeded
    let ids = store1.list_profile_ids().await.unwrap();
    assert_eq!(ids.len(), 10);
}

#[tokio::test]
async fn test_concurrent_credential_replacement() {
    let store = Arc::new(ConfigStore::open_in_memory().await.unwrap());

    // Store initial credential
    store
        .set_credential("openai", "api_key", b"initial-key")
        .await
        .unwrap();

    // Concurrent replacements
    let futures: Vec<_> = (0..5)
        .map(|i| {
            let store = Arc::clone(&store);
            async move {
                let payload = format!("key-{}", i);
                store
                    .set_credential("openai", "api_key", payload.as_bytes())
                    .await
                    .unwrap();
            }
        })
        .collect();

    futures::future::join_all(futures).await;

    // Verify exactly one credential remains
    let slugs = store.list_credential_slugs().await.unwrap();
    assert_eq!(slugs.len(), 1);
    assert_eq!(slugs[0], "openai");

    // Verify it's one of the keys we set (last one wins due to serialization)
    let retrieved = store.get_credential("openai").await.unwrap().unwrap();
    assert!(retrieved.starts_with(b"key-"));
}

#[tokio::test]
async fn test_busy_timeout_on_write_contention() {
    use iron_core::config::ConfigError;
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_busy_timeout.db");

    // Open first store with normal timeout
    let store1 = open_test_store(&db_path).await;

    // Acquire a connection from store1 and start an exclusive transaction
    // to hold the write lock.
    let mut conn = store1.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *conn)
        .await
        .unwrap();

    // Open second store with a zero busy timeout so it fails immediately
    // when the write lock is held.
    let options = OpenOptions {
        cipher: Some(test_cipher()),
        busy_timeout: Some(Duration::from_millis(0)),
    };
    let store2 = ConfigStore::open_at_with_options(&db_path, options)
        .await
        .unwrap();

    // Attempting to write from store2 while store1 holds the lock
    // should produce a busy-timeout error.
    let profile = ProfileInput {
        id: "busy-test".to_string(),
        schema_version: 1,
        payload: json!({"test": true}),
    };
    let err = store2.set_profile(&profile).await.unwrap_err();
    assert!(
        matches!(err, ConfigError::BusyTimeout(_)),
        "Expected ConfigError::BusyTimeout, got: {:?}",
        err
    );

    // Release the lock by ending the transaction and dropping the connection.
    sqlx::query("ROLLBACK").execute(&mut *conn).await.unwrap();
    drop(conn);

    // After releasing the lock, the write should succeed
    store2.set_profile(&profile).await.unwrap();
}

// ============================================================================
// Provider Credential Resolver Integration Tests (Task 8.2)
// ============================================================================

#[tokio::test]
async fn test_durable_store_with_resolver_api_key() {
    let config_store = ConfigStore::open_in_memory().await.unwrap();
    let durable = Arc::new(DurableCredentialStore::new(config_store));
    let store: Arc<dyn ProviderCredentialStore> = durable.clone();

    let slug = ProviderSlug::new("openai");
    store
        .set(&slug, StoredCredential::ApiKey("sk-test".into()))
        .await;

    let resolver = CredentialResolver::with_fallible_store(store, durable);

    let ctx = ProviderPromptContext {
        provider_slug: slug.clone(),
        model: "gpt-4o".into(),
        api_key: None,
    };

    let result = resolver.resolve(&ctx, None).await.unwrap();
    assert_eq!(
        result.mode,
        iron_core::provider_credential::domain::CredentialMode::ApiKey
    );
    assert!(!result.was_refreshed);
}

#[tokio::test]
async fn test_durable_store_with_resolver_oauth() {
    let config_store = ConfigStore::open_in_memory().await.unwrap();
    let durable = Arc::new(DurableCredentialStore::new(config_store));
    let store: Arc<dyn ProviderCredentialStore> = durable.clone();

    let tokens = OAuthTokenSet {
        access_token: "at".into(),
        refresh_token: "rt".into(),
        expires_at: Some(SystemTime::now() + Duration::from_secs(3600)),
        id_token: None,
    };

    let slug = ProviderSlug::new("codex");
    store
        .set(&slug, StoredCredential::OAuthBearer(tokens))
        .await;

    let resolver = CredentialResolver::with_fallible_store(store, durable);

    let ctx = ProviderPromptContext {
        provider_slug: slug.clone(),
        model: "codex-model".into(),
        api_key: None,
    };

    let result = resolver.resolve(&ctx, None).await.unwrap();
    assert_eq!(
        result.mode,
        iron_core::provider_credential::domain::CredentialMode::OAuthBearer
    );
    assert!(!result.was_refreshed);
}

#[tokio::test]
async fn test_durable_store_with_resolver_missing() {
    let config_store = ConfigStore::open_in_memory().await.unwrap();
    let durable = Arc::new(DurableCredentialStore::new(config_store));
    let store: Arc<dyn ProviderCredentialStore> = durable.clone();

    let resolver = CredentialResolver::with_fallible_store(store, durable);

    let ctx = ProviderPromptContext {
        provider_slug: ProviderSlug::new("openai"),
        model: "gpt-4o".into(),
        api_key: None,
    };

    let result = resolver.resolve(&ctx, None).await;
    assert!(
        matches!(result, Err(iron_core::provider_credential::domain::ProviderAuthError::NotConfigured(ref s)) if s == "openai")
    );
}

#[tokio::test]
async fn test_durable_store_with_resolver_api_key_precedence() {
    let config_store = ConfigStore::open_in_memory().await.unwrap();
    let durable = Arc::new(DurableCredentialStore::new(config_store));
    let store: Arc<dyn ProviderCredentialStore> = durable.clone();

    let tokens = OAuthTokenSet {
        access_token: "at".into(),
        refresh_token: "rt".into(),
        expires_at: Some(SystemTime::now() + Duration::from_secs(3600)),
        id_token: None,
    };

    let slug = ProviderSlug::new("kimi-code");
    store
        .set(&slug, StoredCredential::OAuthBearer(tokens))
        .await;

    let resolver = CredentialResolver::with_fallible_store(store, durable);

    let ctx = ProviderPromptContext {
        provider_slug: slug.clone(),
        model: "kimi-model".into(),
        api_key: None,
    };

    let result = resolver
        .resolve(&ctx, Some("explicit-key".into()))
        .await
        .unwrap();
    assert_eq!(
        result.mode,
        iron_core::provider_credential::domain::CredentialMode::ApiKey
    );
    assert_eq!(
        result.provider_credential,
        iron_providers::ProviderCredential::ApiKey("explicit-key".into())
    );
}

#[tokio::test]
async fn test_durable_store_status() {
    let config_store = ConfigStore::open_in_memory().await.unwrap();
    let durable = Arc::new(DurableCredentialStore::new(config_store));
    let store: Arc<dyn ProviderCredentialStore> = durable.clone();

    let slug = ProviderSlug::new("openai");
    store
        .set(&slug, StoredCredential::ApiKey("sk-test".into()))
        .await;

    let resolver = CredentialResolver::with_fallible_store(store, durable);

    let status = resolver.status(&slug, None).await;
    assert!(matches!(
        status,
        iron_core::provider_credential::domain::ProviderAuthStatus::ConfiguredApiKey
    ));
}

#[tokio::test]
async fn test_durable_store_disconnect_oauth() {
    let config_store = ConfigStore::open_in_memory().await.unwrap();
    let durable = Arc::new(DurableCredentialStore::new(config_store));
    let store: Arc<dyn ProviderCredentialStore> = durable.clone();

    let tokens = OAuthTokenSet {
        access_token: "at".into(),
        refresh_token: "rt".into(),
        expires_at: Some(SystemTime::now() + Duration::from_secs(3600)),
        id_token: None,
    };

    let slug = ProviderSlug::new("codex");
    store
        .set(&slug, StoredCredential::OAuthBearer(tokens))
        .await;

    let resolver = CredentialResolver::with_fallible_store(store, durable);

    resolver.disconnect_oauth(&slug).await;

    let status = resolver.status(&slug, None).await;
    assert!(matches!(
        status,
        iron_core::provider_credential::domain::ProviderAuthStatus::NotConfigured
    ));
}

// ============================================================================
// Runtime Settings Tests (Issue #67)
// ============================================================================

use iron_core::config::{
    build_effective_catalog, builtin_model_catalog, CatalogError, CustomModelInput,
    CustomModelRecord, DefaultModelInput, McpServerConfigInput, ProviderConfigInput,
    SkillSettingsInput,
};
use iron_core::mcp::server::{HttpConfig, McpTransport};
use std::collections::HashMap;

#[tokio::test]
async fn test_provider_config_crud() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let input = ProviderConfigInput {
        provider_slug: "openai".to_string(),
        display_name: "OpenAI".to_string(),
        enabled: true,
        base_url: Some("https://api.openai.com".to_string()),
    };

    let record = store.set_provider_config(&input).await.unwrap();
    assert_eq!(record.provider_slug, "openai");
    assert_eq!(record.display_name, "OpenAI");
    assert!(record.enabled);

    let retrieved = store.get_provider_config("openai").await.unwrap().unwrap();
    assert_eq!(retrieved.display_name, "OpenAI");

    let list = store.list_provider_configs().await.unwrap();
    assert_eq!(list.len(), 1);

    store.remove_provider_config("openai").await.unwrap();
    assert!(store.get_provider_config("openai").await.unwrap().is_none());
}

#[tokio::test]
async fn test_provider_config_excludes_credentials() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    // Verify provider config API does not accept credential fields
    let input = ProviderConfigInput {
        provider_slug: "openai".to_string(),
        display_name: "OpenAI".to_string(),
        enabled: true,
        base_url: None,
    };

    let record = store.set_provider_config(&input).await.unwrap();
    // Provider config record should not have api_key or credential fields
    assert_eq!(record.provider_slug, "openai");
}

#[tokio::test]
async fn test_custom_model_crud() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let input = CustomModelInput {
        provider_slug: "openai".to_string(),
        model_id: "gpt-4o-custom".to_string(),
        display_name: "Custom GPT-4o".to_string(),
        context_window: Some(128000),
        output_limit: Some(4096),
        supports_tool_calls: true,
        supports_reasoning: false,
        supports_vision: true,
        supports_streaming: true,
        reasoning_effort_values: vec![],
        cost_input_per_million: Some(5.0),
        cost_output_per_million: Some(15.0),
    };

    let record = store.set_custom_model(&input).await.unwrap();
    assert_eq!(record.model_id, "gpt-4o-custom");

    let retrieved = store
        .get_custom_model("openai", "gpt-4o-custom")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retrieved.display_name, "Custom GPT-4o");
    assert_eq!(retrieved.context_window, Some(128000));

    let list = store.list_custom_models(None).await.unwrap();
    assert_eq!(list.len(), 1);

    let by_provider = store.list_custom_models(Some("openai")).await.unwrap();
    assert_eq!(by_provider.len(), 1);

    store
        .remove_custom_model("openai", "gpt-4o-custom")
        .await
        .unwrap();
    assert!(store
        .get_custom_model("openai", "gpt-4o-custom")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn test_default_model_crud() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let input = DefaultModelInput {
        provider_slug: "openai".to_string(),
        model_id: "gpt-4o".to_string(),
    };

    let record = store.set_default_model(&input).await.unwrap();
    assert_eq!(record.provider_slug, "openai");
    assert_eq!(record.model_id, "gpt-4o");

    let retrieved = store.get_default_model().await.unwrap().unwrap();
    assert_eq!(retrieved.provider_slug, "openai");
    assert_eq!(retrieved.model_id, "gpt-4o");

    store.clear_default_model().await.unwrap();
    assert!(store.get_default_model().await.unwrap().is_none());
}

#[tokio::test]
async fn test_mcp_server_stdio_crud() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let mut env = HashMap::new();
    env.insert("KEY".to_string(), "value".to_string());

    let input = McpServerConfigInput {
        id: "filesystem".to_string(),
        label: "Filesystem".to_string(),
        description: Some("File system access".to_string()),
        transport: McpTransport::Stdio {
            command: "npx".to_string(),
            args: vec!["-y", "@modelcontextprotocol/server-filesystem"]
                .into_iter()
                .map(String::from)
                .collect(),
            env,
        },
        working_dir: Some(std::path::PathBuf::from("/tmp")),
        enabled_by_default: true,
        inherited_env_vars: vec!["HOME".to_string()],
    };

    let record = store.set_mcp_server(&input).await.unwrap();
    assert_eq!(record.id, "filesystem");
    assert_eq!(record.inherited_env_vars, vec!["HOME"]);

    let retrieved = store.get_mcp_server("filesystem").await.unwrap().unwrap();
    assert_eq!(retrieved.label, "Filesystem");
    assert_eq!(retrieved.inherited_env_vars, vec!["HOME"]);

    let list = store.list_mcp_servers().await.unwrap();
    assert_eq!(list.len(), 1);

    store.remove_mcp_server("filesystem").await.unwrap();
    assert!(store.get_mcp_server("filesystem").await.unwrap().is_none());
}

#[tokio::test]
async fn test_mcp_server_http_crud() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let input = McpServerConfigInput {
        id: "remote".to_string(),
        label: "Remote".to_string(),
        description: None,
        transport: McpTransport::Http {
            config: HttpConfig {
                url: "https://example.com/mcp".to_string(),
                headers: Some(
                    [("Authorization".to_string(), "Bearer token".to_string())]
                        .into_iter()
                        .collect(),
                ),
            },
        },
        working_dir: None,
        enabled_by_default: false,
        inherited_env_vars: vec![],
    };

    let record = store.set_mcp_server(&input).await.unwrap();
    assert_eq!(record.id, "remote");

    let retrieved = store.get_mcp_server("remote").await.unwrap().unwrap();
    match retrieved.transport {
        McpTransport::Http { config } => {
            assert_eq!(config.url, "https://example.com/mcp");
        }
        _ => panic!("Expected HTTP transport"),
    }
}

#[tokio::test]
async fn test_skill_settings_defaults() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    // Defaults when not set
    let settings = store.get_skill_settings().await.unwrap();
    assert!(!settings.trust_project_skills);
    assert!(settings.additional_skill_dirs.is_empty());

    let input = SkillSettingsInput {
        trust_project_skills: true,
        additional_skill_dirs: vec![std::path::PathBuf::from("/custom/skills")],
    };

    let record = store.set_skill_settings(&input).await.unwrap();
    assert!(record.trust_project_skills);
    assert_eq!(record.additional_skill_dirs.len(), 1);

    let retrieved = store.get_skill_settings().await.unwrap();
    assert!(retrieved.trust_project_skills);
}

#[tokio::test]
async fn test_runtime_settings_snapshot() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    // Set up some runtime settings
    store
        .set_provider_config(&ProviderConfigInput {
            provider_slug: "openai".to_string(),
            display_name: "OpenAI".to_string(),
            enabled: true,
            base_url: None,
        })
        .await
        .unwrap();

    store
        .set_default_model(&DefaultModelInput {
            provider_slug: "openai".to_string(),
            model_id: "gpt-4o".to_string(),
        })
        .await
        .unwrap();

    let snapshot = store.load_runtime_settings().await.unwrap();
    assert_eq!(snapshot.provider_configs.len(), 1);
    assert!(snapshot.default_model.is_some());
    assert!(!snapshot.skill_settings.trust_project_skills);
}

#[tokio::test]
async fn test_inherited_env_vars_validation() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let input = McpServerConfigInput {
        id: "bad-env".to_string(),
        label: "Bad Env".to_string(),
        description: None,
        transport: McpTransport::Stdio {
            command: "echo".to_string(),
            args: vec![],
            env: HashMap::new(),
        },
        working_dir: None,
        enabled_by_default: true,
        inherited_env_vars: vec!["KEY=VALUE".to_string()],
    };

    let result = store.set_mcp_server(&input).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_custom_model_numeric_validation() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let input = CustomModelInput {
        provider_slug: "openai".to_string(),
        model_id: "bad-cost".to_string(),
        display_name: "Bad Cost".to_string(),
        context_window: Some(0),
        output_limit: Some(4096),
        supports_tool_calls: false,
        supports_reasoning: false,
        supports_vision: false,
        supports_streaming: true,
        reasoning_effort_values: vec![],
        cost_input_per_million: Some(-1.0),
        cost_output_per_million: Some(0.0),
    };

    let result = store.set_custom_model(&input).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_mcp_server_transport_validation() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let input = McpServerConfigInput {
        id: "bad-stdio".to_string(),
        label: "Bad Stdio".to_string(),
        description: None,
        transport: McpTransport::Stdio {
            command: "".to_string(),
            args: vec![],
            env: HashMap::new(),
        },
        working_dir: None,
        enabled_by_default: true,
        inherited_env_vars: vec![],
    };

    let result = store.set_mcp_server(&input).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_skill_settings_reject_empty_additional_dir() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let result = store
        .set_skill_settings(&SkillSettingsInput {
            trust_project_skills: false,
            additional_skill_dirs: vec![std::path::PathBuf::new()],
        })
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_migrations_v1_to_v2() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("v1-config.db");
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
        CREATE TABLE schema_version (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            version INTEGER NOT NULL
        );

        CREATE TABLE profiles (
            id TEXT PRIMARY KEY,
            schema_version INTEGER NOT NULL,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE prompts (
            id TEXT PRIMARY KEY,
            schema_version INTEGER NOT NULL,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE schedule (
            id TEXT PRIMARY KEY,
            schema_version INTEGER NOT NULL,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE credentials (
            provider_slug TEXT PRIMARY KEY,
            credential_mode TEXT NOT NULL,
            encrypted_payload BLOB NOT NULL,
            nonce BLOB NOT NULL,
            encryption_metadata TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        INSERT INTO schema_version (id, version) VALUES (1, 1);
        INSERT INTO profiles (id, schema_version, payload, created_at, updated_at)
        VALUES ('legacy-profile', 1, '{"name":"Legacy"}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();
    drop(pool);

    let store = open_test_store(&db_path).await;

    let legacy_profile = store.get_profile("legacy-profile").await.unwrap().unwrap();
    assert_eq!(legacy_profile.payload, json!({"name": "Legacy"}));

    let mut conn = store.acquire().await.unwrap();
    let version: i64 = sqlx::query_scalar("SELECT version FROM schema_version WHERE id = 1")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(version, 3);

    // Verify v2/v3 tables exist by using the new APIs
    store
        .set_provider_config(&ProviderConfigInput {
            provider_slug: "test".to_string(),
            display_name: "Test".to_string(),
            enabled: true,
            base_url: None,
        })
        .await
        .unwrap();

    let retrieved = store.get_provider_config("test").await.unwrap();
    assert!(retrieved.is_some());
}

// ============================================================================
// XDG Path Verification Tests (Task 8.6)
// ============================================================================

#[cfg(target_os = "linux")]
mod linux_path_tests {
    use super::*;
    use std::sync::Mutex;

    static PATH_LOCK: Mutex<()> = Mutex::new(());

    #[tokio::test(flavor = "current_thread")]
    async fn test_linux_default_path_with_xdg_config_home() {
        let _lock = PATH_LOCK.lock().unwrap();
        let original = std::env::var("XDG_CONFIG_HOME").ok();

        std::env::set_var("XDG_CONFIG_HOME", "/tmp/agentiron-xdg-test");

        let path = default_config_path().unwrap();
        assert_eq!(
            path,
            std::path::PathBuf::from("/tmp/agentiron-xdg-test/agentiron/config.db")
        );

        match original {
            Some(val) => std::env::set_var("XDG_CONFIG_HOME", val),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_linux_default_path_fallback_without_xdg() {
        let _lock = PATH_LOCK.lock().unwrap();
        let original = std::env::var("XDG_CONFIG_HOME").ok();
        let original_home = std::env::var("HOME").ok();

        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::set_var("HOME", "/home/testuser");

        let path = default_config_path().unwrap();
        assert_eq!(
            path,
            std::path::PathBuf::from("/home/testuser/.config/agentiron/config.db")
        );

        match original {
            Some(val) => std::env::set_var("XDG_CONFIG_HOME", val),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        match original_home {
            Some(val) => std::env::set_var("HOME", val),
            None => std::env::remove_var("HOME"),
        }
    }
}

// ============================================================================
// Custom Model Runtime Integration Tests
// ============================================================================

#[tokio::test]
async fn test_provider_slug_validation() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    // Unknown provider slug should be rejected
    let input = CustomModelInput {
        provider_slug: "unknown-provider".to_string(),
        model_id: "some-model".to_string(),
        display_name: "Some Model".to_string(),
        context_window: Some(100_000),
        output_limit: Some(4_096),
        supports_tool_calls: false,
        supports_reasoning: false,
        supports_vision: false,
        supports_streaming: true,
        reasoning_effort_values: vec![],
        cost_input_per_million: None,
        cost_output_per_million: None,
    };
    let err = store.set_custom_model(&input).await.unwrap_err();
    assert!(
        matches!(err, iron_core::config::ConfigError::Validation(ref msg) if msg.contains("unknown-provider")),
        "Expected validation error for unknown provider slug, got {:?}",
        err
    );

    // Built-in provider slug should be accepted
    let builtin_input = CustomModelInput {
        provider_slug: "openai".to_string(),
        model_id: "custom-openai-model".to_string(),
        display_name: "Custom OpenAI Model".to_string(),
        context_window: Some(100_000),
        output_limit: Some(4_096),
        supports_tool_calls: true,
        supports_reasoning: false,
        supports_vision: true,
        supports_streaming: true,
        reasoning_effort_values: vec![],
        cost_input_per_million: None,
        cost_output_per_million: None,
    };
    store.set_custom_model(&builtin_input).await.unwrap();

    // Persisted provider slug should also be accepted
    store
        .set_provider_config(&ProviderConfigInput {
            provider_slug: "custom-persisted".to_string(),
            display_name: "Custom Persisted Provider".to_string(),
            enabled: true,
            base_url: Some("https://example.com".to_string()),
        })
        .await
        .unwrap();

    let persisted_input = CustomModelInput {
        provider_slug: "custom-persisted".to_string(),
        model_id: "custom-persisted-model".to_string(),
        display_name: "Custom Persisted Model".to_string(),
        context_window: Some(100_000),
        output_limit: Some(4_096),
        supports_tool_calls: false,
        supports_reasoning: false,
        supports_vision: false,
        supports_streaming: true,
        reasoning_effort_values: vec![],
        cost_input_per_million: None,
        cost_output_per_million: None,
    };
    store.set_custom_model(&persisted_input).await.unwrap();
}

#[tokio::test]
async fn test_custom_model_streaming_and_reasoning_roundtrip() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    let input = CustomModelInput {
        provider_slug: "openai".to_string(),
        model_id: "reasoning-model".to_string(),
        display_name: "Reasoning Model".to_string(),
        context_window: Some(200_000),
        output_limit: Some(8_192),
        supports_tool_calls: true,
        supports_reasoning: true,
        supports_vision: false,
        supports_streaming: false,
        reasoning_effort_values: vec!["low".to_string(), "medium".to_string(), "high".to_string()],
        cost_input_per_million: Some(3.0),
        cost_output_per_million: Some(12.0),
    };

    let record = store.set_custom_model(&input).await.unwrap();
    assert!(!record.supports_streaming);
    assert_eq!(
        record.reasoning_effort_values,
        vec!["low", "medium", "high"]
    );

    let retrieved = store
        .get_custom_model("openai", "reasoning-model")
        .await
        .unwrap()
        .unwrap();
    assert!(!retrieved.supports_streaming);
    assert_eq!(
        retrieved.reasoning_effort_values,
        vec!["low", "medium", "high"]
    );

    let listed = store.list_custom_models(None).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].supports_streaming);
    assert_eq!(
        listed[0].reasoning_effort_values,
        vec!["low", "medium", "high"]
    );
}

#[tokio::test]
async fn test_custom_model_backward_compatible_defaults() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    // Simulate a v2-era insert without the new columns by using raw SQL
    let mut conn = store.acquire().await.unwrap();
    sqlx::query(
        r#"
        INSERT INTO custom_models (provider_slug, model_id, display_name, context_window, output_limit, supports_tool_calls, supports_reasoning, supports_vision, cost_input_per_million, cost_output_per_million, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind("openai")
    .bind("legacy-model")
    .bind("Legacy Model")
    .bind(100_000_i64)
    .bind(4_096_i64)
    .bind(true)
    .bind(false)
    .bind(false)
    .bind(None::<f64>)
    .bind(None::<f64>)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&mut *conn)
    .await
    .unwrap();
    drop(conn);

    // Reading should default supports_streaming to true and reasoning_effort_values to empty
    let retrieved = store
        .get_custom_model("openai", "legacy-model")
        .await
        .unwrap()
        .unwrap();
    assert!(retrieved.supports_streaming);
    assert!(retrieved.reasoning_effort_values.is_empty());
}

#[tokio::test]
async fn test_default_model_validation_against_effective_catalog() {
    let store = ConfigStore::open_in_memory().await.unwrap();

    // Setting default to a non-existent model should fail validation
    let result = store
        .set_default_model(&DefaultModelInput {
            provider_slug: "openai".to_string(),
            model_id: "nonexistent-model".to_string(),
        })
        .await;
    assert!(
        result.is_err(),
        "Expected validation error for unknown default model"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err, iron_core::config::ConfigError::Validation(ref msg) if msg.contains("nonexistent-model")),
        "Expected validation error mentioning model, got {:?}",
        err
    );

    // Setting default to a built-in model should succeed
    store
        .set_default_model(&DefaultModelInput {
            provider_slug: "openai".to_string(),
            model_id: "gpt-4o".to_string(),
        })
        .await
        .unwrap();

    // Setting default to a custom model should succeed
    store
        .set_custom_model(&CustomModelInput {
            provider_slug: "openai".to_string(),
            model_id: "custom-openai-model".to_string(),
            display_name: "Custom Model".to_string(),
            context_window: Some(100_000),
            output_limit: Some(4_096),
            supports_tool_calls: true,
            supports_reasoning: false,
            supports_vision: false,
            supports_streaming: true,
            reasoning_effort_values: vec![],
            cost_input_per_million: None,
            cost_output_per_million: None,
        })
        .await
        .unwrap();

    store
        .set_default_model(&DefaultModelInput {
            provider_slug: "openai".to_string(),
            model_id: "custom-openai-model".to_string(),
        })
        .await
        .unwrap();

    let snapshot = store.load_runtime_settings().await.unwrap();
    assert!(snapshot.default_model.is_some());
    assert_eq!(
        snapshot.default_model.unwrap().model_id,
        "custom-openai-model"
    );
}

#[tokio::test]
async fn test_migrations_v2_to_v3_custom_models_columns() {
    use sqlx::sqlite::SqlitePoolOptions;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_v2_to_v3.db");

    // Create a raw v2 database
    let pool = SqlitePoolOptions::new()
        .connect(&format!("sqlite://{}?mode=rwc", db_path.display()))
        .await
        .unwrap();

    sqlx::raw_sql(
        r#"
        CREATE TABLE schema_version (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            version INTEGER NOT NULL
        );

        CREATE TABLE custom_models (
            provider_slug TEXT NOT NULL,
            model_id TEXT NOT NULL,
            display_name TEXT NOT NULL,
            context_window INTEGER NULL,
            output_limit INTEGER NULL,
            supports_tool_calls INTEGER NOT NULL DEFAULT 0,
            supports_reasoning INTEGER NOT NULL DEFAULT 0,
            supports_vision INTEGER NOT NULL DEFAULT 0,
            cost_input_per_million REAL NULL,
            cost_output_per_million REAL NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (provider_slug, model_id)
        );

        INSERT INTO schema_version (id, version) VALUES (1, 2);
        INSERT INTO custom_models (provider_slug, model_id, display_name, context_window, output_limit, supports_tool_calls, supports_reasoning, supports_vision, cost_input_per_million, cost_output_per_million, created_at, updated_at)
        VALUES ('openai', 'legacy-model', 'Legacy Model', 100000, 4096, 1, 0, 0, NULL, NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();
    drop(pool);

    let store = open_test_store(&db_path).await;

    let mut conn = store.acquire().await.unwrap();
    let version: i64 = sqlx::query_scalar("SELECT version FROM schema_version WHERE id = 1")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(version, 3);

    // Verify the new columns have correct defaults for existing rows
    let retrieved = store
        .get_custom_model("openai", "legacy-model")
        .await
        .unwrap()
        .unwrap();
    assert!(retrieved.supports_streaming);
    assert!(retrieved.reasoning_effort_values.is_empty());
}

#[tokio::test]
async fn test_effective_model_catalog_rejects_duplicate_builtin() {
    let custom = vec![CustomModelRecord {
        provider_slug: "openai".to_string(),
        model_id: "gpt-4o".to_string(),
        display_name: "Duplicate GPT-4o".to_string(),
        context_window: Some(100_000),
        output_limit: Some(4_096),
        supports_tool_calls: true,
        supports_reasoning: false,
        supports_vision: false,
        supports_streaming: true,
        reasoning_effort_values: vec![],
        cost_input_per_million: None,
        cost_output_per_million: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }];

    let result = build_effective_catalog(&builtin_model_catalog(), &custom);
    assert!(matches!(
        result,
        Err(CatalogError::DuplicateBuiltIn(ref p, ref m)) if p == "openai" && m == "gpt-4o"
    ));
}

#[tokio::test]
async fn test_set_custom_model_rejects_duplicate_builtin() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_test_store(dir.path().join("test.db")).await;

    let input = iron_core::config::CustomModelInput {
        provider_slug: "openai".to_string(),
        model_id: "gpt-4o".to_string(),
        display_name: "Duplicate GPT-4o".to_string(),
        context_window: Some(100_000),
        output_limit: Some(4_096),
        supports_tool_calls: true,
        supports_reasoning: false,
        supports_vision: false,
        supports_streaming: true,
        reasoning_effort_values: vec![],
        cost_input_per_million: None,
        cost_output_per_million: None,
    };

    let result = store.set_custom_model(&input).await;
    assert!(
        matches!(result, Err(iron_core::config::ConfigError::Validation(ref msg)) if msg.contains("conflicts with a built-in model"))
    );
}

#[tokio::test]
async fn test_custom_model_extends_builtin_catalog() {
    let custom = vec![CustomModelRecord {
        provider_slug: "openai".to_string(),
        model_id: "custom-gpt-4o".to_string(),
        display_name: "Custom GPT-4o".to_string(),
        context_window: Some(256_000),
        output_limit: Some(8_192),
        supports_tool_calls: true,
        supports_reasoning: true,
        supports_vision: true,
        supports_streaming: true,
        reasoning_effort_values: vec!["low".to_string(), "high".to_string()],
        cost_input_per_million: Some(5.0),
        cost_output_per_million: Some(15.0),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }];

    let catalog = build_effective_catalog(&builtin_model_catalog(), &custom).unwrap();
    assert!(catalog.get("openai", "gpt-4o").is_some());
    let custom_entry = catalog.get("openai", "custom-gpt-4o").unwrap();
    assert_eq!(custom_entry.context_window, Some(256_000));
    assert!(custom_entry.supports_reasoning);
    assert_eq!(custom_entry.reasoning_effort_values, vec!["low", "high"]);
    assert!(custom_entry.is_custom);
}
