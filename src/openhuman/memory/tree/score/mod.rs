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
#[path = "mod_tests.rs"]
mod tests;
