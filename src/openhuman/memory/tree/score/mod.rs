//! Phase 2: scoring / admission / enrichment pipeline (#708).
//!
//! Wraps extraction, signal computation, admission gate, canonicalisation,
//! and persistence into one call per chunk. Phase 1 `_ingest_one_chunk`
//! passes each chunk through [`score_chunk`] after chunking and before
//! storing.

pub mod embed;
pub mod extract;
pub mod resolver;
pub mod signals;
pub mod store;

use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use futures_util::future::try_join_all;
use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use self::extract::{EntityExtractor, ExtractedEntities};
use self::resolver::{canonicalise, CanonicalEntity};
use self::signals::{ScoreSignals, SignalWeights};
use crate::openhuman::memory::tree::types::{approx_token_count, Chunk, SourceKind};

/// Default drop threshold. Chunks with `total < DEFAULT_DROP_THRESHOLD`
/// are tombstoned and never reach the chunk store.
pub const DEFAULT_DROP_THRESHOLD: f32 = 0.3;

/// If the deterministic (cheap-signals-only) total is at or above this,
/// the chunk is admitted without consulting the LLM extractor.
///
/// Tuned to leave a generous "borderline" band where the LLM signal is
/// most informative while skipping LLM cost on obviously substantive
/// content.
pub const DEFAULT_DEFINITE_KEEP: f32 = 0.85;

/// If the deterministic total is at or below this, the chunk is dropped
/// without consulting the LLM extractor. Catches obvious noise cheaply.
pub const DEFAULT_DEFINITE_DROP: f32 = 0.15;

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
///
/// The `extractor` field always runs (typically a regex-based composite
/// for cheap mechanical entities). `llm_extractor` is consulted **only
/// when the cheap-signals total falls in the band**
/// `(definite_drop_threshold, definite_keep_threshold)` — chunks that are
/// obviously trash or obviously substantive don't pay the LLM cost.
pub struct ScoringConfig {
    pub extractor: Arc<dyn EntityExtractor>,
    pub weights: SignalWeights,
    pub drop_threshold: f32,
    /// Optional second-pass extractor whose output is **merged** into the
    /// regex output before the final combine. Designed for LLM-based NER +
    /// importance signal (see [`extract::LlmEntityExtractor`]). `None`
    /// means LLM augmentation is disabled.
    pub llm_extractor: Option<Arc<dyn EntityExtractor>>,
    /// Cheap-signals total ≥ this → admit without consulting LLM.
    pub definite_keep_threshold: f32,
    /// Cheap-signals total ≤ this → drop without consulting LLM.
    pub definite_drop_threshold: f32,
}

impl ScoringConfig {
    /// Phase 2 default: regex-only extractor, default weights, default threshold.
    pub fn default_regex_only() -> Self {
        Self {
            extractor: Arc::new(extract::CompositeExtractor::regex_only()),
            weights: SignalWeights::default(),
            drop_threshold: DEFAULT_DROP_THRESHOLD,
            llm_extractor: None,
            definite_keep_threshold: DEFAULT_DEFINITE_KEEP,
            definite_drop_threshold: DEFAULT_DEFINITE_DROP,
        }
    }

    /// Convenience constructor: regex always + LLM extractor on borderline
    /// chunks. The `llm_importance` weight is enabled in [`SignalWeights`]
    /// so the LLM signal actually influences the final total.
    pub fn with_llm_extractor(llm: Arc<dyn EntityExtractor>) -> Self {
        Self {
            extractor: Arc::new(extract::CompositeExtractor::regex_only()),
            weights: SignalWeights::with_llm_enabled(),
            drop_threshold: DEFAULT_DROP_THRESHOLD,
            llm_extractor: Some(llm),
            definite_keep_threshold: DEFAULT_DEFINITE_KEEP,
            definite_drop_threshold: DEFAULT_DEFINITE_DROP,
        }
    }

    /// Build a [`ScoringConfig`] from the workspace [`Config`]. When
    /// `memory_tree.llm_extractor_endpoint` and `llm_extractor_model`
    /// are both set, wires [`extract::LlmEntityExtractor`] as the
    /// second-pass extractor. Otherwise falls back to
    /// [`Self::default_regex_only`]. Construction errors in the LLM
    /// extractor (rare — only client-builder failures) also fall back
    /// to regex-only with a warn log; scoring never blocks on LLM
    /// availability.
    pub fn from_config(config: &crate::openhuman::config::Config) -> Self {
        let endpoint = config
            .memory_tree
            .llm_extractor_endpoint
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let model = config
            .memory_tree
            .llm_extractor_model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let (Some(endpoint), Some(model)) = (endpoint, model) else {
            log::debug!("[memory_tree::score] llm_extractor not configured — using regex-only");
            return Self::default_regex_only();
        };

        let timeout_ms = config
            .memory_tree
            .llm_extractor_timeout_ms
            .unwrap_or(15_000);

        let cfg = extract::LlmExtractorConfig {
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            timeout: std::time::Duration::from_millis(timeout_ms),
            ..extract::LlmExtractorConfig::default()
        };
        match extract::LlmEntityExtractor::new(cfg) {
            Ok(llm) => {
                log::info!(
                    "[memory_tree::score] using LlmEntityExtractor endpoint={} model={} timeout_ms={}",
                    endpoint,
                    model,
                    timeout_ms
                );
                Self::with_llm_extractor(Arc::new(llm))
            }
            Err(err) => {
                log::warn!(
                    "[memory_tree::score] LlmEntityExtractor construction failed: {err:#} — \
                     falling back to regex-only"
                );
                Self::default_regex_only()
            }
        }
    }
}

/// Compute the score for one chunk.
///
/// Pure function — does not touch the store. Callers decide what to persist
/// based on [`ScoreResult::kept`].
///
/// Pipeline:
/// 1. Run the always-on extractor (typically regex).
/// 2. Compute cheap signals; combine **excluding** `llm_importance` weight.
/// 3. Short-circuit:
///    - If cheap total ≥ `definite_keep_threshold`: admit without LLM.
///    - If cheap total ≤ `definite_drop_threshold`: drop without LLM.
///    - Else: borderline — run the LLM extractor (if configured), merge
///      its output, recompute signals, recombine with full weights.
/// 4. Apply final admission gate against `drop_threshold`.
pub async fn score_chunk(chunk: &Chunk, cfg: &ScoringConfig) -> Result<ScoreResult> {
    log::debug!(
        "[memory_tree::score] score_chunk chunk_id={} tokens={}",
        chunk.id,
        chunk.token_count
    );

    let scoring_content = scoring_content_for_chunk(chunk);
    let scoring_token_count = approx_token_count(&scoring_content);

    // 1. Always-on extraction (regex / mechanical).
    let mut extracted = cfg.extractor.extract(&scoring_content).await?;

    // 2. Compute cheap signals + combine excluding LLM importance.
    let mut signals = self::signals::compute(
        &chunk.metadata,
        &scoring_content,
        scoring_token_count,
        &extracted,
    );
    let cheap_total = self::signals::combine_cheap_only(&signals, &cfg.weights);

    // 3. Short-circuit decision.
    let in_band =
        cheap_total > cfg.definite_drop_threshold && cheap_total < cfg.definite_keep_threshold;
    let llm_consulted = if in_band {
        if let Some(llm) = cfg.llm_extractor.as_ref() {
            log::debug!(
                "[memory_tree::score] borderline chunk_id={} cheap_total={:.3} — consulting LLM",
                chunk.id,
                cheap_total
            );
            match llm.extract(&scoring_content).await {
                Ok(more) => {
                    extracted.merge(more);
                    // Recompute signals so llm_importance flows in.
                    signals = self::signals::compute(
                        &chunk.metadata,
                        &scoring_content,
                        scoring_token_count,
                        &extracted,
                    );
                    true
                }
                Err(e) => {
                    log::warn!(
                        "[memory_tree::score] LLM extractor `{}` failed: {e} — \
                         falling back to cheap signals only",
                        llm.name()
                    );
                    false
                }
            }
        } else {
            false
        }
    } else {
        log::debug!(
            "[memory_tree::score] short-circuit chunk_id={} cheap_total={:.3} \
             ({}, skipping LLM)",
            chunk.id,
            cheap_total,
            if cheap_total >= cfg.definite_keep_threshold {
                "definite_keep"
            } else {
                "definite_drop"
            }
        );
        false
    };

    // 4. Final weighted combine.
    //
    // If the LLM ran, its importance signal is populated → use the full
    // `combine` which includes the `llm_importance` weight.
    //
    // If the LLM was skipped (short-circuited or not configured) OR failed
    // (caught above, sets `llm_consulted=false`), using the full combine
    // would pin `llm_importance * w.llm_importance = 0 * 2.0` into the
    // numerator while still dividing by the full denominator — artificially
    // dragging the total down. Fall back to `combine_cheap_only` which
    // excludes that term from both numerator and denominator, so the cheap
    // signals alone produce the total.
    let total = if llm_consulted {
        self::signals::combine(&signals, &cfg.weights)
    } else {
        self::signals::combine_cheap_only(&signals, &cfg.weights)
    };

    // 5. Admission gate. Source and interaction priors are deliberately
    // non-zero, so guard against very short entity-free chatter being kept by
    // metadata alone.
    let tiny_entity_free =
        scoring_token_count < self::signals::token_count::TOKEN_MIN && extracted.is_empty();
    let kept = !tiny_entity_free && total >= cfg.drop_threshold;
    let drop_reason = if kept {
        None
    } else if tiny_entity_free {
        Some(format!(
            "token_count {} < minimum {} and no entities extracted",
            scoring_token_count,
            self::signals::token_count::TOKEN_MIN
        ))
    } else {
        Some(format!(
            "total {total:.3} < threshold {:.3}",
            cfg.drop_threshold
        ))
    };

    // 6. Canonicalise for indexing (only meaningful when kept — but we
    //    canonicalise unconditionally so the result is inspectable in tests)
    let canonical_entities = canonicalise(&extracted);

    if !kept {
        log::debug!(
            "[memory_tree::score] drop chunk_id={} total={:.3} reason={:?} llm_consulted={}",
            chunk.id,
            total,
            drop_reason,
            llm_consulted
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

fn scoring_content_for_chunk(chunk: &Chunk) -> String {
    if chunk.metadata.source_kind != SourceKind::Chat {
        return chunk.content.clone();
    }

    chunk
        .content
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("# Chat transcript") && !trimmed.starts_with("## ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Score a batch of chunks. Errors from any single chunk fail the batch —
/// scoring is pure-ish (only the extractor may error) and a failure here is
/// a real bug, not a per-chunk issue to tolerate silently.
pub async fn score_chunks(chunks: &[Chunk], cfg: &ScoringConfig) -> Result<Vec<ScoreResult>> {
    try_join_all(chunks.iter().map(|chunk| score_chunk(chunk, cfg))).await
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
    let row = score_row(result);
    store::upsert_score(config, &row)?;

    if result.kept {
        // Clear any stale entity-index rows for this chunk before re-indexing.
        // INSERT OR REPLACE on (entity_id, node_id) never deletes rows whose
        // entity_id is no longer present in the new extraction — so a re-score
        // that drops an entity would otherwise leave a phantom index row.
        store::clear_entity_index_for_node(config, &result.chunk_id)?;
        if !result.canonical_entities.is_empty() {
            store::index_entities(
                config,
                &result.canonical_entities,
                &result.chunk_id,
                "leaf",
                timestamp_ms,
                tree_id,
            )?;
        }
    }

    Ok(())
}

pub(crate) fn persist_score_tx(
    tx: &Transaction<'_>,
    result: &ScoreResult,
    timestamp_ms: i64,
    tree_id: Option<&str>,
) -> Result<()> {
    let row = score_row(result);
    store::upsert_score_tx(tx, &row)?;

    if result.kept {
        // See persist_score for why we clear before re-indexing.
        store::clear_entity_index_for_node_tx(tx, &result.chunk_id)?;
        if !result.canonical_entities.is_empty() {
            store::index_entities_tx(
                tx,
                &result.canonical_entities,
                &result.chunk_id,
                "leaf",
                timestamp_ms,
                tree_id,
            )?;
        }
    }

    Ok(())
}

fn score_row(result: &ScoreResult) -> store::ScoreRow {
    // Score rows keep wall-clock scoring time; the separate timestamp_ms
    // argument used for entity indexes is the source/ingest ordering time.
    store::ScoreRow {
        chunk_id: result.chunk_id.clone(),
        total: result.total,
        signals: result.signals.clone(),
        dropped: !result.kept,
        reason: result.drop_reason.clone(),
        computed_at_ms: Utc::now().timestamp_millis(),
        llm_importance_reason: result.extracted.llm_importance_reason.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind};
    use chrono::Utc;

    fn test_chunk(content: &str) -> Chunk {
        let meta = Metadata::point_in_time(SourceKind::Email, "t1", "alice", Utc::now());
        Chunk {
            id: chunk_id(SourceKind::Email, "t1", 0, "test-content"),
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

    // ── Short-circuit / LLM-extractor tests ─────────────────────────────

    /// Test extractor that returns a fixed importance value and records call count.
    struct FakeLlm {
        importance: f32,
        call_count: std::sync::atomic::AtomicUsize,
    }

    impl FakeLlm {
        fn new(importance: f32) -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                importance,
                call_count: std::sync::atomic::AtomicUsize::new(0),
            })
        }
        fn calls(&self) -> usize {
            self.call_count.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    #[async_trait::async_trait]
    impl extract::EntityExtractor for FakeLlm {
        fn name(&self) -> &'static str {
            "fake-llm"
        }
        async fn extract(&self, _text: &str) -> Result<extract::ExtractedEntities> {
            self.call_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(extract::ExtractedEntities {
                entities: vec![],
                topics: vec![],
                llm_importance: Some(self.importance),
                llm_importance_reason: Some("fake".into()),
            })
        }
    }

    #[tokio::test]
    async fn short_circuit_skips_llm_when_cheap_total_is_definite_keep() {
        // A substantive chunk with high cheap-total should bypass the LLM.
        let c = test_chunk(
            "We decided to ship Phoenix on Friday after reviewing alice@example.com and \
             the migration plan carefully. @bob will coordinate and we discussed \
             #launch-q2 details extensively in the email thread.",
        );
        let llm = FakeLlm::new(0.5);
        let mut cfg = ScoringConfig::with_llm_extractor(llm.clone());
        // Force the cheap total well above the keep threshold by lowering
        // the keep threshold so this test is robust to weight tuning.
        cfg.definite_keep_threshold = 0.10;
        let r = score_chunk(&c, &cfg).await.unwrap();
        assert!(r.kept);
        assert_eq!(llm.calls(), 0, "LLM should not be consulted");
        // signals.llm_importance stays at 0 (no LLM call happened)
        assert_eq!(r.signals.llm_importance, 0.0);
    }

    #[tokio::test]
    async fn short_circuit_skips_llm_when_cheap_total_is_definite_drop() {
        // A noisy chunk with very low cheap total should bypass the LLM
        // and be dropped.
        let c = test_chunk("ok");
        let llm = FakeLlm::new(0.99);
        let mut cfg = ScoringConfig::with_llm_extractor(llm.clone());
        // Force the cheap total to look like definite_drop.
        cfg.definite_drop_threshold = 0.99;
        let r = score_chunk(&c, &cfg).await.unwrap();
        assert!(!r.kept);
        assert_eq!(
            llm.calls(),
            0,
            "LLM should not be consulted on definite_drop"
        );
    }

    #[tokio::test]
    async fn borderline_chunk_consults_llm() {
        // Pick content that will land in the borderline band and verify the LLM
        // gets called. Use generous band edges so the test isn't sensitive
        // to weight nudges.
        let c = test_chunk("This is a moderately interesting note about a project.");
        let llm = FakeLlm::new(0.9);
        let mut cfg = ScoringConfig::with_llm_extractor(llm.clone());
        cfg.definite_drop_threshold = 0.0;
        cfg.definite_keep_threshold = 1.0;
        let r = score_chunk(&c, &cfg).await.unwrap();
        assert_eq!(llm.calls(), 1, "LLM should be consulted exactly once");
        assert!(r.signals.llm_importance > 0.0);
        assert_eq!(r.extracted.llm_importance_reason.as_deref(), Some("fake"));
    }

    #[tokio::test]
    async fn llm_failure_falls_back_gracefully() {
        struct FailingLlm;
        #[async_trait::async_trait]
        impl extract::EntityExtractor for FailingLlm {
            fn name(&self) -> &'static str {
                "failing-llm"
            }
            async fn extract(&self, _text: &str) -> Result<extract::ExtractedEntities> {
                Err(anyhow::anyhow!("simulated failure"))
            }
        }
        let c = test_chunk("This is a moderately interesting note about a project.");
        let mut cfg = ScoringConfig::with_llm_extractor(std::sync::Arc::new(FailingLlm));
        cfg.definite_drop_threshold = 0.0;
        cfg.definite_keep_threshold = 1.0;
        // Should not error out; should produce a result based on cheap signals only.
        let r = score_chunk(&c, &cfg).await.unwrap();
        assert_eq!(r.signals.llm_importance, 0.0);
    }

    /// When LLM is skipped (short-circuit or failure), the reported `total`
    /// must equal `combine_cheap_only(signals, weights)` — not the
    /// LLM-weighted `combine` (which would drag `llm_importance=0` through
    /// a 2.0 weight and artificially lower the total).
    #[tokio::test]
    async fn short_circuit_reports_cheap_only_total() {
        let c = test_chunk(
            "We decided to ship Phoenix on Friday after reviewing alice@example.com and \
             the migration plan carefully. @bob will coordinate and we discussed \
             #launch-q2 details extensively in the email thread.",
        );
        let llm = FakeLlm::new(0.99);
        let mut cfg = ScoringConfig::with_llm_extractor(llm.clone());
        cfg.definite_keep_threshold = 0.10; // force short-circuit keep
        let r = score_chunk(&c, &cfg).await.unwrap();
        assert_eq!(llm.calls(), 0);
        let expected = self::signals::combine_cheap_only(&r.signals, &cfg.weights);
        assert!(
            (r.total - expected).abs() < 1e-6,
            "total={} expected(cheap_only)={}",
            r.total,
            expected
        );
        // And explicitly NOT the full combine (which would include a 0-value
        // llm_importance term in a 0..1-clamped weighted average, dragging
        // the total down).
        let with_llm = self::signals::combine(&r.signals, &cfg.weights);
        assert!(
            r.total > with_llm,
            "cheap-only total ({}) should exceed LLM-weighted total \
             ({}) when llm_importance is zero",
            r.total,
            with_llm
        );
    }

    /// When the LLM *does* run, the reported total uses the full combine —
    /// the llm_importance contribution is actually in the sum.
    #[tokio::test]
    async fn llm_consulted_reports_full_total() {
        let c = test_chunk("This is a moderately interesting note about a project.");
        let llm = FakeLlm::new(0.9);
        let mut cfg = ScoringConfig::with_llm_extractor(llm.clone());
        cfg.definite_drop_threshold = 0.0;
        cfg.definite_keep_threshold = 1.0;
        let r = score_chunk(&c, &cfg).await.unwrap();
        assert_eq!(llm.calls(), 1);
        let expected = self::signals::combine(&r.signals, &cfg.weights);
        assert!(
            (r.total - expected).abs() < 1e-6,
            "total={} expected(full combine)={}",
            r.total,
            expected
        );
    }
}
