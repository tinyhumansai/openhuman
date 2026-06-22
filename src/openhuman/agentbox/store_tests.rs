use super::store::JobStore;
use super::types::{JobRecord, JobStatus, RunResult};
use std::time::Duration;

#[test]
fn insert_and_get_round_trip() {
    let store = JobStore::new(Duration::from_secs(3600));
    let id = store.insert_pending();
    let view = store.get(&id).expect("just inserted");
    assert_eq!(view.status, JobStatus::Pending);
    assert!(view.result.is_none());
    assert!(view.error.is_none());
}

#[test]
fn get_unknown_returns_none() {
    let store = JobStore::new(Duration::from_secs(3600));
    assert!(store.get("nope").is_none());
}

#[test]
fn mark_completed_sets_status_result_and_terminal_at() {
    let store = JobStore::new(Duration::from_secs(3600));
    let id = store.insert_pending();
    let result = RunResult {
        message: "hi".into(),
        thread_id: "t-1".into(),
    };
    store.mark_completed(&id, result.clone());
    let view = store.get(&id).unwrap();
    assert_eq!(view.status, JobStatus::Completed);
    assert_eq!(view.result, Some(result));
    assert!(view.error.is_none());
}

#[test]
fn mark_failed_sets_status_and_error() {
    let store = JobStore::new(Duration::from_secs(3600));
    let id = store.insert_pending();
    store.mark_failed(&id, "boom".into());
    let view = store.get(&id).unwrap();
    assert_eq!(view.status, JobStatus::Failed);
    assert!(view.result.is_none());
    assert_eq!(view.error.as_deref(), Some("boom"));
}

#[test]
fn mark_running_sets_status_only() {
    let store = JobStore::new(Duration::from_secs(3600));
    let id = store.insert_pending();
    store.mark_running(&id);
    let view = store.get(&id).unwrap();
    assert_eq!(view.status, JobStatus::Running);
}

#[test]
fn sweep_evicts_terminal_jobs_older_than_retention() {
    // Retention=0 means any terminal job is immediately sweepable.
    let store = JobStore::new(Duration::from_secs(0));
    let id_done = store.insert_pending();
    store.mark_completed(
        &id_done,
        RunResult {
            message: "".into(),
            thread_id: "t".into(),
        },
    );
    let id_running = store.insert_pending();
    store.mark_running(&id_running);

    let evicted = store.sweep_now();

    assert_eq!(evicted, 1);
    assert!(store.get(&id_done).is_none(), "terminal job evicted");
    assert!(store.get(&id_running).is_some(), "running job retained");
}

#[test]
fn sweep_leaves_recent_terminal_jobs() {
    let store = JobStore::new(Duration::from_secs(3600));
    let id = store.insert_pending();
    store.mark_completed(
        &id,
        RunResult {
            message: "".into(),
            thread_id: "t".into(),
        },
    );
    assert_eq!(store.sweep_now(), 0);
    assert!(store.get(&id).is_some());
}

#[test]
fn insert_pending_returns_uuid_v4_format() {
    let store = JobStore::new(Duration::from_secs(3600));
    let id = store.insert_pending();
    // v4 UUIDs are 36 chars (32 hex + 4 dashes).
    assert_eq!(id.len(), 36, "uuid v4 string length");
    assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);
}
