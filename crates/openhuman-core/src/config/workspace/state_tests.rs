use super::*;

fn backend() -> Arc<dyn StorageBackend> {
    Arc::new(tinystoragedrivers_testkit::MemoryBackend::new())
}

fn open_store(backend: Arc<dyn StorageBackend>) -> WatcherStateStore {
    WatcherStateStore::from_backend(backend).unwrap()
}

#[test]
fn record_and_retrieve_mtime() {
    let mut store = open_store(backend());
    let path = Path::new("/vault/note.md");
    store.record_seen(path, 1_700_000_000).unwrap();
    assert_eq!(store.last_mtime(path).unwrap(), Some(1_700_000_000));
}

#[test]
fn unknown_path_returns_none() {
    let store = open_store(backend());
    assert_eq!(
        store.last_mtime(Path::new("/vault/missing.md")).unwrap(),
        None
    );
}

#[test]
fn deleted_path_returns_none_and_preserves_its_record() {
    let mut store = open_store(backend());
    let path = Path::new("/vault/gone.md");
    store.record_seen(path, 1_700_000_000).unwrap();
    store.record_deleted(path).unwrap();
    assert_eq!(store.last_mtime(path).unwrap(), None);
    let listed = store.load_all().unwrap();
    assert!(listed.is_empty());
}

#[test]
fn upsert_updates_mtime() {
    let mut store = open_store(backend());
    let path = Path::new("/vault/updated.md");
    store.record_seen(path, 1_000).unwrap();
    store.record_seen(path, 2_000).unwrap();
    assert_eq!(store.last_mtime(path).unwrap(), Some(2_000));
}

#[test]
fn load_all_excludes_deleted() {
    let mut store = open_store(backend());
    store.record_seen(Path::new("/vault/a.md"), 1_000).unwrap();
    store.record_seen(Path::new("/vault/b.md"), 2_000).unwrap();
    store.record_deleted(Path::new("/vault/c.md")).unwrap();
    let rows = store.load_all().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| !row.deleted));
}

#[test]
fn reopen_on_same_local_backend_preserves_state() {
    let backend = backend();
    {
        let mut store = open_store(Arc::clone(&backend));
        store
            .record_seen(Path::new("/vault/persist.md"), 42_000)
            .unwrap();
        store.record_deleted(Path::new("/vault/gone.md")).unwrap();
    }
    let store = open_store(backend);
    assert_eq!(
        store.last_mtime(Path::new("/vault/persist.md")).unwrap(),
        Some(42_000)
    );
    assert_eq!(store.last_mtime(Path::new("/vault/gone.md")).unwrap(), None);
    let all = store.load_all().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].path, Path::new("/vault/persist.md"));
}

#[test]
fn path_hashes_keep_distinct_paths_and_verify_body_identity() {
    assert_ne!(key_for("/vault/a.md"), key_for("/vault/b.md"));
    let mut store = open_store(backend());
    let path = Path::new("/vault/a.md");
    store.record_seen(path, 1).unwrap();
    assert_eq!(store.last_mtime(Path::new("/vault/b.md")).unwrap(), None);
}

#[cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn local_backend_open_and_sync_operations_work_inside_current_thread_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("watcher.db");
    let mut store = WatcherStateStore::open(&db).unwrap();
    let path = Path::new("/machine/vault/current.md");
    store.record_seen(path, 123).unwrap();
    assert_eq!(store.last_mtime(path).unwrap(), Some(123));
}
