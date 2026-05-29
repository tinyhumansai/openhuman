//! RPC-facing operations for the vault domain.

use chrono::Utc;
use futures::FutureExt;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

use crate::openhuman::config::Config;
use crate::openhuman::memory::ops::{clear_namespace, ClearNamespaceParams};
use crate::openhuman::memory_store::chunks::store::delete_chunks_by_source_prefix;
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::rpc::RpcOutcome;

use super::state;
use super::store;
use super::sync;
use super::types::{
    Vault, VaultFile, VaultSyncState, VaultSyncStatus, VaultWriteMarkdownReport, VaultWriteState,
};

/// Derive a stable memory namespace for a vault without embedding the raw UUID.
///
/// Memory writes reject namespace/key values that resemble PII. Raw UUID hex can
/// occasionally match strict alphanumeric identifier patterns, so vault
/// namespaces use an alphabet-only digest suffix instead.
pub(crate) fn vault_namespace_for_id(id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    let suffix: String = digest
        .iter()
        .take(24)
        .map(|byte| char::from(b'a' + (byte % 26)))
        .collect();
    format!("vault-{suffix}")
}

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
    let namespace = vault_namespace_for_id(&id);
    let canonical_root = root
        .canonicalize()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| trimmed_root.to_string());
    let (write_state, write_state_reason) = store::vault_write_state_for_root_path(&canonical_root);
    let vault = Vault {
        id: id.clone(),
        name: trimmed_name.to_string(),
        root_path: canonical_root,
        host_os: Some(store::current_host_os().to_string()),
        namespace,
        include_globs,
        exclude_globs,
        created_at: Utc::now(),
        last_synced_at: None,
        file_count: 0,
        write_state,
        write_state_reason,
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

/// Write an explicitly approved markdown/wiki artifact into a registered vault.
pub async fn vault_write_markdown(
    config: &Config,
    id: &str,
    rel_path: &str,
    content: &str,
    overwrite: bool,
    approved: bool,
) -> Result<RpcOutcome<VaultWriteMarkdownReport>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("vault_id must not be empty".to_string());
    }
    if !approved {
        log::debug!("[vault] write_markdown: rejected missing approval id={id}");
        return Err("vault markdown writes require explicit user approval".to_string());
    }

    let vault = store::get_vault(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("vault not found: {id}"))?;
    let (write_state, write_reason) = store::vault_write_state_for_root_path(&vault.root_path);
    if write_state != VaultWriteState::Writable {
        log::debug!(
            "[vault] write_markdown: rejected non-writable id={id} state={write_state:?} reason={write_reason:?}"
        );
        return Err(store::vault_write_state_reason_message(write_reason.as_deref()).to_string());
    }

    let rel = validate_markdown_rel_path(rel_path)?;
    let bytes = content.as_bytes().len() as u64;
    let root = std::fs::canonicalize(&vault.root_path)
        .map_err(|err| format!("failed to resolve vault folder: {err}"))?;
    ensure_existing_ancestors_stay_in_root(&root, &rel)?;
    let target = root.join(&rel);
    let parent = target
        .parent()
        .ok_or_else(|| "target path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("failed to create vault note directory: {err}"))?;
    let parent_canon = std::fs::canonicalize(parent)
        .map_err(|err| format!("failed to resolve vault note directory: {err}"))?;
    if !parent_canon.starts_with(&root) {
        return Err("vault note path resolves outside the vault folder".to_string());
    }
    if let Ok(meta) = std::fs::symlink_metadata(&target) {
        if meta.file_type().is_symlink() {
            return Err("refusing to write through a symlink inside the vault".to_string());
        }
    }
    let created = !target.exists();
    if !created && !overwrite {
        return Err(
            "vault markdown file already exists; set overwrite=true to update it".to_string(),
        );
    }

    log::debug!(
        "[vault] write_markdown: writing id={id} rel_path={} bytes={bytes} overwrite={overwrite} created={created}",
        rel.display()
    );
    std::fs::write(&target, content)
        .map_err(|err| format!("failed to write vault markdown file: {err}"))?;

    Ok(RpcOutcome::single_log(
        VaultWriteMarkdownReport {
            vault_id: id.to_string(),
            rel_path: rel.to_string_lossy().replace('\\', "/"),
            bytes_written: bytes,
            created,
        },
        format!("vault markdown written: {id}"),
    ))
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

fn validate_markdown_rel_path(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("rel_path must not be empty".to_string());
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err("rel_path must be relative to the vault folder".to_string());
    }

    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err("rel_path must not contain '..' segments".to_string());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("rel_path must stay inside the vault folder".to_string());
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err("rel_path must name a markdown file".to_string());
    }
    let ext = clean
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown") {
        return Err("rel_path must end with .md or .markdown".to_string());
    }
    Ok(clean)
}

fn ensure_existing_ancestors_stay_in_root(root: &Path, rel_path: &Path) -> Result<(), String> {
    let Some(parent) = rel_path.parent() else {
        return Ok(());
    };

    let mut current = root.to_path_buf();
    for component in parent.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        let next = current.join(part);
        match std::fs::symlink_metadata(&next) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let resolved = std::fs::canonicalize(&next)
                    .map_err(|err| format!("failed to resolve vault note directory: {err}"))?;
                if !resolved.starts_with(root) {
                    return Err(
                        "vault note directory resolves outside the vault folder".to_string()
                    );
                }
                current = resolved;
            }
            Ok(meta) if meta.is_dir() => {
                current = next;
            }
            Ok(_) => {
                return Err("vault note parent path is not a directory".to_string());
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                break;
            }
            Err(err) => {
                return Err(format!("failed to inspect vault note directory: {err}"));
            }
        }
    }

    Ok(())
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
