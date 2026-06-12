//! One-shot SQLite migrations for the chunks DB.
//!
//! These functions are called from [`super::connection`] during DB initialisation.
//! Each migration is version-gated via `PRAGMA user_version` so it runs exactly
//! once per vault.

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::{
    has_uncovered_reembed_work, set_chunk_embedding_for_signature_tx,
    GLOBAL_TOPIC_PURGE_MIGRATION_VERSION, TREE_EMBEDDING_MIGRATION_VERSION,
};
use crate::openhuman::config::Config;

/// One-shot migration: copy legacy per-chunk/summary `.embedding` blobs into the
/// normalised `mem_tree_chunk_embeddings` / `mem_tree_summary_embeddings` sidecar
/// tables introduced in #1574.
///
/// Version-gated: `PRAGMA user_version < 1` triggers the copy; `>= 1` is a no-op.
pub(super) fn migrate_legacy_embeddings_to_sidecar(
    conn: &Connection,
    config: &Config,
) -> Result<()> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .context("read PRAGMA user_version for #1574 migration")?;
    if version >= TREE_EMBEDDING_MIGRATION_VERSION {
        return Ok(());
    }

    let (provider, model, dims) = crate::openhuman::memory_store::effective_embedding_settings(
        &config.memory,
        config.workload_local_model("embeddings").as_deref(),
    );
    let sig = crate::openhuman::embeddings::format_embedding_signature(&provider, &model, dims);
    log::info!(
        "[memory_tree::migrate] #1574 §7: copying legacy embeddings → sidecar at sig={sig} (dims={dims})"
    );

    let tx = conn.unchecked_transaction()?;
    let mut copied_chunks = 0usize;
    let mut copied_summaries = 0usize;
    let mut skipped_dim_mismatch = 0usize;

    for (table, is_chunk) in [("mem_tree_chunks", true), ("mem_tree_summaries", false)] {
        let mut stmt = tx.prepare(&format!(
            "SELECT id, embedding FROM {table} WHERE embedding IS NOT NULL"
        ))?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;
        for row in rows {
            let (id, blob) = row?;
            if !blob.len().is_multiple_of(4) {
                log::warn!(
                    "[memory_tree::migrate] {table} id={id}: legacy blob len {} not /4, skipping",
                    blob.len()
                );
                continue;
            }
            if blob.len() / 4 != dims {
                // Different embedding space — unrecoverable from the blob.
                // Leave for the §6 re-embed backfill.
                skipped_dim_mismatch += 1;
                continue;
            }
            let vec: Vec<f32> = blob
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            if is_chunk {
                set_chunk_embedding_for_signature_tx(&tx, &id, &sig, &vec)?;
                copied_chunks += 1;
            } else {
                crate::openhuman::memory_store::trees::store::set_summary_embedding_for_signature_tx(
                    &tx, &id, &sig, &vec,
                )?;
                copied_summaries += 1;
            }
        }
    }

    // #1574 §6: enqueue the re-embed backfill ONLY if there is genuinely
    // uncovered work at the active signature (the dim-mismatch slice, or
    // content-bearing rows with no vector). Gating this avoids queuing a
    // no-op job on every DB open — which would otherwise pollute the jobs
    // table for unrelated callers/tests. Enqueued atomically with the
    // migration; dedupe key = signature, so exactly one chain per space.
    let has_uncovered = has_uncovered_reembed_work(&*tx, &sig)?;
    if has_uncovered {
        let backfill_job = crate::openhuman::memory_queue::types::NewJob::reembed_backfill(
            &crate::openhuman::memory_queue::types::ReembedBackfillPayload {
                signature: sig.clone(),
            },
        )?;
        crate::openhuman::memory_queue::enqueue_tx(&tx, &backfill_job)?;
    }

    tx.commit()?;
    conn.pragma_update(None, "user_version", TREE_EMBEDDING_MIGRATION_VERSION)
        .context("set PRAGMA user_version after #1574 migration")?;
    if has_uncovered {
        crate::openhuman::memory_queue::set_backfill_in_progress(true);
    }
    log::info!(
        "[memory_tree::migrate] #1574 §7 done: copied chunks={copied_chunks} summaries={copied_summaries} \
         skipped_dim_mismatch={skipped_dim_mismatch} (left for §6 re-embed); user_version={TREE_EMBEDDING_MIGRATION_VERSION}"
    );
    Ok(())
}

/// One-shot purge of the removed global + topic trees.
///
/// The global (time-axis) and topic (subject-axis) trees were deleted in
/// favour of the source trees (which hold all content). This migration
/// removes their now-orphaned DB rows and on-disk summary folders so old
/// vaults clean themselves up on next open. Version-gated via
/// `PRAGMA user_version` (see [`GLOBAL_TOPIC_PURGE_MIGRATION_VERSION`]); a
/// no-op on workspaces that never had those trees.
pub(super) fn purge_global_topic_trees(conn: &Connection, config: &Config) -> Result<()> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .context("read PRAGMA user_version for global/topic purge")?;
    if version >= GLOBAL_TOPIC_PURGE_MIGRATION_VERSION {
        return Ok(());
    }

    let tx = conn.unchecked_transaction()?;
    // Child rows first (summary sidecars / skip-lists are keyed by
    // summary_id; entity-index + buffers carry an FK on tree_id).
    let removed_summary_sidecars = tx.execute(
        "DELETE FROM mem_tree_summary_embeddings WHERE summary_id IN \
         (SELECT id FROM mem_tree_summaries WHERE tree_kind IN ('global','topic'))",
        [],
    )?;
    tx.execute(
        "DELETE FROM mem_tree_summary_reembed_skipped WHERE summary_id IN \
         (SELECT id FROM mem_tree_summaries WHERE tree_kind IN ('global','topic'))",
        [],
    )?;
    tx.execute(
        "DELETE FROM mem_tree_entity_index WHERE tree_id IN \
         (SELECT id FROM mem_tree_trees WHERE kind IN ('global','topic'))",
        [],
    )?;
    let removed_summaries = tx.execute(
        "DELETE FROM mem_tree_summaries WHERE tree_kind IN ('global','topic')",
        [],
    )?;
    tx.execute(
        "DELETE FROM mem_tree_buffers WHERE tree_id IN \
         (SELECT id FROM mem_tree_trees WHERE kind IN ('global','topic'))",
        [],
    )?;
    let removed_trees = tx.execute(
        "DELETE FROM mem_tree_trees WHERE kind IN ('global','topic')",
        [],
    )?;
    // Drain any queued jobs for the retired kinds so the worker loop never
    // trips over a payload it can no longer parse.
    let removed_jobs = tx.execute(
        "DELETE FROM mem_tree_jobs WHERE kind IN ('topic_route','digest_daily')",
        [],
    )?;
    tx.commit()?;

    // On-disk: drop the `wiki/summaries/global*` (both the legacy per-day
    // `global-<date>/` folders and the singleton `global/`) and `topic-*`
    // summary folders. Best-effort — a filesystem error must not abort the
    // version bump, or the purge would retry forever.
    let summaries_root = config
        .memory_tree_content_root()
        .join("wiki")
        .join("summaries");
    let mut removed_dirs = 0usize;
    if let Ok(entries) = std::fs::read_dir(&summaries_root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("global") || name.starts_with("topic-") {
                match std::fs::remove_dir_all(entry.path()) {
                    Ok(()) => removed_dirs += 1,
                    Err(e) => log::warn!(
                        "[memory_tree::migrate] purge: failed to remove {} : {e}",
                        entry.path().display()
                    ),
                }
            }
        }
    }

    conn.pragma_update(None, "user_version", GLOBAL_TOPIC_PURGE_MIGRATION_VERSION)
        .context("set PRAGMA user_version after global/topic purge")?;
    log::info!(
        "[memory_tree::migrate] global/topic purge done: trees={removed_trees} \
         summaries={removed_summaries} sidecars={removed_summary_sidecars} jobs={removed_jobs} \
         dirs={removed_dirs}; user_version={GLOBAL_TOPIC_PURGE_MIGRATION_VERSION}"
    );
    Ok(())
}
