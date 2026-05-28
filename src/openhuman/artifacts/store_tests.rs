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

    let (items, total) = list_artifacts(tmp.path(), 0, 100).await.unwrap();
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
    let (items, total) = list_artifacts(tmp.path(), 0, 50).await.unwrap();
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

    let (items, total) = list_artifacts(tmp.path(), 1, 2).await.unwrap();
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

    let (items, total) = list_artifacts(tmp.path(), 0, 100).await.unwrap();
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
