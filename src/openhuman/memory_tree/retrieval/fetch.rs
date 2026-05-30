//! `memory_tree_fetch_leaves` — batch-fetch raw chunks by id (Phase 4 /
//! #710).
//!
//! The LLM-facing contract: "given these chunk ids, give me the full
//! content + metadata so I can cite." We cap the batch at 20 to keep the
//! round-trip bounded. Missing ids are silently skipped — the return is
//! best-effort so partial failures are visible via `hits.len() < ids.len()`.
//!
//! Each hit is annotated with the chunk's score from `mem_tree_score` when
//! available; score is 0.0 when the chunk has no row in `mem_tree_score`
//! (e.g. pre-Phase 2 backfill).

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::chunks::store::get_chunks_batch;
use crate::openhuman::memory_store::content::read as content_read;
use crate::openhuman::memory_tree::retrieval::types::{hit_from_chunk, RetrievalHit};
use crate::openhuman::memory_tree::score::store::get_scores_batch;

/// Max batch size. Callers that pass more than this get truncated with a
/// warn log — no error surface so the LLM sees a partial result.
pub const MAX_BATCH: usize = 20;

/// Fetch chunk rows by id in the provided order. Missing ids are dropped
/// from the response.
pub async fn fetch_leaves(config: &Config, chunk_ids: &[String]) -> Result<Vec<RetrievalHit>> {
    if chunk_ids.is_empty() {
        log::debug!("[retrieval::fetch] empty request — returning empty vec");
        return Ok(Vec::new());
    }

    let ids: Vec<String> = if chunk_ids.len() > MAX_BATCH {
        log::warn!(
            "[retrieval::fetch] batch size {} exceeds cap {} — truncating",
            chunk_ids.len(),
            MAX_BATCH
        );
        chunk_ids[..MAX_BATCH].to_vec()
    } else {
        chunk_ids.to_vec()
    };

    // Count only — individual chunk ids can include source scope (e.g.
    // `chat:slack:#<channel>:0`) and are redacted from logs.
    log::debug!("[retrieval::fetch] fetch_leaves n={}", ids.len());

    let config_owned = config.clone();
    let hits = tokio::task::spawn_blocking(move || -> Result<Vec<RetrievalHit>> {
        // Two batched SQLite reads up front instead of 2N per-id queries
        // inside the loop. With the `MAX_BATCH = 20` cap above, this turns
        // 40 round-trips into 2. Per-row decoders are reused inside both
        // helpers so the returned `Chunk` and `score.total` values are
        // byte-identical to the old per-id path. Missing ids are absent
        // from the maps (same contract as `get_chunk` / `get_score`
        // returning `Ok(None)`).
        let chunk_by_id = get_chunks_batch(&config_owned, &ids)?;
        let score_by_id = get_scores_batch(&config_owned, &ids)?;

        // Walk the input ids in order so the response preserves caller
        // ordering. Missing ids are dropped exactly as before — callers
        // detect partial results via `hits.len() < ids.len()`. File I/O
        // (`read_chunk_body`) stays per-id: each MD body lives in its
        // own on-disk file, so batching there would mean concurrent file
        // opens, not a single round-trip — left untouched.
        let mut out: Vec<RetrievalHit> = Vec::with_capacity(ids.len());
        for (idx, id) in ids.iter().enumerate() {
            let Some(chunk) = chunk_by_id.get(id) else {
                log::debug!(
                    "[retrieval::fetch] chunk not found at index {}/{} — skipping",
                    idx + 1,
                    ids.len()
                );
                continue;
            };
            let score = score_by_id.get(id).copied().unwrap_or(0.0);
            // Leaves are not attached to a materialised tree id via the
            // chunk row. `scope` falls back to the chunk's own source_id so
            // consumers still see provenance (e.g. "slack:#eng").
            let scope = chunk.metadata.source_id.clone();
            // Hydrate the full body from disk before building the hit.
            // The `content` column in SQLite holds a ≤500-char preview after
            // the MD-on-disk migration; the retrieval API must return the
            // complete chunk text so the LLM sees untruncated content.
            let mut chunk_with_body = chunk.clone();
            match content_read::read_chunk_body(&config_owned, id) {
                Ok(body) => chunk_with_body.content = body,
                Err(e) => {
                    log::warn!(
                        "[retrieval::fetch] read_chunk_body failed for chunk — serving preview: {e:#}"
                    );
                    // Non-fatal: fall back to the preview already in the struct.
                    // This handles pre-MD-migration rows gracefully.
                }
            }
            out.push(hit_from_chunk(&chunk_with_body, "", &scope, score));
        }
        Ok(out)
    })
    .await
    .map_err(|e| anyhow::anyhow!("fetch_leaves join error: {e}"))??;

    log::debug!("[retrieval::fetch] returning hits={}", hits.len());
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_store::chunks::store::upsert_chunks;
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    use crate::openhuman::memory_store::content as content_store;
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn stage_test_chunks(cfg: &Config, chunks: &[Chunk]) {
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).expect("create content_root for test");
        let staged = content_store::stage_chunks(&content_root, chunks)
            .expect("stage_chunks for test chunks");
        crate::openhuman::memory_store::chunks::store::with_connection(cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            crate::openhuman::memory_store::chunks::store::upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .expect("persist staged chunk pointers");
    }

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Phase 4 (#710): inert embedder for tests.
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    fn sample_chunk(source: &str, seq: u32) -> Chunk {
        let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        Chunk {
            id: chunk_id(SourceKind::Chat, source, seq, "test-content"),
            content: format!("content-{source}-{seq}"),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: source.into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new(format!("slack://{source}/{seq}"))),
            },
            token_count: 20,
            seq_in_source: seq,
            created_at: ts,
            partial_message: false,
        }
    }

    #[tokio::test]
    async fn empty_input_returns_empty() {
        let (_tmp, cfg) = test_config();
        let out = fetch_leaves(&cfg, &[]).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn returns_existing_chunks_in_order() {
        let (_tmp, cfg) = test_config();
        let c1 = sample_chunk("slack:#eng", 0);
        let c2 = sample_chunk("slack:#eng", 1);
        upsert_chunks(&cfg, &[c1.clone(), c2.clone()]).unwrap();
        stage_test_chunks(&cfg, &[c1.clone(), c2.clone()]);
        let out = fetch_leaves(&cfg, &[c1.id.clone(), c2.id.clone()])
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].node_id, c1.id);
        assert_eq!(out[1].node_id, c2.id);
    }

    #[tokio::test]
    async fn missing_ids_are_skipped() {
        let (_tmp, cfg) = test_config();
        let c1 = sample_chunk("slack:#eng", 0);
        upsert_chunks(&cfg, &[c1.clone()]).unwrap();
        stage_test_chunks(&cfg, &[c1.clone()]);
        let out = fetch_leaves(
            &cfg,
            &[c1.id.clone(), "ghost:nonexistent".into(), c1.id.clone()],
        )
        .await
        .unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|h| h.node_id == c1.id));
    }

    #[tokio::test]
    async fn over_cap_is_truncated() {
        let (_tmp, cfg) = test_config();
        let mut ids: Vec<String> = Vec::new();
        for i in 0..(MAX_BATCH + 5) as u32 {
            let c = sample_chunk("slack:#eng", i);
            upsert_chunks(&cfg, &[c.clone()]).unwrap();
            stage_test_chunks(&cfg, &[c.clone()]);
            ids.push(c.id);
        }
        let out = fetch_leaves(&cfg, &ids).await.unwrap();
        assert_eq!(out.len(), MAX_BATCH);
    }

    #[tokio::test]
    async fn leaf_hit_carries_source_ref_and_scope() {
        let (_tmp, cfg) = test_config();
        let c = sample_chunk("slack:#eng", 0);
        upsert_chunks(&cfg, &[c.clone()]).unwrap();
        stage_test_chunks(&cfg, &[c.clone()]);
        let out = fetch_leaves(&cfg, &[c.id.clone()]).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].source_ref.as_deref(), Some("slack://slack:#eng/0"));
        assert_eq!(out[0].tree_scope, "slack:#eng");
    }

    /// After the batch refactor, ordering and score propagation rely on
    /// walking the input slice and looking each id up in two HashMaps
    /// (chunk + score). This test pins both invariants: interleaved
    /// present/missing ids keep their input order, and each kept hit
    /// carries the score from its own `mem_tree_score` row (not the
    /// row of a neighbour, not the 0.0 fallback when a row exists).
    #[tokio::test]
    async fn fetch_leaves_preserves_input_order_and_propagates_scores() {
        use crate::openhuman::memory_tree::score::signals::ScoreSignals;
        use crate::openhuman::memory_tree::score::store::{upsert_score, ScoreRow};

        let (_tmp, cfg) = test_config();
        let c1 = sample_chunk("slack:#eng", 0);
        let c2 = sample_chunk("slack:#eng", 1);
        let c3 = sample_chunk("slack:#eng", 2);
        upsert_chunks(&cfg, &[c1.clone(), c2.clone(), c3.clone()]).unwrap();
        stage_test_chunks(&cfg, &[c1.clone(), c2.clone(), c3.clone()]);

        // Distinct totals so we can tell which chunk a hit's score came
        // from — proves the per-id HashMap lookup keys by chunk_id and
        // not by iteration index.
        let mk_row = |id: &str, total: f32| ScoreRow {
            chunk_id: id.to_string(),
            total,
            signals: ScoreSignals {
                token_count: 0.0,
                unique_words: 0.0,
                metadata_weight: 0.0,
                source_weight: 0.0,
                interaction: 0.0,
                entity_density: 0.0,
                llm_importance: 0.0,
            },
            llm_importance_reason: None,
            dropped: false,
            reason: None,
            computed_at_ms: 0,
        };
        upsert_score(&cfg, &mk_row(&c1.id, 0.1)).unwrap();
        upsert_score(&cfg, &mk_row(&c2.id, 0.2)).unwrap();
        // c3 intentionally has NO score row so we also pin the 0.0
        // fallback after the get_scores_batch contract.

        // Request order: c2, ghost, c3, c1 — none in natural id order.
        let out = fetch_leaves(
            &cfg,
            &[
                c2.id.clone(),
                "ghost:no-such".into(),
                c3.id.clone(),
                c1.id.clone(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(out.len(), 3, "ghost dropped, 3 real chunks returned");
        assert_eq!(out[0].node_id, c2.id);
        assert_eq!(out[1].node_id, c3.id);
        assert_eq!(out[2].node_id, c1.id);
        assert!((out[0].score - 0.2).abs() < 1e-6, "c2 score");
        assert!(
            out[1].score.abs() < 1e-6,
            "c3 has no score row → 0.0 fallback"
        );
        assert!((out[2].score - 0.1).abs() < 1e-6, "c1 score");
    }
}
