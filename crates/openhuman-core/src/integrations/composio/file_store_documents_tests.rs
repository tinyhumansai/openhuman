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

#[test]
fn values_are_tsd_kv_entries_that_kv_itself_reads_and_updates_in_place() {
    let storage = MemoryStorage::new();
    let docs = docs_in(&storage, "local");
    let mut value = BTreeMap::new();
    value.insert("gmail".to_string(), 1u32);
    docs.save(path(), &value).unwrap();

    // `update` writes the same document `Kv` reads, so the two never drift.
    let bumped = docs
        .update::<BTreeMap<String, u32>, u32>(path(), |map| {
            let next = map.get("gmail").copied().unwrap_or(0) + 1;
            map.insert("gmail".into(), next);
            (next, true)
        })
        .unwrap();
    assert_eq!(bumped, 2);

    let scoped = storage.for_scope(&Scope::new("local").unwrap()).unwrap();
    let kv = tinystoragedrivers::Kv::ns(scoped.documents(), "composio", "state").unwrap();
    let read: Option<BTreeMap<String, u32>> =
        crate::storage::block_on(async move { kv.get(path()).await }).unwrap();
    assert_eq!(read.unwrap()["gmail"], 2);
}

#[test]
fn an_unchanged_update_writes_nothing_and_returns_its_result() {
    let docs = docs_in(&MemoryStorage::new(), "local");
    let seen = docs
        .update::<BTreeMap<String, u32>, usize>(path(), |map| (map.len(), false))
        .unwrap();
    assert_eq!(seen, 0);
    let loaded: BTreeMap<String, u32> = docs.load(path()).unwrap();
    assert!(loaded.is_empty());
}

#[test]
fn the_import_marker_is_a_migrations_entry_and_stops_a_second_import() {
    let docs = docs_in(&MemoryStorage::new(), "local");
    assert!(!docs.legacy_imported(path()).unwrap());
    assert!(docs
        .import_if_absent(path(), serde_json::json!({ "gmail": 5 }))
        .unwrap());
    assert!(docs.legacy_imported(path()).unwrap());
    let loaded: BTreeMap<String, u32> = docs.load(path()).unwrap();
    assert_eq!(loaded["gmail"], 5);
}

#[tokio::test]
async fn v1_state_documents_move_into_kv_without_replacing_a_newer_value() {
    use tinystoragedrivers::Precondition;
    let storage = MemoryStorage::new();
    let scoped = storage.for_scope(&Scope::new("local").unwrap()).unwrap();
    let raw = scoped.documents();
    raw.ensure_collection(&tinystoragedrivers::CollectionSpec::new("composio_state"))
        .await
        .unwrap();
    for (name, value) in [("a.json", 1), ("b.json", 2)] {
        raw.put(
            "composio_state",
            name,
            serde_json::json!({ "value": { "n": value } }),
            Precondition::Absent,
        )
        .await
        .unwrap();
    }
    let kv = tinystoragedrivers::Kv::ns(raw, "composio", "state").unwrap();
    for spec in tinystoragedrivers::primitives::collection_specs() {
        raw.ensure_collection(&spec).await.unwrap();
    }
    kv.put("b.json", &serde_json::json!({ "n": 99 }))
        .await
        .unwrap();

    assert_eq!(migrate_legacy_state(&scoped).await.unwrap(), 1);
    assert_eq!(
        kv.get::<serde_json::Value>("a.json").await.unwrap(),
        Some(serde_json::json!({ "n": 1 }))
    );
    assert_eq!(
        kv.get::<serde_json::Value>("b.json").await.unwrap(),
        Some(serde_json::json!({ "n": 99 })),
        "a value already in kv is newer"
    );
    assert_eq!(migrate_legacy_state(&scoped).await.unwrap(), 0);
}
