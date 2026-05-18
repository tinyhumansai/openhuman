//! Unit tests for the vault domain. Hits a real SQLite db in a tempdir,
//! but skips memory ingestion (covered in higher-level integration tests).

use std::path::PathBuf;
use tempfile::TempDir;

use crate::openhuman::config::Config;

use super::store;
use super::sync::supported_extension;
use super::types::{Vault, VaultFile, VaultFileStatus};

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
