use super::with_connection;
use crate::openhuman::config::Config;
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;

// ── Phase 2: embedding column accessors ─────────────────────────────────

/// Resolve the active embedding signature for the memory tree from the global
/// [`Config`] — the canonical key every per-model sidecar read/write is scoped
/// by (#1574). Reuses the established local-AI workload derivation
/// ([`Config::workload_local_model`]) and the probe-stable
/// `active_embedding_signature`; introduces no parallel resolution path.
/// `pub(crate)` so the sibling `tree` summary store shares the exact
/// same resolution.
pub(crate) fn tree_active_signature(config: &Config) -> String {
    let local_model = config.workload_local_model("embeddings");
    crate::openhuman::memory_store::active_embedding_signature(
        &config.memory,
        local_model.as_deref(),
    )
}

/// Store a chunk's embedding under the active model signature.
///
/// #1574 cutover: this now writes the per-model `mem_tree_chunk_embeddings`
/// sidecar (via [`set_chunk_embedding_for_signature`]) instead of the legacy
/// `mem_tree_chunks.embedding` column. Call sites are unchanged — the signature
/// is resolved internally from `config`. The legacy column is left intact for
/// the §7 one-shot migration to read; it is dropped only in a later release.
pub fn set_chunk_embedding(config: &Config, chunk_id: &str, embedding: &[f32]) -> Result<()> {
    let signature = tree_active_signature(config);
    log::debug!(
        "[memory::chunk_store] set_chunk_embedding: chunk_id={chunk_id} sig={signature} dims={}",
        embedding.len()
    );
    set_chunk_embedding_for_signature(config, chunk_id, &signature, embedding)
}

/// Core upsert into `mem_tree_chunk_embeddings` over an arbitrary
/// `&Connection`. Shared by the standalone ([`set_chunk_embedding_for_signature`])
/// and in-transaction ([`set_chunk_embedding_for_signature_tx`]) write paths so
/// the SQL exists exactly once. `rusqlite::Transaction` derefs to `Connection`,
/// so an in-tx caller passes `&tx` and the sidecar row commits atomically with
/// the surrounding work (#1574 write-side cutover).
fn upsert_chunk_embedding_conn(
    conn: &rusqlite::Connection,
    chunk_id: &str,
    model_signature: &str,
    embedding: &[f32],
) -> Result<()> {
    let bytes = embedding_to_blob(embedding);
    let dim = i64::try_from(embedding.len()).context("embedding dimension does not fit i64")?;
    let created_at = Utc::now().timestamp_millis() as f64 / 1000.0;
    conn.execute(
        "INSERT INTO mem_tree_chunk_embeddings
             (chunk_id, model_signature, vector, dim, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(chunk_id, model_signature) DO UPDATE SET
                vector = excluded.vector,
                dim = excluded.dim,
                created_at = excluded.created_at",
        rusqlite::params![chunk_id, model_signature, bytes, dim, created_at],
    )?;
    Ok(())
}

/// Store a chunk embedding for a specific provider/model/dimension signature.
///
/// Per-model table write path for #1574. The legacy
/// `mem_tree_chunks.embedding` column is intentionally left untouched by this
/// helper (read by the §7 migration; dropped only in a later release).
pub fn set_chunk_embedding_for_signature(
    config: &Config,
    chunk_id: &str,
    model_signature: &str,
    embedding: &[f32],
) -> Result<()> {
    with_connection(config, |conn| {
        upsert_chunk_embedding_conn(conn, chunk_id, model_signature, embedding)
    })
}

/// `true` when at least one chunk or summary still needs an embedding at
/// `model_signature` and is not tombstoned as terminally unembeddable.
///
/// Shared by `ensure_reembed_backfill`, the §7 migration enqueue probe, and
/// tests so the worklist and coverage probes cannot drift (#2358).
pub(crate) fn has_uncovered_reembed_work(
    conn: &Connection,
    model_signature: &str,
) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM mem_tree_chunks c
              WHERE NOT EXISTS (SELECT 1 FROM mem_tree_chunk_embeddings e
                                 WHERE e.chunk_id = c.id AND e.model_signature = ?1)
                AND NOT EXISTS (SELECT 1 FROM mem_tree_chunk_reembed_skipped sk
                                 WHERE sk.chunk_id = c.id AND sk.model_signature = ?1))
           OR EXISTS(
             SELECT 1 FROM mem_tree_summaries s
              WHERE s.deleted = 0
                AND NOT EXISTS (SELECT 1 FROM mem_tree_summary_embeddings e
                                 WHERE e.summary_id = s.id AND e.model_signature = ?1)
                AND NOT EXISTS (SELECT 1 FROM mem_tree_summary_reembed_skipped sk
                                 WHERE sk.summary_id = s.id AND sk.model_signature = ?1))",
        rusqlite::params![model_signature],
        |r| r.get(0),
    )
}

/// Persistently record that `(chunk_id, signature)` cannot be re-embedded.
///
/// Called by `handle_reembed_backfill` when the per-chunk body file is
/// missing on disk (orphan) or the embedder rejects the row terminally
/// (wrong dim / unrecoverable embed error). Inserting a row here causes
/// the next backfill batch's worklist query to exclude this chunk via the
/// `NOT EXISTS … mem_tree_chunk_reembed_skipped …` predicate, so the
/// runaway "skipping" loop terminates instead of revisiting the same row
/// every 5 s forever (#1574 §6 fix).
pub fn mark_chunk_reembed_skipped(
    config: &Config,
    chunk_id: &str,
    model_signature: &str,
    reason: &str,
) -> Result<()> {
    let chunk_id = validate_reembed_skip_key("chunk_id", chunk_id)?;
    let model_signature = validate_reembed_skip_key("model_signature", model_signature)?;
    with_connection(config, |conn| {
        let now_ms = Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO mem_tree_chunk_reembed_skipped
                 (chunk_id, model_signature, reason, skipped_at_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(chunk_id, model_signature) DO UPDATE SET
                    reason = excluded.reason,
                    skipped_at_ms = excluded.skipped_at_ms",
            rusqlite::params![chunk_id, model_signature, reason, now_ms],
        )?;
        log::debug!(
            "[memory::chunk_store] mark_chunk_reembed_skipped chunk_id={chunk_id} sig={model_signature} reason={reason}"
        );
        Ok(())
    })
}

/// Remove a single chunk tombstone so re-embed backfill can retry the row.
///
/// Idempotent: deleting a missing `(chunk_id, model_signature)` pair is a
/// no-op. Intended for operator recovery after environmental failures (moved
/// workspace, restored body files, fixed embedder config) — see #2358.
pub fn clear_chunk_reembed_skipped(
    config: &Config,
    chunk_id: &str,
    model_signature: &str,
) -> Result<()> {
    let chunk_id = validate_reembed_skip_key("chunk_id", chunk_id)?;
    let model_signature = validate_reembed_skip_key("model_signature", model_signature)?;
    with_connection(config, |conn| {
        conn.execute(
            "DELETE FROM mem_tree_chunk_reembed_skipped
              WHERE chunk_id = ?1 AND model_signature = ?2",
            rusqlite::params![chunk_id, model_signature],
        )?;
        log::debug!(
            "[memory::chunk_store] clear_chunk_reembed_skipped chunk_id={chunk_id} sig={model_signature}"
        );
        Ok(())
    })
}

/// Clear all chunk and summary tombstones for a model signature.
///
/// Returns the total number of rows removed across both tombstone tables.
/// Idempotent when no tombstones exist for the signature.
pub fn clear_reembed_skipped_for_signature(
    config: &Config,
    model_signature: &str,
) -> Result<usize> {
    let model_signature = validate_reembed_skip_key("model_signature", model_signature)?;
    with_connection(config, |conn| {
        let chunk_deleted = conn.execute(
            "DELETE FROM mem_tree_chunk_reembed_skipped WHERE model_signature = ?1",
            rusqlite::params![model_signature],
        )?;
        let summary_deleted = conn.execute(
            "DELETE FROM mem_tree_summary_reembed_skipped WHERE model_signature = ?1",
            rusqlite::params![model_signature],
        )?;
        log::debug!(
            "[memory::chunk_store] clear_reembed_skipped_for_signature sig={model_signature} chunk_rows={chunk_deleted} summary_rows={summary_deleted}"
        );
        Ok(chunk_deleted + summary_deleted)
    })
}

/// Bounds attacker-controlled ids/signatures passed to reembed-skipped admin
/// helpers without affecting legitimate rows (typical ids are well under 512
/// chars). Rejects NUL bytes so SQLite bindings cannot be truncated.
pub(crate) const REEMBED_SKIP_KEY_MAX_LEN: usize = 2048;

pub(crate) fn validate_reembed_skip_key<'a>(label: &str, value: &'a str) -> Result<&'a str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{label} must be non-empty");
    }
    if trimmed.len() > REEMBED_SKIP_KEY_MAX_LEN {
        anyhow::bail!("{label} exceeds maximum length ({REEMBED_SKIP_KEY_MAX_LEN})");
    }
    if trimmed.as_bytes().contains(&0) {
        anyhow::bail!("{label} must not contain NUL bytes");
    }
    Ok(trimmed)
}

/// Transaction-scoped variant of [`set_chunk_embedding_for_signature`].
///
/// For callers that already hold a `Transaction` (e.g. the chunk-admission
/// handler, which commits the sidecar row in the SAME tx as the lifecycle
/// + score + job-enqueue writes — #1574 write-side cutover). Opening a fresh
/// connection there would break atomicity / deadlock on the busy DB.
pub(crate) fn set_chunk_embedding_for_signature_tx(
    tx: &rusqlite::Transaction<'_>,
    chunk_id: &str,
    model_signature: &str,
    embedding: &[f32],
) -> Result<()> {
    upsert_chunk_embedding_conn(tx, chunk_id, model_signature, embedding)
}

/// Fetch a chunk embedding for exactly one provider/model/dimension signature.
pub fn get_chunk_embedding_for_signature(
    config: &Config,
    chunk_id: &str,
    model_signature: &str,
) -> Result<Option<Vec<f32>>> {
    with_connection(config, |conn| {
        let row: Option<(Vec<u8>, i64)> = conn
            .query_row(
                "SELECT vector, dim
                   FROM mem_tree_chunk_embeddings
                  WHERE chunk_id = ?1 AND model_signature = ?2",
                rusqlite::params![chunk_id, model_signature],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            None => Ok(None),
            Some((bytes, dim)) => embedding_from_blob(&bytes, dim, "chunk embedding"),
        }
    })
}

/// Fetch a chunk's embedding for the active model signature.
///
/// #1574 cutover: reads the per-model `mem_tree_chunk_embeddings` sidecar at
/// the active signature (via [`get_chunk_embedding_for_signature`]) instead of
/// the legacy `mem_tree_chunks.embedding` column. Returns `Ok(None)` if the
/// chunk has no vector under the active signature — e.g. during the §7
/// backfill window, where this degrades retrieval gracefully (the row is
/// simply absent from vector results, never cross-space compared).
pub fn get_chunk_embedding(config: &Config, chunk_id: &str) -> Result<Option<Vec<f32>>> {
    let signature = tree_active_signature(config);
    get_chunk_embedding_for_signature(config, chunk_id, &signature)
}

pub(crate) fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn embedding_from_blob(bytes: &[u8], dim: i64, label: &str) -> Result<Option<Vec<f32>>> {
    if dim < 0 {
        anyhow::bail!("{label} has negative dimension {dim}");
    }
    if !bytes.len().is_multiple_of(4) {
        anyhow::bail!("{label} blob length {} not a multiple of 4", bytes.len());
    }
    let floats: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if floats.len() != dim as usize {
        anyhow::bail!(
            "{label} dimension mismatch: dim column says {dim}, blob contains {} floats",
            floats.len()
        );
    }
    Ok(Some(floats))
}
/// SQLite's compile-time hard cap on `?` parameters per prepared statement is
/// `SQLITE_MAX_VARIABLE_NUMBER` (32 766 since SQLite 3.32, bundled with
/// rusqlite). We chunk well below that for two reasons:
///
/// 1. **Headroom against schema growth.** The batched query binds one `?` per
///    chunk_id *plus* the `model_signature` parameter. Picking 500 as the
///    per-chunk cap gives ~65× safety margin even if a future caller bumps
///    `LOOKUP_HEADROOM` from 200 into the thousands — the batch helper
///    silently splits the request, the caller sees no semantic change.
/// 2. **Sane SQL string length.** A 500-`?` `IN(...)` clause is ~1 KB of SQL
///    text — small enough that prepare-and-discard per chunk is cheap and the
///    planner produces a simple covered-index lookup.
///
/// Lowering this constant is safe (just more round-trips). Raising it past
/// the SQLite limit will fail at prepare time with `too many SQL variables`.
const MAX_EMBEDDING_BATCH: usize = 500;

/// Batched read of chunk embeddings under a single `model_signature`.
///
/// Returns a `HashMap<chunk_id, Vec<f32>>` containing **only the chunks
/// that have a vector under `model_signature`**. Missing chunks are simply
/// absent from the map — callers handle them the same way as a `None`
/// return from the single-row [`get_chunk_embedding_for_signature`].
///
/// ## Why this exists
///
/// The retrieval rerank path (`memory_tree::retrieval::topic::
/// rerank_by_semantic_similarity`) used to call the single-row helper inside
/// a `for h in hits { spawn_blocking(...).await }` loop. With
/// `LOOKUP_HEADROOM = 200` that meant 200 sequential SQLite round-trips on
/// every entity-scoped query with `query=…`. This helper collapses that to
/// `ceil(n / MAX_EMBEDDING_BATCH)` round-trips while preserving the exact
/// `Option<Vec<f32>>` semantics of the per-row helper (missing row → absent
/// key → caller treats as `None`).
///
/// Order of input ids is irrelevant; callers re-decorate from the returned
/// map by id, so the rerank loop preserves its original hit iteration
/// order and therefore its tie-break behaviour.
pub fn get_chunk_embeddings_for_signature_batch(
    config: &Config,
    chunk_ids: &[String],
    model_signature: &str,
) -> Result<HashMap<String, Vec<f32>>> {
    if chunk_ids.is_empty() {
        return Ok(HashMap::new());
    }
    with_connection(config, |conn| {
        let mut out: HashMap<String, Vec<f32>> = HashMap::with_capacity(chunk_ids.len());
        // Chunk the id list to stay safely under SQLite's
        // SQLITE_MAX_VARIABLE_NUMBER cap. For the current
        // LOOKUP_HEADROOM=200 callsite this loop executes exactly once;
        // the chunking only kicks in if a future caller passes >500
        // ids in a single batch.
        for window in chunk_ids.chunks(MAX_EMBEDDING_BATCH) {
            // Build `IN (?,?,?,...)` with `window.len()` placeholders.
            // model_signature is bound as the last parameter (?{n+1}).
            let placeholders = std::iter::repeat_n("?", window.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT chunk_id, vector, dim
                   FROM mem_tree_chunk_embeddings
                  WHERE chunk_id IN ({placeholders})
                    AND model_signature = ?{sig_idx}",
                sig_idx = window.len() + 1,
            );
            let mut stmt = conn
                .prepare(&sql)
                .context("prepare get_chunk_embeddings_for_signature_batch")?;
            // Bind chunk_ids then model_signature. rusqlite wants
            // ToSql trait objects in a uniform iterator.
            let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(window.len() + 1);
            for id in window {
                params.push(id as &dyn rusqlite::ToSql);
            }
            params.push(&model_signature as &dyn rusqlite::ToSql);
            let rows = stmt
                .query_map(params.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .context("query get_chunk_embeddings_for_signature_batch")?;
            for row in rows {
                let (chunk_id, bytes, dim) = row?;
                // Reuse the single-row decoder so corrupt-blob errors
                // surface with the same diagnostic shape as the
                // single-row path.
                if let Some(v) = embedding_from_blob(&bytes, dim, "chunk embedding")? {
                    out.insert(chunk_id, v);
                }
            }
        }
        Ok(out)
    })
}

/// Batched read of chunk embeddings under the **active** model signature.
/// Convenience wrapper mirroring [`get_chunk_embedding`] for the per-row
/// path: resolves `tree_active_signature(config)` exactly once and forwards
/// to [`get_chunk_embeddings_for_signature_batch`].
pub fn get_chunk_embeddings_batch(
    config: &Config,
    chunk_ids: &[String],
) -> Result<HashMap<String, Vec<f32>>> {
    let signature = tree_active_signature(config);
    get_chunk_embeddings_for_signature_batch(config, chunk_ids, &signature)
}
