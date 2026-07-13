use anyhow::{Context, Result};
use rusqlite::params;

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::chunks::store::{
    delete_chunks_by_source, delete_orphaned_source_tree, with_connection,
};
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::rpc::RpcOutcome;

use super::types::{
    DeleteSourceResponse, FlushNowResponse, FlushSourceTreeResponse, ResetTreeResponse,
    WipeAllResponse,
};

// ── wipe_all ─────────────────────────────────────────────────────────────

pub async fn wipe_all_rpc(config: &Config) -> Result<RpcOutcome<WipeAllResponse>, String> {
    let cfg = config.clone();
    let (rows_deleted, sync_state_cleared) =
        tokio::task::spawn_blocking(move || -> Result<(u64, u64)> {
            const TABLES: &[&str] = &[
                "mem_tree_score",
                "mem_tree_entity_index",
                "mem_tree_entity_hotness",
                "mem_tree_jobs",
                "mem_tree_buffers",
                "mem_tree_summaries",
                "mem_tree_trees",
                "mem_tree_chunks",
                "mem_tree_ingested_sources",
            ];
            let rows_deleted: u64 = with_connection(&cfg, |conn| {
                let tx = conn.unchecked_transaction()?;
                let mut total: u64 = 0;
                for table in TABLES {
                    let n = tx
                        .execute(&format!("DELETE FROM {table}"), [])
                        .with_context(|| format!("delete from {table}"))?;
                    total += n as u64;
                }
                tx.commit()?;
                Ok(total)
            })?;

            let sync_state_cleared: u64 = {
                let unified_db = cfg.workspace_dir.join("memory").join("memory.db");
                if !unified_db.exists() {
                    log::debug!(
                        "[memory_tree::read::wipe] unified memory DB not present — skipping sync-state clear"
                    );
                    0
                } else {
                    clear_composio_sync_state(&unified_db)
                        .context("clear composio-sync-state during wipe_all")?
                }
            };

            Ok((rows_deleted, sync_state_cleared))
        })
        .await
        .map_err(|e| format!("wipe_all join error: {e}"))?
        .map_err(|e| format!("wipe_all: {e:#}"))?;

    const DIRS: &[&str] = &["raw", "wiki", "chat", "document", "email", "summaries"];
    let content_root = config.memory_tree_content_root();
    let mut dirs_removed: Vec<String> = Vec::new();
    for dir in DIRS {
        let path = content_root.join(dir);
        let remove_result = crate::openhuman::util::retry_with_backoff_async(
            &format!("remove dir {}", dir),
            6,
            200,
            || async {
                tokio::fs::remove_dir_all(&path)
                    .await
                    .context("remove_dir_all")
            },
        )
        .await;

        match remove_result {
            Ok(()) => dirs_removed.push((*dir).to_string()),
            Err(e) => {
                let is_not_found = e
                    .chain()
                    .find_map(|e| e.downcast_ref::<std::io::Error>())
                    .is_some_and(|ioe| ioe.kind() == std::io::ErrorKind::NotFound);
                if !is_not_found {
                    log::warn!(
                        "[memory_tree::read::wipe] failed to remove dir={} err={:#}",
                        dir,
                        e
                    );
                }
            }
        }
    }

    let resp = WipeAllResponse {
        rows_deleted,
        dirs_removed,
        sync_state_cleared,
    };

    let log = format!(
        "memory_tree::read: wipe_all rows={} dirs={:?} sync_state={}",
        resp.rows_deleted, resp.dirs_removed, resp.sync_state_cleared
    );
    Ok(RpcOutcome::single_log(resp, log))
}

pub(crate) fn clear_composio_sync_state(db_path: &std::path::Path) -> Result<u64> {
    use crate::openhuman::tinycortex::HOST_SYNC_STATE_NAMESPACE;
    let conn = rusqlite::Connection::open(db_path)
        .with_context(|| format!("open unified memory db {}", db_path.display()))?;
    let n = conn
        .execute(
            "DELETE FROM kv_namespace WHERE namespace = ?1",
            params![HOST_SYNC_STATE_NAMESPACE],
        )
        .context("delete composio-sync-state rows")?;
    Ok(n as u64)
}

// ── reset_tree ───────────────────────────────────────────────────────────

pub async fn reset_tree_rpc(config: &Config) -> Result<RpcOutcome<ResetTreeResponse>, String> {
    use crate::openhuman::memory_queue::store as jobs_store;
    use crate::openhuman::memory_queue::types::{ExtractChunkPayload, NewJob};

    let cfg = config.clone();
    let (tree_rows_deleted, chunks_requeued, jobs_enqueued) =
        tokio::task::spawn_blocking(move || -> Result<(u64, u64, u64)> {
            const TREE_TABLES: &[&str] = &[
                "mem_tree_summaries",
                "mem_tree_buffers",
                "mem_tree_jobs",
                "mem_tree_entity_index",
                "mem_tree_trees",
            ];
            let tree_rows_deleted: u64 = with_connection(&cfg, |conn| {
                let tx = conn.unchecked_transaction()?;
                let mut total: u64 = 0;
                for table in TREE_TABLES {
                    let n = tx
                        .execute(&format!("DELETE FROM {table}"), [])
                        .with_context(|| format!("delete from {table}"))?;
                    total += n as u64;
                }
                tx.commit()?;
                Ok(total)
            })?;

            let (chunks_requeued, jobs_enqueued) =
                with_connection(&cfg, |conn| -> anyhow::Result<(u64, u64)> {
                    let tx = conn.unchecked_transaction()?;
                    let chunks_requeued = tx.execute(
                        "UPDATE mem_tree_chunks SET lifecycle_status = 'pending_extraction'",
                        [],
                    )? as u64;
                    let chunk_ids: Vec<String> = {
                        let mut stmt = tx.prepare("SELECT id FROM mem_tree_chunks")?;
                        let rows = stmt
                            .query_map([], |r| r.get::<_, String>(0))?
                            .collect::<rusqlite::Result<Vec<_>>>()
                            .context("collect chunk ids")?;
                        rows
                    };
                    let mut jobs_enqueued: u64 = 0;
                    for id in &chunk_ids {
                        let payload = ExtractChunkPayload {
                            chunk_id: id.clone(),
                        };
                        let job = NewJob::extract_chunk(&payload)
                            .context("build extract_chunk NewJob")?;
                        if jobs_store::enqueue_tx(&tx, &job)
                            .context("enqueue extract_chunk")?
                            .is_some()
                        {
                            jobs_enqueued += 1;
                        }
                    }
                    tx.commit()?;
                    Ok((chunks_requeued, jobs_enqueued))
                })?;

            Ok((tree_rows_deleted, chunks_requeued, jobs_enqueued))
        })
        .await
        .map_err(|e| format!("reset_tree join error: {e}"))?
        .map_err(|e| format!("reset_tree: {e:#}"))?;

    let summaries_dir = config
        .memory_tree_content_root()
        .join("wiki")
        .join("summaries");
    let remove_result = crate::openhuman::util::retry_with_backoff_async(
        "remove wiki/summaries",
        6,
        200,
        || async {
            tokio::fs::remove_dir_all(&summaries_dir)
                .await
                .context("remove_dir_all")
        },
    )
    .await;

    match remove_result {
        Ok(()) => log::debug!("[memory_tree::read::reset_tree] removed wiki/summaries"),
        Err(e) => {
            let is_not_found = e
                .chain()
                .find_map(|e| e.downcast_ref::<std::io::Error>())
                .is_some_and(|ioe| ioe.kind() == std::io::ErrorKind::NotFound);
            if !is_not_found {
                log::warn!(
                    "[memory_tree::read::reset_tree] failed to remove wiki/summaries: {:#}",
                    e
                )
            }
        }
    }

    crate::openhuman::memory_queue::wake_workers();

    let resp = ResetTreeResponse {
        tree_rows_deleted,
        chunks_requeued,
        jobs_enqueued,
    };

    let log = format!(
        "memory_tree::read: reset_tree tree_rows={} chunks={} jobs={}",
        resp.tree_rows_deleted, resp.chunks_requeued, resp.jobs_enqueued
    );
    Ok(RpcOutcome::single_log(resp, log))
}

// ── flush_source_tree ────────────────────────────────────────────────────

pub async fn flush_source_tree_rpc(
    config: &Config,
    source_scope: &str,
) -> Result<RpcOutcome<FlushSourceTreeResponse>, String> {
    use crate::openhuman::memory::tree_source::get_or_create_source_tree;

    use crate::openhuman::memory_tree::tree::flush::force_flush_tree;
    use crate::openhuman::memory_tree::tree::TreeFactory;
    use std::collections::HashSet;
    use std::sync::Mutex;

    static ACTIVE: std::sync::LazyLock<Mutex<HashSet<String>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

    let scope = source_scope.to_string();

    {
        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if !active.insert(scope.clone()) {
            return Ok(RpcOutcome::single_log(
                FlushSourceTreeResponse {
                    tree_scope: scope,
                    seals_fired: 0,
                },
                "memory_tree::read: flush_source_tree already running for this scope".to_string(),
            ));
        }
    }

    let cfg = config.clone();
    let scope_for_task = scope.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<FlushSourceTreeResponse> {
        let tree = get_or_create_source_tree(&cfg, &scope_for_task)
            .context("get_or_create_source_tree")?;
        let _strategy = TreeFactory::from_tree(&tree).label_strategy(&cfg);
        Ok(FlushSourceTreeResponse {
            tree_scope: scope_for_task,
            seals_fired: 0,
        })
    })
    .await
    .map_err(|e| format!("flush_source_tree join error: {e}"))?;

    let _tree_info = result.map_err(|e| format!("flush_source_tree: {e:#}"))?;

    let cfg2 = config.clone();
    let scope2 = scope.clone();
    let resp = tokio::spawn(async move {
        let tree = get_or_create_source_tree(&cfg2, &scope2)?;
        let strategy = TreeFactory::from_tree(&tree).label_strategy(&cfg2);
        let sealed = force_flush_tree(&cfg2, &tree.id, Some(chrono::Utc::now()), &strategy).await?;
        Ok::<_, anyhow::Error>(FlushSourceTreeResponse {
            tree_scope: scope2,
            seals_fired: sealed.len() as u32,
        })
    })
    .await
    .map_err(|e| format!("flush_source_tree join error: {e}"))?
    .map_err(|e| format!("flush_source_tree: {e:#}"))?;

    {
        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        active.remove(&scope);
    }

    let log = format!(
        "memory_tree::read: flush_source_tree scope={} seals={}",
        resp.tree_scope, resp.seals_fired
    );
    Ok(RpcOutcome::single_log(resp, log))
}

// ── flush_now ─────────────────────────────────────────────────────────────

pub async fn flush_now_rpc(config: &Config) -> Result<RpcOutcome<FlushNowResponse>, String> {
    use crate::openhuman::memory_queue::store as jobs_store;
    use crate::openhuman::memory_queue::types::{FlushStalePayload, NewJob};
    use crate::openhuman::memory_tree::tree::store as tree_store;

    let cfg = config.clone();
    let resp = tokio::task::spawn_blocking(move || -> Result<FlushNowResponse> {
        let stale = tree_store::list_stale_buffers(&cfg, chrono::Utc::now())
            .context("list stale buffers")?;
        let stale_buffers = stale.len() as u32;

        let payload = FlushStalePayload {
            max_age_secs: Some(0),
        };
        let now = chrono::Utc::now();
        let date_iso = now.format("%Y-%m-%d").to_string();
        let hour_block = chrono::Timelike::hour(&now) / 3;
        let job = NewJob::flush_stale(&payload, &date_iso, hour_block)
            .context("build flush_stale NewJob")?;
        let enqueued = jobs_store::enqueue(&cfg, &job)
            .context("enqueue flush_stale job")?
            .is_some();
        Ok(FlushNowResponse {
            enqueued,
            stale_buffers,
        })
    })
    .await
    .map_err(|e| format!("flush_now join error: {e}"))?
    .map_err(|e| format!("flush_now: {e:#}"))?;

    let log = format!(
        "memory_tree::read: flush_now enqueued={} stale_buffers={}",
        resp.enqueued, resp.stale_buffers
    );
    Ok(RpcOutcome::single_log(resp, log))
}

// ── delete_source ──────────────────────────────────────────────────────────

/// Fully delete one document source by its **exact** `source_id`.
///
/// Unlike [`super::entities::delete_chunk_rpc`] (which removes a single chunk and
/// leaves the source tree intact), this wraps
/// [`delete_chunks_by_source`] so the whole logical source is purged: every
/// chunk plus its score / entity-index / embedding / reembed-skip side rows and
/// chunk content files, the ingest dedup gate, and — when the source becomes
/// fully orphaned — its source summary tree via `delete_tree_cascade_tx`
/// (summaries, summary embeddings + reembed-skip, tree-keyed entity-index,
/// buffers, the tree row, and summary content files). This prevents stale
/// summaries of a deleted note/event/meeting from resurfacing in semantic
/// recall.
///
/// Matching is **exact** (never a prefix), so sibling sources sharing a prefix
/// are untouched. Idempotent: an unknown `source_id` removes nothing and returns
/// `deleted = false`.
///
/// Legacy cleanup: a source whose chunks were already removed earlier by the
/// per-chunk path keeps a now-stale summary tree (which `delete_chunks_by_source`
/// won't touch, since it only cascades trees for chunks it deletes in the same
/// call). After the chunk delete we therefore also run
/// [`delete_orphaned_source_tree`], which cascades the source-scoped tree and
/// clears the bare + versioned (`{source_id}@{version_ms}`) ingest gates iff zero
/// chunks remain — so calling delete_source over such a source finishes the job.
///
/// Scope: this deletes the exact document source and its **source-scoped** orphan
/// tree. It intentionally does NOT tear down shared collection / `path_scope`
/// trees (e.g. Notion `notion:{connection}`), which may summarise many documents;
/// per-document pruning inside a shared collection summary is out of scope here.
pub async fn delete_source_rpc(
    config: &Config,
    source_id: String,
) -> Result<RpcOutcome<DeleteSourceResponse>, String> {
    let source_id = source_id.trim().to_string();
    if source_id.is_empty() {
        return Err("delete_source: source_id must be a non-empty string".to_string());
    }
    let cfg = config.clone();
    let id = source_id.clone();
    let (chunks_removed, tree_cleaned) =
        tokio::task::spawn_blocking(move || -> Result<(usize, bool)> {
            // Exact-match document delete; cascade logic lives in the store.
            let removed = delete_chunks_by_source(&cfg, SourceKind::Document, &id)
                .context("delete_chunks_by_source during delete_source")?;
            // Finish off any legacy partial delete: cascade a now-orphaned tree
            // (no-op when chunks were just deleted — the tree is already gone —
            // or when there is no stale tree).
            let tree_cleaned = delete_orphaned_source_tree(&cfg, SourceKind::Document, &id)
                .context("delete_orphaned_source_tree during delete_source")?;
            Ok((removed, tree_cleaned))
        })
        .await
        .map_err(|e| format!("delete_source join error: {e}"))?
        .map_err(|e| format!("delete_source: {e:#}"))?;

    let resp = DeleteSourceResponse {
        // `deleted` is true if we removed chunks OR cleaned a stale orphaned tree
        // (the legacy-cleanup case has chunks_removed == 0 but still did work).
        deleted: chunks_removed > 0 || tree_cleaned,
        chunks_removed: chunks_removed as u64,
    };
    let log = format!(
        // Redact the source id: it can embed user-linked identifiers.
        "memory_tree::read: delete_source source_id_hash={} deleted={} chunks_removed={} tree_cleaned={}",
        crate::openhuman::memory::util::redact::redact(&source_id),
        resp.deleted,
        resp.chunks_removed,
        tree_cleaned
    );
    Ok(RpcOutcome::single_log(resp, log))
}
