//! ConfigStore-level tests for the atomic minimum-valid-profile deletion
//! policy and its cross-connection concurrency guarantees.

use iron_core::config::crypto::XChaCha20Poly1305Cipher;
use iron_core::config::{ConfigError, ConfigStore, OpenOptions, ProfileInput, PromptInput};
use iron_core::profile::{
    AgentProfile, AgentProfileId, ProfileDeletePolicy, PROFILE_SCHEMA_VERSION,
};
use iron_core::stored_prompt::{normalize_prompt_name, StoredPrompt, STORED_PROMPT_SCHEMA_VERSION};
use serde_json::json;
use std::sync::Arc;

/// Persist a structurally valid profile under the current schema version.
async fn save_valid_profile(store: &ConfigStore, id: &str, name: &str) {
    store
        .set_profile(&ProfileInput {
            id: id.to_string(),
            schema_version: PROFILE_SCHEMA_VERSION,
            payload: serde_json::to_value(AgentProfile::with_name(name)).unwrap(),
        })
        .await
        .unwrap();
}

/// Persist a raw profile row bypassing typed validation.
async fn save_raw_profile(
    store: &ConfigStore,
    id: &str,
    schema_version: i64,
    payload: serde_json::Value,
) {
    store
        .set_profile(&ProfileInput {
            id: id.to_string(),
            schema_version,
            payload,
        })
        .await
        .unwrap();
}

/// Persist a prompt that directly references a profile.
async fn save_prompt_referencing(store: &ConfigStore, id: &str, display: &str, profile_id: &str) {
    let prompt = StoredPrompt {
        display_name: display.to_string(),
        normalized_name: normalize_prompt_name(display),
        instructions: "Do the work".to_string(),
        skills: vec![],
        profile: Some(AgentProfileId::from(profile_id)),
    };
    store
        .set_prompt(&PromptInput {
            id: id.to_string(),
            schema_version: STORED_PROMPT_SCHEMA_VERSION,
            payload: serde_json::to_value(&prompt).unwrap(),
            display_name: display.to_string(),
            normalized_name: normalize_prompt_name(display),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn require_minimum_rejects_deleting_only_valid_profile() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    save_valid_profile(&store, "prof", "Profile").await;

    let err = store
        .delete_profile_with_policy("prof", ProfileDeletePolicy::RequireMinimumValid(1))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ConfigError::MinimumValidProfiles {
            minimum: 1,
            remaining: 0
        }
    ));
    assert!(store.get_profile("prof").await.unwrap().is_some());
}

#[tokio::test]
async fn malformed_unsupported_and_invalid_records_do_not_satisfy_minimum() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    save_valid_profile(&store, "valid", "Valid").await;
    // Malformed JSON payload.
    save_raw_profile(
        &store,
        "malformed",
        PROFILE_SCHEMA_VERSION,
        json!({"nope": true}),
    )
    .await;
    // Unsupported schema version with an otherwise valid payload.
    save_raw_profile(
        &store,
        "unsupported",
        999,
        serde_json::to_value(AgentProfile::with_name("Unsupported")).unwrap(),
    )
    .await;
    // Structurally invalid: empty name.
    save_raw_profile(
        &store,
        "invalid-name",
        PROFILE_SCHEMA_VERSION,
        json!({"name": ""}),
    )
    .await;

    let err = store
        .delete_profile_with_policy("valid", ProfileDeletePolicy::RequireMinimumValid(1))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ConfigError::MinimumValidProfiles {
            minimum: 1,
            remaining: 0
        }
    ));
}

#[tokio::test]
async fn deleting_invalid_target_contributes_zero_to_count() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    save_valid_profile(&store, "valid", "Valid").await;
    save_raw_profile(
        &store,
        "malformed",
        PROFILE_SCHEMA_VERSION,
        json!({"nope": true}),
    )
    .await;
    save_raw_profile(
        &store,
        "unsupported",
        999,
        serde_json::to_value(AgentProfile::with_name("Unsupported")).unwrap(),
    )
    .await;
    save_raw_profile(
        &store,
        "invalid-name",
        PROFILE_SCHEMA_VERSION,
        json!({"name": ""}),
    )
    .await;

    for target in &["malformed", "unsupported", "invalid-name"] {
        store
            .delete_profile_with_policy(target, ProfileDeletePolicy::RequireMinimumValid(1))
            .await
            .unwrap();
        assert!(
            store.get_profile("valid").await.unwrap().is_some(),
            "valid profile must remain after deleting {}",
            target
        );
    }
}

#[tokio::test]
async fn deleting_missing_target_with_satisfied_minimum_is_idempotent() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    save_valid_profile(&store, "valid", "Valid").await;

    store
        .delete_profile_with_policy("missing", ProfileDeletePolicy::RequireMinimumValid(1))
        .await
        .unwrap();
    assert!(store.get_profile("valid").await.unwrap().is_some());
}

#[tokio::test]
async fn missing_or_invalid_target_rejected_when_postcondition_unsatisfied() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    // No valid profiles at all; a minimum of 1 cannot be satisfied.
    save_raw_profile(
        &store,
        "malformed",
        PROFILE_SCHEMA_VERSION,
        json!({"nope": true}),
    )
    .await;

    let err = store
        .delete_profile_with_policy("missing", ProfileDeletePolicy::RequireMinimumValid(1))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ConfigError::MinimumValidProfiles {
            minimum: 1,
            remaining: 0
        }
    ));

    let err = store
        .delete_profile_with_policy("malformed", ProfileDeletePolicy::RequireMinimumValid(1))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ConfigError::MinimumValidProfiles {
            minimum: 1,
            remaining: 0
        }
    ));
}

#[tokio::test]
async fn zero_minimum_permits_deleting_final_valid_profile() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    save_valid_profile(&store, "valid", "Valid").await;

    store
        .delete_profile_with_policy("valid", ProfileDeletePolicy::RequireMinimumValid(0))
        .await
        .unwrap();
    assert!(store.get_profile("valid").await.unwrap().is_none());
}

#[tokio::test]
async fn unrestricted_deletion_removes_final_valid_profile() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    save_valid_profile(&store, "valid", "Valid").await;

    store.delete_profile("valid").await.unwrap();
    assert!(store.get_profile("valid").await.unwrap().is_none());
}

#[tokio::test]
async fn unrestricted_deletion_cleans_up_invalid_record() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    save_raw_profile(
        &store,
        "malformed",
        PROFILE_SCHEMA_VERSION,
        json!({"nope": true}),
    )
    .await;

    store.delete_profile_checked("malformed").await.unwrap();
    assert!(store.get_profile_raw("malformed").await.unwrap().is_none());
}

#[tokio::test]
async fn prompt_reference_conflict_precedes_minimum_failure() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    save_valid_profile(&store, "prof", "Profile").await;
    save_prompt_referencing(&store, "prompt-a", "Alpha", "prof").await;

    let err = store
        .delete_profile_with_policy("prof", ProfileDeletePolicy::RequireMinimumValid(1))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ConfigError::ProfileReferencedByPrompts { .. }
    ));
    assert!(store.get_profile("prof").await.unwrap().is_some());
}

#[tokio::test]
async fn minimum_failure_preserves_target_and_other_records() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    save_valid_profile(&store, "alpha", "Alpha").await;
    save_valid_profile(&store, "beta", "Beta").await;

    let err = store
        .delete_profile_with_policy("alpha", ProfileDeletePolicy::RequireMinimumValid(2))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ConfigError::MinimumValidProfiles {
            minimum: 2,
            remaining: 1
        }
    ));
    assert!(store.get_profile("alpha").await.unwrap().is_some());
    assert!(store.get_profile("beta").await.unwrap().is_some());
}

// ============================================================================
// Cross-connection concurrency
// ============================================================================

/// Shared cipher so two file-backed stores can read the same database.
fn shared_cipher() -> Arc<dyn iron_core::config::crypto::CredentialCipher> {
    let key = XChaCha20Poly1305Cipher::generate_key();
    Arc::new(XChaCha20Poly1305Cipher::new(&key))
}

async fn open_file_store(
    path: &std::path::Path,
    cipher: Arc<dyn iron_core::config::crypto::CredentialCipher>,
) -> ConfigStore {
    ConfigStore::open_at_with_options(
        path,
        OpenOptions {
            cipher: Some(cipher),
            busy_timeout: None,
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn concurrent_deletions_leave_at_least_one_valid_profile() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("policy.db");
    let cipher = shared_cipher();

    // Seed both profiles through the first client.
    {
        let store = open_file_store(&db_path, cipher.clone()).await;
        save_valid_profile(&store, "alpha", "Alpha").await;
        save_valid_profile(&store, "beta", "Beta").await;
    }

    let store_a = open_file_store(&db_path, cipher.clone()).await;
    let store_b = open_file_store(&db_path, cipher).await;

    // Both clients concurrently delete a different valid profile while
    // requiring at least one valid profile to remain.
    let (result_a, result_b) = tokio::join!(
        store_a.delete_profile_with_policy("alpha", ProfileDeletePolicy::RequireMinimumValid(1)),
        store_b.delete_profile_with_policy("beta", ProfileDeletePolicy::RequireMinimumValid(1)),
    );

    let successes = [&result_a, &result_b].iter().filter(|r| r.is_ok()).count();
    assert!(
        successes <= 1,
        "at most one concurrent deletion may succeed, got {} (a={:?}, b={:?})",
        successes,
        result_a.as_ref().err(),
        result_b.as_ref().err()
    );

    // At least one valid profile remains durably.
    let a_present = store_a.get_profile("alpha").await.unwrap().is_some();
    let b_present = store_a.get_profile("beta").await.unwrap().is_some();
    assert!(
        a_present || b_present,
        "at least one valid profile must remain"
    );
    // Exactly one remains.
    assert_eq!(a_present as u8 + b_present as u8, 1);

    // The loser must receive a typed minimum-valid-profile failure (the second
    // deleter observes the first's committed state, not the stale snapshot).
    let loser = if result_a.is_ok() {
        &result_b
    } else {
        &result_a
    };
    match loser.as_ref().unwrap_err() {
        ConfigError::MinimumValidProfiles { minimum, remaining } => {
            assert_eq!(*minimum, 1);
            assert_eq!(*remaining, 0);
        }
        other => panic!("expected typed MinimumValidProfiles, got {:?}", other),
    }
}

#[tokio::test]
async fn prompt_reference_protection_survives_concurrent_unrelated_writer() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("refs.db");
    let cipher = shared_cipher();

    // profile "prof" is referenced by a prompt; "other" is unreferenced.
    {
        let store = open_file_store(&db_path, cipher.clone()).await;
        save_valid_profile(&store, "prof", "Profile").await;
        save_valid_profile(&store, "other", "Other").await;
        save_prompt_referencing(&store, "prompt-a", "Alpha", "prof").await;
    }

    let store_a = open_file_store(&db_path, cipher.clone()).await;
    let store_b = open_file_store(&db_path, cipher).await;

    // Concurrently: delete the referenced profile (must stay blocked) and
    // delete the unrelated profile (must succeed). The write reservation
    // serializes them, and the prompt-reference check is not bypassed.
    let (delete_prof, delete_other) = tokio::join!(
        store_a.delete_profile_with_policy("prof", ProfileDeletePolicy::AllowZero),
        store_b.delete_profile_with_policy("other", ProfileDeletePolicy::AllowZero),
    );

    assert!(
        matches!(
            delete_prof.unwrap_err(),
            ConfigError::ProfileReferencedByPrompts { .. }
        ),
        "referenced profile must remain protected under concurrency"
    );
    delete_other.unwrap();

    assert!(store_a.get_profile("prof").await.unwrap().is_some());
    assert!(store_a.get_profile("other").await.unwrap().is_none());
}
