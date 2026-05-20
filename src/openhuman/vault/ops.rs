//! RPC-facing operations for the vault domain.

use chrono::Utc;
use uuid::Uuid;

use crate::openhuman::config::Config;
use crate::openhuman::memory::ops::{clear_namespace, ClearNamespaceParams};
use crate::rpc::RpcOutcome;

use super::store;
use super::sync;
use super::types::{Vault, VaultFile, VaultSyncReport};

/// Create a new vault pointing at a local folder.
pub async fn vault_create(
    config: &Config,
    name: &str,
    root_path: &str,
    include_globs: Vec<String>,
    exclude_globs: Vec<String>,
) -> Result<RpcOutcome<Vault>, String> {
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err("vault name must not be empty".to_string());
    }
    let trimmed_root = root_path.trim();
    if trimmed_root.is_empty() {
        return Err("root_path must not be empty".to_string());
    }
    let root = std::path::Path::new(trimmed_root);
    if !root.is_absolute() {
        return Err(format!("root_path must be absolute: {trimmed_root}"));
    }
    if !root.is_dir() {
        return Err(format!("root_path is not a directory: {trimmed_root}"));
    }

    let id = Uuid::new_v4().to_string();
    log::debug!(
        "[vault] create: name={trimmed_name:?} root={trimmed_root:?} id={id} \
         include_globs={} exclude_globs={}",
        include_globs.len(),
        exclude_globs.len(),
    );
    let namespace = format!("vault:{id}");
    let vault = Vault {
        id: id.clone(),
        name: trimmed_name.to_string(),
        root_path: root
            .canonicalize()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| trimmed_root.to_string()),
        namespace,
        include_globs,
        exclude_globs,
        created_at: Utc::now(),
        last_synced_at: None,
        file_count: 0,
    };

    store::insert_vault(config, &vault).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        vault,
        format!("vault created: {id}"),
    ))
}

pub async fn vault_list(config: &Config) -> Result<RpcOutcome<Vec<Vault>>, String> {
    let vaults = store::list_vaults(config).map_err(|e| e.to_string())?;
    log::debug!("[vault] list: count={}", vaults.len());
    Ok(RpcOutcome::single_log(vaults, "vaults listed"))
}

pub async fn vault_get(config: &Config, id: &str) -> Result<RpcOutcome<Vault>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("vault_id must not be empty".to_string());
    }
    let vault = store::get_vault(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("vault not found: {id}"))?;
    log::debug!("[vault] get: id={id} files={}", vault.file_count);
    Ok(RpcOutcome::single_log(vault, "vault loaded"))
}

pub async fn vault_files(config: &Config, id: &str) -> Result<RpcOutcome<Vec<VaultFile>>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("vault_id must not be empty".to_string());
    }
    let files = store::list_files(config, id).map_err(|e| e.to_string())?;
    log::debug!("[vault] files: id={id} count={}", files.len());
    Ok(RpcOutcome::single_log(files, "vault files listed"))
}

pub async fn vault_remove(
    config: &Config,
    id: &str,
    purge_memory: bool,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("vault_id must not be empty".to_string());
    }
    let vault = store::get_vault(config, id).map_err(|e| e.to_string())?;
    let removed = store::remove_vault(config, id).map_err(|e| e.to_string())?;
    log::debug!("[vault] remove: id={id} removed={removed} purge_memory={purge_memory}");

    let mut purged = false;
    if removed && purge_memory {
        if let Some(v) = vault {
            if let Err(err) = clear_namespace(ClearNamespaceParams {
                namespace: v.namespace.clone(),
            })
            .await
            {
                log::warn!("[vault] remove: id={id} purge_namespace_failed err={err}");
                return Ok(RpcOutcome::single_log(
                    serde_json::json!({
                        "vault_id": id,
                        "removed": removed,
                        "purged": false,
                        "purge_error": err,
                    }),
                    format!("vault removed with purge error: {id}"),
                ));
            }
            purged = true;
        }
    }

    Ok(RpcOutcome::single_log(
        serde_json::json!({
            "vault_id": id,
            "removed": removed,
            "purged": purged,
        }),
        format!("vault removed: {id}"),
    ))
}

/// Trigger an immediate sync of a vault. Blocks until complete.
pub async fn vault_sync(config: &Config, id: &str) -> Result<RpcOutcome<VaultSyncReport>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("vault_id must not be empty".to_string());
    }
    let vault = store::get_vault(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("vault not found: {id}"))?;
    log::debug!("[vault] sync: entry id={id} root={:?}", vault.root_path);
    let report = sync::sync_vault(config, &vault).await;
    log::debug!(
        "[vault] sync: exit id={id} scanned={} ingested={} unchanged={} removed={} failed={} skipped={} duration_ms={}",
        report.scanned,
        report.ingested,
        report.unchanged,
        report.removed,
        report.failed,
        report.skipped_unsupported,
        report.duration_ms,
    );
    let msg = format!(
        "vault sync done — ingested {}, unchanged {}, removed {}, failed {}",
        report.ingested, report.unchanged, report.removed, report.failed
    );
    Ok(RpcOutcome::single_log(report, msg))
}
