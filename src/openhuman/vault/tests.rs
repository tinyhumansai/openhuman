//! Unit tests for the vault domain. Hits a real SQLite db in a tempdir,
//! but skips memory ingestion (covered in higher-level integration tests).

use std::path::PathBuf;
use tempfile::TempDir;

use crate::openhuman::config::Config;

use super::ops;
use super::state;
use super::store;
use super::sync::supported_extension;
use super::types::{Vault, VaultFile, VaultFileStatus, VaultSyncState, VaultSyncStatus};

fn make_config(tmp: &TempDir) -> Config {
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    config
}

fn sample_vault(root: PathBuf) -> Vault {
    Vault {
        id: "vault-test-1".to_string(),
        name: "Test".to_string(),
        root_path: root.to_string_lossy().to_string(),
        namespace: "vault:vault-test-1".to_string(),
        include_globs: vec![],
        exclude_globs: vec![],
        created_at: chrono::Utc::now(),
        last_synced_at: None,
        file_count: 0,
    }
}

#[test]
fn supported_extension_accepts_md_and_code() {
    assert!(supported_extension("md"));
    assert!(supported_extension("MD"));
    assert!(supported_extension("rs"));
    assert!(supported_extension("tsx"));
    assert!(!supported_extension("png"));
    assert!(!supported_extension("zip"));
    assert!(!supported_extension(""));
}

#[test]
fn store_insert_get_list_remove_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp);
    let vault = sample_vault(tmp.path().to_path_buf());

    store::insert_vault(&config, &vault).unwrap();

    let listed = store::list_vaults(&config).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, vault.id);
    assert_eq!(listed[0].namespace, vault.namespace);
    assert_eq!(listed[0].file_count, 0);

    let fetched = store::get_vault(&config, &vault.id).unwrap();
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().name, "Test");

    let removed = store::remove_vault(&config, &vault.id).unwrap();
    assert!(removed);
    assert!(store::list_vaults(&config).unwrap().is_empty());
}

#[test]
fn store_files_upsert_and_delete() {
    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp);
    let vault = sample_vault(tmp.path().to_path_buf());
    store::insert_vault(&config, &vault).unwrap();

    let file = VaultFile {
        vault_id: vault.id.clone(),
        rel_path: "notes/one.md".to_string(),
        document_id: "doc-1".to_string(),
        content_hash: "h1".to_string(),
        mtime_ms: 100,
        bytes: 42,
        ingested_at: chrono::Utc::now(),
        status: VaultFileStatus::Ok,
    };
    store::upsert_file(&config, &file).unwrap();

    let listed = store::list_files(&config, &vault.id).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].document_id, "doc-1");

    // Re-upsert with same key should update, not duplicate.
    let mut updated = file.clone();
    updated.content_hash = "h2".to_string();
    updated.mtime_ms = 200;
    store::upsert_file(&config, &updated).unwrap();
    let listed = store::list_files(&config, &vault.id).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].content_hash, "h2");
    assert_eq!(listed[0].mtime_ms, 200);

    // File count on vault list should reflect 1 OK row.
    let vaults = store::list_vaults(&config).unwrap();
    assert_eq!(vaults[0].file_count, 1);

    store::delete_file(&config, &vault.id, "notes/one.md").unwrap();
    assert!(store::list_files(&config, &vault.id).unwrap().is_empty());
}

#[test]
fn remove_vault_cascades_files() {
    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp);
    let vault = sample_vault(tmp.path().to_path_buf());
    store::insert_vault(&config, &vault).unwrap();

    let file = VaultFile {
        vault_id: vault.id.clone(),
        rel_path: "a.md".to_string(),
        document_id: "doc-a".to_string(),
        content_hash: "h".to_string(),
        mtime_ms: 1,
        bytes: 1,
        ingested_at: chrono::Utc::now(),
        status: VaultFileStatus::Ok,
    };
    store::upsert_file(&config, &file).unwrap();

    store::remove_vault(&config, &vault.id).unwrap();
    // Cascade should have wiped vault_files rows for this id.
    assert!(store::list_files(&config, &vault.id).unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// state.rs — in-memory sync state registry
// ---------------------------------------------------------------------------

fn make_state(vault_id: &str, status: VaultSyncStatus) -> VaultSyncState {
    VaultSyncState {
        vault_id: vault_id.to_string(),
        status,
        scanned: 0,
        ingested: 0,
        unchanged: 0,
        removed: 0,
        failed: 0,
        skipped_unsupported: 0,
        total: 0,
        started_at_ms: 100,
        finished_at_ms: None,
        duration_ms: 0,
        errors: vec![],
    }
}

#[test]
fn state_get_returns_none_for_unknown() {
    // Use a unique ID so parallel tests can't collide via the global map.
    assert!(state::get("__test_unknown_99z__").is_none());
}

#[test]
fn state_set_and_get_roundtrip() {
    let id = "__test_set_1__";
    state::set(make_state(id, VaultSyncStatus::Completed));
    let st = state::get(id).unwrap();
    assert_eq!(st.status, VaultSyncStatus::Completed);
    assert_eq!(st.vault_id, id);
}

#[test]
fn state_start_creates_running_entry() {
    let id = "__test_start_1__";
    state::start(id, 12345).unwrap();
    let st = state::get(id).unwrap();
    assert_eq!(st.status, VaultSyncStatus::Running);
    assert_eq!(st.started_at_ms, 12345);
    assert_eq!(st.ingested, 0);
}

#[test]
fn state_start_rejects_duplicate_running() {
    let id = "__test_start_dup__";
    state::start(id, 1).unwrap();
    let err = state::start(id, 2).unwrap_err();
    assert!(err.contains("already syncing"));
}

#[test]
fn state_start_allowed_after_completed() {
    let id = "__test_start_after_completed__";
    state::start(id, 1).unwrap();
    // Mark as completed, then start again — must succeed.
    state::update_progress(id, |s| s.status = VaultSyncStatus::Completed);
    state::start(id, 2).unwrap();
    assert_eq!(state::get(id).unwrap().status, VaultSyncStatus::Running);
}

#[test]
fn state_start_allowed_after_failed() {
    let id = "__test_start_after_failed__";
    state::start(id, 1).unwrap();
    state::update_progress(id, |s| s.status = VaultSyncStatus::Failed);
    state::start(id, 2).unwrap();
    assert_eq!(state::get(id).unwrap().status, VaultSyncStatus::Running);
}

#[test]
fn state_update_progress_mutates_entry() {
    let id = "__test_update_1__";
    state::start(id, 1).unwrap();
    state::update_progress(id, |s| {
        s.ingested = 7;
        s.scanned = 10;
        s.total = 10;
    });
    let st = state::get(id).unwrap();
    assert_eq!(st.ingested, 7);
    assert_eq!(st.scanned, 10);
}

#[test]
fn state_update_progress_noop_on_missing() {
    // Must not panic when vault_id is absent from the map.
    state::update_progress("__test_noop_xyz__", |s| {
        s.ingested = 999; // should never execute
    });
}

// ---------------------------------------------------------------------------
// ops.rs — vault_sync_status RPC operation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn vault_sync_status_returns_idle_for_unknown_vault() {
    let outcome = ops::vault_sync_status("__ops_status_unknown__")
        .await
        .unwrap();
    assert_eq!(outcome.value.status, VaultSyncStatus::Idle);
    assert_eq!(outcome.value.vault_id, "__ops_status_unknown__");
    assert_eq!(outcome.value.ingested, 0);
}

#[tokio::test]
async fn vault_sync_status_returns_state_when_present() {
    let id = "__ops_status_running__";
    let mut st = make_state(id, VaultSyncStatus::Running);
    st.scanned = 10;
    st.ingested = 5;
    st.total = 10;
    state::set(st);

    let outcome = ops::vault_sync_status(id).await.unwrap();
    assert_eq!(outcome.value.status, VaultSyncStatus::Running);
    assert_eq!(outcome.value.scanned, 10);
    assert_eq!(outcome.value.ingested, 5);
    assert_eq!(outcome.value.total, 10);
}

#[tokio::test]
async fn vault_sync_status_returns_completed_state() {
    let id = "__ops_status_completed__";
    let mut st = make_state(id, VaultSyncStatus::Completed);
    st.ingested = 12;
    st.failed = 1;
    st.duration_ms = 500;
    st.errors = vec!["file.txt: too large".to_string()];
    state::set(st);

    let outcome = ops::vault_sync_status(id).await.unwrap();
    assert_eq!(outcome.value.status, VaultSyncStatus::Completed);
    assert_eq!(outcome.value.ingested, 12);
    assert_eq!(outcome.value.failed, 1);
    assert_eq!(outcome.value.errors.len(), 1);
}

#[tokio::test]
async fn vault_sync_status_rejects_empty_id() {
    let err = ops::vault_sync_status("").await.unwrap_err();
    assert!(err.contains("vault_id must not be empty"));
}

#[tokio::test]
async fn vault_sync_panic_guard_marks_state_failed_and_allows_retry() {
    // Simulate the panic-recovery path that the catch_unwind guard in
    // ops::vault_sync triggers: vault goes Running -> Failed (with a panic
    // message), then can be restarted.  This verifies the invariant that no
    // panic can permanently lock the state in `Running`.
    let id = "__test_panic_guard_recovery__";
    state::start(id, 1_000).unwrap();
    assert_eq!(state::get(id).unwrap().status, VaultSyncStatus::Running);

    // Simulate what the Err(_) branch of the catch_unwind match does.
    state::update_progress(id, |s| {
        s.status = VaultSyncStatus::Failed;
        s.errors = vec!["sync task panicked unexpectedly".to_string()];
    });

    let st = state::get(id).unwrap();
    assert_eq!(st.status, VaultSyncStatus::Failed);
    assert!(st.errors[0].contains("panicked"));

    // A subsequent sync attempt must not be blocked by the old Running entry.
    state::start(id, 2_000).unwrap();
    assert_eq!(state::get(id).unwrap().status, VaultSyncStatus::Running);
}
