//! Phase 2: scoring / admission / enrichment pipeline (#708).
//!
//! Wraps extraction, signal computation, admission gate, canonicalisation,
//! and persistence into one call per chunk. Phase 1 `_ingest_one_chunk`
//! passes each chunk through [`score_chunk`] after chunking and before
//! storing.

pub mod extract;
pub mod resolver;
pub mod signals;
pub mod store;

use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use self::extract::{EntityExtractor, ExtractedEntities};
use self::resolver::{canonicalise, CanonicalEntity};
use self::signals::{ScoreSignals, SignalWeights};
use crate::openhuman::memory::tree::types::Chunk;

/// Default drop threshold. Chunks with `total < DEFAULT_DROP_THRESHOLD`
/// are tombstoned and never reach the chunk store.
pub const DEFAULT_DROP_THRESHOLD: f32 = 0.3;

/// Whole outcome of [`score_chunk`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoreResult {
    pub chunk_id: String,
    pub total: f32,
    pub signals: ScoreSignals,
    pub kept: bool,
    pub drop_reason: Option<String>,
    pub extracted: ExtractedEntities,
    pub canonical_entities: Vec<CanonicalEntity>,
}

/// Configuration passed through the ingest pipeline for Phase 2 behaviour.
///
/// Held as a struct (vs config struct fields) so callers can override per-run
/// without mutating global config — useful for tests and explicit threshold
/// tuning.
pub struct ScoringConfig {
    pub extractor: Arc<dyn EntityExtractor>,
    pub weights: SignalWeights,
    pub drop_threshold: f32,
}

impl ScoringConfig {
    /// Phase 2 default: regex-only extractor, default weights, default threshold.
    pub fn default_regex_only() -> Self {
        Self {
            extractor: Arc::new(extract::CompositeExtractor::regex_only()),
            weights: SignalWeights::default(),
            drop_threshold: DEFAULT_DROP_THRESHOLD,
        }
    }
}

/// Compute the score for one chunk.
///
/// Pure function — does not touch the store. Callers decide what to persist
/// based on [`ScoreResult::kept`].
pub async fn score_chunk(chunk: &Chunk, cfg: &ScoringConfig) -> Result<ScoreResult> {
    log::debug!(
        "[memory_tree::score] score_chunk chunk_id={} tokens={}",
        chunk.id,
        chunk.token_count
    );

    // 1. Extract entities (regex + any configured semantic extractors)
    let extracted = cfg.extractor.extract(&chunk.content).await?;

    // 2. Compute signals
    let signals = self::signals::compute(
        &chunk.metadata,
        &chunk.content,
        chunk.token_count,
        &extracted,
    );

    // 3. Weighted combine
    let total = self::signals::combine(&signals, &cfg.weights);

    // 4. Admission gate
    let kept = total >= cfg.drop_threshold;
    let drop_reason = if kept {
        None
    } else {
        Some(format!(
            "total {total:.3} < threshold {:.3}",
            cfg.drop_threshold
        ))
    };

    // 5. Canonicalise for indexing (only meaningful when kept — but we
    //    canonicalise unconditionally so the result is inspectable in tests)
    let canonical_entities = canonicalise(&extracted);

    if !kept {
        log::debug!(
            "[memory_tree::score] drop chunk_id={} total={:.3} reason={:?}",
            chunk.id,
            total,
            drop_reason
        );
    }

    Ok(ScoreResult {
        chunk_id: chunk.id.clone(),
        total,
        signals,
        kept,
        drop_reason,
        extracted,
        canonical_entities,
    })
}

/// Score a batch of chunks. Errors from any single chunk fail the batch —
/// scoring is pure-ish (only the extractor may error) and a failure here is
/// a real bug, not a per-chunk issue to tolerate silently.
pub async fn score_chunks(chunks: &[Chunk], cfg: &ScoringConfig) -> Result<Vec<ScoreResult>> {
    let mut out = Vec::with_capacity(chunks.len());
    for c in chunks {
        out.push(score_chunk(c, cfg).await?);
    }
    Ok(out)
}

// ── Persistence helpers used by the ingest orchestrator ─────────────────

/// Persist the score row + entity-index rows for one kept chunk.
///
/// The caller is responsible for having already written the chunk itself
/// into `mem_tree_chunks` (so the FK-like relation is satisfied). Dropped
/// chunks still get a score row persisted for diagnostics — callers should
/// pass `None` for `tree_id` in that case, since the chunk won't appear in
/// a tree.
pub fn persist_score(
    config: &crate::openhuman::config::Config,
    result: &ScoreResult,
    timestamp_ms: i64,
    tree_id: Option<&str>,
) -> Result<()> {
    let row = store::ScoreRow {
        chunk_id: result.chunk_id.clone(),
        total: result.total,
        signals: result.signals.clone(),
        dropped: !result.kept,
        reason: result.drop_reason.clone(),
        computed_at_ms: Utc::now().timestamp_millis(),
    };
    store::upsert_score(config, &row)?;

    if result.kept && !result.canonical_entities.is_empty() {
        store::index_entities(
            config,
            &result.canonical_entities,
            &result.chunk_id,
            "leaf",
            timestamp_ms,
            tree_id,
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind};
    use chrono::Utc;

    fn test_chunk(content: &str) -> Chunk {
        let meta = Metadata::point_in_time(SourceKind::Email, "t1", "alice", Utc::now());
        Chunk {
            id: chunk_id(SourceKind::Email, "t1", 0),
            content: content.to_string(),
            token_count: crate::openhuman::memory::tree::types::approx_token_count(content),
            metadata: meta,
            seq_in_source: 0,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn substantive_chunk_is_kept() {
        let c = test_chunk(
            "We decided to ship Phoenix on Friday after reviewing \
             alice@example.com and the migration plan carefully. \
             @bob will coordinate and we discussed #launch-q2 details.",
        );
        let cfg = ScoringConfig::default_regex_only();
        let r = score_chunk(&c, &cfg).await.unwrap();
        assert!(r.kept, "expected kept, got total={}", r.total);
        assert!(r.drop_reason.is_none());
        assert!(!r.extracted.entities.is_empty());
        assert!(!r.canonical_entities.is_empty());
    }

    #[tokio::test]
    async fn noise_chunk_is_dropped() {
        // Very short — below TOKEN_MIN — and no entities.
        let c = test_chunk("lol");
        let cfg = ScoringConfig::default_regex_only();
        let r = score_chunk(&c, &cfg).await.unwrap();
        assert!(!r.kept);
        assert!(r.drop_reason.is_some());
    }

    #[tokio::test]
    async fn threshold_override_respected() {
        let c = test_chunk("just ok content, mid-signal");
        let mut cfg = ScoringConfig::default_regex_only();
        cfg.drop_threshold = 0.99; // unreasonably high
        let r = score_chunk(&c, &cfg).await.unwrap();
        assert!(!r.kept);
    }

    #[tokio::test]
    async fn entities_are_canonicalised() {
        let c = test_chunk("ping Alice@Example.com — she @alice replied to thread");
        let cfg = ScoringConfig::default_regex_only();
        let r = score_chunk(&c, &cfg).await.unwrap();
        // Email (lowercased) and handle canonical ids should both appear
        let ids: Vec<_> = r
            .canonical_entities
            .iter()
            .map(|e| e.canonical_id.as_str())
            .collect();
        assert!(ids.iter().any(|id| *id == "email:alice@example.com"));
        assert!(ids.iter().any(|id| *id == "handle:alice"));
    }
}
