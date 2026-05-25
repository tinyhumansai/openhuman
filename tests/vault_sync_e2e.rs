//! End-to-end coverage for vault sync lifecycle.
//!
//! Runs the public `vault.*` operations against a real temp workspace:
//! create a vault, sync supported files, verify per-file ledger + memory
//! metadata, then modify/delete/add files and sync again.

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tempfile::tempdir;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::global as memory_global;
use openhuman_core::openhuman::vault::ops;
use openhuman_core::openhuman::vault::VaultSyncStatus;

static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("test lock poisoned")
}

fn make_config(workspace_dir: &Path) -> Config {
    let mut config = Config::default();
    config.workspace_dir = workspace_dir.to_path_buf();
    config
}

async fn wait_for_sync(vault_id: &str) -> openhuman_core::openhuman::vault::VaultSyncState {
    for _ in 0..100 {
        let state = ops::vault_sync_status(vault_id)
            .await
            .expect("vault_sync_status")
            .value;
        if state.status != VaultSyncStatus::Running {
            return state;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("vault sync did not finish within polling window");
}

fn documents_from_payload(payload: &serde_json::Value) -> Vec<serde_json::Value> {
    payload
        .get("documents")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[tokio::test]
async fn vault_sync_roundtrip_updates_memory_and_ledger() {
    let _guard = test_lock();
    let tmp = tempdir().expect("tempdir");
    let workspace_dir = tmp.path().join("workspace");
    let vault_root = tmp.path().join("vault-root");
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
    std::fs::create_dir_all(vault_root.join("notes")).expect("notes dir");
    std::fs::create_dir_all(vault_root.join("docs")).expect("docs dir");
    std::fs::create_dir_all(vault_root.join("node_modules")).expect("excluded dir");

    std::fs::write(
        vault_root.join("notes").join("one.md"),
        "# One\n\nPhoenix migration checklist.\n",
    )
    .expect("write one.md");
    std::fs::write(
        vault_root.join("docs").join("two.json"),
        "{\"status\":\"green\",\"owner\":\"alice\"}\n",
    )
    .expect("write two.json");
    std::fs::write(vault_root.join("image.png"), b"not a real png").expect("write image.png");
    std::fs::write(
        vault_root.join("node_modules").join("skip.md"),
        "should be excluded",
    )
    .expect("write excluded file");

    memory_global::init(workspace_dir.clone()).expect("init global memory client");
    let config = make_config(&workspace_dir);

    let vault = ops::vault_create(
        &config,
        "Project Vault",
        vault_root.to_str().expect("vault root utf-8"),
        vec![],
        vec![],
    )
    .await
    .expect("vault_create")
    .value;

    ops::vault_sync(&config, &vault.id)
        .await
        .expect("vault_sync first");
    let first = wait_for_sync(&vault.id).await;
    assert_eq!(first.status, VaultSyncStatus::Completed);
    assert_eq!(first.ingested, 2);
    assert_eq!(first.removed, 0);
    assert_eq!(first.failed, 0);
    assert_eq!(first.skipped_unsupported, 1);
    assert_eq!(first.scanned, 3);

    let files = ops::vault_files(&config, &vault.id)
        .await
        .expect("vault_files after first sync")
        .value;
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|file| file.rel_path == "notes/one.md"));
    assert!(files.iter().any(|file| file.rel_path == "docs/two.json"));

    let docs = memory_global::client()
        .expect("global memory client")
        .list_documents(Some(&vault.namespace))
        .await
        .expect("list vault documents");
    let docs = documents_from_payload(&docs);
    assert_eq!(docs.len(), 2);
    assert!(docs.iter().any(|doc| {
        doc.get("key").and_then(serde_json::Value::as_str) == Some("notes/one.md")
            && doc.get("sourceType").and_then(serde_json::Value::as_str) == Some("vault")
    }));
    assert!(docs.iter().any(|doc| {
        doc.get("key").and_then(serde_json::Value::as_str) == Some("docs/two.json")
    }));

    let note_ledger = files
        .iter()
        .find(|file| file.rel_path == "notes/one.md")
        .expect("note ledger entry");
    assert!(note_ledger.bytes > 0);
    assert_eq!(note_ledger.vault_id, vault.id);

    std::fs::write(
        vault_root.join("notes").join("one.md"),
        "# One\n\nPhoenix migration checklist updated with rollback steps.\n",
    )
    .expect("rewrite one.md");
    std::fs::remove_file(vault_root.join("docs").join("two.json")).expect("remove two.json");
    std::fs::write(
        vault_root.join("docs").join("three.toml"),
        "status = \"ready\"\nowner = \"bob\"\n",
    )
    .expect("write three.toml");

    ops::vault_sync(&config, &vault.id)
        .await
        .expect("vault_sync second");
    let second = wait_for_sync(&vault.id).await;
    assert_eq!(second.status, VaultSyncStatus::Completed);
    assert_eq!(second.ingested, 2);
    assert_eq!(second.removed, 1);
    assert_eq!(second.failed, 0);
    assert_eq!(second.skipped_unsupported, 1);

    let files = ops::vault_files(&config, &vault.id)
        .await
        .expect("vault_files after second sync")
        .value;
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|file| file.rel_path == "notes/one.md"));
    assert!(files.iter().any(|file| file.rel_path == "docs/three.toml"));
    assert!(!files.iter().any(|file| file.rel_path == "docs/two.json"));
}
