use super::*;
use serde_json::json;
use sha2::Digest;
use tinystoragedrivers::{MemoryStorage, Precondition, Scope, StorageBackend};

fn scoped() -> ScopedStorage {
    MemoryStorage::new()
        .for_scope(&Scope::new("module:tinybox@local").unwrap())
        .unwrap()
}

const INDEX: &str = r#"{"jails":{"j1":{"id":"j1","label":"build","dir":"/w/jails/j1","backend_at_create":"landlock","created_at_unix":10,"updated_at_unix":20,"notes":"keep"}}}"#;

fn legacy_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join(BASE_DIR);
    std::fs::create_dir_all(base.join("j1")).unwrap();
    std::fs::write(base.join(INDEX_FILE), INDEX).unwrap();
    dir
}

#[tokio::test]
async fn the_jail_index_moves_into_one_document_with_the_same_shape() {
    let legacy = legacy_workspace();
    let scoped = scoped();
    let outcome = import_into(&scoped, &legacy.path().join(BASE_DIR))
        .await
        .unwrap();
    assert_eq!(outcome, Outcome::Imported { written: true });
    let stored = scoped
        .documents()
        .get(JAIL_INDEX_COLLECTION, "jails")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.doc, serde_json::from_str::<Value>(INDEX).unwrap());
}

#[tokio::test]
async fn the_legacy_index_is_left_untouched() {
    let legacy = legacy_workspace();
    let file = legacy.path().join(BASE_DIR).join(INDEX_FILE);
    let before = sha2::Sha256::digest(std::fs::read(&file).unwrap());
    import_into(&scoped(), &legacy.path().join(BASE_DIR))
        .await
        .unwrap();
    assert_eq!(sha2::Sha256::digest(std::fs::read(&file).unwrap()), before);
}

#[tokio::test]
async fn an_index_the_module_already_wrote_is_kept_and_a_rerun_is_a_noop() {
    let legacy = legacy_workspace();
    let base = legacy.path().join(BASE_DIR);
    let scoped = scoped();
    scoped
        .documents()
        .ensure_collection(&CollectionSpec::new(JAIL_INDEX_COLLECTION))
        .await
        .unwrap();
    scoped
        .documents()
        .put(
            JAIL_INDEX_COLLECTION,
            "jails",
            json!({"jails": {}}),
            Precondition::Absent,
        )
        .await
        .unwrap();
    assert_eq!(
        import_into(&scoped, &base).await.unwrap(),
        Outcome::Imported { written: false }
    );
    let stored = scoped
        .documents()
        .get(JAIL_INDEX_COLLECTION, "jails")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.doc, json!({"jails": {}}));
    assert_eq!(
        import_into(&scoped, &base).await.unwrap(),
        Outcome::AlreadyDone
    );
}

#[tokio::test]
async fn a_workspace_without_an_index_writes_no_marker() {
    let empty = tempfile::tempdir().unwrap();
    let scoped = scoped();
    assert_eq!(
        import_into(&scoped, &empty.path().join(BASE_DIR))
            .await
            .unwrap(),
        Outcome::NoSource
    );
    let legacy = legacy_workspace();
    assert_eq!(
        import_into(&scoped, &legacy.path().join(BASE_DIR))
            .await
            .unwrap(),
        Outcome::Imported { written: true }
    );
}

#[tokio::test]
async fn an_index_that_is_not_a_json_object_is_skipped_not_retried_forever() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join(BASE_DIR);
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join(INDEX_FILE), "[1, 2]").unwrap();
    let scoped = scoped();
    assert_eq!(
        import_into(&scoped, &base).await.unwrap(),
        Outcome::Imported { written: false }
    );
    assert_eq!(
        import_into(&scoped, &base).await.unwrap(),
        Outcome::AlreadyDone
    );
}
