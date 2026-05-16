use tempfile::tempdir;

use super::store::CanvasTrackerStore;
use super::types::{CanvasTask, LocalStatus, SyncSummary, UrgencyLevel};

fn task(id: &str, status: LocalStatus) -> CanvasTask {
    CanvasTask {
        course_id: "101".into(),
        course_name: "361100-Secrets of the Soil-Lec.001 | 801[3/68]".into(),
        assignment_id: id.into(),
        assignment_name: format!("Assignment {id}"),
        due_at: Some("2026-05-20T06:00:00Z".into()),
        due_at_unclear: false,
        instructions_summary: "Submit a PDF.".into(),
        submission_type: Some("online_upload".into()),
        canvas_workflow_state: Some("published".into()),
        canvas_submission_state: None,
        local_status: status,
        urgency_level: UrgencyLevel::Medium,
        recommended_start_at: Some("2026-05-18T06:00:00Z".into()),
        reminders_needed: vec![],
        source_url: Some("/courses/101/assignments/55".into()),
        last_seen_at: "2026-05-16T06:00:00Z".into(),
    }
}

fn task_with_status_and_urgency(
    id: &str,
    status: LocalStatus,
    urgency: UrgencyLevel,
) -> CanvasTask {
    let mut task = task(id, status);
    task.urgency_level = urgency;
    task
}

#[test]
fn settings_round_trip_uses_defaults() {
    let temp = tempdir().unwrap();
    let store = CanvasTrackerStore::new(temp.path()).unwrap();
    let settings = store.get_settings().unwrap();

    assert!(settings.enabled);
    assert_eq!(settings.allowlisted_courses.len(), 2);
    assert_eq!(settings.host, "https://mango-cmu.instructure.com");
}

#[test]
fn task_upsert_preserves_existing_local_status() {
    let temp = tempdir().unwrap();
    let store = CanvasTrackerStore::new(temp.path()).unwrap();

    store
        .upsert_tasks(&[task("55", LocalStatus::NotStarted)])
        .unwrap();
    store
        .update_local_status("101", "55", LocalStatus::InProgress)
        .unwrap();
    store
        .upsert_tasks(&[task("55", LocalStatus::NotStarted)])
        .unwrap();

    let rows = store.list_tasks().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].local_status, LocalStatus::InProgress);
}

#[test]
fn task_upsert_allows_incoming_submitted_to_override_local_status() {
    let temp = tempdir().unwrap();
    let store = CanvasTrackerStore::new(temp.path()).unwrap();

    store
        .upsert_tasks(&[task("55", LocalStatus::NotStarted)])
        .unwrap();
    store
        .update_local_status("101", "55", LocalStatus::InProgress)
        .unwrap();
    store
        .upsert_tasks(&[task("55", LocalStatus::Submitted)])
        .unwrap();

    let rows = store.list_tasks().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].local_status, LocalStatus::Submitted);
}

#[test]
fn update_local_status_errors_for_missing_task() {
    let temp = tempdir().unwrap();
    let store = CanvasTrackerStore::new(temp.path()).unwrap();

    let result = store.update_local_status("101", "missing", LocalStatus::InProgress);

    assert!(result.is_err());
}

#[test]
fn record_sync_run_stores_summary() {
    let temp = tempdir().unwrap();
    let store = CanvasTrackerStore::new(temp.path()).unwrap();
    let summary = SyncSummary {
        synced: true,
        courses_seen: 4,
        courses_used: 2,
        courses_ignored: 2,
        assignments_seen: 8,
        tasks_upserted: 3,
        previous_tasks_preserved: true,
        errors: vec!["skipped unpublished assignment".into()],
        synced_at: "2026-05-16T06:30:00Z".into(),
    };

    store.record_sync_run(&summary).unwrap();

    assert_eq!(store.sync_run_count().unwrap(), 1);
}

#[test]
fn task_upsert_round_trips_all_local_status_and_urgency_variants() {
    let temp = tempdir().unwrap();
    let store = CanvasTrackerStore::new(temp.path()).unwrap();
    let cases = [
        (LocalStatus::NotStarted, UrgencyLevel::Critical),
        (LocalStatus::InProgress, UrgencyLevel::High),
        (LocalStatus::Waiting, UrgencyLevel::Medium),
        (LocalStatus::Submitted, UrgencyLevel::Low),
        (LocalStatus::Done, UrgencyLevel::Unclear),
        (LocalStatus::Unclear, UrgencyLevel::Critical),
    ];
    let tasks: Vec<_> = cases
        .iter()
        .enumerate()
        .map(|(index, (status, urgency))| {
            task_with_status_and_urgency(&format!("variant-{index}"), *status, *urgency)
        })
        .collect();

    store.upsert_tasks(&tasks).unwrap();
    let rows = store.list_tasks().unwrap();

    for task in tasks {
        let row = rows
            .iter()
            .find(|row| row.assignment_id == task.assignment_id)
            .unwrap();
        assert_eq!(row.local_status, task.local_status);
        assert_eq!(row.urgency_level, task.urgency_level);
    }
}
