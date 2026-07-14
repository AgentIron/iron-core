use crate::automation_task::AutomationTaskInput;
use crate::config::records::PromptInput;
use crate::config::{ConfigError, ConfigStore};
use crate::scheduled_task::ScheduledTaskInput;
use serde_json::json;

async fn setup_store_with_task() -> ConfigStore {
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
        .set_automation_task(&AutomationTaskInput {
            id: "task-1".to_string(),
            display_name: "Task One".to_string(),
            stored_prompt_id: "prompt-1".to_string(),
            expected_outcome: "Done".to_string(),
            project_root: std::env::temp_dir(),
            timeout_seconds: 300,
        })
        .await
        .unwrap();

    store
}

fn schedule_input(id: &str, task_id: &str) -> ScheduledTaskInput {
    ScheduledTaskInput {
        id: id.to_string(),
        automation_task_id: task_id.to_string(),
        cron_expression: "0 9 * * *".to_string(),
        enabled: true,
    }
}

#[tokio::test]
async fn schedule_crud_create_get_list_delete() {
    let store = setup_store_with_task().await;

    let schedule = store
        .set_scheduled_task(&schedule_input("morning", "task-1"))
        .await
        .unwrap();
    assert_eq!(schedule.id, "morning");
    assert_eq!(schedule.automation_task_id, "task-1");
    assert_eq!(schedule.cron_expression, "0 9 * * *");
    assert!(schedule.enabled);
    assert_eq!(schedule.created_at, schedule.updated_at);

    let fetched = store.get_scheduled_task("morning").await.unwrap().unwrap();
    assert_eq!(fetched, schedule);

    store.delete_scheduled_task("morning").await.unwrap();
    assert!(store.get_scheduled_task("morning").await.unwrap().is_none());
}

#[tokio::test]
async fn schedule_list_deterministic_by_id() {
    let store = setup_store_with_task().await;
    store
        .set_scheduled_task(&schedule_input("charlie", "task-1"))
        .await
        .unwrap();
    store
        .set_scheduled_task(&schedule_input("alpha", "task-1"))
        .await
        .unwrap();
    store
        .set_scheduled_task(&schedule_input("bravo", "task-1"))
        .await
        .unwrap();

    let list = store.list_scheduled_tasks().await.unwrap();
    let ids: Vec<&str> = list.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["alpha", "bravo", "charlie"]);
}

#[tokio::test]
async fn schedule_replacement_preserves_created_at() {
    let store = setup_store_with_task().await;
    let original = store
        .set_scheduled_task(&schedule_input("s1", "task-1"))
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let updated = store
        .set_scheduled_task(&ScheduledTaskInput {
            id: "s1".to_string(),
            automation_task_id: "task-1".to_string(),
            cron_expression: "*/15 * * * *".to_string(),
            enabled: false,
        })
        .await
        .unwrap();

    assert_eq!(updated.created_at, original.created_at);
    assert!(updated.updated_at > original.updated_at);
    assert_eq!(updated.cron_expression, "*/15 * * * *");
    assert!(!updated.enabled);
}

#[tokio::test]
async fn schedule_missing_task_rejected() {
    let store = setup_store_with_task().await;
    let result = store
        .set_scheduled_task(&schedule_input("s1", "nonexistent-task"))
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ConfigError::UnknownAutomationTask(task_id) => {
            assert_eq!(task_id, "nonexistent-task");
        }
        other => panic!("expected UnknownAutomationTask, got {:?}", other),
    }
    assert!(store.get_scheduled_task("s1").await.unwrap().is_none());
}

#[tokio::test]
async fn schedule_invalid_cron_rejected() {
    let store = setup_store_with_task().await;
    let result = store
        .set_scheduled_task(&ScheduledTaskInput {
            id: "s1".to_string(),
            automation_task_id: "task-1".to_string(),
            cron_expression: "not-valid".to_string(),
            enabled: true,
        })
        .await;
    assert!(matches!(result, Err(ConfigError::Validation(_))));
}

#[tokio::test]
async fn schedule_macro_cron_rejected() {
    let store = setup_store_with_task().await;
    let result = store
        .set_scheduled_task(&ScheduledTaskInput {
            id: "s1".to_string(),
            automation_task_id: "task-1".to_string(),
            cron_expression: "@daily".to_string(),
            enabled: true,
        })
        .await;
    assert!(matches!(result, Err(ConfigError::Validation(_))));
}

#[tokio::test]
async fn schedule_stepped_cron_accepted() {
    let store = setup_store_with_task().await;
    let result = store
        .set_scheduled_task(&ScheduledTaskInput {
            id: "s1".to_string(),
            automation_task_id: "task-1".to_string(),
            cron_expression: "*/15 9-17/2 1,15 * 1-5".to_string(),
            enabled: true,
        })
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn task_deletion_blocked_when_schedule_references_it() {
    let store = setup_store_with_task().await;
    store
        .set_scheduled_task(&schedule_input("s1", "task-1"))
        .await
        .unwrap();
    store
        .set_scheduled_task(&schedule_input("s2", "task-1"))
        .await
        .unwrap();

    let result = store.delete_automation_task("task-1").await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ConfigError::TaskReferencedBySchedules {
            task_id,
            schedule_ids,
        } => {
            assert_eq!(task_id, "task-1");
            assert_eq!(schedule_ids, vec!["s1", "s2"]);
        }
        other => panic!("expected TaskReferencedBySchedules, got {:?}", other),
    }

    assert!(store.get_automation_task("task-1").await.unwrap().is_some());
}

#[tokio::test]
async fn task_deletion_allowed_when_no_schedule_references_it() {
    let store = setup_store_with_task().await;
    store.delete_automation_task("task-1").await.unwrap();
    assert!(store.get_automation_task("task-1").await.unwrap().is_none());
}

#[tokio::test]
async fn schedules_referencing_task_helper() {
    let store = setup_store_with_task().await;
    store
        .set_scheduled_task(&schedule_input("s1", "task-1"))
        .await
        .unwrap();

    let refs = store.schedules_referencing_task("task-1").await.unwrap();
    assert_eq!(refs, vec!["s1"]);

    let no_refs = store.schedules_referencing_task("other").await.unwrap();
    assert!(no_refs.is_empty());
}

#[tokio::test]
async fn schedule_delete_does_not_affect_task() {
    let store = setup_store_with_task().await;
    store
        .set_scheduled_task(&schedule_input("s1", "task-1"))
        .await
        .unwrap();
    store.delete_scheduled_task("s1").await.unwrap();
    assert!(store.get_automation_task("task-1").await.unwrap().is_some());
}

#[tokio::test]
async fn schedule_input_is_trimmed() {
    let store = setup_store_with_task().await;
    let input = ScheduledTaskInput {
        id: "  spaced-id  ".to_string(),
        automation_task_id: "  task-1  ".to_string(),
        cron_expression: "  0 9 * * *  ".to_string(),
        enabled: true,
    };
    let schedule = store.set_scheduled_task(&input).await.unwrap();
    assert_eq!(schedule.id, "spaced-id");
    assert_eq!(schedule.automation_task_id, "task-1");
    assert_eq!(schedule.cron_expression, "0 9 * * *");
}
