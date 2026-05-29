//! RPC-facing operations for the vault domain.

use chrono::Utc;
use futures::FutureExt;
use uuid::Uuid;

use crate::openhuman::config::Config;
use crate::openhuman::memory::ops::{clear_namespace, ClearNamespaceParams};
use crate::openhuman::memory_store::chunks::store::delete_chunks_by_source_prefix;
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::rpc::RpcOutcome;

use super::state;
use super::store;
use super::sync;
use super::types::{Vault, VaultFile, VaultSyncState, VaultSyncStatus};

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
        host_os: Some(store::current_host_os().to_string()),
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
    store::get_vault(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("vault not found: {id}"))?;
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
    let mut memory_tree_chunks_deleted: usize = 0;
    if removed && purge_memory {
        if let Some(v) = vault {
            // Memory-tree cleanup is the canonical path post-#2705: vault
            // sync writes to `mem_tree_chunks` / `mem_tree_ingested_sources`
            // keyed by `vault:{id}:{rel_path}`. A prefix delete with
            // `vault:{id}:` catches every per-file row for this vault. The
            // companion `clear_namespace` call below still drains any
            // pre-#2705 ledger rows that landed in the legacy
            // `memory_docs` table during the migration window.
            let cfg_for_blocking = config.clone();
            let prefix = format!("vault:{}:", v.id);
            let tree_result = tokio::task::spawn_blocking(move || {
                delete_chunks_by_source_prefix(&cfg_for_blocking, SourceKind::Document, &prefix)
            })
            .await;
            match tree_result {
                Ok(Ok(removed_chunks)) => {
                    memory_tree_chunks_deleted = removed_chunks;
                    log::debug!(
                        "[vault] remove: id={id} memory_tree_chunks_deleted={removed_chunks}"
                    );
                }
                Ok(Err(err)) => {
                    log::warn!("[vault] remove: id={id} memory_tree_purge_failed err={err}");
                    return Ok(RpcOutcome::single_log(
                        serde_json::json!({
                            "vault_id": id,
                            "removed": removed,
                            "purged": false,
                            "purge_error": format!("memory_tree purge failed: {err}"),
                        }),
                        format!("vault removed with purge error: {id}"),
                    ));
                }
                Err(join_err) => {
                    log::warn!(
                        "[vault] remove: id={id} memory_tree_purge_join_failed err={join_err}"
                    );
                    return Ok(RpcOutcome::single_log(
                        serde_json::json!({
                            "vault_id": id,
                            "removed": removed,
                            "purged": false,
                            "purge_error": format!("memory_tree purge join error: {join_err}"),
                        }),
                        format!("vault removed with purge error: {id}"),
                    ));
                }
            }

            // Best-effort legacy UnifiedMemory purge for pre-#2705 ledger
            // rows whose chunks still live in `memory_docs`. Failure here
            // doesn't undo the canonical memory_tree cleanup above.
            if let Err(err) = clear_namespace(ClearNamespaceParams {
                namespace: v.namespace.clone(),
            })
            .await
            {
                log::debug!(
                    "[vault] remove: id={id} legacy_clear_namespace_failed (best-effort) err={err}"
                );
            }
            purged = true;
        }
    }

    Ok(RpcOutcome::single_log(
        serde_json::json!({
            "vault_id": id,
            "removed": removed,
            "purged": purged,
            "memory_tree_chunks_deleted": memory_tree_chunks_deleted,
        }),
        format!("vault removed: {id}"),
    ))
}

/// Trigger a vault sync as a background task and return immediately.
///
/// The caller should poll `vault_sync_status` to track progress and retrieve
/// the final outcome.  Returns an error if a sync is already running for this
/// vault so the caller can surface a user-friendly message instead of silently
/// queuing a duplicate.
pub async fn vault_sync(
    config: &Config,
    id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("vault_id must not be empty".to_string());
    }
    let vault = store::get_vault(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("vault not found: {id}"))?;

    // Register in the state map; returns Err if already running.
    let started_at_ms = Utc::now().timestamp_millis();
    state::start(id, started_at_ms).map_err(|e| format!("sync already in progress: {e}"))?;

    log::debug!(
        "[vault] sync: background task spawned id={id} root={:?}",
        vault.root_path,
    );

    // Clone what the background task needs — Config is Clone (derives it).
    let config_clone = config.clone();
    let vault_id = id.to_string();

    tokio::spawn(async move {
        log::debug!("[vault] sync: background task running id={vault_id}");

        // Wrap the work in catch_unwind so a panic inside sync_vault cannot leave
        // the vault state permanently stuck in `Running`.  Without this guard a
        // panic would unwind the task, the state map entry would never be updated,
        // and every subsequent sync attempt would be rejected with "already in progress"
        // until the app is restarted.
        let result =
            std::panic::AssertUnwindSafe(async { sync::sync_vault(&config_clone, &vault).await })
                .catch_unwind()
                .await;

        match result {
            Ok(report) => {
                let success = report.failed == 0;
                let finished_at_ms = Utc::now().timestamp_millis();

                // Write final counters back into the state map.
                state::update_progress(&vault_id, |s| {
                    s.status = if success {
                        VaultSyncStatus::Completed
                    } else {
                        VaultSyncStatus::Failed
                    };
                    s.finished_at_ms = Some(finished_at_ms);
                    s.ingested = report.ingested;
                    s.unchanged = report.unchanged;
                    s.removed = report.removed;
                    s.failed = report.failed;
                    s.skipped_unsupported = report.skipped_unsupported;
                    s.scanned = report.scanned;
                    s.duration_ms = report.duration_ms;
                    s.errors = report.errors.clone();
                });

                log::debug!(
                    "[vault] sync: background task done id={vault_id} ingested={} failed={} duration_ms={}",
                    report.ingested,
                    report.failed,
                    report.duration_ms,
                );
            }
            Err(_) => {
                log::error!(
                    "[vault] sync: background task panicked id={vault_id} — marking state as Failed"
                );
                state::update_progress(&vault_id, |s| {
                    s.status = VaultSyncStatus::Failed;
                    s.errors = vec!["sync task panicked unexpectedly".to_string()];
                });
            }
        }
    });

    Ok(RpcOutcome::single_log(
        serde_json::json!({ "status": "started", "vault_id": id }),
        format!("vault sync started in background: {id}"),
    ))
}

/// Return the current sync progress for a vault.
///
/// Returns an `Idle` state if no sync has ever run for this vault.
pub async fn vault_sync_status(id: &str) -> Result<RpcOutcome<VaultSyncState>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("vault_id must not be empty".to_string());
    }
    let st = state::get(id).unwrap_or_else(|| VaultSyncState {
        vault_id: id.to_string(),
        status: VaultSyncStatus::Idle,
        scanned: 0,
        ingested: 0,
        unchanged: 0,
        removed: 0,
        failed: 0,
        skipped_unsupported: 0,
        total: 0,
        started_at_ms: 0,
        finished_at_ms: None,
        duration_ms: 0,
        errors: vec![],
    });
    log::debug!(
        "[vault] sync_status: id={id} status={:?} ingested={} total={}",
        st.status,
        st.ingested,
        st.total,
    );
    Ok(RpcOutcome::single_log(st, "vault sync status"))
}
