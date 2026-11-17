use super::*;
use crate::storage::{test_support::memory_backend, Scope, StorageBackend};
use chrono::{Duration, Utc};

use super::super::types::TokenUsage;

fn docs_in(storage: &dyn StorageBackend, scope: &str) -> CostDocs {
    CostDocs::over(&storage.for_scope(&Scope::new(scope).unwrap()).unwrap())
}

fn record(model: &str, cost: f64, secs_ago: i64) -> CostRecord {
    let mut usage = TokenUsage::new(model, 10, 5, 1.0, 2.0);
    usage.cost_usd = cost;
    usage.timestamp = Utc::now() - Duration::seconds(secs_ago);
    CostRecord::new("session", usage)
}

#[test]
fn records_round_trip_oldest_first() {
    let storage = memory_backend();
    let docs = docs_in(storage.as_ref(), "local");
    let newer = record("b/model", 0.5, 1);
    let older = record("a/model", 0.25, 60);
    docs.add(&newer).unwrap();
    docs.add(&older).unwrap();
    let all = docs.all().unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, older.id);
    assert_eq!(all[1].id, newer.id);
    assert_eq!(all[1].usage.cost_usd, 0.5);
    assert_eq!(all[1].session_id, "session");
}

#[test]
fn a_record_is_written_once() {
    let storage = memory_backend();
    let docs = docs_in(storage.as_ref(), "local");
    let rec = record("a/model", 0.1, 0);
    docs.add(&rec).unwrap();
    assert!(docs.add(&rec).is_err());
    assert_eq!(docs.all().unwrap().len(), 1);
}

#[test]
fn scopes_do_not_see_each_others_costs() {
    let storage = memory_backend();
    let alice = docs_in(storage.as_ref(), "alice");
    let bob = docs_in(storage.as_ref(), "bob");
    alice.add(&record("a/model", 1.0, 0)).unwrap();
    assert_eq!(alice.all().unwrap().len(), 1);
    assert!(bob.all().unwrap().is_empty());
}

#[test]
fn a_legacy_jsonl_ledger_is_imported_once_and_set_aside() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("costs.jsonl");
    let first = record("a/model", 0.5, 30);
    let second = record("b/model", 0.25, 10);
    std::fs::write(
        &path,
        format!(
            "{}\n\nnot json\n{}\n",
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        ),
    )
    .unwrap();
    let storage = memory_backend();
    let docs = docs_in(storage.as_ref(), "local");
    assert_eq!(docs.import_legacy(&path).unwrap(), 2);
    assert_eq!(docs.all().unwrap().len(), 2);
    assert!(!path.exists());
    assert!(dir.path().join("costs.jsonl.migrated").exists());
    // Nothing left to import.
    assert_eq!(docs.import_legacy(&path).unwrap(), 0);
}

#[test]
fn a_legacy_import_skips_ids_already_stored_or_repeated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("costs.jsonl");
    let first = record("a/model", 0.5, 30);
    let second = record("b/model", 0.25, 10);
    let line = |r: &CostRecord| serde_json::to_string(r).unwrap();
    // `second` repeats in the file; `first` was stored by another core.
    std::fs::write(
        &path,
        format!("{}\n{}\n{}\n", line(&first), line(&second), line(&second)),
    )
    .unwrap();
    let storage = memory_backend();
    let docs = docs_in(storage.as_ref(), "local");
    docs.add(&first).unwrap();
    assert_eq!(docs.import_legacy(&path).unwrap(), 1);
    assert_eq!(docs.all().unwrap().len(), 2);
}

#[test]
fn an_earlier_set_aside_ledger_is_kept_and_bad_utf8_lines_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("costs.jsonl");
    let rec = record("a/model", 0.5, 30);
    let mut raw = b"\xff\xfe not utf-8\n".to_vec();
    raw.extend_from_slice(serde_json::to_string(&rec).unwrap().as_bytes());
    raw.push(b'\n');
    std::fs::write(&path, raw).unwrap();
    let earlier = dir.path().join("costs.jsonl.migrated");
    std::fs::write(&earlier, "earlier").unwrap();

    let storage = memory_backend();
    let docs = docs_in(storage.as_ref(), "local");
    assert_eq!(docs.import_legacy(&path).unwrap(), 1);
    assert!(!path.exists());
    assert_eq!(std::fs::read_to_string(&earlier).unwrap(), "earlier");
    assert!(dir.path().join("costs.jsonl.migrated.1").exists());
}

#[test]
fn a_ledger_another_core_already_set_aside_counts_as_done() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("costs.jsonl");
    assert!(set_aside(&path).is_ok());
    // An unreadable ledger (here a directory) fails the import instead.
    std::fs::create_dir_all(&path).unwrap();
    let storage = memory_backend();
    let docs = docs_in(storage.as_ref(), "local");
    assert!(docs.import_legacy(&path).is_err());
}
