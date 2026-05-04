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
use crate::openhuman::memory::tree::content_store::read as content_read;
use crate::openhuman::memory::tree::retrieval::types::{hit_from_chunk, RetrievalHit};
use crate::openhuman::memory::tree::score::store::get_score;
use crate::openhuman::memory::tree::store::get_chunk;

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
        let mut out: Vec<RetrievalHit> = Vec::with_capacity(ids.len());
        for id in &ids {
            let chunk = match get_chunk(&config_owned, id)? {
                Some(c) => c,
                None => {
                    log::debug!(
                        "[retrieval::fetch] chunk not found — skipping (1 of {} requested)",
                        ids.len()
                    );
                    continue;
                }
            };
            let score = match get_score(&config_owned, id)? {
                Some(s) => s.total,
                None => 0.0,
            };
            // Leaves are not attached to a materialised tree id via the
            // chunk row. `scope` falls back to the chunk's own source_id so
            // consumers still see provenance (e.g. "slack:#eng").
            let scope = chunk.metadata.source_id.clone();
            // Hydrate the full body from disk before building the hit.
            // The `content` column in SQLite holds a ≤500-char preview after
            // the MD-on-disk migration; the retrieval API must return the
            // complete chunk text so the LLM sees untruncated content.
            let mut chunk_with_body = chunk;
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
    use crate::openhuman::memory::tree::content_store;
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn stage_test_chunks(cfg: &Config, chunks: &[Chunk]) {
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).expect("create content_root for test");
        let staged = content_store::stage_chunks(&content_root, chunks)
            .expect("stage_chunks for test chunks");
        crate::openhuman::memory::tree::store::with_connection(cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            crate::openhuman::memory::tree::store::upsert_staged_chunks_tx(&tx, &staged)?;
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
}
