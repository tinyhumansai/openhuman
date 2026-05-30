//! Unit tests for the vault domain. Hits a real SQLite db in a tempdir,
//! but skips memory ingestion (covered in higher-level integration tests).

use std::path::PathBuf;
use tempfile::TempDir;

use crate::openhuman::config::Config;

use super::ops;
use super::state;
use super::store;
use super::sync::supported_extension;
use super::types::{
    Vault, VaultFile, VaultFileStatus, VaultSyncState, VaultSyncStatus, VaultWriteState,
};

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
        host_os: None,
        namespace: "vault:vault-test-1".to_string(),
        include_globs: vec![],
        exclude_globs: vec![],
        created_at: chrono::Utc::now(),
        last_synced_at: None,
        file_count: 0,
        write_state: VaultWriteState::Writable,
        write_state_reason: None,
    }
}

fn incompatible_path_for_current_host() -> &'static str {
    if cfg!(windows) {
        "/home/leigh/OHvault"
    } else {
        r"C:\Users\leigh\OHvault"
    }
}

#[test]
fn path_compatibility_rejects_cross_platform_absolute_paths() {
    assert!(store::path_looks_compatible_with_host_os(
        r"C:\Users\leigh\OHvault",
        "windows"
    ));
    assert!(store::path_looks_compatible_with_host_os(
        r"\\server\share\OHvault",
        "windows"
    ));
    // Forward-slash `//…` is POSIX-legal, not Windows UNC.
    assert!(!store::path_looks_compatible_with_host_os(
        "//server/share/OHvault",
        "windows"
    ));
    assert!(!store::path_looks_compatible_with_host_os(
        "/home/leigh/OHvault",
        "windows"
    ));

    assert!(store::path_looks_compatible_with_host_os(
        "/home/leigh/OHvault",
        "linux"
    ));
    assert!(store::path_looks_compatible_with_host_os(
        "/Users/leigh/OHvault",
        "macos"
    ));
    assert!(!store::path_looks_compatible_with_host_os(
        r"C:\Users\leigh\OHvault",
        "linux"
    ));
    assert!(!store::path_looks_compatible_with_host_os(
        r"\\server\share\OHvault",
        "macos"
    ));
    // Forward-slash `//…` is POSIX-legal — compatible with Unix hosts.
    assert!(store::path_looks_compatible_with_host_os(
        "//server/share/OHvault",
        "macos"
    ));
    assert!(store::path_looks_compatible_with_host_os(
        "//server/share/OHvault",
        "linux"
    ));
}

#[test]
fn store_stamps_new_vaults_with_current_host_os() {
    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp);
    let vault = sample_vault(tmp.path().to_path_buf());

    store::insert_vault(&config, &vault).unwrap();

    let listed = store::list_vaults(&config).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].host_os.as_deref(), Some(std::env::consts::OS));
    assert_eq!(listed[0].write_state, VaultWriteState::Writable);
    assert_eq!(
        listed[0].write_state_reason.as_deref(),
        Some(store::VAULT_WRITE_REASON_WRITABLE)
    );
}

#[test]
fn store_marks_missing_vault_folder_unavailable_instead_of_hiding_it() {
    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp);
    let vault_root = tmp.path().join("vault-root");
    std::fs::create_dir_all(&vault_root).unwrap();
    let vault = sample_vault(vault_root.clone());

    store::insert_vault(&config, &vault).unwrap();
    std::fs::remove_dir_all(&vault_root).unwrap();

    let listed = store::list_vaults(&config).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].write_state, VaultWriteState::Unavailable);
    assert_eq!(
        listed[0].write_state_reason.as_deref(),
        Some(store::VAULT_WRITE_REASON_UNAVAILABLE)
    );
}

#[test]
fn store_filters_legacy_vaults_whose_path_belongs_to_another_host_family() {
    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp);
    let mut vault = sample_vault(PathBuf::from(incompatible_path_for_current_host()));
    vault.host_os = None;

    store::insert_vault_preserving_host_for_tests(&config, &vault).unwrap();

    assert!(store::list_vaults(&config).unwrap().is_empty());
    assert!(store::get_vault(&config, &vault.id).unwrap().is_none());
}

#[test]
fn store_filters_vaults_created_on_a_different_host_os() {
    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp);
    let mut vault = sample_vault(tmp.path().to_path_buf());
    vault.host_os = Some(if cfg!(windows) { "linux" } else { "windows" }.to_string());

    store::insert_vault_preserving_host_for_tests(&config, &vault).unwrap();

    assert!(store::list_vaults(&config).unwrap().is_empty());
    assert!(store::get_vault(&config, &vault.id).unwrap().is_none());
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
async fn vault_create_returns_current_host_os() {
    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp);

    let outcome = ops::vault_create(
        &config,
        "Test",
        tmp.path().to_str().unwrap(),
        vec![],
        vec![],
    )
    .await
    .unwrap();

    assert_eq!(outcome.value.host_os.as_deref(), Some(std::env::consts::OS));
    assert_eq!(outcome.value.write_state, VaultWriteState::Writable);
    assert_eq!(
        outcome.value.write_state_reason.as_deref(),
        Some(store::VAULT_WRITE_REASON_WRITABLE)
    );
}

#[tokio::test]
async fn vault_create_uses_pii_safe_memory_namespace() {
    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp);

    let outcome = ops::vault_create(
        &config,
        "Test",
        tmp.path().to_str().unwrap(),
        vec![],
        vec![],
    )
    .await
    .unwrap();

    let namespace = &outcome.value.namespace;
    assert!(namespace.starts_with("vault-"));
    assert!(!namespace.contains(&outcome.value.id));
    assert!(!crate::openhuman::memory_store::safety::has_likely_secret(
        namespace
    ));
    assert!(!crate::openhuman::memory_store::safety::pii::has_likely_pii(namespace));
}

#[test]
fn vault_namespace_derivation_does_not_embed_pii_like_ids() {
    let namespace = ops::vault_namespace_for_id("VECJ880326XK4");

    assert!(namespace.starts_with("vault-"));
    assert!(!namespace.contains("VECJ880326XK4"));
    assert!(!crate::openhuman::memory_store::safety::has_likely_secret(
        &namespace
    ));
    assert!(!crate::openhuman::memory_store::safety::pii::has_likely_pii(&namespace));
}

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
async fn vault_write_markdown_requires_explicit_approval() {
    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp);

    let vault = ops::vault_create(
        &config,
        "Test",
        tmp.path().to_str().unwrap(),
        vec![],
        vec![],
    )
    .await
    .unwrap()
    .value;

    let err = ops::vault_write_markdown(
        &config,
        &vault.id,
        "wiki/summary.md",
        "# Summary\n",
        false,
        false,
    )
    .await
    .unwrap_err();
    assert!(err.contains("explicit user approval"));
}

#[tokio::test]
async fn vault_write_markdown_creates_and_updates_relative_markdown() {
    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp);
    let vault_root = tmp.path().join("vault-root");
    std::fs::create_dir_all(&vault_root).unwrap();

    let vault = ops::vault_create(
        &config,
        "Test",
        vault_root.to_str().unwrap(),
        vec![],
        vec![],
    )
    .await
    .unwrap()
    .value;

    let first = ops::vault_write_markdown(
        &config,
        &vault.id,
        "wiki/summary.md",
        "# Summary\n\nInitial.",
        false,
        true,
    )
    .await
    .unwrap()
    .value;
    assert!(first.created);
    assert_eq!(first.rel_path, "wiki/summary.md");
    assert_eq!(first.bytes_written, "# Summary\n\nInitial.".len() as u64);
    assert_eq!(
        std::fs::read_to_string(vault_root.join("wiki/summary.md")).unwrap(),
        "# Summary\n\nInitial."
    );

    let duplicate = ops::vault_write_markdown(
        &config,
        &vault.id,
        "wiki/summary.md",
        "# Summary\n\nUpdated.",
        false,
        true,
    )
    .await
    .unwrap_err();
    assert!(duplicate.contains("already exists"));

    let updated = ops::vault_write_markdown(
        &config,
        &vault.id,
        "wiki/summary.md",
        "# Summary\n\nUpdated.",
        true,
        true,
    )
    .await
    .unwrap()
    .value;
    assert!(!updated.created);
    assert_eq!(
        std::fs::read_to_string(vault_root.join("wiki/summary.md")).unwrap(),
        "# Summary\n\nUpdated."
    );
}

#[tokio::test]
async fn vault_write_markdown_rejects_escape_paths_and_non_markdown() {
    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp);
    let vault = ops::vault_create(
        &config,
        "Test",
        tmp.path().to_str().unwrap(),
        vec![],
        vec![],
    )
    .await
    .unwrap()
    .value;

    let traversal = ops::vault_write_markdown(&config, &vault.id, "../x.md", "x", false, true)
        .await
        .unwrap_err();
    assert!(traversal.contains(".."));

    let non_markdown =
        ops::vault_write_markdown(&config, &vault.id, "notes/out.txt", "x", false, true)
            .await
            .unwrap_err();
    assert!(non_markdown.contains(".md"));
}

#[cfg(unix)]
#[tokio::test]
async fn vault_write_markdown_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp);
    let vault_root = tmp.path().join("vault-root");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&vault_root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    symlink(&outside, vault_root.join("linked")).unwrap();

    let vault = ops::vault_create(
        &config,
        "Test",
        vault_root.to_str().unwrap(),
        vec![],
        vec![],
    )
    .await
    .unwrap()
    .value;

    let err = ops::vault_write_markdown(&config, &vault.id, "linked/escape.md", "x", false, true)
        .await
        .unwrap_err();
    assert!(err.contains("outside the vault"));
    assert!(!outside.join("escape.md").exists());
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
