use super::*;
use crate::config::Config;
use crate::storage::{MemoryStorage, Scope, StorageBackend};
use std::collections::BTreeMap;

fn docs_in(storage: &MemoryStorage, scope: &str) -> Docs {
    Docs::over(&storage.for_scope(&Scope::new(scope).unwrap()).unwrap())
}

fn path() -> &'static str {
    "composio_identities.json"
}

#[test]
fn a_missing_file_loads_the_default() {
    let loaded: BTreeMap<String, u32> = docs_in(&MemoryStorage::new(), "local")
        .load(path())
        .unwrap();
    assert!(loaded.is_empty());
}

#[test]
fn save_then_load_round_trips_by_file_name() {
    let docs = docs_in(&MemoryStorage::new(), "local");
    let mut value = BTreeMap::new();
    value.insert("gmail".to_string(), 3u32);
    docs.save(path(), &value).unwrap();
    let loaded: BTreeMap<String, u32> = docs.load("composio_identities.json").unwrap();
    assert_eq!(loaded, value);
    let other: BTreeMap<String, u32> = docs.load("composio_user_scopes.json").unwrap();
    assert!(other.is_empty());
}

#[test]
fn scopes_do_not_see_each_others_state() {
    let storage = MemoryStorage::new();
    let mut value = BTreeMap::new();
    value.insert("gmail".to_string(), 1u32);
    docs_in(&storage, "alice").save(path(), &value).unwrap();
    let bob: BTreeMap<String, u32> = docs_in(&storage, "bob").load(path()).unwrap();
    assert!(bob.is_empty());
}

#[tokio::test]
async fn the_file_store_dispatches_to_documents_when_a_backend_is_pinned() {
    let storage = MemoryStorage::new();
    let docs = docs_in(&storage, "local");
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let mut value = BTreeMap::new();
    value.insert("gmail".to_string(), 7u32);
    // Pin on this thread for the awaits below (a current-thread runtime).
    super::TEST_OVERRIDE.with(|slot| *slot.borrow_mut() = Some(docs.clone()));
    super::super::file_store::save(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "composio_identities.json",
        &value,
    )
    .await
    .unwrap();
    let loaded: BTreeMap<String, u32> = super::super::file_store::load(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "composio_identities.json",
    )
    .await
    .unwrap();
    super::TEST_OVERRIDE.with(|slot| *slot.borrow_mut() = None);
    assert_eq!(loaded, value);
    assert!(!dir.path().join("storage").exists());
}

#[test]
fn update_creates_applies_and_skips_unchanged() {
    let docs = docs_in(&MemoryStorage::new(), "local");
    let added = docs
        .update(path(), |v: &mut BTreeMap<String, u32>| {
            v.insert("gmail".into(), 1);
            (v.len(), true)
        })
        .unwrap();
    assert_eq!(added, 1);
    let unchanged = docs
        .update(path(), |v: &mut BTreeMap<String, u32>| (v.len(), false))
        .unwrap();
    assert_eq!(unchanged, 1);
    let loaded: BTreeMap<String, u32> = docs.load(path()).unwrap();
    assert_eq!(loaded.get("gmail"), Some(&1));
}

#[test]
fn concurrent_updates_do_not_lose_each_other() {
    let docs = docs_in(&MemoryStorage::new(), "local");
    let handles: Vec<_> = (0..8u32)
        .map(|n| {
            let docs = docs.clone();
            std::thread::spawn(move || {
                docs.update(path(), move |v: &mut BTreeMap<String, u32>| {
                    v.insert(format!("k{n}"), n);
                    ((), true)
                })
                .unwrap();
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let loaded: BTreeMap<String, u32> = docs.load(path()).unwrap();
    assert_eq!(loaded.len(), 8);
}
