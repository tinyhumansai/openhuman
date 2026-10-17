use super::*;
use crate::storage::{MemoryStorage, Scope, StorageBackend};
use std::collections::BTreeMap;

fn state(storage: &MemoryStorage, scope: &str) -> StateKv {
    StateKv::over(
        &storage.for_scope(&Scope::new(scope).unwrap()).unwrap(),
        "test_owner",
        "state",
    )
}

#[test]
fn a_missing_value_reads_as_none() {
    let kv = state(&MemoryStorage::new(), "local");
    assert!(kv.get::<BTreeMap<String, u32>>("jobs").unwrap().is_none());
}

#[test]
fn put_get_list_delete_round_trip() {
    let kv = state(&MemoryStorage::new(), "local");
    kv.put("a/1", &1u32).unwrap();
    kv.put("a/2", &2u32).unwrap();
    kv.put("b/1", &3u32).unwrap();
    assert_eq!(kv.get::<u32>("a/2").unwrap(), Some(2));
    let listed = kv.list::<u32>("a/").unwrap();
    assert_eq!(listed, vec![("a/1".to_string(), 1), ("a/2".to_string(), 2)]);
    assert!(kv.delete("a/1").unwrap());
    assert!(!kv.delete("a/1").unwrap());
    assert!(kv.get::<u32>("a/1").unwrap().is_none());
}

#[test]
fn scopes_do_not_see_each_others_state() {
    let storage = MemoryStorage::new();
    state(&storage, "alice").put("x", &1u32).unwrap();
    assert!(state(&storage, "bob").get::<u32>("x").unwrap().is_none());
}

#[test]
fn insert_if_absent_keeps_the_first_value() {
    let kv = state(&MemoryStorage::new(), "local");
    assert!(kv.insert_if_absent("root", &"first").unwrap());
    assert!(!kv.insert_if_absent("root", &"second").unwrap());
    assert_eq!(kv.get::<String>("root").unwrap().as_deref(), Some("first"));
}

#[test]
fn update_applies_the_change_and_skips_an_unchanged_write() {
    let kv = state(&MemoryStorage::new(), "local");
    let total = kv
        .update::<u32, u32>("n", |n| {
            *n += 2;
            (*n, true)
        })
        .unwrap();
    assert_eq!(total, 2);
    let seen = kv.update::<u32, u32>("n", |n| (*n, false)).unwrap();
    assert_eq!(seen, 2);
    assert_eq!(kv.get::<u32>("n").unwrap(), Some(2));
}

#[test]
fn import_file_copies_a_legacy_file_once_and_leaves_it_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("jobs.json");
    std::fs::write(&legacy, r#"{"a": 1}"#).unwrap();
    let kv = state(&MemoryStorage::new(), "local");
    kv.import_file("jobs", &legacy).unwrap();
    assert_eq!(
        kv.get::<BTreeMap<String, u32>>("jobs").unwrap().unwrap()["a"],
        1
    );
    assert!(legacy.exists(), "the legacy file is not touched");

    // A later change is not overwritten by a re-import, nor resurrected after a delete.
    kv.put("jobs", &BTreeMap::from([("a".to_string(), 9u32)])).unwrap();
    std::fs::write(&legacy, r#"{"a": 5}"#).unwrap();
    kv.import_file("jobs", &legacy).unwrap();
    assert_eq!(
        kv.get::<BTreeMap<String, u32>>("jobs").unwrap().unwrap()["a"],
        9
    );
    kv.delete("jobs").unwrap();
    kv.import_file("jobs", &legacy).unwrap();
    assert!(kv.get::<serde_json::Value>("jobs").unwrap().is_none());
}

#[test]
fn import_file_does_not_replace_an_existing_value() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("x.json");
    std::fs::write(&legacy, "7").unwrap();
    let kv = state(&MemoryStorage::new(), "local");
    kv.put("x", &1u32).unwrap();
    kv.import_file("x", &legacy).unwrap();
    assert_eq!(kv.get::<u32>("x").unwrap(), Some(1));
}

#[test]
fn import_file_records_a_missing_or_corrupt_file_without_failing() {
    let dir = tempfile::tempdir().unwrap();
    let kv = state(&MemoryStorage::new(), "local");
    kv.import_file("none", &dir.path().join("missing.json")).unwrap();
    let corrupt = dir.path().join("bad.json");
    std::fs::write(&corrupt, "{not json").unwrap();
    kv.import_file("bad", &corrupt).unwrap();
    assert!(kv.get::<serde_json::Value>("bad").unwrap().is_none());
    // A file that shows up after the import was recorded is not imported.
    std::fs::write(dir.path().join("missing.json"), "1").unwrap();
    kv.import_file("none", &dir.path().join("missing.json")).unwrap();
    assert!(kv.get::<u32>("none").unwrap().is_none());
}
