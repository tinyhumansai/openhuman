use super::*;
use crate::artifacts::store::{
    delete_artifact, get_artifact, list_artifacts, read_artifact_args, save_artifact_args,
    save_artifact_meta,
};
use crate::artifacts::types::{ArtifactKind, ArtifactStatus};
use crate::artifacts::FileRoots;
use crate::storage::{test_support::memory_backend, Scope, StorageBackend};
use chrono::{TimeZone, Utc};

fn docs_in(storage: &dyn StorageBackend, scope: &str) -> Docs {
    Docs::over(&storage.for_scope(&Scope::new(scope).unwrap()).unwrap())
}

fn meta(id: &str, month: u32, thread: Option<&str>) -> ArtifactMeta {
    ArtifactMeta {
        id: id.to_string(),
        kind: ArtifactKind::Document,
        title: format!("title {id}"),
        path: format!("{id}.txt"),
        file: None,
        file_root: None,
        size_bytes: 10,
        status: ArtifactStatus::Ready,
        created_at: Utc.with_ymd_and_hms(2025, month, 1, 0, 0, 0).unwrap(),
        error: None,
        thread_id: thread.map(str::to_string),
        tool_call_id: None,
    }
}

#[test]
fn records_round_trip_newest_first() {
    let storage = memory_backend();
    let docs = docs_in(storage.as_ref(), "local");
    docs.put_meta(&meta("a", 1, None)).unwrap();
    docs.put_meta(&meta("c", 3, None)).unwrap();
    docs.put_meta(&meta("b", 2, None)).unwrap();
    let ids: Vec<_> = docs
        .list_meta()
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, ["c", "b", "a"]);
    assert_eq!(docs.get_meta("b").unwrap().unwrap().title, "title b");
    assert!(docs.get_meta("missing").unwrap().is_none());
}

#[test]
fn args_and_delete() {
    let storage = memory_backend();
    let docs = docs_in(storage.as_ref(), "local");
    docs.put_meta(&meta("a", 1, None)).unwrap();
    docs.put_args("a", &serde_json::json!({ "k": 1 })).unwrap();
    assert_eq!(docs.get_args("a").unwrap().unwrap()["k"], 1);
    assert!(docs.delete("a").unwrap());
    assert!(docs.get_meta("a").unwrap().is_none());
    assert!(docs.get_args("a").unwrap().is_none());
    assert!(!docs.delete("a").unwrap());
}

#[test]
fn scopes_do_not_see_each_others_artifacts() {
    let storage = memory_backend();
    docs_in(storage.as_ref(), "alice")
        .put_meta(&meta("a", 1, None))
        .unwrap();
    assert!(docs_in(storage.as_ref(), "bob")
        .list_meta()
        .unwrap()
        .is_empty());
    assert_eq!(
        docs_in(storage.as_ref(), "alice")
            .list_meta()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn the_legacy_files_are_imported_without_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::write(
        a.join("meta.json"),
        serde_json::to_string(&meta("a", 1, Some("t"))).unwrap(),
    )
    .unwrap();
    std::fs::write(a.join("args.json"), r#"{"x":2}"#).unwrap();
    let b = dir.path().join("b");
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(
        b.join("meta.json"),
        serde_json::to_string(&meta("b", 2, None)).unwrap(),
    )
    .unwrap();
    let bad = dir.path().join("bad");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("meta.json"), "{nope").unwrap();
    std::fs::create_dir_all(dir.path().join("empty")).unwrap();
    // A copy under another folder name is not the record.
    let copy = dir.path().join("copy-of-a");
    std::fs::create_dir_all(&copy).unwrap();
    let stray = meta("z", 1, Some("t"));
    std::fs::write(
        copy.join("meta.json"),
        serde_json::to_string(&stray).unwrap(),
    )
    .unwrap();
    std::fs::write(copy.join("args.json"), r#"{"x":5}"#).unwrap();

    let storage = memory_backend();
    let docs = docs_in(storage.as_ref(), "local");
    // A record and arguments written before the import are not overwritten.
    let mut existing = meta("a", 1, Some("t"));
    existing.title = "kept".into();
    docs.put_meta(&existing).unwrap();
    docs.put_args("a", &serde_json::json!({ "x": 9 })).unwrap();
    assert_eq!(docs.import_legacy(dir.path()).unwrap(), 1, "only b is new");
    assert_eq!(docs.get_meta("a").unwrap().unwrap().title, "kept");
    assert_eq!(docs.get_args("a").unwrap().unwrap()["x"], 9);
    assert!(docs.get_meta("b").unwrap().is_some());
    assert!(
        docs.get_meta("z").unwrap().is_none(),
        "folder name must match"
    );

    // Repeating it creates nothing.
    assert_eq!(docs.import_legacy(dir.path()).unwrap(), 0);
    assert_eq!(docs.import_legacy(&dir.path().join("none")).unwrap(), 0);
}

#[test]
fn a_legacy_folder_that_cannot_be_read_fails_the_import() {
    let dir = tempfile::tempdir().unwrap();
    let not_a_dir = dir.path().join("artifacts");
    std::fs::write(&not_a_dir, "a file").unwrap();
    let storage = memory_backend();
    let docs = docs_in(storage.as_ref(), "local");
    assert!(docs.import_legacy(&not_a_dir).is_err());

    // An unreadable sidecar (here a directory named args.json) fails it too.
    let a = dir.path().join("legacy/a");
    std::fs::create_dir_all(a.join("args.json")).unwrap();
    std::fs::write(
        a.join("meta.json"),
        serde_json::to_string(&meta("a", 1, None)).unwrap(),
    )
    .unwrap();
    assert!(docs.import_legacy(&dir.path().join("legacy")).is_err());
}

#[test]
fn delete_removes_the_record_before_its_arguments() {
    let storage = memory_backend();
    let docs = docs_in(storage.as_ref(), "local");
    docs.put_args("orphan", &serde_json::json!({})).unwrap();
    // No record: reported absent, and the orphaned arguments are swept.
    assert!(!docs.delete("orphan").unwrap());
    assert!(docs.get_args("orphan").unwrap().is_none());
}

#[tokio::test]
async fn the_store_dispatches_to_documents_when_a_backend_is_pinned() {
    let storage = memory_backend();
    let docs = docs_in(storage.as_ref(), "local");
    let workspace = tempfile::tempdir().unwrap();
    let ws = workspace.path();
    super::TEST_OVERRIDE.with(|slot| *slot.borrow_mut() = Some(docs.clone()));

    save_artifact_meta(ws, &meta("a", 1, Some("t1")))
        .await
        .unwrap();
    save_artifact_meta(ws, &meta("b", 2, Some("t2")))
        .await
        .unwrap();
    assert_eq!(get_artifact(ws, "a").await.unwrap().id, "a");
    assert!(get_artifact(ws, "zz").await.is_err());
    let (page, total) = list_artifacts(ws, 0, 10, Some("t1")).await.unwrap();
    assert_eq!((page.len(), total), (1, 1));
    let (all, _) = list_artifacts(ws, 0, 10, None).await.unwrap();
    assert_eq!(all[0].id, "b");

    save_artifact_args(ws, "a", &serde_json::json!({ "n": 1 }))
        .await
        .unwrap();
    assert_eq!(read_artifact_args(ws, "a").await.unwrap()["n"], 1);
    assert!(read_artifact_args(ws, "b").await.is_err());

    std::fs::create_dir_all(ws.join("artifacts/a")).unwrap();
    std::fs::write(ws.join("artifacts/a/meta.json"), "{}").unwrap();
    delete_artifact(ws, FileRoots::from(ws.to_path_buf()), "a")
        .await
        .unwrap();
    let missing = delete_artifact(ws, FileRoots::from(ws.to_path_buf()), "a")
        .await
        .unwrap_err();
    assert!(missing.contains("not found"), "{missing}");
    super::TEST_OVERRIDE.with(|slot| *slot.borrow_mut() = None);

    assert!(docs.get_meta("a").unwrap().is_none());
    assert!(docs.get_meta("b").unwrap().is_some());
    assert!(!ws.join("artifacts/a/meta.json").exists());
}
