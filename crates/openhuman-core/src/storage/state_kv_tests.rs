use super::*;
use crate::storage::{Scope, StorageBackend};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTED_A_DESERIALIZATIONS: AtomicUsize = AtomicUsize::new(0);
static COUNTED_B_DESERIALIZATIONS: AtomicUsize = AtomicUsize::new(0);

struct CountedA;

impl<'de> serde::Deserialize<'de> for CountedA {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let _: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;
        COUNTED_A_DESERIALIZATIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Self)
    }
}

struct CountedB;

impl<'de> serde::Deserialize<'de> for CountedB {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let _: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;
        COUNTED_B_DESERIALIZATIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Self)
    }
}

fn state(scope: &str) -> StateKv {
    StateKv::over(
        &tinystoragedrivers_testkit::memory_storage(&Scope::new(scope).unwrap()),
        "test_owner",
        "state",
    )
}

#[cfg(feature = "storage-sqlite")]
fn state_with_import_claims(scope: &str) -> (crate::storage::test_support::SqliteBackend, StateKv) {
    let backend = crate::storage::test_support::sqlite_backend().unwrap();
    let state = StateKv::on_backend(
        &backend.backend,
        &Scope::new(scope).unwrap(),
        "test_owner",
        "state",
    )
    .unwrap();
    (backend, state)
}

#[test]
fn a_missing_value_reads_as_none() {
    let kv = state("local");
    assert!(kv.get::<BTreeMap<String, u32>>("jobs").unwrap().is_none());
}

#[test]
fn put_get_list_delete_round_trip() {
    let kv = state("local");
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
    let backend: std::sync::Arc<dyn StorageBackend> =
        std::sync::Arc::new(tinystoragedrivers_testkit::MemoryBackend::new());
    let alice = StateKv::on_backend(
        &backend,
        &Scope::new("alice").unwrap(),
        "test_owner",
        "state",
    )
    .unwrap();
    let bob =
        StateKv::on_backend(&backend, &Scope::new("bob").unwrap(), "test_owner", "state").unwrap();
    alice.put("x", &1u32).unwrap();
    assert!(bob.get::<u32>("x").unwrap().is_none());
}

#[test]
fn opaque_keys_and_json_values_round_trip_without_human_memory_sanitization() {
    let backend: std::sync::Arc<dyn StorageBackend> =
        std::sync::Arc::new(tinystoragedrivers_testkit::MemoryBackend::new());
    let scope = Scope::local();
    let state = StateKv::on_backend(&backend, &scope, "persona", "cursor").unwrap();
    let other = StateKv::on_backend(&backend, &scope, "persona", "other").unwrap();
    let key = "/Users/alice@example.com/skills/private-token:sk_live_abcd";
    let value = serde_json::json!({
        "path": key,
        "api_key": "sk_live_abcd",
        "null": null,
        "number": 1.25
    });

    state.put(key, &value).unwrap();

    assert_eq!(state.get::<Value>(key).unwrap(), Some(value.clone()));
    assert_eq!(
        state.list::<Value>("/Users/alice@example.com/").unwrap(),
        vec![(key.to_owned(), value)]
    );
    assert!(other.get::<Value>(key).unwrap().is_none());
    assert!(state.delete(key).unwrap());
    assert!(state.get::<Value>(key).unwrap().is_none());
}

#[test]
fn insert_if_absent_keeps_the_first_value() {
    let kv = state("local");
    assert!(kv.insert_if_absent("root", &"first").unwrap());
    assert!(!kv.insert_if_absent("root", &"second").unwrap());
    assert_eq!(kv.get::<String>("root").unwrap().as_deref(), Some("first"));
}

#[test]
fn update_applies_the_change_and_skips_an_unchanged_write() {
    let kv = state("local");
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
#[cfg(feature = "storage-sqlite")]
fn import_file_copies_a_legacy_file_once_and_leaves_it_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("jobs.json");
    std::fs::write(&legacy, r#"{"a": 1}"#).unwrap();
    let (_backend, kv) = state_with_import_claims("local");
    kv.import_file("jobs", &legacy).unwrap();
    assert_eq!(
        kv.get::<BTreeMap<String, u32>>("jobs").unwrap().unwrap()["a"],
        1
    );
    assert!(legacy.exists(), "the legacy file is not touched");

    // A later change is not overwritten by a re-import, nor resurrected after a delete.
    kv.put("jobs", &BTreeMap::from([("a".to_string(), 9u32)]))
        .unwrap();
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
#[cfg(feature = "storage-sqlite")]
fn import_file_does_not_replace_an_existing_value() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("x.json");
    std::fs::write(&legacy, "7").unwrap();
    let (_backend, kv) = state_with_import_claims("local");
    kv.put("x", &1u32).unwrap();
    kv.import_file("x", &legacy).unwrap();
    assert_eq!(kv.get::<u32>("x").unwrap(), Some(1));
}

#[test]
#[cfg(feature = "storage-sqlite")]
fn import_file_records_a_missing_or_corrupt_file_without_failing() {
    let dir = tempfile::tempdir().unwrap();
    let (_backend, kv) = state_with_import_claims("local");
    kv.import_file("none", &dir.path().join("missing.json"))
        .unwrap();
    let corrupt = dir.path().join("bad.json");
    std::fs::write(&corrupt, "{not json").unwrap();
    kv.import_file("bad", &corrupt).unwrap();
    assert!(kv.get::<serde_json::Value>("bad").unwrap().is_none());
    // A file that shows up after the import was recorded is not imported.
    std::fs::write(dir.path().join("missing.json"), "1").unwrap();
    kv.import_file("none", &dir.path().join("missing.json"))
        .unwrap();
    assert!(kv.get::<u32>("none").unwrap().is_none());
}

#[test]
#[cfg(feature = "storage-sqlite")]
fn typed_import_quarantines_invalid_json_and_wrong_legacy_shapes() {
    let dir = tempfile::tempdir().unwrap();
    let (_backend, kv) = state_with_import_claims("local");
    let cases: [(&str, &[u8]); 2] = [
        ("malformed", b"{not json"),
        ("wrong_shape", br#"{"a":"not a number"}"#),
    ];

    for (name, bytes) in cases {
        let legacy = dir.path().join(format!("{name}.json"));
        std::fs::write(&legacy, bytes).unwrap();
        kv.import_file_quarantining::<BTreeMap<String, u32>>(name, &legacy)
            .unwrap();

        assert!(!legacy.exists(), "invalid {name} state was quarantined");
        assert_eq!(
            std::fs::read(legacy.with_extension("json.corrupt")).unwrap(),
            bytes,
            "the raw {name} bytes are preserved"
        );
        assert!(kv.get::<BTreeMap<String, u32>>(name).unwrap().is_none());
    }
}

#[test]
#[cfg(feature = "storage-sqlite")]
fn typed_import_keeps_valid_legacy_file_and_does_not_resurrect_deleted_value() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("valid.json");
    let bytes = br#"{"a":7}"#;
    std::fs::write(&legacy, bytes).unwrap();
    let (_backend, kv) = state_with_import_claims("local");

    kv.import_file_quarantining::<BTreeMap<String, u32>>("valid", &legacy)
        .unwrap();
    assert_eq!(
        kv.get::<BTreeMap<String, u32>>("valid").unwrap().unwrap()["a"],
        7
    );
    assert_eq!(std::fs::read(&legacy).unwrap(), bytes);

    kv.delete("valid").unwrap();
    kv.import_file_quarantining::<BTreeMap<String, u32>>("valid", &legacy)
        .unwrap();
    assert!(kv.get::<BTreeMap<String, u32>>("valid").unwrap().is_none());
    assert_eq!(std::fs::read(legacy).unwrap(), bytes);
}

#[test]
#[cfg(feature = "storage-sqlite")]
fn typed_quarantine_validation_is_cached_per_backend_scope_name_and_type() {
    COUNTED_A_DESERIALIZATIONS.store(0, Ordering::SeqCst);
    COUNTED_B_DESERIALIZATIONS.store(0, Ordering::SeqCst);
    let dir = tempfile::tempdir().unwrap();
    let first_legacy = dir.path().join("first.json");
    let second_legacy = dir.path().join("second.json");
    std::fs::write(&first_legacy, r#"{"ok":true}"#).unwrap();
    std::fs::write(&second_legacy, r#"{"ok":true}"#).unwrap();

    let backend = crate::storage::test_support::sqlite_backend().unwrap();
    let local =
        StateKv::on_backend(&backend.backend, &Scope::local(), "test_owner", "state").unwrap();
    local
        .import_file_quarantining::<CountedA>("first", &first_legacy)
        .unwrap();
    let first_count = COUNTED_A_DESERIALIZATIONS.load(Ordering::SeqCst);
    assert!(
        first_count > 0,
        "the legacy value was validated on first import"
    );

    local
        .import_file_quarantining::<CountedA>("first", &first_legacy)
        .unwrap();
    assert_eq!(
        COUNTED_A_DESERIALIZATIONS.load(Ordering::SeqCst),
        first_count,
        "reopening a valid legacy file does not reparse it"
    );

    local
        .import_file_quarantining::<CountedA>("second", &second_legacy)
        .unwrap();
    assert!(
        COUNTED_A_DESERIALIZATIONS.load(Ordering::SeqCst) > first_count,
        "another state name gets its own validation"
    );

    let another_scope = StateKv::on_backend(
        &backend.backend,
        &Scope::new("another").unwrap(),
        "test_owner",
        "state",
    )
    .unwrap();
    let before_other_scope = COUNTED_A_DESERIALIZATIONS.load(Ordering::SeqCst);
    another_scope
        .import_file_quarantining::<CountedA>("first", &first_legacy)
        .unwrap();
    assert!(
        COUNTED_A_DESERIALIZATIONS.load(Ordering::SeqCst) > before_other_scope,
        "the same state name in another scope is validated independently"
    );

    local
        .import_file_quarantining::<CountedB>("first", &first_legacy)
        .unwrap();
    assert!(
        COUNTED_B_DESERIALIZATIONS.load(Ordering::SeqCst) > 0,
        "a different requested value type gets its own validation"
    );
}
