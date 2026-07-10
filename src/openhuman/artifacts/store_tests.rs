use chrono::{TimeZone, Utc};
use tempfile::TempDir;

use super::*;
use crate::openhuman::artifacts::types::{ArtifactKind, ArtifactMeta, ArtifactStatus};

fn make_meta(id: &str, title: &str, created_at: chrono::DateTime<Utc>) -> ArtifactMeta {
    ArtifactMeta {
        id: id.to_string(),
        kind: ArtifactKind::Document,
        title: title.to_string(),
        path: format!("{id}/file.txt"),
        size_bytes: 100,
        status: ArtifactStatus::Ready,
        created_at,
        error: None,
        thread_id: None,
    }
}

#[tokio::test]
async fn save_and_get_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let meta = make_meta(
        "test-id-1",
        "My Document",
        Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap(),
    );
    save_artifact_meta(tmp.path(), &meta).await.unwrap();
    let got = get_artifact(tmp.path(), "test-id-1").await.unwrap();
    assert_eq!(got.id, meta.id);
    assert_eq!(got.title, meta.title);
    assert_eq!(got.kind, meta.kind);
    assert_eq!(got.status, meta.status);
    assert_eq!(got.size_bytes, meta.size_bytes);
    assert_eq!(got.created_at, meta.created_at);
}

#[tokio::test]
async fn list_returns_saved_items_sorted_by_created_at() {
    let tmp = TempDir::new().unwrap();

    let t1 = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();
    let t3 = Utc.with_ymd_and_hms(2025, 12, 1, 0, 0, 0).unwrap();

    save_artifact_meta(tmp.path(), &make_meta("a", "A", t1))
        .await
        .unwrap();
    save_artifact_meta(tmp.path(), &make_meta("b", "B", t3))
        .await
        .unwrap();
    save_artifact_meta(tmp.path(), &make_meta("c", "C", t2))
        .await
        .unwrap();

    let (items, total) = list_artifacts(tmp.path(), 0, 100, None).await.unwrap();
    assert_eq!(total, 3);
    assert_eq!(items.len(), 3);
    // Newest first
    assert_eq!(items[0].id, "b");
    assert_eq!(items[1].id, "c");
    assert_eq!(items[2].id, "a");
}

#[tokio::test]
async fn list_empty_workspace() {
    let tmp = TempDir::new().unwrap();
    let (items, total) = list_artifacts(tmp.path(), 0, 50, None).await.unwrap();
    assert_eq!(total, 0);
    assert!(items.is_empty());
}

#[tokio::test]
async fn list_pagination() {
    let tmp = TempDir::new().unwrap();

    for i in 0..5_u32 {
        let ts = Utc
            .with_ymd_and_hms(2025, 1, i as u32 + 1, 0, 0, 0)
            .unwrap();
        save_artifact_meta(tmp.path(), &make_meta(&format!("id-{i}"), "x", ts))
            .await
            .unwrap();
    }

    let (items, total) = list_artifacts(tmp.path(), 1, 2, None).await.unwrap();
    assert_eq!(total, 5);
    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn delete_removes_directory_and_meta() {
    let tmp = TempDir::new().unwrap();
    let meta = make_meta(
        "del-id",
        "Delete Me",
        Utc.with_ymd_and_hms(2025, 3, 1, 0, 0, 0).unwrap(),
    );
    save_artifact_meta(tmp.path(), &meta).await.unwrap();

    // Confirm it exists
    get_artifact(tmp.path(), "del-id").await.unwrap();

    delete_artifact(tmp.path(), "del-id").await.unwrap();

    // Should now be gone
    let err = get_artifact(tmp.path(), "del-id").await.unwrap_err();
    assert!(
        err.contains("not found") || err.contains("No such file"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn delete_nonexistent_returns_error() {
    let tmp = TempDir::new().unwrap();
    let err = delete_artifact(tmp.path(), "nonexistent-id")
        .await
        .unwrap_err();
    assert!(
        err.contains("failed to delete") || err.contains("No such file"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn get_rejects_path_traversal() {
    let tmp = TempDir::new().unwrap();
    for bad_id in ["../secrets", "foo/../bar"] {
        let err = get_artifact(tmp.path(), bad_id).await.unwrap_err();
        assert!(
            err.contains("must not contain")
                || err.contains("traversal")
                || err.contains("escapes"),
            "id={bad_id:?} error was: {err}"
        );
    }
}

#[tokio::test]
async fn get_rejects_absolute_paths() {
    let tmp = TempDir::new().unwrap();
    let err = get_artifact(tmp.path(), "/tmp/evil").await.unwrap_err();
    assert!(
        err.contains("must not contain") || err.contains("absolute") || err.contains("escapes"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn list_skips_corrupt_meta() {
    let tmp = TempDir::new().unwrap();

    // Write a valid artifact
    let ts = Utc.with_ymd_and_hms(2025, 5, 1, 0, 0, 0).unwrap();
    save_artifact_meta(tmp.path(), &make_meta("good-id", "Good", ts))
        .await
        .unwrap();

    // Create a subdirectory with invalid JSON as meta.json
    let corrupt_dir = tmp.path().join("artifacts").join("corrupt-id");
    std::fs::create_dir_all(&corrupt_dir).unwrap();
    std::fs::write(corrupt_dir.join("meta.json"), b"this is not json").unwrap();

    let (items, total) = list_artifacts(tmp.path(), 0, 100, None).await.unwrap();
    // Only the valid one should be returned
    assert_eq!(total, 1);
    assert_eq!(items[0].id, "good-id");
}

#[tokio::test]
async fn validate_artifact_id_rejects_dot() {
    let tmp = TempDir::new().unwrap();
    let err = get_artifact(tmp.path(), ".").await.unwrap_err();
    assert!(err.contains("must not be '.'"), "unexpected error: {err}");
}

#[tokio::test]
async fn validate_artifact_id_rejects_slashes() {
    let tmp = TempDir::new().unwrap();
    let err = get_artifact(tmp.path(), "a/b").await.unwrap_err();
    assert!(
        err.contains("must not contain '/'"),
        "unexpected error: {err}"
    );

    let err = get_artifact(tmp.path(), "a\\b").await.unwrap_err();
    assert!(
        err.contains("must not contain '\\'"),
        "unexpected error: {err}"
    );
}

// ── create_artifact event publication (#3162) ─────────────────────────────

use crate::core::event_bus::{
    init_global, subscribe_global, DomainEvent, EventHandler, SubscriptionHandle,
};
use async_trait::async_trait;
use std::sync::{Arc, Mutex as StdMutex};

#[derive(Clone)]
struct PendingCollector {
    events: Arc<StdMutex<Vec<DomainEvent>>>,
}

impl PendingCollector {
    fn new() -> Self {
        Self {
            events: Arc::new(StdMutex::new(Vec::new())),
        }
    }

    fn subscribe(&self) -> Option<SubscriptionHandle> {
        subscribe_global(Arc::new(self.clone()))
    }

    fn snapshot(&self) -> Vec<DomainEvent> {
        self.events.lock().unwrap().clone()
    }
}

#[async_trait]
impl EventHandler for PendingCollector {
    fn name(&self) -> &str {
        "test::pending_collector"
    }

    /// Filter at the bus boundary so the broadcast channel never delivers
    /// non-artifact traffic to this subscriber — keeps the per-test
    /// receive buffer small even when other tests pump unrelated events
    /// in parallel.
    fn domains(&self) -> Option<&[&str]> {
        Some(&["artifact"])
    }

    async fn handle(&self, event: &DomainEvent) {
        // domains() filter guarantees only artifact-domain variants
        // arrive, but match defensively in case the enum grows.
        if matches!(
            event,
            DomainEvent::ArtifactPending { .. }
                | DomainEvent::ArtifactReady { .. }
                | DomainEvent::ArtifactFailed { .. }
        ) {
            self.events.lock().unwrap().push(event.clone());
        }
    }
}

/// #3162: `create_artifact` publishes `DomainEvent::ArtifactPending`
/// the moment the row is reserved, so the chat surface can render an
/// in-progress "Generating…" card before the file lands on disk.
#[tokio::test]
async fn create_artifact_publishes_artifact_pending_event() {
    init_global(256);
    let collector = PendingCollector::new();
    let _handle = collector.subscribe();

    let tmp = TempDir::new().unwrap();
    let (meta, _path) = create_artifact(tmp.path(), ArtifactKind::Presentation, "Q3 Deck", "pptx")
        .await
        .expect("create_artifact succeeds");
    let expected_workspace = tmp.path().to_string_lossy().into_owned();

    // The bus is broadcast-based and processed off-task — wait until the
    // subscriber's tokio task delivers OUR event. Filtering by
    // artifact_id + workspace_dir keeps the test robust against parallel
    // `cargo test` runs that may publish unrelated artifact lifecycle
    // events into the same process-wide bus.
    let matches_this_artifact = |event: &DomainEvent| {
        matches!(
            event,
            DomainEvent::ArtifactPending {
                artifact_id,
                workspace_dir,
                ..
            } if artifact_id == &meta.id && workspace_dir == &expected_workspace
        )
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if collector.snapshot().iter().any(matches_this_artifact) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "ArtifactPending event for id={} was not observed within 2s",
                meta.id
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let mine: Vec<DomainEvent> = collector
        .snapshot()
        .into_iter()
        .filter(matches_this_artifact)
        .collect();
    assert_eq!(
        mine.len(),
        1,
        "exactly one ArtifactPending for {} expected, got {mine:?}",
        meta.id
    );
    let DomainEvent::ArtifactPending {
        artifact_id,
        kind,
        title,
        workspace_dir,
        path,
        thread_id,
        client_id,
    } = &mine[0]
    else {
        unreachable!("filter pinned us to ArtifactPending");
    };
    assert_eq!(*artifact_id, meta.id);
    assert_eq!(kind, "presentation");
    assert_eq!(title, "Q3 Deck");
    assert_eq!(*workspace_dir, expected_workspace);
    assert_eq!(*path, meta.path);
    // No chat context bound on this test task, so the routing fields are
    // None — the web bridge drops the event in that case, which is the
    // intended degradation path for CLI / cron / sub-agent callers.
    assert!(thread_id.is_none(), "thread_id leaked, got {thread_id:?}");
    assert!(client_id.is_none(), "client_id leaked, got {client_id:?}");
}

// ── args sidecar + regenerate id reuse (#3162) ────────────────────────────

#[tokio::test]
async fn save_and_read_args_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let args = serde_json::json!({
        "title": "Q3 Deck",
        "slides": [{ "heading": "Intro", "bullets": ["a", "b"] }],
    });
    save_artifact_args(tmp.path(), "deck-1", &args)
        .await
        .unwrap();
    let got = read_artifact_args(tmp.path(), "deck-1").await.unwrap();
    assert_eq!(got, args);
}

#[tokio::test]
async fn read_args_errors_when_absent() {
    let tmp = TempDir::new().unwrap();
    // No args.json written → not regenerable, surfaces an Err rather than
    // a silent empty value.
    let err = read_artifact_args(tmp.path(), "missing").await.unwrap_err();
    assert!(err.contains("not regenerable"), "unexpected error: {err}");
}

#[tokio::test]
async fn create_artifact_mints_fresh_id_without_scope() {
    let tmp = TempDir::new().unwrap();
    let (meta, _path) = create_artifact(tmp.path(), ArtifactKind::Presentation, "Q3 Deck", "pptx")
        .await
        .unwrap();
    // A normal (non-regenerate) create mints a UUID, never an empty id.
    assert!(!meta.id.is_empty());
    assert_eq!(meta.status, ArtifactStatus::Pending);
}

#[tokio::test]
async fn create_artifact_reuses_id_inside_regenerate_scope() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().to_path_buf();
    let (meta, _path) = REGENERATE_TARGET_ID
        .scope("reused-id".to_string(), async move {
            create_artifact(&workspace, ArtifactKind::Presentation, "Q3 Deck", "pptx").await
        })
        .await
        .unwrap();
    // The scoped target id is reused verbatim so the card swaps in place.
    assert_eq!(meta.id, "reused-id");
    // And the meta is actually persisted under that id.
    let got = get_artifact(tmp.path(), "reused-id").await.unwrap();
    assert_eq!(got.id, "reused-id");
}

#[tokio::test]
async fn regenerate_preserves_original_created_at() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().to_path_buf();

    // First create stamps `created_at = now`.
    let (first, _) = create_artifact(tmp.path(), ArtifactKind::Presentation, "Deck", "pptx")
        .await
        .unwrap();
    let original_created = first.created_at;

    // Regenerate reuses the id; created_at must NOT be bumped, otherwise the
    // artifact jumps to the top of the created_at-sorted list (#3162).
    let id = first.id.clone();
    let ws = workspace.clone();
    let (second, _) = REGENERATE_TARGET_ID
        .scope(id.clone(), async move {
            create_artifact(&ws, ArtifactKind::Presentation, "Deck", "pptx").await
        })
        .await
        .unwrap();
    assert_eq!(second.id, id);
    assert_eq!(
        second.created_at, original_created,
        "regenerate must preserve created_at, not bump it"
    );
}
