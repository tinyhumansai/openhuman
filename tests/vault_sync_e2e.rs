//! End-to-end coverage for vault sync lifecycle.
//!
//! Runs the public `vault.*` operations against a real temp workspace:
//! create a vault, sync supported files, verify per-file ledger + memory
//! tree state, then modify/delete/add files and sync again.
//!
//! Memory-side assertions target the **memory_tree backend** (the canonical
//! RAG layer for `memory.search` / `tree.read_chunk` / agent recall).
//! Prior to #2720 vault sync wrote to the legacy `UnifiedMemory.memory_docs`
//! table — silently invisible to retrieval. The fix migrated vault sync to
//! the memory-tree pipeline, so this test now probes `mem_tree_chunks` and
//! `mem_tree_ingested_sources` instead of `list_documents`.

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tempfile::tempdir;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::global as memory_global;
use openhuman_core::openhuman::memory_store::chunks::store::{count_chunks, is_source_ingested};
use openhuman_core::openhuman::memory_store::chunks::types::SourceKind;
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
    assert_eq!(
        first.status,
        VaultSyncStatus::Completed,
        "first sync failed with errors: {:?}",
        first.errors
    );
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

    // After #2720, vault sync writes to memory_tree (mem_tree_chunks +
    // mem_tree_ingested_sources). Both files must register as sources, and
    // the chunk count must be > 0 (otherwise the chunker / pipeline silently
    // dropped them — the exact failure mode this test guards against).
    let chunks_after_first = count_chunks(&config).expect("count_chunks after first sync");
    assert!(
        chunks_after_first > 0,
        "vault sync must populate mem_tree_chunks; got {chunks_after_first}"
    );
    let one_id = format!("vault:{}:notes/one.md", vault.id);
    let two_id = format!("vault:{}:docs/two.json", vault.id);
    assert!(
        is_source_ingested(&config, SourceKind::Document, &one_id).expect("source check one.md"),
        "notes/one.md missing from mem_tree_ingested_sources (source_id={one_id})"
    );
    assert!(
        is_source_ingested(&config, SourceKind::Document, &two_id).expect("source check two.json"),
        "docs/two.json missing from mem_tree_ingested_sources (source_id={two_id})"
    );

    // Vault ledger continues to track the per-file row count and rel_paths
    // — same contract as before #2720; only the `document_id` semantic
    // changed (now holds the memory-tree source_id).
    for file in &files {
        assert!(
            file.document_id.starts_with("vault:"),
            "ledger document_id must encode memory-tree source_id, got {}",
            file.document_id
        );
    }

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
    assert_eq!(
        second.status,
        VaultSyncStatus::Completed,
        "second sync failed with errors: {:?}",
        second.errors
    );
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

    // Memory-tree side of the lifecycle (post-#2720):
    //   - notes/one.md  : content updated → delete_chunks_by_source then
    //                     re-ingest; source must still be registered.
    //   - docs/two.json : file removed   → delete_chunks_by_source dropped
    //                     it; source must no longer register as ingested.
    //   - docs/three.toml: brand-new file → freshly registered.
    let three_id = format!("vault:{}:docs/three.toml", vault.id);
    assert!(
        is_source_ingested(&config, SourceKind::Document, &one_id)
            .expect("source check one.md after update"),
        "notes/one.md must remain ingested after re-sync of updated content (source_id={one_id})"
    );
    assert!(
        is_source_ingested(&config, SourceKind::Document, &three_id)
            .expect("source check three.toml"),
        "docs/three.toml (new file) missing from mem_tree_ingested_sources (source_id={three_id})"
    );
    assert!(
        !is_source_ingested(&config, SourceKind::Document, &two_id).expect("source check two.json"),
        "docs/two.json was deleted on disk; vault sync's Phase 4 must remove \
         it from mem_tree_ingested_sources (source_id={two_id})"
    );
}
