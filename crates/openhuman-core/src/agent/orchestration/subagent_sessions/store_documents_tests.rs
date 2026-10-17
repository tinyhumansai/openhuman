use super::*;
use crate::agent::orchestration::subagent_sessions::types::DurableSubagentStatus;
use crate::agent::orchestration::subagent_sessions::types::SubagentSessionStore;
use crate::storage::{MemoryStorage, Scope, StorageBackend};
use serde_json::json;

fn docs_in(storage: &MemoryStorage, scope: &str) -> Docs {
    Docs::over(&storage.for_scope(&Scope::new(scope).unwrap()).unwrap())
}

fn session(id: &str) -> DurableSubagentSession {
    serde_json::from_value(json!({
        "subagentSessionId": id,
        "parentSession": "parent",
        "parentThreadId": "thread",
        "workerThreadId": null,
        "agentId": "researcher",
        "displayName": null,
        "model": null,
        "sandboxMode": "workspace",
        "actionRoot": null,
        "taskKey": "task",
        "taskTitle": "Task",
        "currentTaskId": null,
        "status": "idle",
        "reusable": true,
        "latestHistory": null,
        "latestError": null,
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-01-01T00:00:00Z",
        "lastUsedAt": "2026-01-01T00:00:00Z",
    }))
    .unwrap()
}

#[test]
fn a_missing_list_loads_empty() {
    assert!(docs_in(&MemoryStorage::new(), "local")
        .load()
        .unwrap()
        .is_empty());
}

#[test]
fn save_upserts_sessions_and_keeps_the_others() {
    let docs = docs_in(&MemoryStorage::new(), "local");
    docs.save(&[session("a"), session("b")]).unwrap();
    let loaded = docs.load().unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].subagent_session_id, "a");
    assert_eq!(loaded[1].status, DurableSubagentStatus::Idle);
    // A save from a stale snapshot never removes a session it does not know.
    docs.save(&[session("c")]).unwrap();
    let ids: Vec<_> = docs
        .load()
        .unwrap()
        .into_iter()
        .map(|s| s.subagent_session_id)
        .collect();
    assert_eq!(ids, ["a", "b", "c"]);
}

#[test]
fn scopes_do_not_see_each_others_sessions() {
    let storage = MemoryStorage::new();
    docs_in(&storage, "alice").save(&[session("a")]).unwrap();
    assert!(docs_in(&storage, "bob").load().unwrap().is_empty());
    assert_eq!(docs_in(&storage, "alice").load().unwrap().len(), 1);
}

#[test]
fn the_store_uses_the_workspace_default_with_no_backend_installed() {
    let dir = tempfile::tempdir().unwrap();
    let store = SubagentSessionStore::new(dir.path().to_path_buf());
    assert!(store.load().unwrap().is_empty());
    store.save(&[session("a")]).unwrap();
    assert_eq!(store.load().unwrap().len(), 1);
    // No JSON file store: the records are in the workspace's own database.
    assert!(!dir
        .path()
        .join(".openhuman/subagent_sessions.json")
        .exists());
    assert!(dir.path().join("storage").exists());
}

#[test]
fn sessions_come_back_in_creation_order() {
    let docs = docs_in(&MemoryStorage::new(), "local");
    let mut early = session("z");
    early.created_at = "2026-01-01T00:00:00Z".into();
    let mut late = session("a");
    late.created_at = "2026-02-01T00:00:00Z".into();
    docs.save(&[late, early]).unwrap();
    let ids: Vec<_> = docs
        .load()
        .unwrap()
        .into_iter()
        .map(|s| s.subagent_session_id)
        .collect();
    assert_eq!(ids, ["z", "a"]);
}

#[test]
fn the_store_dispatches_to_documents_when_a_backend_is_pinned() {
    let storage = MemoryStorage::new();
    let dir = tempfile::tempdir().unwrap();
    let store = SubagentSessionStore::new(dir.path().to_path_buf());
    let docs = docs_in(&storage, "local");
    with_override(docs.clone(), || {
        assert!(store.load().unwrap().is_empty());
        store.save(&[session("a")]).unwrap();
        assert_eq!(store.load().unwrap().len(), 1);
    });
    assert_eq!(docs.load().unwrap().len(), 1);
    assert!(!dir
        .path()
        .join(".openhuman/subagent_sessions.json")
        .exists());
}

#[test]
fn a_stale_snapshot_does_not_revert_a_newer_session() {
    let docs = docs_in(&MemoryStorage::new(), "local");
    let old = session("a");
    docs.save(std::slice::from_ref(&old)).unwrap();

    // Another core finishes the session.
    let mut newer = old.clone();
    newer.status = DurableSubagentStatus::Failed;
    newer.updated_at = "2026-03-01T00:00:00Z".into();
    docs.save(&[newer]).unwrap();

    // This core still holds the old snapshot and saves it with a new session.
    docs.save(&[old, session("b")]).unwrap();

    let loaded = docs.load().unwrap();
    assert_eq!(loaded.len(), 2);
    let a = loaded
        .iter()
        .find(|s| s.subagent_session_id == "a")
        .unwrap();
    assert_eq!(a.status, DurableSubagentStatus::Failed);
}

#[test]
fn a_legacy_session_file_is_imported_once_and_left_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(".openhuman");
    std::fs::create_dir_all(&legacy).unwrap();
    let original = serde_json::to_string(&vec![session("legacy-1")]).unwrap();
    std::fs::write(legacy.join("subagent_sessions.json"), &original).unwrap();
    let store = SubagentSessionStore::new(dir.path().to_path_buf());

    let loaded = store.load().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].subagent_session_id, "legacy-1");
    // The legacy file is read only: same name, same bytes, no `.migrated`.
    assert_eq!(
        std::fs::read_to_string(legacy.join("subagent_sessions.json")).unwrap(),
        original
    );
    assert!(!legacy.join("subagent_sessions.json.migrated").exists());
    assert_eq!(store.load().unwrap().len(), 1);
    // The marker, not the file, stops a second import: a file that changed
    // afterwards is not imported again.
    let changed = serde_json::to_string(&vec![session("legacy-2")]).unwrap();
    std::fs::write(legacy.join("subagent_sessions.json"), &changed).unwrap();
    let again = store.load().unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].subagent_session_id, "legacy-1");
}

#[test]
fn workspaces_sharing_a_backend_each_import_their_own_legacy_file() {
    let docs = docs_in(&MemoryStorage::new(), "local");
    let (one, two) = (
        std::path::Path::new("/ws/one"),
        std::path::Path::new("/ws/two"),
    );
    assert!(!docs.legacy_imported(one).unwrap());
    docs.import_legacy(one, &[session("from-one")]).unwrap();
    assert!(docs.legacy_imported(one).unwrap());
    assert!(
        !docs.legacy_imported(two).unwrap(),
        "importing one workspace must not suppress another's import"
    );
    docs.import_legacy(two, &[session("from-two")]).unwrap();
    assert_eq!(docs.load().unwrap().len(), 2);
}

#[test]
fn a_late_importer_rechecks_the_marker_under_the_claim() {
    let docs = docs_in(&MemoryStorage::new(), "local");
    let source = std::path::Path::new("/legacy/subagent_sessions.json");
    docs.import_legacy(source, &[session("first")]).unwrap();
    assert!(docs.legacy_imported(source).unwrap());
    // A process that passed the marker check before the first finished
    // reaches the claim afterwards: it must not copy its list again.
    docs.import_legacy(source, &[session("second")]).unwrap();
    let ids: Vec<_> = docs
        .load()
        .unwrap()
        .into_iter()
        .map(|s| s.subagent_session_id)
        .collect();
    assert_eq!(ids, ["first"]);
}
