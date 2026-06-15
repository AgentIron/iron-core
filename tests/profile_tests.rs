use futures::stream::{self, BoxStream};
use futures::StreamExt;
use iron_core::config::{ConfigStore, ProfileInput, PromptInput};
use iron_core::STORED_PROMPT_SCHEMA_VERSION;
use iron_core::{
    profile::{
        default_identity_prompt, normalize_profile_name, AgentApproval, AgentProfile,
        AgentProfileId, AgentProfileProvider, ProfileLoadIssue, ResolvedProfileProvider,
        SkillFilter, ToolFilter, PROFILE_SCHEMA_VERSION,
    },
    provider_credential::{
        domain::{
            CredentialMode, OAuthTokenSet, ProviderPromptContext, ProviderSlug, StoredCredential,
        },
        store::{InMemoryCredentialStore, ProviderCredentialStore},
        DurableCredentialStore,
    },
    Config, IronAgent, IronRuntime, PromptOutcome, StoredPrompt,
    PROFILE_SCHEMA_VERSION as EXPORTED_SCHEMA_VERSION,
};
use iron_providers::{InferenceRequest, Provider, ProviderEvent};
use serde_json::json;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Test provider
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestProvider;

impl Provider for TestProvider {
    fn infer(
        &self,
        _request: InferenceRequest,
    ) -> iron_providers::ProviderFuture<'_, Vec<ProviderEvent>> {
        Box::pin(async { Ok(vec![ProviderEvent::Complete]) })
    }

    fn infer_stream(
        &self,
        _request: InferenceRequest,
    ) -> iron_providers::ProviderFuture<
        '_,
        BoxStream<'static, iron_providers::ProviderResult<ProviderEvent>>,
    > {
        Box::pin(async { Ok(stream::empty().boxed()) })
    }
}

fn test_agent() -> IronAgent {
    IronAgent::new(Config::default(), TestProvider)
}

fn test_profile(name: &str) -> AgentProfile {
    AgentProfile {
        name: name.to_string(),
        provider: AgentProfileProvider::RuntimeDefault,
        tools: ToolFilter::Inherit,
        skills: SkillFilter::Inherit,
        approval: AgentApproval::PerTool,
        identity_prompt: None,
    }
}

fn managed_profile(name: &str, provider_slug: &str, model: &str) -> AgentProfile {
    AgentProfile {
        name: name.to_string(),
        provider: AgentProfileProvider::Managed {
            provider_slug: ProviderSlug::new(provider_slug),
            model: model.to_string(),
        },
        tools: ToolFilter::Inherit,
        skills: SkillFilter::Inherit,
        approval: AgentApproval::PerTool,
        identity_prompt: None,
    }
}

// ---------------------------------------------------------------------------
// Domain type tests
// ---------------------------------------------------------------------------

#[test]
fn schema_version_constant_is_one() {
    assert_eq!(PROFILE_SCHEMA_VERSION, 1);
    assert_eq!(EXPORTED_SCHEMA_VERSION, 1);
}

#[test]
fn profile_serialization_roundtrip() {
    let profile = AgentProfile {
        name: "code-reviewer".to_string(),
        provider: AgentProfileProvider::Managed {
            provider_slug: ProviderSlug::new("openai"),
            model: "gpt-4o".to_string(),
        },
        tools: ToolFilter::Allow(vec!["read".to_string(), "search".to_string()]),
        skills: SkillFilter::Allow(vec!["rust".to_string()]),
        approval: AgentApproval::ReadOnly,
        identity_prompt: Some("You review code.".to_string()),
    };

    let value = serde_json::to_value(&profile).unwrap();
    let decoded: AgentProfile = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, profile);
}

#[test]
fn runtime_default_provider_serialization() {
    let profile = AgentProfile {
        name: "assistant".to_string(),
        provider: AgentProfileProvider::RuntimeDefault,
        tools: ToolFilter::Inherit,
        skills: SkillFilter::Inherit,
        approval: AgentApproval::PerTool,
        identity_prompt: None,
    };

    let value = serde_json::to_value(&profile).unwrap();
    assert!(value.get("RuntimeDefault").is_some() || value.get("runtime_default").is_some());
    let decoded: AgentProfile = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, profile);
}

#[test]
fn managed_provider_context_has_no_api_key() {
    let profile = managed_profile("assistant", "openai", "gpt-4o");
    let ctx = iron_core::profile::managed_profile_prompt_context(&profile.provider);
    assert_eq!(
        ctx,
        Some(ProviderPromptContext {
            provider_slug: ProviderSlug::new("openai"),
            model: "gpt-4o".to_string(),
            api_key: None,
        })
    );
}

#[test]
fn normalize_profile_name_trims_and_rejects_reserved() {
    assert_eq!(normalize_profile_name("  Foo  "), Some("Foo".to_string()));
    assert_eq!(normalize_profile_name("default"), None);
    assert_eq!(normalize_profile_name("Default"), None);
    assert_eq!(normalize_profile_name(""), None);
    assert_eq!(normalize_profile_name("\t"), None);
    assert_eq!(normalize_profile_name("a\nb"), None);
}

#[test]
fn effective_identity_prompt_fallback_and_override() {
    let mut profile = AgentProfile::with_name("test");
    assert_eq!(
        profile.effective_identity_prompt(),
        default_identity_prompt()
    );

    profile.identity_prompt = Some("Custom identity.".to_string());
    assert_eq!(profile.effective_identity_prompt(), "Custom identity.");

    profile.identity_prompt = Some("   ".to_string());
    assert_eq!(
        profile.effective_identity_prompt(),
        default_identity_prompt()
    );
}

// ---------------------------------------------------------------------------
// IronAgent registry tests
// ---------------------------------------------------------------------------

#[test]
fn default_profile_is_always_registered() {
    let agent = test_agent();
    let profiles = agent.list_profiles();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].id.as_str(), "default");
    assert_eq!(profiles[0].profile.name, "default");
    assert!(matches!(
        profiles[0].profile.provider,
        AgentProfileProvider::RuntimeDefault
    ));
    assert!(matches!(profiles[0].profile.tools, ToolFilter::Inherit));
    assert!(matches!(profiles[0].profile.skills, SkillFilter::Inherit));
    assert!(matches!(
        profiles[0].profile.approval,
        AgentApproval::PerTool
    ));
}

#[test]
fn register_and_list_profile() {
    let agent = test_agent();
    let profile = test_profile("My Profile");
    agent
        .register_profile(AgentProfileId::from("prof-1"), profile.clone())
        .unwrap();

    let profiles = agent.list_profiles();
    assert_eq!(profiles.len(), 2);
    let entry = profiles.iter().find(|e| e.id.as_str() == "prof-1").unwrap();
    assert_eq!(entry.profile.name, "My Profile");
}

#[test]
fn register_rejects_reserved_default_id() {
    let agent = test_agent();
    let profile = test_profile("custom");
    assert!(agent
        .register_profile(AgentProfileId::from("default"), profile.clone())
        .is_err());
    assert!(agent
        .register_profile(AgentProfileId::from("Default"), profile.clone())
        .is_err());
}

#[test]
fn register_rejects_reserved_default_name() {
    let agent = test_agent();
    let mut profile = test_profile("default");
    assert!(agent
        .register_profile(AgentProfileId::from("prof-1"), profile.clone())
        .is_err());
    profile.name = "Default".to_string();
    assert!(agent
        .register_profile(AgentProfileId::from("prof-1"), profile.clone())
        .is_err());
}

#[test]
fn register_rejects_invalid_name() {
    let agent = test_agent();
    let profile = AgentProfile::with_name("");
    assert!(agent
        .register_profile(AgentProfileId::from("prof-1"), profile)
        .is_err());

    let profile = AgentProfile::with_name("   ");
    assert!(agent
        .register_profile(AgentProfileId::from("prof-1"), profile)
        .is_err());

    let profile = AgentProfile::with_name("a\nb");
    assert!(agent
        .register_profile(AgentProfileId::from("prof-1"), profile)
        .is_err());
}

#[test]
fn register_rejects_duplicate_name() {
    let agent = test_agent();
    let profile1 = test_profile("Unique Name");
    let profile2 = test_profile("Unique Name");
    agent
        .register_profile(AgentProfileId::from("prof-1"), profile1)
        .unwrap();
    assert!(agent
        .register_profile(AgentProfileId::from("prof-2"), profile2)
        .is_err());
}

#[test]
fn register_normalizes_name() {
    let agent = test_agent();
    let profile = AgentProfile::with_name("  Spaced Name  ");
    agent
        .register_profile(AgentProfileId::from("prof-1"), profile)
        .unwrap();
    let entry = agent.get_profile(&AgentProfileId::from("prof-1")).unwrap();
    assert_eq!(entry.profile.name, "Spaced Name");
}

#[test]
fn replace_profile_keeps_id() {
    let agent = test_agent();
    let profile1 = test_profile("First");
    let profile2 = test_profile("Second");
    agent
        .register_profile(AgentProfileId::from("prof-1"), profile1)
        .unwrap();
    agent
        .register_profile(AgentProfileId::from("prof-1"), profile2)
        .unwrap();

    let entry = agent.get_profile(&AgentProfileId::from("prof-1")).unwrap();
    assert_eq!(entry.profile.name, "Second");
}

#[test]
fn replace_rejects_duplicate_name_owned_by_other() {
    let agent = test_agent();
    let profile1 = test_profile("First");
    let profile2 = test_profile("Second");
    agent
        .register_profile(AgentProfileId::from("prof-1"), profile1)
        .unwrap();
    agent
        .register_profile(AgentProfileId::from("prof-2"), profile2)
        .unwrap();

    // prof-1 cannot be replaced with name "Second" because prof-2 owns it.
    let replacement = test_profile("Second");
    assert!(agent
        .register_profile(AgentProfileId::from("prof-1"), replacement)
        .is_err());

    // prof-1 should remain unchanged.
    let entry = agent.get_profile(&AgentProfileId::from("prof-1")).unwrap();
    assert_eq!(entry.profile.name, "First");
}

#[test]
fn invalid_replacement_leaves_existing_unchanged() {
    let agent = test_agent();
    let valid = test_profile("Valid");
    agent
        .register_profile(AgentProfileId::from("prof-1"), valid)
        .unwrap();

    let invalid = AgentProfile::with_name("");
    assert!(agent
        .register_profile(AgentProfileId::from("prof-1"), invalid)
        .is_err());

    let entry = agent.get_profile(&AgentProfileId::from("prof-1")).unwrap();
    assert_eq!(entry.profile.name, "Valid");
}

#[test]
fn unregister_profile() {
    let agent = test_agent();
    agent
        .register_profile(AgentProfileId::from("prof-1"), test_profile("One"))
        .unwrap();
    assert!(agent.unregister_profile(&AgentProfileId::from("prof-1")));
    assert!(!agent.unregister_profile(&AgentProfileId::from("prof-1")));
    assert!(agent.get_profile(&AgentProfileId::from("prof-1")).is_none());
}

#[test]
fn unregister_default_is_noop() {
    let agent = test_agent();
    assert!(!agent.unregister_profile(&AgentProfileId::from("default")));
    assert!(agent
        .get_profile(&AgentProfileId::from("default"))
        .is_some());
}

#[test]
fn unregister_frees_name() {
    let agent = test_agent();
    agent
        .register_profile(AgentProfileId::from("prof-1"), test_profile("Freed"))
        .unwrap();
    agent.unregister_profile(&AgentProfileId::from("prof-1"));
    agent
        .register_profile(AgentProfileId::from("prof-2"), test_profile("Freed"))
        .unwrap();
}

#[test]
fn list_profiles_is_deterministic() {
    let agent = test_agent();
    agent
        .register_profile(AgentProfileId::from("charlie"), test_profile("Charlie"))
        .unwrap();
    agent
        .register_profile(AgentProfileId::from("alpha"), test_profile("Alpha"))
        .unwrap();
    agent
        .register_profile(AgentProfileId::from("bravo"), test_profile("Bravo"))
        .unwrap();

    let ids: Vec<String> = agent
        .list_profiles()
        .into_iter()
        .map(|e| e.id.as_str().to_string())
        .collect();
    assert_eq!(ids, vec!["alpha", "bravo", "charlie", "default"]);
}

// ---------------------------------------------------------------------------
// ConfigStore loading tests
// ---------------------------------------------------------------------------

async fn store_with_profile_payload(
    id: &str,
    schema_version: i64,
    payload: serde_json::Value,
) -> ConfigStore {
    let store = ConfigStore::open_in_memory().await.unwrap();
    store
        .set_profile(&ProfileInput {
            id: id.to_string(),
            schema_version,
            payload,
        })
        .await
        .unwrap();
    store
}

#[tokio::test]
async fn load_profiles_success() {
    let store = store_with_profile_payload(
        "prof-1",
        PROFILE_SCHEMA_VERSION,
        json!({"name": "Loaded", "RuntimeDefault": {}}),
    )
    .await;

    let agent = test_agent();
    let report = agent.load_profiles(&store).await.unwrap();
    assert_eq!(report.loaded.len(), 1);
    assert_eq!(report.loaded[0].id.as_str(), "prof-1");
    assert_eq!(report.loaded[0].profile.name, "Loaded");
    assert!(report.diagnostics.is_empty());
}

#[tokio::test]
async fn load_profiles_replaces_existing() {
    let store = store_with_profile_payload(
        "prof-1",
        PROFILE_SCHEMA_VERSION,
        json!({"name": "From Store", "RuntimeDefault": {}}),
    )
    .await;

    let agent = test_agent();
    agent
        .register_profile(AgentProfileId::from("prof-1"), test_profile("Original"))
        .unwrap();

    let report = agent.load_profiles(&store).await.unwrap();
    assert_eq!(report.loaded.len(), 1);
    assert_eq!(report.loaded[0].profile.name, "From Store");
}

#[tokio::test]
async fn load_profiles_unsupported_schema_version() {
    let store = store_with_profile_payload(
        "prof-1",
        999,
        json!({"name": "Future", "RuntimeDefault": {}}),
    )
    .await;

    let agent = test_agent();
    let report = agent.load_profiles(&store).await.unwrap();
    assert!(report.loaded.is_empty());
    assert_eq!(report.diagnostics.len(), 1);
    assert!(matches!(
        report.diagnostics[0].issue,
        ProfileLoadIssue::UnsupportedSchemaVersion { version: 999 }
    ));
}

#[tokio::test]
async fn load_profiles_invalid_payload() {
    let store = store_with_profile_payload(
        "prof-1",
        PROFILE_SCHEMA_VERSION,
        json!({"not_a_name": "oops"}),
    )
    .await;

    let agent = test_agent();
    let report = agent.load_profiles(&store).await.unwrap();
    assert!(report.loaded.is_empty());
    assert_eq!(report.diagnostics.len(), 1);
    assert!(matches!(
        report.diagnostics[0].issue,
        ProfileLoadIssue::InvalidPayload
    ));
}

#[tokio::test]
async fn load_profiles_invalid_id() {
    let store = store_with_profile_payload(
        "default",
        PROFILE_SCHEMA_VERSION,
        json!({"name": "Evil", "RuntimeDefault": {}}),
    )
    .await;

    let agent = test_agent();
    let report = agent.load_profiles(&store).await.unwrap();
    assert!(report.loaded.is_empty());
    assert!(report.diagnostics.iter().any(|d| matches!(
        d.issue,
        ProfileLoadIssue::ReservedDefault | ProfileLoadIssue::InvalidProfileId
    )));
}

#[tokio::test]
async fn load_profiles_reserved_name() {
    let store = store_with_profile_payload(
        "prof-1",
        PROFILE_SCHEMA_VERSION,
        json!({"name": "Default", "RuntimeDefault": {}}),
    )
    .await;

    let agent = test_agent();
    let report = agent.load_profiles(&store).await.unwrap();
    assert!(report.loaded.is_empty());
    assert!(report
        .diagnostics
        .iter()
        .any(|d| matches!(d.issue, ProfileLoadIssue::ReservedDefault)));
}

#[tokio::test]
async fn load_profiles_invalid_name() {
    let store = store_with_profile_payload(
        "prof-1",
        PROFILE_SCHEMA_VERSION,
        json!({"name": "", "RuntimeDefault": {}}),
    )
    .await;

    let agent = test_agent();
    let report = agent.load_profiles(&store).await.unwrap();
    assert!(report.loaded.is_empty());
    assert!(report
        .diagnostics
        .iter()
        .any(|d| matches!(d.issue, ProfileLoadIssue::InvalidName)));
}

#[tokio::test]
async fn load_profiles_duplicate_name_skips_later() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    store
        .set_profile(&ProfileInput {
            id: "b".to_string(),
            schema_version: PROFILE_SCHEMA_VERSION,
            payload: json!({"name": "Dup", "RuntimeDefault": {}}),
        })
        .await
        .unwrap();
    store
        .set_profile(&ProfileInput {
            id: "a".to_string(),
            schema_version: PROFILE_SCHEMA_VERSION,
            payload: json!({"name": "Dup", "RuntimeDefault": {}}),
        })
        .await
        .unwrap();

    let agent = test_agent();
    let report = agent.load_profiles(&store).await.unwrap();
    assert_eq!(report.loaded.len(), 1);
    assert_eq!(report.loaded[0].id.as_str(), "a");
    assert_eq!(report.diagnostics.len(), 1);
    assert!(matches!(
        report.diagnostics[0].issue,
        ProfileLoadIssue::DuplicateName
    ));
}

#[tokio::test]
async fn load_profiles_existing_registry_wins_duplicate() {
    let store = store_with_profile_payload(
        "prof-1",
        PROFILE_SCHEMA_VERSION,
        json!({"name": "Existing", "RuntimeDefault": {}}),
    )
    .await;

    let agent = test_agent();
    agent
        .register_profile(AgentProfileId::from("prof-2"), test_profile("Existing"))
        .unwrap();

    let report = agent.load_profiles(&store).await.unwrap();
    assert!(report.loaded.is_empty());
    assert_eq!(report.diagnostics.len(), 1);
    assert!(matches!(
        report.diagnostics[0].issue,
        ProfileLoadIssue::DuplicateName
    ));
}

#[tokio::test]
async fn load_profiles_is_additive() {
    let store = store_with_profile_payload(
        "prof-1",
        PROFILE_SCHEMA_VERSION,
        json!({"name": "From Store", "RuntimeDefault": {}}),
    )
    .await;

    let agent = test_agent();
    agent
        .register_profile(AgentProfileId::from("prof-2"), test_profile("In Memory"))
        .unwrap();

    let report = agent.load_profiles(&store).await.unwrap();
    assert_eq!(report.loaded.len(), 1);
    let profiles = agent.list_profiles();
    assert_eq!(profiles.len(), 3);
    assert!(profiles.iter().any(|e| e.id.as_str() == "prof-1"));
    assert!(profiles.iter().any(|e| e.id.as_str() == "prof-2"));
    assert!(profiles.iter().any(|e| e.id.as_str() == "default"));
}

#[tokio::test]
async fn load_profiles_invalid_replacement_leaves_existing() {
    let store = store_with_profile_payload(
        "prof-1",
        PROFILE_SCHEMA_VERSION,
        json!({"name": "", "RuntimeDefault": {}}),
    )
    .await;

    let agent = test_agent();
    agent
        .register_profile(AgentProfileId::from("prof-1"), test_profile("Original"))
        .unwrap();

    let report = agent.load_profiles(&store).await.unwrap();
    assert!(report.loaded.is_empty());
    assert_eq!(report.diagnostics.len(), 1);

    let entry = agent.get_profile(&AgentProfileId::from("prof-1")).unwrap();
    assert_eq!(entry.profile.name, "Original");
}

#[tokio::test]
async fn load_profiles_best_effort() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    store
        .set_profile(&ProfileInput {
            id: "good".to_string(),
            schema_version: PROFILE_SCHEMA_VERSION,
            payload: json!({"name": "Good", "RuntimeDefault": {}}),
        })
        .await
        .unwrap();
    store
        .set_profile(&ProfileInput {
            id: "bad".to_string(),
            schema_version: PROFILE_SCHEMA_VERSION,
            payload: json!({"name": "", "RuntimeDefault": {}}),
        })
        .await
        .unwrap();

    let agent = test_agent();
    let report = agent.load_profiles(&store).await.unwrap();
    assert_eq!(report.loaded.len(), 1);
    assert_eq!(report.loaded[0].id.as_str(), "good");
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].profile_id.as_str(), "bad");
}

#[tokio::test]
async fn load_profiles_excludes_builtin_default() {
    let store = store_with_profile_payload(
        "prof-1",
        PROFILE_SCHEMA_VERSION,
        json!({"name": "Loaded", "RuntimeDefault": {}}),
    )
    .await;

    let agent = test_agent();
    let report = agent.load_profiles(&store).await.unwrap();
    assert!(!report.loaded.iter().any(|e| e.id.as_str() == "default"));
}

// ---------------------------------------------------------------------------
// Provider resolution tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_runtime_default_provider() {
    let agent = test_agent();
    let default = agent.default_profile();
    let resolved = agent
        .runtime()
        .resolve_profile_provider(&default.profile)
        .await
        .unwrap();
    assert!(matches!(
        resolved,
        ResolvedProfileProvider::RuntimeDefault(_)
    ));
}

#[tokio::test]
async fn resolve_managed_provider_with_credentials() {
    let config_store = ConfigStore::open_in_memory().await.unwrap();
    let durable = Arc::new(DurableCredentialStore::new(config_store));
    let store: Arc<dyn ProviderCredentialStore> = durable.clone();
    store
        .set(
            &ProviderSlug::new("openai"),
            StoredCredential::ApiKey("sk-test".into()),
        )
        .await;

    let runtime = IronRuntime::new_with_credential_store(Config::default(), TestProvider, store);
    let profile = managed_profile("managed", "openai", "gpt-4o");
    let resolved = runtime.resolve_profile_provider(&profile).await.unwrap();
    assert!(matches!(resolved, ResolvedProfileProvider::Managed(_)));
}

#[tokio::test]
async fn resolve_managed_provider_missing_resolver() {
    let runtime = IronRuntime::new(Config::default(), TestProvider);
    let profile = managed_profile("managed", "openai", "gpt-4o");
    let result = runtime.resolve_profile_provider(&profile).await;
    assert!(matches!(
        result,
        Err(iron_core::provider_credential::domain::ProviderAuthError::NotConfigured(_))
    ));
}

#[tokio::test]
async fn resolve_managed_provider_missing_credential() {
    let config_store = ConfigStore::open_in_memory().await.unwrap();
    let durable = Arc::new(DurableCredentialStore::new(config_store));
    let store: Arc<dyn ProviderCredentialStore> = durable.clone();

    let runtime = IronRuntime::new_with_credential_store(Config::default(), TestProvider, store);
    let profile = managed_profile("managed", "openai", "gpt-4o");
    let result = runtime.resolve_profile_provider(&profile).await;
    assert!(matches!(
        result,
        Err(iron_core::provider_credential::domain::ProviderAuthError::NotConfigured(_))
    ));
}

#[tokio::test]
async fn resolve_managed_provider_nonexistent_provider_is_not_configured() {
    // A provider slug with no stored credential resolves as NotConfigured.
    let in_memory = Arc::new(InMemoryCredentialStore::new());
    let store: Arc<dyn ProviderCredentialStore> = in_memory.clone();
    let runtime = IronRuntime::new_with_credential_store(Config::default(), TestProvider, store);
    let profile = managed_profile("managed", "nonexistent-provider", "model");
    let result = runtime.resolve_profile_provider(&profile).await;
    assert!(matches!(
        result,
        Err(iron_core::provider_credential::domain::ProviderAuthError::NotConfigured(_))
    ));
}

// ---------------------------------------------------------------------------
// Default profile selection / identity prompt tests
// ---------------------------------------------------------------------------

#[test]
fn unspecified_profile_resolves_to_default() {
    let agent = test_agent();
    let default = agent.default_profile();
    assert_eq!(default.id.as_str(), "default");
    assert_eq!(
        default.profile.effective_identity_prompt(),
        default_identity_prompt()
    );
}

// ---------------------------------------------------------------------------
// Profile registry sharing tests
// ---------------------------------------------------------------------------

#[test]
fn iron_agent_clone_shares_profile_registry() {
    let agent = test_agent();
    let clone = agent.clone();

    agent
        .register_profile(AgentProfileId::from("prof-1"), test_profile("Original"))
        .unwrap();
    assert!(clone.get_profile(&AgentProfileId::from("prof-1")).is_some());

    clone
        .register_profile(AgentProfileId::from("prof-2"), test_profile("From Clone"))
        .unwrap();
    assert!(agent.get_profile(&AgentProfileId::from("prof-2")).is_some());
}

// ---------------------------------------------------------------------------
// Profile-backed prompt execution tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn prompt_applies_default_profile_identity_prompt() {
    let agent = test_agent();
    let conn = agent.connect();
    let session = conn.create_session().unwrap();

    let _ = session.prompt("hello").await;

    let durable = agent
        .runtime()
        .get_session(session.id())
        .expect("session exists");
    assert_eq!(
        durable.lock().profile_identity,
        Some(default_identity_prompt().to_string())
    );
    assert_eq!(durable.lock().instructions, None);
}

#[tokio::test]
async fn prompt_preserves_explicit_session_instructions() {
    let agent = test_agent();
    let conn = agent.connect();
    let session = conn.create_session().unwrap();
    session.set_instructions("Explicit instructions");

    let _ = session.prompt("hello").await;

    let durable = agent
        .runtime()
        .get_session(session.id())
        .expect("session exists");
    assert_eq!(
        durable.lock().instructions,
        Some("Explicit instructions".to_string())
    );
}

#[tokio::test]
async fn prompt_managed_applies_default_profile_identity_prompt() {
    let agent = test_agent();
    let conn = agent.connect();
    let session = conn.create_session().unwrap();

    let context = ProviderPromptContext {
        provider_slug: ProviderSlug::new("openai"),
        model: "gpt-4o".to_string(),
        api_key: Some("sk-test".to_string()),
    };
    let _ = session.prompt_managed("hello", context).await;

    let durable = agent
        .runtime()
        .get_session(session.id())
        .expect("session exists");
    assert_eq!(
        durable.lock().profile_identity,
        Some(default_identity_prompt().to_string())
    );
    assert_eq!(durable.lock().instructions, None);
}

// ---------------------------------------------------------------------------
// Explicit profile selection execution tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn prompt_with_profile_uses_custom_identity_prompt() {
    let agent = test_agent();
    agent
        .register_profile(
            AgentProfileId::from("rust-expert"),
            AgentProfile {
                name: "Rust Expert".to_string(),
                provider: AgentProfileProvider::RuntimeDefault,
                tools: ToolFilter::Inherit,
                skills: SkillFilter::Inherit,
                approval: AgentApproval::PerTool,
                identity_prompt: Some("You are a Rust expert.".to_string()),
            },
        )
        .unwrap();

    let conn = agent.connect();
    let session = conn
        .create_session_with_profile(AgentProfileId::from("rust-expert"))
        .unwrap();
    let _ = session.prompt("hello").await;

    let durable = agent
        .runtime()
        .get_session(session.id())
        .expect("session exists");
    assert_eq!(
        durable.lock().profile_identity,
        Some("You are a Rust expert.".to_string())
    );
    assert_eq!(durable.lock().instructions, None);
}

#[tokio::test]
async fn prompt_with_profile_unknown_returns_error_without_mutating_session() {
    let agent = test_agent();
    let conn = agent.connect();
    let session = conn
        .create_session_with_profile(AgentProfileId::from("unknown"))
        .unwrap();
    let outcome = session.prompt("hello").await;

    assert_eq!(outcome, PromptOutcome::EndTurn);
    let durable = agent
        .runtime()
        .get_session(session.id())
        .expect("session exists");
    assert_eq!(durable.lock().profile_identity, None);
    assert_eq!(durable.lock().instructions, None);
}

#[tokio::test]
async fn prompt_with_profile_uses_managed_provider_resolution() {
    let config_store = ConfigStore::open_in_memory().await.unwrap();
    let durable = Arc::new(DurableCredentialStore::new(config_store));
    let store: Arc<dyn ProviderCredentialStore> = durable.clone();
    store
        .set(
            &ProviderSlug::new("openai"),
            StoredCredential::ApiKey("sk-test".into()),
        )
        .await;

    let agent = IronAgent::new_with_credential_store(Config::default(), TestProvider, store);
    agent
        .register_profile(
            AgentProfileId::from("managed-assistant"),
            managed_profile("Managed Assistant", "openai", "gpt-4o"),
        )
        .unwrap();

    let conn = agent.connect();
    let session = conn
        .create_session_with_profile(AgentProfileId::from("managed-assistant"))
        .unwrap();
    let _ = session.prompt("hello").await;

    let durable = agent
        .runtime()
        .get_session(session.id())
        .expect("session exists");
    assert_eq!(durable.lock().profile_identity, None);
    assert_eq!(durable.lock().instructions, None);
}

#[tokio::test]
async fn resolve_managed_provider_unsupported_oauth_for_openai() {
    // openai supports only API keys; storing an OAuth credential yields
    // UnsupportedCredential from the resolver.
    let config_store = ConfigStore::open_in_memory().await.unwrap();
    let durable = Arc::new(DurableCredentialStore::new(config_store));
    let store: Arc<dyn ProviderCredentialStore> = durable.clone();
    store
        .set(
            &ProviderSlug::new("openai"),
            StoredCredential::OAuthBearer(OAuthTokenSet {
                access_token: "at".into(),
                refresh_token: "rt".into(),
                expires_at: None,
                id_token: None,
            }),
        )
        .await;

    let runtime = IronRuntime::new_with_credential_store(Config::default(), TestProvider, store);
    let profile = managed_profile("managed", "openai", "gpt-4o");
    let result = runtime.resolve_profile_provider(&profile).await;
    assert!(matches!(
        result,
        Err(iron_core::provider_credential::domain::ProviderAuthError::UnsupportedCredential {
            provider,
            mode: CredentialMode::OAuthBearer,
        }) if provider == "openai"
    ));
}

#[tokio::test]
async fn resolve_managed_provider_unsupported_api_key_for_codex() {
    // codex supports only OAuth; providing an API key yields UnsupportedCredential.
    let config_store = ConfigStore::open_in_memory().await.unwrap();
    let durable = Arc::new(DurableCredentialStore::new(config_store));
    let store: Arc<dyn ProviderCredentialStore> = durable.clone();

    let runtime = IronRuntime::new_with_credential_store(Config::default(), TestProvider, store);
    let mut profile = managed_profile("managed", "codex", "codex-model");
    profile.provider = AgentProfileProvider::Managed {
        provider_slug: ProviderSlug::new("codex"),
        model: "codex-model".to_string(),
    };
    // Build a context with an explicit API key to bypass stored-credential lookup.
    let context = ProviderPromptContext {
        provider_slug: ProviderSlug::new("codex"),
        model: "codex-model".to_string(),
        api_key: Some("sk-test".to_string()),
    };
    let result = runtime.resolve_managed_provider(&context).await;
    assert!(matches!(
        result,
        Err(iron_core::provider_credential::domain::ProviderAuthError::UnsupportedCredential {
            provider,
            mode: CredentialMode::ApiKey,
        }) if provider == "codex"
    ));
}

#[test]
fn custom_identity_prompt_replaces_default() {
    let profile = AgentProfile {
        name: "custom".to_string(),
        provider: AgentProfileProvider::RuntimeDefault,
        tools: ToolFilter::Inherit,
        skills: SkillFilter::Inherit,
        approval: AgentApproval::PerTool,
        identity_prompt: Some("You are a Rust expert.".to_string()),
    };
    assert_eq!(
        profile.effective_identity_prompt(),
        "You are a Rust expert."
    );
}

#[test]
fn blank_identity_prompt_falls_back_to_default() {
    let profile = AgentProfile {
        name: "custom".to_string(),
        provider: AgentProfileProvider::RuntimeDefault,
        tools: ToolFilter::Inherit,
        skills: SkillFilter::Inherit,
        approval: AgentApproval::PerTool,
        identity_prompt: Some("   ".to_string()),
    };
    assert_eq!(
        profile.effective_identity_prompt(),
        default_identity_prompt()
    );
}

#[test]
fn stored_prompt_registry_crud_is_deterministic() {
    let agent = test_agent();
    let prompt = StoredPrompt {
        instructions: "Review the latest changes".to_string(),
        skills: Vec::new(),
        profile: None,
    };

    agent
        .register_stored_prompt("review", prompt.clone())
        .unwrap();

    assert_eq!(agent.get_stored_prompt("review").unwrap().prompt, prompt);
    assert_eq!(agent.list_stored_prompts()[0].id, "review");
    assert!(agent.unregister_stored_prompt("review"));
    assert!(agent.get_stored_prompt("review").is_none());
}

#[tokio::test]
async fn load_stored_prompts_populates_registry() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let prompt = StoredPrompt {
        instructions: "Summarize this session".to_string(),
        skills: vec!["summarizer".to_string()],
        profile: None,
    };
    store
        .set_prompt(&PromptInput {
            id: "summary".to_string(),
            schema_version: STORED_PROMPT_SCHEMA_VERSION,
            payload: serde_json::to_value(&prompt).unwrap(),
        })
        .await
        .unwrap();

    let agent = test_agent();
    let report = agent.load_stored_prompts(&store).await.unwrap();

    assert_eq!(report.loaded.len(), 1);
    assert_eq!(agent.get_stored_prompt("summary").unwrap().prompt, prompt);
}

#[test]
fn connection_can_inspect_hidden_child_sessions() {
    let agent = test_agent();
    let conn = agent.connect();
    let parent = conn.create_session().unwrap();
    let (child_session_id, _) = agent.runtime().create_hidden_session(conn.id()).unwrap();

    agent
        .runtime()
        .register_child(parent.id(), child_session_id)
        .unwrap();

    assert!(!conn.active_sessions().contains(&child_session_id));
    assert!(conn
        .active_sessions_include_hidden()
        .contains(&child_session_id));
    assert_eq!(
        conn.child_sessions(&parent).unwrap(),
        vec![child_session_id]
    );

    let hidden = conn.hidden_sessions();
    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0].session_id, child_session_id);
    assert_eq!(hidden[0].connection_id, conn.id());
    assert_eq!(hidden[0].parent_session_id, Some(parent.id()));
}
