//! Ingest orchestrator for the async memory-tree pipeline.
//!
//! The hot path now does:
//! `canonicalise -> chunk -> fast score -> persist chunks/score rows -> enqueue extract jobs`
//!
//! The slower work (full extraction, admission, tree buffering, sealing,
//! topic routing, daily digests) runs out of the SQLite-backed jobs queue.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::canonicalize::{
    chat::{self, ChatBatch},
    document::{self, DocumentInput},
    email::{self, EmailThread},
    CanonicalisedSource,
};
use crate::openhuman::memory::tree::chunker::{chunk_markdown, ChunkerInput, ChunkerOptions};
use crate::openhuman::memory::tree::content_store;
use crate::openhuman::memory::tree::jobs::{self, ExtractChunkPayload, NewJob};
use crate::openhuman::memory::tree::score::{self, ScoreResult, ScoringConfig};
use crate::openhuman::memory::tree::store;

/// Outcome of one ingest call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngestResult {
    pub source_id: String,
    /// Number of chunks persisted and queued for async extraction.
    pub chunks_written: usize,
    /// Number of chunks the cheap fast-score path would drop. Final admission
    /// still happens later in the extract job.
    pub chunks_dropped: usize,
    /// IDs of all chunks written and queued.
    pub chunk_ids: Vec<String>,
}

impl IngestResult {
    fn empty(source_id: &str) -> Self {
        Self {
            source_id: source_id.to_string(),
            chunks_written: 0,
            chunks_dropped: 0,
            chunk_ids: Vec::new(),
        }
    }
}

pub async fn ingest_chat(
    config: &Config,
    source_id: &str,
    owner: &str,
    tags: Vec<String>,
    batch: ChatBatch,
) -> Result<IngestResult> {
    let canonical =
        match chat::canonicalise(source_id, owner, &tags, batch).map_err(anyhow::Error::msg)? {
            Some(c) => c,
            None => return Ok(IngestResult::empty(source_id)),
        };
    persist(config, source_id, canonical).await
}

pub async fn ingest_email(
    config: &Config,
    source_id: &str,
    owner: &str,
    tags: Vec<String>,
    thread: EmailThread,
) -> Result<IngestResult> {
    let canonical =
        match email::canonicalise(source_id, owner, &tags, thread).map_err(anyhow::Error::msg)? {
            Some(c) => c,
            None => return Ok(IngestResult::empty(source_id)),
        };
    persist(config, source_id, canonical).await
}

pub async fn ingest_document(
    config: &Config,
    source_id: &str,
    owner: &str,
    tags: Vec<String>,
    doc: DocumentInput,
) -> Result<IngestResult> {
    let canonical =
        match document::canonicalise(source_id, owner, &tags, doc).map_err(anyhow::Error::msg)? {
            Some(c) => c,
            None => return Ok(IngestResult::empty(source_id)),
        };
    persist(config, source_id, canonical).await
}

async fn persist(
    config: &Config,
    source_id: &str,
    canonical: CanonicalisedSource,
) -> Result<IngestResult> {
    let input = ChunkerInput {
        source_kind: canonical.metadata.source_kind,
        source_id: source_id.to_string(),
        markdown: canonical.markdown,
        metadata: canonical.metadata,
    };
    let chunks = chunk_markdown(&input, &ChunkerOptions::default());
    if chunks.is_empty() {
        return Ok(IngestResult::empty(source_id));
    }

    // Phase MD-content: write chunk bodies to disk before the SQLite upsert.
    // stage_chunks is sync I/O; run it here (still on the tokio thread) before
    // spawn_blocking so errors surface before the DB transaction opens.
    let content_root = config.memory_tree_content_root();
    let staged = content_store::stage_chunks(&content_root, &chunks)
        .map_err(|e| anyhow::anyhow!("[memory_tree::ingest] stage_chunks failed: {e}"))?;

    let scoring_cfg = ScoringConfig::from_config(config);
    let scores = score::score_chunks_fast(&chunks, &scoring_cfg).await?;
    if scores.len() != chunks.len() {
        anyhow::bail!(
            "[memory_tree::ingest] scorer length mismatch: chunks={} scores={}",
            chunks.len(),
            scores.len()
        );
    }

    let all_results: Vec<(ScoreResult, i64)> = chunks
        .iter()
        .zip(scores.into_iter())
        .map(|(chunk, result)| (result, chunk.metadata.timestamp.timestamp_millis()))
        .collect();
    let dropped = all_results.iter().filter(|(r, _)| !r.kept).count();

    let config_owned = config.clone();
    let staged_for_store = staged.clone();
    let results_for_store = all_results.clone();
    let written = tokio::task::spawn_blocking(move || -> Result<usize> {
        use std::collections::{HashMap, HashSet};
        store::with_connection(&config_owned, |conn| {
            let tx = conn.unchecked_transaction()?;

            // Read each chunk's CURRENT lifecycle BEFORE the upsert. This
            // is the "did this chunk exist before this batch" snapshot,
            // because `upsert_staged_chunks_tx` will either preserve the
            // existing row's lifecycle (UPDATE doesn't touch the column) or
            // insert a new row that picks up the column DEFAULT — so reading
            // post-upsert can't distinguish "brand new" from
            // "already-admitted-from-prior-ingest".
            let mut prior: HashMap<String, Option<String>> = HashMap::new();
            for s in &staged_for_store {
                let status = store::get_chunk_lifecycle_status_tx(&tx, &s.chunk.id)?;
                prior.insert(s.chunk.id.clone(), status);
            }

            let n = store::upsert_staged_chunks_tx(&tx, &staged_for_store)?;

            // Re-ingest of identical content (same chunk_id) must NOT
            // downgrade chunks that have already progressed through the
            // async pipeline. Without this guard, a re-ingest would reset
            // every chunk to 'pending_extraction' and enqueue a fresh
            // `extract_chunk` job — sending already-buffered/sealed
            // chunks back through extract → admit → append, ultimately
            // duplicating them into a second summary in the same tree.
            //
            // Schedule a chunk for processing when its PRE-upsert state
            // was either absent (genuinely new) or already
            // `pending_extraction` (a prior ingest crashed before extract
            // ran). Anything else — `admitted`, `buffered`, `sealed`,
            // `dropped` — is past the point of accepting new work, so
            // leave the lifecycle alone and skip the extract enqueue.
            let mut to_schedule: HashSet<String> = HashSet::new();
            for s in &staged_for_store {
                let pre = prior.get(&s.chunk.id).cloned().flatten();
                let needs_processing = matches!(
                    pre.as_deref(),
                    None | Some(store::CHUNK_STATUS_PENDING_EXTRACTION),
                );
                if needs_processing {
                    store::set_chunk_lifecycle_status_tx(
                        &tx,
                        &s.chunk.id,
                        store::CHUNK_STATUS_PENDING_EXTRACTION,
                    )?;
                    to_schedule.insert(s.chunk.id.clone());
                }
            }

            for (result, ts_ms) in &results_for_store {
                if !to_schedule.contains(&result.chunk_id) {
                    // Chunk has already progressed past pending_extraction
                    // on a prior ingest — skip score re-persist and don't
                    // enqueue a duplicate extract job.
                    continue;
                }
                score::persist_score_tx(&tx, result, *ts_ms, None)?;
                let extract = NewJob::extract_chunk(&ExtractChunkPayload {
                    chunk_id: result.chunk_id.clone(),
                })?;
                let _ = jobs::enqueue_tx(&tx, &extract)?;
            }
            tx.commit()?;
            Ok(n)
        })
    })
    .await
    .map_err(|e| anyhow::anyhow!("persist join error: {e}"))??;

    jobs::wake_workers();

    Ok(IngestResult {
        source_id: source_id.to_string(),
        chunks_written: written,
        chunks_dropped: dropped,
        chunk_ids: staged.iter().map(|s| s.chunk.id.clone()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::canonicalize::chat::ChatMessage;
    use crate::openhuman::memory::tree::jobs::drain_until_idle;
    use crate::openhuman::memory::tree::score::store::{count_scores, lookup_entity};
    use crate::openhuman::memory::tree::store::{
        count_chunks, count_chunks_by_lifecycle_status, get_chunk_embedding, list_chunks,
        ListChunksQuery, CHUNK_STATUS_BUFFERED, CHUNK_STATUS_DROPPED,
    };
    use crate::openhuman::memory::tree::types::SourceKind;
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    fn substantive_batch() -> ChatBatch {
        ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![
                ChatMessage {
                    author: "alice".into(),
                    timestamp: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
                    text: "We are planning to ship the Phoenix migration on Friday after reviewing the runbook and staging results. alice@example.com"
                        .into(),
                    source_ref: Some("slack://m1".into()),
                },
                ChatMessage {
                    author: "bob".into(),
                    timestamp: Utc.timestamp_millis_opt(1_700_000_010_000).unwrap(),
                    text: "Confirmed, I will handle the coordination and launch tracking tonight."
                        .into(),
                    source_ref: None,
                },
            ],
        }
    }

    #[tokio::test]
    async fn ingest_chat_writes_and_queue_drains_to_admitted_chunk() {
        let (_tmp, cfg) = test_config();
        let out = ingest_chat(&cfg, "slack:#eng", "alice", vec![], substantive_batch())
            .await
            .unwrap();
        // Greedy packing: both small messages fit under 10k token budget
        // and are packed into a single chunk.
        assert_eq!(out.chunks_written, 1);
        assert_eq!(count_chunks(&cfg).unwrap(), 1);

        drain_until_idle(&cfg).await.unwrap();

        // Final lifecycle is `buffered`: extract → admitted → append_buffer → buffered.
        // The single packed chunk does not cross TOKEN_BUDGET so no seal fires.
        assert_eq!(
            count_chunks_by_lifecycle_status(&cfg, CHUNK_STATUS_BUFFERED).unwrap(),
            1
        );
        assert!(count_scores(&cfg).unwrap() >= 1);
        assert_eq!(
            lookup_entity(&cfg, "email:alice@example.com", None)
                .unwrap()
                .len(),
            1
        );
        let rows = list_chunks(&cfg, &ListChunksQuery::default()).unwrap();
        assert_eq!(rows[0].metadata.source_kind, SourceKind::Chat);
        assert!(get_chunk_embedding(&cfg, &out.chunk_ids[0])
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn low_signal_chunks_end_up_dropped_after_queue_processing() {
        let (_tmp, cfg) = test_config();
        let batch = ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![ChatMessage {
                author: "alice".into(),
                timestamp: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
                text: "+1".into(),
                source_ref: None,
            }],
        };
        let out = ingest_chat(&cfg, "slack:#eng", "alice", vec![], batch)
            .await
            .unwrap();
        assert_eq!(out.chunks_written, 1);
        assert_eq!(count_chunks(&cfg).unwrap(), 1);

        drain_until_idle(&cfg).await.unwrap();

        assert_eq!(
            count_chunks_by_lifecycle_status(&cfg, CHUNK_STATUS_DROPPED).unwrap(),
            1
        );
        assert_eq!(count_scores(&cfg).unwrap(), 1);
    }

    #[tokio::test]
    async fn ingest_chat_empty_batch_is_noop() {
        let (_tmp, cfg) = test_config();
        let batch = ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![],
        };
        let out = ingest_chat(&cfg, "slack:#eng", "alice", vec![], batch)
            .await
            .unwrap();
        assert_eq!(out.chunks_written, 0);
        assert_eq!(count_chunks(&cfg).unwrap(), 0);
        assert_eq!(count_scores(&cfg).unwrap(), 0);
    }

    #[tokio::test]
    async fn re_ingest_is_idempotent_on_chunks_and_scores() {
        let (_tmp, cfg) = test_config();
        let doc = DocumentInput {
            provider: "notion".into(),
            title: "Launch plan".into(),
            body: "We are planning to ship Phoenix on Friday after review. alice@example.com owns this."
                .into(),
            modified_at: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
            source_ref: Some("notion://page/abc".into()),
        };
        ingest_document(&cfg, "notion:abc", "alice", vec![], doc.clone())
            .await
            .unwrap();
        ingest_document(&cfg, "notion:abc", "alice", vec![], doc)
            .await
            .unwrap();
        drain_until_idle(&cfg).await.unwrap();
        assert_eq!(count_chunks(&cfg).unwrap(), 1);
        assert_eq!(count_scores(&cfg).unwrap(), 1);
    }
}
