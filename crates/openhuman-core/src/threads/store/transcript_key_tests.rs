use super::*;
use crate::core::runtime::DispatchCtx;

fn store(dir: &tempfile::TempDir) -> ConversationStore {
    ConversationStore::new(DispatchCtx::unbound(), dir.path().to_path_buf())
}

#[test]
fn a_thread_with_no_record_has_no_transcript_key() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(store(&dir).transcript_key("thread-a").unwrap(), None);
}

#[test]
fn the_first_recorded_key_wins() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    assert_eq!(
        store
            .record_transcript_key("thread-a", "orchestrator")
            .unwrap(),
        "orchestrator"
    );
    // A later session under another chat agent keeps the original key.
    assert_eq!(
        store
            .record_transcript_key("thread-a", "researcher")
            .unwrap(),
        "orchestrator"
    );
    assert_eq!(
        store.transcript_key("thread-a").unwrap().as_deref(),
        Some("orchestrator")
    );
    // Another thread is independent.
    assert_eq!(store.transcript_key("thread-b").unwrap(), None);
}

#[test]
fn deleting_a_thread_forgets_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    store
        .ensure_thread(tinyagents_session::threads::CreateConversationThread {
            id: "thread-a".into(),
            title: "A".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        })
        .unwrap();
    store
        .record_transcript_key("thread-a", "orchestrator")
        .unwrap();
    store
        .record_transcript_key("thread-b", "orchestrator")
        .unwrap();

    assert!(store
        .delete_thread("thread-a", "2026-01-02T00:00:00Z")
        .unwrap());
    assert_eq!(store.transcript_key("thread-a").unwrap(), None);
    assert!(store.transcript_key("thread-b").unwrap().is_some());

    store.purge_threads().unwrap();
    assert_eq!(store.transcript_key("thread-b").unwrap(), None);
    // A recreated thread records afresh.
    assert_eq!(
        store
            .record_transcript_key("thread-a", "researcher")
            .unwrap(),
        "researcher"
    );
}

#[tokio::test]
async fn bind_returns_the_recorded_key_and_records_the_first_one() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = DispatchCtx::unbound();
    let first = bind_transcript_key(&ctx, dir.path(), "thread-a", "orchestrator".into()).await;
    let second = bind_transcript_key(&ctx, dir.path(), "thread-a", "researcher".into()).await;
    assert_eq!(first, "orchestrator");
    assert_eq!(second, "orchestrator");
}
