use crate::automation_task::{AutomationTaskInput, AUTOMATION_TASK_SCHEMA_VERSION};
use crate::config::records::PromptInput;
use crate::config::{ConfigError, ConfigStore};
use serde_json::json;
use sqlx::Row;
use std::path::PathBuf;

async fn setup_store() -> ConfigStore {
    let store = ConfigStore::open_in_memory().await.unwrap();
    store
        .set_prompt(&PromptInput {
            id: "prompt-1".to_string(),
            schema_version: crate::stored_prompt::STORED_PROMPT_SCHEMA_VERSION,
            payload: json!({
                "display_name": "Test Prompt",
                "normalized_name": "test-prompt",
                "instructions": "Do the thing",
                "skills": [],
            }),
            display_name: "Test Prompt".to_string(),
            normalized_name: "test-prompt".to_string(),
        })
        .await
        .unwrap();
    store
}

fn task_input(id: &str, prompt_id: &str) -> AutomationTaskInput {
    AutomationTaskInput {
        id: id.to_string(),
        display_name: format!("Task {}", id),
        stored_prompt_id: prompt_id.to_string(),
        expected_outcome: "Done".to_string(),
        project_root: std::env::temp_dir(),
        timeout_seconds: 300,
    }
}

#[tokio::test]
async fn task_crud_create_get_list_delete() {
    let store = setup_store().await;

    let task = store
        .set_automation_task(&task_input("daily-report", "prompt-1"))
        .await
        .unwrap();
    assert_eq!(task.id, "daily-report");
    assert_eq!(task.display_name, "Task daily-report");
    assert_eq!(task.stored_prompt_id, "prompt-1");
    assert_eq!(task.expected_outcome, "Done");
    assert_eq!(task.created_at, task.updated_at);

    let fetched = store
        .get_automation_task("daily-report")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched, task);

    store.delete_automation_task("daily-report").await.unwrap();
    assert!(store
        .get_automation_task("daily-report")
        .await
        .unwrap()
        .is_none());

    assert!(store.get_prompt("prompt-1").await.unwrap().is_some());
}

#[tokio::test]
async fn task_list_is_deterministic_by_id() {
    let store = setup_store().await;
    store
        .set_automation_task(&task_input("charlie", "prompt-1"))
        .await
        .unwrap();
    store
        .set_automation_task(&task_input("alpha", "prompt-1"))
        .await
        .unwrap();
    store
        .set_automation_task(&task_input("bravo", "prompt-1"))
        .await
        .unwrap();

    let list = store.list_automation_tasks().await.unwrap();
    let ids: Vec<&str> = list.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, vec!["alpha", "bravo", "charlie"]);

    let list2 = store.list_automation_tasks().await.unwrap();
    let ids2: Vec<&str> = list2.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ids2);
}

#[tokio::test]
async fn task_replacement_preserves_created_at_and_advances_updated_at() {
    let store = setup_store().await;
    let original = store
        .set_automation_task(&task_input("t1", "prompt-1"))
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let updated_input = AutomationTaskInput {
        id: "t1".to_string(),
        display_name: "Updated Name".to_string(),
        stored_prompt_id: "prompt-1".to_string(),
        expected_outcome: "New outcome".to_string(),
        project_root: std::env::temp_dir(),
        timeout_seconds: 600,
    };
    let updated = store.set_automation_task(&updated_input).await.unwrap();

    assert_eq!(updated.created_at, original.created_at);
    assert_ne!(updated.updated_at, original.updated_at);
    assert!(updated.updated_at > original.updated_at);
    assert_eq!(updated.display_name, "Updated Name");
    assert_eq!(updated.expected_outcome, "New outcome");
}

#[tokio::test]
async fn task_missing_prompt_rejected() {
    let store = setup_store().await;
    let result = store
        .set_automation_task(&task_input("t1", "nonexistent-prompt"))
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ConfigError::UnknownStoredPrompt(prompt_id) => {
            assert_eq!(prompt_id, "nonexistent-prompt");
        }
        other => panic!("expected UnknownStoredPrompt, got {:?}", other),
    }

    assert!(store.get_automation_task("t1").await.unwrap().is_none());
}

#[tokio::test]
async fn task_validation_rejects_empty_fields() {
    let store = setup_store().await;

    let bad_input = AutomationTaskInput {
        id: "  ".to_string(),
        display_name: "Name".to_string(),
        stored_prompt_id: "prompt-1".to_string(),
        expected_outcome: "Done".to_string(),
        project_root: std::env::temp_dir(),
        timeout_seconds: 300,
    };
    let result = store.set_automation_task(&bad_input).await;
    assert!(matches!(result, Err(ConfigError::Validation(_))));
}

#[tokio::test]
async fn task_delete_preserves_prompt() {
    let store = setup_store().await;
    store
        .set_automation_task(&task_input("t1", "prompt-1"))
        .await
        .unwrap();
    store.delete_automation_task("t1").await.unwrap();
    assert!(store.get_prompt("prompt-1").await.unwrap().is_some());
}

#[tokio::test]
async fn prompt_deletion_blocked_when_task_references_it() {
    let store = setup_store().await;
    store
        .set_automation_task(&task_input("t1", "prompt-1"))
        .await
        .unwrap();
    store
        .set_automation_task(&task_input("t2", "prompt-1"))
        .await
        .unwrap();

    let result = store.delete_prompt("prompt-1").await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ConfigError::PromptReferencedByTasks {
            prompt_id,
            task_ids,
        } => {
            assert_eq!(prompt_id, "prompt-1");
            assert_eq!(task_ids, vec!["t1", "t2"]);
        }
        other => panic!("expected PromptReferencedByTasks, got {:?}", other),
    }

    assert!(store.get_prompt("prompt-1").await.unwrap().is_some());
    assert!(store.get_automation_task("t1").await.unwrap().is_some());
    assert!(store.get_automation_task("t2").await.unwrap().is_some());
}

#[tokio::test]
async fn prompt_deletion_allowed_when_no_task_references_it() {
    let store = setup_store().await;
    store.delete_prompt("prompt-1").await.unwrap();
    assert!(store.get_prompt("prompt-1").await.unwrap().is_none());
}

#[tokio::test]
async fn tasks_referencing_prompt_helper() {
    let store = setup_store().await;
    store
        .set_automation_task(&task_input("t1", "prompt-1"))
        .await
        .unwrap();

    let refs = store.tasks_referencing_prompt("prompt-1").await.unwrap();
    assert_eq!(refs, vec!["t1"]);

    let no_refs = store.tasks_referencing_prompt("other").await.unwrap();
    assert!(no_refs.is_empty());
}

#[tokio::test]
async fn task_input_is_trimmed_on_store() {
    let store = setup_store().await;
    let input = AutomationTaskInput {
        id: "  spaced-id  ".to_string(),
        display_name: "  Spaced  ".to_string(),
        stored_prompt_id: "  prompt-1  ".to_string(),
        expected_outcome: "  Outcome  ".to_string(),
        project_root: std::env::temp_dir(),
        timeout_seconds: 300,
    };
    let task = store.set_automation_task(&input).await.unwrap();
    assert_eq!(task.id, "spaced-id");
    assert_eq!(task.display_name, "Spaced");
    assert_eq!(task.stored_prompt_id, "prompt-1");
    assert_eq!(task.expected_outcome, "Outcome");
}

#[tokio::test]
async fn schema_version_is_stored() {
    let store = setup_store().await;
    store
        .set_automation_task(&task_input("t1", "prompt-1"))
        .await
        .unwrap();

    let row = sqlx::query("SELECT schema_version FROM automation_tasks WHERE id = ?")
        .bind("t1")
        .fetch_one(store.pool())
        .await
        .unwrap();
    let sv: i64 = row.get("schema_version");
    assert_eq!(sv, AUTOMATION_TASK_SCHEMA_VERSION);
}

#[tokio::test]
async fn get_rejects_unsupported_schema_version() {
    let store = setup_store().await;
    store
        .set_automation_task(&task_input("t1", "prompt-1"))
        .await
        .unwrap();

    sqlx::query("UPDATE automation_tasks SET schema_version = ? WHERE id = ?")
        .bind(999)
        .bind("t1")
        .execute(store.pool())
        .await
        .unwrap();

    let result = store.get_automation_task("t1").await;
    assert!(matches!(result, Err(ConfigError::Deserialization(_))));
}

#[tokio::test]
async fn list_rejects_unsupported_schema_version() {
    let store = setup_store().await;
    store
        .set_automation_task(&task_input("t1", "prompt-1"))
        .await
        .unwrap();

    sqlx::query("UPDATE automation_tasks SET schema_version = ? WHERE id = ?")
        .bind(999)
        .bind("t1")
        .execute(store.pool())
        .await
        .unwrap();

    let result = store.list_automation_tasks().await;
    assert!(matches!(result, Err(ConfigError::Deserialization(_))));
}

#[tokio::test]
async fn normalized_name_collision_rejected() {
    let store = setup_store().await;
    store
        .set_automation_task(&task_input("task-a", "prompt-1"))
        .await
        .unwrap();

    let colliding = AutomationTaskInput {
        id: "task-b".to_string(),
        display_name: "Task Task-a".to_string(),
        stored_prompt_id: "prompt-1".to_string(),
        expected_outcome: "Done".to_string(),
        project_root: std::env::temp_dir(),
        timeout_seconds: 300,
    };
    let result = store.set_automation_task(&colliding).await;
    assert!(matches!(result, Err(ConfigError::TaskNameConflict { .. })));
    assert!(store.get_automation_task("task-b").await.unwrap().is_none());
}

#[tokio::test]
async fn normalized_name_case_insensitive_collision() {
    let store = setup_store().await;
    let input_a = AutomationTaskInput {
        id: "a".to_string(),
        display_name: "Daily Report".to_string(),
        stored_prompt_id: "prompt-1".to_string(),
        expected_outcome: "Done".to_string(),
        project_root: std::env::temp_dir(),
        timeout_seconds: 300,
    };
    store.set_automation_task(&input_a).await.unwrap();

    let input_b = AutomationTaskInput {
        id: "b".to_string(),
        display_name: "DAILY REPORT".to_string(),
        stored_prompt_id: "prompt-1".to_string(),
        expected_outcome: "Done".to_string(),
        project_root: std::env::temp_dir(),
        timeout_seconds: 300,
    };
    let result = store.set_automation_task(&input_b).await;
    assert!(matches!(result, Err(ConfigError::TaskNameConflict { .. })));
}

#[tokio::test]
async fn rename_preserves_id_and_updates_normalized_name() {
    let store = setup_store().await;
    store
        .set_automation_task(&task_input("t1", "prompt-1"))
        .await
        .unwrap();

    let renamed = AutomationTaskInput {
        id: "t1".to_string(),
        display_name: "New Name".to_string(),
        stored_prompt_id: "prompt-1".to_string(),
        expected_outcome: "Done".to_string(),
        project_root: std::env::temp_dir(),
        timeout_seconds: 300,
    };
    let task = store.set_automation_task(&renamed).await.unwrap();
    assert_eq!(task.id, "t1");
    assert_eq!(task.display_name, "New Name");
    assert_eq!(task.normalized_name, "new-name");
}

#[tokio::test]
async fn nonexistent_project_root_rejected() {
    let store = setup_store().await;
    let input = AutomationTaskInput {
        id: "bad".to_string(),
        display_name: "Bad Root".to_string(),
        stored_prompt_id: "prompt-1".to_string(),
        expected_outcome: "Done".to_string(),
        project_root: PathBuf::from("/nonexistent/path/that/does/not/exist"),
        timeout_seconds: 300,
    };
    let result = store.set_automation_task(&input).await;
    assert!(matches!(result, Err(ConfigError::Validation(_))));
}
