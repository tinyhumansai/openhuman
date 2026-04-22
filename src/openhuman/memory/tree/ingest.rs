//! Ingest orchestrator (Phase 1 + Phase 2):
//!
//!   canonicalise → chunk → score → admission gate → persist (chunks + scores + entity index)
//!
//! Phase 2 inserts scoring between chunker and persistence. Low-scoring
//! chunks are dropped (their rationale is still persisted to
//! `mem_tree_score` for diagnostics); surviving chunks get their entities
//! indexed so later phases can resolve "which chunks mention Alice?" in
//! O(lookup).

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
use crate::openhuman::memory::tree::score::{self, ScoreResult, ScoringConfig};
use crate::openhuman::memory::tree::source_tree::{
    append_leaf, get_or_create_source_tree, InertSummariser, LeafRef,
};
use crate::openhuman::memory::tree::store;
use crate::openhuman::memory::tree::types::Chunk;

/// Outcome of one ingest call — extended with per-chunk admission info.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngestResult {
    pub source_id: String,
    /// Number of chunks that passed the admission gate and were persisted.
    pub chunks_written: usize,
    /// Number of chunks that failed the admission gate and were NOT persisted
    /// (their score rationale IS persisted for diagnostics).
    pub chunks_dropped: usize,
    /// IDs of all chunks that were persisted (in source order).
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

/// Ingest a batch of chat messages scoped to one channel/group.
pub async fn ingest_chat(
    config: &Config,
    source_id: &str,
    owner: &str,
    tags: Vec<String>,
    batch: ChatBatch,
) -> Result<IngestResult> {
    log::debug!(
        "[memory_tree::ingest] chat source_id={} msg_count={}",
        source_id,
        batch.messages.len()
    );
    let canonical =
        match chat::canonicalise(source_id, owner, &tags, batch).map_err(anyhow::Error::msg)? {
            Some(c) => c,
            None => return Ok(IngestResult::empty(source_id)),
        };
    persist(config, source_id, canonical).await
}

/// Ingest a single email thread.
pub async fn ingest_email(
    config: &Config,
    source_id: &str,
    owner: &str,
    tags: Vec<String>,
    thread: EmailThread,
) -> Result<IngestResult> {
    log::debug!(
        "[memory_tree::ingest] email source_id={} msg_count={}",
        source_id,
        thread.messages.len()
    );
    let canonical =
        match email::canonicalise(source_id, owner, &tags, thread).map_err(anyhow::Error::msg)? {
            Some(c) => c,
            None => return Ok(IngestResult::empty(source_id)),
        };
    persist(config, source_id, canonical).await
}

/// Ingest a single standalone document.
pub async fn ingest_document(
    config: &Config,
    source_id: &str,
    owner: &str,
    tags: Vec<String>,
    doc: DocumentInput,
) -> Result<IngestResult> {
    let title_len = doc.title.chars().count();
    log::debug!(
        "[memory_tree::ingest] document source_id={} has_title={} title_len={}",
        source_id,
        !doc.title.trim().is_empty(),
        title_len
    );
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
    // 1. Chunk
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

    // 2. Score (async; uses configured extractor)
    let scoring_cfg = ScoringConfig::default_regex_only();
    let scores = score::score_chunks(&chunks, &scoring_cfg).await?;

    // Fail fast on scorer length mismatch — silently truncating via zip would
    // drop chunks (or their score rationale) without trace.
    if scores.len() != chunks.len() {
        anyhow::bail!(
            "[memory_tree::ingest] scorer length mismatch: chunks={} scores={}",
            chunks.len(),
            scores.len()
        );
    }

    // 3. Partition kept vs dropped
    let mut kept_chunks: Vec<Chunk> = Vec::new();
    let mut all_results: Vec<(ScoreResult, i64)> = Vec::new();
    for (chunk, result) in chunks.iter().zip(scores.into_iter()) {
        let ts_ms = chunk.metadata.timestamp.timestamp_millis();
        if result.kept {
            kept_chunks.push(chunk.clone());
        }
        all_results.push((result, ts_ms));
    }

    let dropped = all_results.iter().filter(|(r, _)| !r.kept).count();
    log::debug!(
        "[memory_tree::ingest] scoring source_id={} kept={} dropped={}",
        source_id,
        kept_chunks.len(),
        dropped
    );

    // 4. Persist (blocking SQLite — isolate on a dedicated thread)
    let config_owned = config.clone();
    let kept_for_store = kept_chunks.clone();
    let results_for_store = all_results.clone();
    let written = tokio::task::spawn_blocking(move || -> Result<usize> {
        store::with_connection(&config_owned, |conn| {
            let tx = conn.unchecked_transaction()?;
            let n = store::upsert_chunks_tx(&tx, &kept_for_store)?;
            for (result, ts_ms) in &results_for_store {
                // Persist rationale for EVERY chunk (kept or dropped).
                // Index entities only for kept chunks (handled inside persist_score_tx).
                score::persist_score_tx(&tx, result, *ts_ms, None)?;
            }
            tx.commit()?;
            Ok(n)
        })
    })
    .await
    .map_err(|e| anyhow::anyhow!("persist join error: {e}"))??;

    // 5. Source-tree append (Phase 3a #709). Each kept leaf pushes into
    //    the tree's L0 buffer and cascades upward when token_sum crosses
    //    the budget. Entities/topics from the scorer are threaded in so
    //    sealed summaries inherit the child signal set. Failures here
    //    log at warn level but don't fail the ingest — leaves are already
    //    persisted, and a later flush/retry can still rebuild the tree.
    if let Err(e) = append_leaves_to_tree(config, source_id, &kept_chunks, &all_results).await {
        log::warn!(
            "[memory_tree::ingest] source_tree append failed source_id={} err={:#}",
            source_id,
            e
        );
    }

    Ok(IngestResult {
        source_id: source_id.to_string(),
        chunks_written: written,
        chunks_dropped: dropped,
        chunk_ids: kept_chunks.iter().map(|c| c.id.clone()).collect(),
    })
}

/// Push every kept chunk into its source tree. Scoped to Phase 3a — all
/// chunks from one ingest batch share the same `source_id`, so they share
/// one tree lookup. The inert summariser is the default; future wiring
/// can swap in an LLM summariser here.
async fn append_leaves_to_tree(
    config: &Config,
    source_id: &str,
    kept_chunks: &[Chunk],
    all_results: &[(ScoreResult, i64)],
) -> Result<()> {
    if kept_chunks.is_empty() {
        return Ok(());
    }
    let tree = get_or_create_source_tree(config, source_id)?;
    let summariser = InertSummariser::new();

    // Build a chunk_id → (score, entities, topics) map for quick lookup.
    use std::collections::HashMap;
    let mut score_by_id: HashMap<String, &ScoreResult> = HashMap::new();
    for (r, _) in all_results {
        score_by_id.insert(r.chunk_id.clone(), r);
    }

    for chunk in kept_chunks {
        let (score_value, entities, topics) = match score_by_id.get(&chunk.id) {
            Some(r) => (
                r.total,
                r.canonical_entities
                    .iter()
                    .map(|e| e.canonical_id.clone())
                    .collect(),
                chunk.metadata.tags.clone(),
            ),
            None => (0.0, Vec::new(), chunk.metadata.tags.clone()),
        };
        let leaf = LeafRef {
            chunk_id: chunk.id.clone(),
            token_count: chunk.token_count,
            timestamp: chunk.metadata.timestamp,
            content: chunk.content.clone(),
            entities,
            topics,
            score: score_value,
        };
        append_leaf(config, &tree, &leaf, &summariser).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::canonicalize::chat::ChatMessage;
    use crate::openhuman::memory::tree::score::store::{count_scores, lookup_entity};
    use crate::openhuman::memory::tree::store::{count_chunks, list_chunks, ListChunksQuery};
    use crate::openhuman::memory::tree::types::SourceKind;
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    /// Build a substantive batch that reliably passes the admission gate.
    fn substantive_batch() -> ChatBatch {
        ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![
                ChatMessage {
                    author: "alice".into(),
                    timestamp: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
                    text: "We are planning to ship the Phoenix migration on Friday \
                           after reviewing the runbook and staging results. Please \
                           confirm availability by replying here. alice@example.com"
                        .into(),
                    source_ref: Some("slack://m1".into()),
                },
                ChatMessage {
                    author: "bob".into(),
                    timestamp: Utc.timestamp_millis_opt(1_700_000_010_000).unwrap(),
                    text: "Confirmed — I'll handle the coordination and cut a release \
                           candidate tonight. #launch-q2 will be tracked in Notion."
                        .into(),
                    source_ref: None,
                },
            ],
        }
    }

    #[tokio::test]
    async fn ingest_chat_writes_substantive_chunks() {
        let (_tmp, cfg) = test_config();
        let out = ingest_chat(&cfg, "slack:#eng", "alice", vec![], substantive_batch())
            .await
            .unwrap();
        assert_eq!(out.chunks_written, 1);
        assert_eq!(out.chunks_dropped, 0);
        assert_eq!(count_chunks(&cfg).unwrap(), 1);
        // Score row persisted for the kept chunk
        assert_eq!(count_scores(&cfg).unwrap(), 1);
        // Entity index populated from regex extraction (alice@example.com + hashtag)
        let alice_hits = lookup_entity(&cfg, "email:alice@example.com", None).unwrap();
        assert_eq!(alice_hits.len(), 1);
        let rows = list_chunks(&cfg, &ListChunksQuery::default()).unwrap();
        assert_eq!(rows[0].metadata.source_kind, SourceKind::Chat);
    }

    #[tokio::test]
    async fn low_signal_chunks_are_dropped_but_score_persists() {
        let (_tmp, cfg) = test_config();
        let batch = ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![ChatMessage {
                author: "alice".into(),
                timestamp: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
                text: "+1".into(), // extremely low-signal
                source_ref: None,
            }],
        };
        let out = ingest_chat(&cfg, "slack:#eng", "alice", vec![], batch)
            .await
            .unwrap();
        assert_eq!(out.chunks_written, 0);
        assert_eq!(out.chunks_dropped, 1);
        // Chunk NOT in chunks table
        assert_eq!(count_chunks(&cfg).unwrap(), 0);
        // Score row IS persisted for diagnostics
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
        assert_eq!(out.chunks_dropped, 0);
        assert_eq!(count_chunks(&cfg).unwrap(), 0);
        assert_eq!(count_scores(&cfg).unwrap(), 0);
    }

    #[tokio::test]
    async fn re_ingest_is_idempotent_on_chunks_and_scores() {
        let (_tmp, cfg) = test_config();
        let doc = DocumentInput {
            provider: "notion".into(),
            title: "Launch plan".into(),
            body: "We are planning to ship Phoenix on Friday after review. \
                   Coordination is via email and the launch thread tracks \
                   the relevant decisions. alice@example.com owns this."
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
        assert_eq!(count_chunks(&cfg).unwrap(), 1);
        assert_eq!(count_scores(&cfg).unwrap(), 1);
    }

    #[tokio::test]
    async fn chunks_preserve_source_ref_when_kept() {
        let (_tmp, cfg) = test_config();
        let doc = DocumentInput {
            provider: "notion".into(),
            title: "t".into(),
            body: "Phoenix launch plan with enough substance to pass the admission \
                   gate: we are reviewing the migration runbook alice@example.com \
                   on Friday evening."
                .into(),
            modified_at: Utc::now(),
            source_ref: Some("notion://x".into()),
        };
        ingest_document(&cfg, "notion:x", "alice", vec![], doc)
            .await
            .unwrap();
        let rows = list_chunks(&cfg, &ListChunksQuery::default()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].metadata.source_ref.as_ref().unwrap().value,
            "notion://x"
        );
    }
}
