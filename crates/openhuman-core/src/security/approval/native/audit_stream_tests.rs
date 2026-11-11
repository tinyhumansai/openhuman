use super::*;
use tinystoragedrivers::{Scope, StorageBackend};

fn stream() -> AuditStream {
    let storage = tinystoragedrivers_testkit::memory_backend();
    let scoped = storage.for_scope(&Scope::new("local").unwrap()).unwrap();
    AuditStream {
        entry: Arc::new(ScopeEntry::new(
            SubjectScope {
                user: "local".into(),
                workspace: "/work".into(),
                agent: "agent".into(),
            },
            "7".repeat(32),
            super::super::rows::Rows::over(Arc::clone(scoped.documents())),
        )),
    }
}

#[tokio::test]
async fn correlations_carry_the_generation_the_committed_tail_and_a_fresh_event_id() {
    let stream = stream();
    assert_eq!(stream.stream_id(), "7".repeat(32));
    let first = stream.correlation(4).await.unwrap();
    let second = stream.correlation(4).await.unwrap();
    assert_eq!((first.generation, first.expected_tail), (4, 0));
    assert_ne!(first.event_id, second.event_id);
    assert_eq!(first.event_id.len(), 32);
    assert!(first.event_id.bytes().all(|b| b.is_ascii_hexdigit()));
}

#[tokio::test]
async fn the_stream_lock_serializes_correlation_allocation() {
    let stream = stream();
    let guard = stream.lock().await;
    let contender = stream.clone();
    let waiting = tokio::spawn(async move {
        let _guard = contender.lock().await;
    });
    tokio::task::yield_now().await;
    assert!(!waiting.is_finished());
    drop(guard);
    waiting.await.unwrap();
}

#[test]
fn debug_output_names_no_subject_or_stream() {
    let text = format!("{:?}", stream());
    assert!(!text.contains("local") && !text.contains(&"7".repeat(32)));
}
