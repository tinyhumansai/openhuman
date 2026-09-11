use super::*;
use tempfile::NamedTempFile;

fn open_tmp() -> (WatcherStateStore, NamedTempFile) {
    let f = NamedTempFile::new().unwrap();
    let store = WatcherStateStore::open(f.path()).unwrap();
    (store, f)
}

#[test]
fn record_and_retrieve_mtime() {
    let (mut store, _f) = open_tmp();
    let p = Path::new("/vault/note.md");
    store.record_seen(p, 1_700_000_000).unwrap();
    assert_eq!(store.last_mtime(p).unwrap(), Some(1_700_000_000));
}

#[test]
fn unknown_path_returns_none() {
    let (store, _f) = open_tmp();
    assert_eq!(
        store.last_mtime(Path::new("/vault/missing.md")).unwrap(),
        None
    );
}

#[test]
fn deleted_path_returns_none() {
    let (mut store, _f) = open_tmp();
    let p = Path::new("/vault/gone.md");
    store.record_seen(p, 1_700_000_000).unwrap();
    store.record_deleted(p).unwrap();
    assert_eq!(store.last_mtime(p).unwrap(), None);
}

#[test]
fn upsert_updates_mtime() {
    let (mut store, _f) = open_tmp();
    let p = Path::new("/vault/updated.md");
    store.record_seen(p, 1_000).unwrap();
    store.record_seen(p, 2_000).unwrap();
    assert_eq!(store.last_mtime(p).unwrap(), Some(2_000));
}

#[test]
fn load_all_excludes_deleted() {
    let (mut store, _f) = open_tmp();
    store.record_seen(Path::new("/vault/a.md"), 1_000).unwrap();
    store.record_seen(Path::new("/vault/b.md"), 2_000).unwrap();
    store.record_deleted(Path::new("/vault/c.md")).unwrap();
    let rows = store.load_all().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| !r.deleted));
}

#[test]
fn reopen_persists_state() {
    let f = NamedTempFile::new().unwrap();
    let db_path = f.path().to_owned();
    {
        let mut store = WatcherStateStore::open(&db_path).unwrap();
        store
            .record_seen(Path::new("/vault/persist.md"), 42_000)
            .unwrap();
        store.record_deleted(Path::new("/vault/gone.md")).unwrap();
    }
    let store = WatcherStateStore::open(&db_path).unwrap();
    assert_eq!(
        store.last_mtime(Path::new("/vault/persist.md")).unwrap(),
        Some(42_000)
    );
    assert_eq!(store.last_mtime(Path::new("/vault/gone.md")).unwrap(), None);
    let all = store.load_all().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].path, Path::new("/vault/persist.md"));
}
