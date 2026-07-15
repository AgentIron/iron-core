//! End-to-end management flow integration tests.
//!
//! Exercises the typed ConfigManagementService across profiles, prompts,
//! automation tasks, scheduled tasks, dependency impact, credential impact,
//! structural deletion warnings, and managed-record diagnostics.

use iron_core::automation_task::AutomationTaskInput;
use iron_core::config::ConfigStore;
use iron_core::management::{
    ConfigManagementService, DependencyDirection, DependencyEntity, ManagedAutomationTaskRecord,
    ManagedPromptRecord, ManagedScheduledTaskRecord, ManagementError,
};
use iron_core::profile::{AgentProfile, AgentProfileId, AgentProfileProvider};
use iron_core::provider_credential::domain::ProviderSlug;
use iron_core::scheduled_task::ScheduledTaskInput;
use iron_core::stored_prompt::StoredPrompt;

#[tokio::test]
async fn end_to_end_management_flow() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let svc = ConfigManagementService::new(store);

    // 1. Save a managed profile.
    let profile = AgentProfile {
        name: "Automation".to_string(),
        provider: AgentProfileProvider::Managed {
            provider_slug: ProviderSlug::from("test-provider"),
            model: "test-model".to_string(),
        },
        ..AgentProfile::with_name("Automation")
    };
    svc.save_profile("auto-prof", &profile).await.unwrap();

    // 2. Create a stored prompt referencing the profile.
    let (prompt_id, _) = svc
        .create_prompt(
            "Daily Report",
            "Generate a daily report",
            vec![],
            Some(AgentProfileId::from("auto-prof")),
        )
        .await
        .unwrap();

    // 3. Create an automation task referencing the prompt.
    svc.save_automation_task(&AutomationTaskInput {
        id: "daily-task".to_string(),
        display_name: "Daily Task".to_string(),
        stored_prompt_id: prompt_id.clone(),
        expected_outcome: "A summary of today's activity".to_string(),
        project_root: std::env::temp_dir(),
        timeout_seconds: 300,
    })
    .await
    .unwrap();

    // 4. Create a scheduled task referencing the automation task.
    svc.save_scheduled_task(&ScheduledTaskInput {
        id: "daily-sched".to_string(),
        automation_task_id: "daily-task".to_string(),
        cron_expression: "0 9 * * *".to_string(),
        enabled: true,
    })
    .await
    .unwrap();

    // 5. Query profile impact: prompt (direct dependent), task + schedule
    //    (transitive), credential (direct dependency).
    let profile_report = svc.profile_impact("auto-prof").await.unwrap();
    assert!(profile_report.links.iter().any(|l| {
        l.direction == DependencyDirection::Dependent
            && matches!(&l.entity, DependencyEntity::Prompt { id } if id == &prompt_id)
    }));
    assert!(profile_report.links.iter().any(|l| {
        l.direction == DependencyDirection::Depends
            && matches!(&l.entity, DependencyEntity::ProviderCredential { slug } if slug == "test-provider")
    }));

    // 6. Query credential impact: profiles (direct), prompts/tasks/schedules
    //    (transitive).
    let cred_report = svc.credential_impact("test-provider").await.unwrap();
    assert!(cred_report.links.iter().any(|l| {
        l.direction == DependencyDirection::Dependent
            && matches!(&l.entity, DependencyEntity::Profile { id } if id == "auto-prof")
    }));
    assert!(cred_report.links.iter().any(|l| {
        matches!(&l.entity, DependencyEntity::ScheduledTask { id } if id == "daily-sched")
    }));

    // 7. Query task impact: credential as transitive dependency.
    let task_report = svc.task_impact("daily-task").await.unwrap();
    assert!(task_report.links.iter().any(|l| {
        l.direction == DependencyDirection::Depends
            && matches!(&l.entity, DependencyEntity::ProviderCredential { slug } if slug == "test-provider")
    }));

    // 8. Structural deletion: profile blocked by prompt.
    let result = svc.delete_profile("auto-prof").await;
    assert!(matches!(result, Err(ManagementError::Conflict { .. })));

    // 9. Structural deletion: task blocked by schedule.
    let result = svc.delete_automation_task("daily-task").await;
    assert!(matches!(result, Err(ManagementError::Conflict { .. })));

    // 10. Delete schedule, then task, then prompt, then profile.
    svc.delete_scheduled_task("daily-sched").await.unwrap();
    svc.delete_automation_task("daily-task").await.unwrap();
    svc.delete_prompt(&prompt_id).await.unwrap();
    svc.delete_profile("auto-prof").await.unwrap();
    assert!(svc.get_profile("auto-prof").await.unwrap().is_none());
}

#[tokio::test]
async fn schedule_operations_without_scheduler() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let svc = ConfigManagementService::new(store);

    let (prompt_id, _) = svc
        .create_prompt("Test", "Instructions", vec![], None)
        .await
        .unwrap();

    svc.save_automation_task(&AutomationTaskInput {
        id: "task-1".to_string(),
        display_name: "Task".to_string(),
        stored_prompt_id: prompt_id,
        expected_outcome: "Done".to_string(),
        project_root: std::env::temp_dir(),
        timeout_seconds: 300,
    })
    .await
    .unwrap();

    // Schedule inspection requires an attached scheduler.
    let result = svc.inspect_all_schedules().await;
    assert!(matches!(result, Err(ManagementError::SchedulerUnavailable)));

    // Non-scheduler management still works.
    let prompts = svc.list_prompts().await.unwrap();
    assert!(!prompts.is_empty());
    assert!(prompts
        .iter()
        .all(|p| !matches!(p, ManagedPromptRecord::NeedsAttention { .. })));
}

#[tokio::test]
async fn scheduled_task_managed_record_reads() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let svc = ConfigManagementService::new(store);

    let (prompt_id, _) = svc
        .create_prompt("Test", "Instructions", vec![], None)
        .await
        .unwrap();

    svc.save_automation_task(&AutomationTaskInput {
        id: "task-1".to_string(),
        display_name: "Task".to_string(),
        stored_prompt_id: prompt_id,
        expected_outcome: "Done".to_string(),
        project_root: std::env::temp_dir(),
        timeout_seconds: 300,
    })
    .await
    .unwrap();

    svc.save_scheduled_task(&ScheduledTaskInput {
        id: "sched-1".to_string(),
        automation_task_id: "task-1".to_string(),
        cron_expression: "0 9 * * *".to_string(),
        enabled: true,
    })
    .await
    .unwrap();

    // Get returns Ready.
    let result = svc.get_scheduled_task("sched-1").await.unwrap();
    assert!(matches!(
        result,
        Some(ManagedScheduledTaskRecord::Ready(t)) if t.id == "sched-1"
    ));

    // List includes the schedule as Ready.
    let list = svc.list_scheduled_tasks().await.unwrap();
    assert!(list.iter().any(|r| matches!(
        r,
        ManagedScheduledTaskRecord::Ready(t) if t.id == "sched-1"
    )));

    // Non-existent schedule returns None.
    let result = svc.get_scheduled_task("missing").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn prompt_and_task_handle_lookup() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let svc = ConfigManagementService::new(store);

    let (prompt_id, _) = svc
        .create_prompt("Check Email", "Check inbox", vec![], None)
        .await
        .unwrap();

    // Prompt handle lookup is case-insensitive.
    let result = svc.get_prompt_by_handle("CHECK EMAIL").await.unwrap();
    assert!(result.is_some());

    svc.save_automation_task(&AutomationTaskInput {
        id: "task-1".to_string(),
        display_name: "My Task".to_string(),
        stored_prompt_id: prompt_id,
        expected_outcome: "Done".to_string(),
        project_root: std::env::temp_dir(),
        timeout_seconds: 300,
    })
    .await
    .unwrap();

    // Automation task handle lookup.
    let result = svc.get_automation_task_by_handle("MY TASK").await.unwrap();
    assert!(matches!(
        result,
        Some(ManagedAutomationTaskRecord::Ready(t)) if t.id == "task-1"
    ));
}

#[tokio::test]
async fn save_prompt_rejects_nonexistent_id() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let svc = ConfigManagementService::new(store);

    let prompt = StoredPrompt {
        display_name: "Test".to_string(),
        normalized_name: "test".to_string(),
        instructions: "Do thing".to_string(),
        skills: Vec::new(),
        profile: None,
    };
    let result = svc.save_prompt("nonexistent", &prompt).await;
    assert!(matches!(result, Err(ManagementError::Reference(_))));
}

#[tokio::test]
async fn default_profile_deletion_rejected() {
    let store = ConfigStore::open_in_memory().await.unwrap();
    let svc = ConfigManagementService::new(store);

    let result = svc.delete_profile("default").await;
    assert!(matches!(result, Err(ManagementError::Validation(_))));
}
