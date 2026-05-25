//! End-of-day digest builder for the global activity tree (#709 Phase 3b).
//!
//! Once per calendar day we walk every active source tree, collect the
//! summary material that covers that day, fold it into one cross-source
//! recap, and persist it as an L0 node in the singleton global tree. A
//! cascade then checks whether enough daily nodes have accumulated to seal
//! the weekly/monthly/yearly levels.
//!
//! Design:
//! - Populated day → exactly one L0 (daily) node emitted + cascade.
//! - Empty day (no source tree touched today) → no-op, logs the skip.
//! - The digest picks the best "representative" input from each source
//!   tree in priority order: (a) the latest L1+ summary whose time range
//!   intersects the target day, else (b) the most recent chunk that day's
//!   L0 buffer still holds, else (c) skip that tree. This keeps the digest
//!   accurate for both high-volume sources (where material has already
//!   sealed into an L1) and low-volume sources (where the day's activity
//!   is still in the L0 buffer).
//! - Idempotency: if an L0 daily node already exists for the target day,
//!   return `DigestOutcome::Skipped` rather than emitting a duplicate.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use rusqlite::OptionalExtension;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree_global::seal::append_daily_and_cascade;
use crate::openhuman::memory_store::chunks::store::with_connection;
use crate::openhuman::memory_store::content::{
    atomic::stage_summary, paths::slugify_source_id, read as content_read, SummaryComposeInput,
    SummaryTreeKind,
};
use crate::openhuman::memory_store::trees::types::{SummaryNode, Tree, TreeKind};
use crate::openhuman::memory_tree::score::embed::build_embedder_from_config;
use crate::openhuman::memory_tree::summarise::{summarise, SummaryContext, SummaryInput};
use crate::openhuman::memory_tree::tree::registry::new_summary_id;
use crate::openhuman::memory_tree::tree::store;

/// Outcome of a single `end_of_day_digest` call — lets the caller decide
/// whether to log skip details or propagate seal counts to telemetry.
#[derive(Debug, Clone)]
pub enum DigestOutcome {
    /// Emitted one L0 daily node covering `date`, and possibly cascaded
    /// into higher-level seals. `sealed_ids` lists any L1/L2/L3 nodes that
    /// sealed during the cascade (empty when the weekly threshold wasn't
    /// crossed).
    Emitted {
        daily_id: String,
        source_count: usize,
        sealed_ids: Vec<String>,
    },
    /// No source tree had material to contribute for `date` — nothing was
    /// written.
    EmptyDay,
    /// An L0 node already exists for `date` (e.g. this is a re-run of the
    /// same day's digest). Nothing was written.
    Skipped { existing_id: String },
}

/// Run an end-of-day digest for `day`, appending one L0 node to the global
/// tree and cascade-sealing upward if thresholds are crossed. The
/// `summarise` function is called once to fold the per-source material into
/// a single cross-source recap; on failure it falls back to a deterministic
/// concat-and-truncate so the digest never aborts due to an LLM error.
///
/// `day` is the calendar date in UTC the digest should cover. Callers that
/// simply want "yesterday" can pass `Utc::now().date_naive() - Duration::days(1)`.
pub async fn end_of_day_digest(config: &Config, day: NaiveDate) -> Result<DigestOutcome> {
    let (day_start, day_end) = day_bounds_utc(day)?;
    log::info!(
        "[tree_global::digest] end_of_day_digest day={} window=[{}, {})",
        day,
        day_start,
        day_end
    );

    let global = crate::openhuman::memory::tree_global::factory().get_or_create(config)?;

    // Idempotency: check for an existing L0 daily node whose time range
    // matches this day.
    if let Some(existing) = find_existing_daily(config, &global.id, day_start, day_end)? {
        log::info!(
            "[tree_global::digest] daily already exists for {day} id={} — skipping",
            existing.id
        );
        return Ok(DigestOutcome::Skipped {
            existing_id: existing.id,
        });
    }

    // Gather one contribution per active source tree.
    let source_trees = store::list_trees_by_kind(config, TreeKind::Source)?;
    log::debug!(
        "[tree_global::digest] scanning {} source trees",
        source_trees.len()
    );
    let mut inputs: Vec<SummaryInput> = Vec::with_capacity(source_trees.len());
    for source_tree in &source_trees {
        match pick_source_contribution(config, source_tree, day_start, day_end)? {
            Some(inp) => {
                log::debug!(
                    "[tree_global::digest] source={} contributed id={} tokens={}",
                    source_tree.scope,
                    inp.id,
                    inp.token_count
                );
                inputs.push(inp);
            }
            None => {
                log::debug!(
                    "[tree_global::digest] source={} had no material for {day}",
                    source_tree.scope
                );
            }
        }
    }

    if inputs.is_empty() {
        log::info!(
            "[tree_global::digest] empty day — no source trees contributed material for {day}"
        );
        return Ok(DigestOutcome::EmptyDay);
    }

    // Fold cross-source material into one daily recap.
    let ctx = SummaryContext {
        tree_id: &global.id,
        tree_kind: TreeKind::Global,
        target_level: 0, // daily node lives at L0 on the global tree
        token_budget: crate::openhuman::memory::tree_global::GLOBAL_TOKEN_BUDGET,
    };
    let output = match summarise(config, &inputs, &ctx).await {
        Ok(o) => o,
        Err(e) => {
            log::warn!(
                "[tree_global::digest] summarise failed for day={day}: {e:#} — using fallback"
            );
            crate::openhuman::memory_tree::summarise::fallback_summary(&inputs, ctx.token_budget)
        }
    };

    // Envelope: time range is the day's bounds, score carries the max
    // contribution score so recall still has a ranking signal.
    let score = inputs
        .iter()
        .map(|i| i.score)
        .fold(f32::NEG_INFINITY, f32::max)
        .max(0.0);

    // Phase 4 (#710): embed before opening the write tx so an embedder
    // error aborts the digest without leaving a half-committed row.
    let embedder =
        build_embedder_from_config(config).context("build embedder during end_of_day_digest")?;
    let embedding = embedder
        .embed(&output.content)
        .await
        .context("embed daily summary during end_of_day_digest")?;

    // L0 daily node inherits entities/topics by union of contributing
    // source-tree summaries. Each input was already labeled at source-tree
    // seal time, so emergent themes don't need another extractor pass
    // here — global is a sink; union preserves "days that mentioned X"
    // retrieval without an extra LLM call. See LabelStrategy in
    // tree::bucket_seal for the full design.
    let mut entities_set: BTreeSet<String> = BTreeSet::new();
    let mut topics_set: BTreeSet<String> = BTreeSet::new();
    for inp in &inputs {
        for e in &inp.entities {
            entities_set.insert(e.clone());
        }
        for t in &inp.topics {
            topics_set.insert(t.clone());
        }
    }
    let daily_entities: Vec<String> = entities_set.into_iter().collect();
    let daily_topics: Vec<String> = topics_set.into_iter().collect();

    let now = Utc::now();
    let daily_id = new_summary_id(0);
    let daily = SummaryNode {
        id: daily_id.clone(),
        tree_id: global.id.clone(),
        tree_kind: TreeKind::Global,
        level: 0,
        parent_id: None,
        child_ids: inputs.iter().map(|i| i.id.clone()).collect(),
        content: output.content,
        token_count: output.token_count,
        entities: daily_entities,
        topics: daily_topics,
        time_range_start: day_start,
        time_range_end: day_end,
        score,
        sealed_at: now,
        deleted: false,
        embedding: Some(embedding),
    };

    // Phase MD-content: stage the L0 daily .md file before the write tx.
    // `date_for_global` = day_start (the calendar day this digest covers).
    let daily_compose_input = SummaryComposeInput {
        summary_id: &daily.id,
        tree_kind: SummaryTreeKind::Global,
        tree_id: &daily.tree_id,
        tree_scope: &global.scope,
        level: daily.level,
        child_ids: &daily.child_ids,
        child_basenames: None,
        child_count: daily.child_ids.len(),
        time_range_start: daily.time_range_start,
        time_range_end: daily.time_range_end,
        sealed_at: daily.sealed_at,
        body: &daily.content,
    };
    // Stage the summary .md file — abort the digest on failure so the database
    // never commits a row with content_path = NULL. The digest job is retried
    // via the normal job-retry path.
    let content_root_daily = config.memory_tree_content_root();
    let global_scope_slug = slugify_source_id(&global.scope);
    let staged_daily = stage_summary(
        &content_root_daily,
        &daily_compose_input,
        &global_scope_slug,
        Some(day_start),
    )
    .with_context(|| {
        format!(
            "stage_summary failed for daily {}; digest aborted for retry",
            daily.id
        )
    })?;
    log::debug!(
        "[tree_global::digest] staged daily summary {} → {}",
        daily.id,
        staged_daily.content_path
    );

    // Persist the daily node. Note: we do NOT backlink parent_id on the
    // child summaries here — their parents are their own source trees, not
    // the global tree. The global-tree child_ids are cross-source
    // *references*, not ownership.
    let daily_clone = daily.clone();
    let tree_id_clone = global.id.clone();
    with_connection(config, move |conn| {
        let tx = conn.unchecked_transaction()?;
        store::insert_summary_tx(
            &tx,
            &daily_clone,
            Some(&staged_daily),
            &crate::openhuman::memory_store::chunks::store::tree_active_signature(config),
        )?;
        // Index any entities the summariser emitted (no-op under inert).
        crate::openhuman::memory_tree::score::store::index_summary_entity_ids_tx(
            &tx,
            &daily_clone.entities,
            &daily_clone.id,
            daily_clone.score,
            now.timestamp_millis(),
            Some(&tree_id_clone),
        )?;
        tx.commit()?;
        Ok(())
    })?;

    log::info!(
        "[tree_global::digest] emitted daily id={} sources={} tokens={}",
        daily.id,
        inputs.len(),
        daily.token_count
    );

    // Append into L0 buffer + cascade-seal if thresholds crossed.
    let sealed_ids = append_daily_and_cascade(config, &global, &daily).await?;

    Ok(DigestOutcome::Emitted {
        daily_id: daily.id,
        source_count: inputs.len(),
        sealed_ids,
    })
}

/// Compute [00:00, 24:00) UTC bounds for a calendar day.
fn day_bounds_utc(day: NaiveDate) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let start_naive = day
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow::anyhow!("invalid day {day} — failed to build 00:00 timestamp"))?;
    let start = Utc
        .from_local_datetime(&start_naive)
        .single()
        .ok_or_else(|| anyhow::anyhow!("non-unique UTC time for day {day}"))?;
    Ok((start, start + Duration::days(1)))
}

/// Look for an already-emitted L0 daily node for this day. Matches on
/// `tree_kind='global' AND level=0 AND time_range_start=day_start AND deleted=0`.
fn find_existing_daily(
    config: &Config,
    global_tree_id: &str,
    day_start: DateTime<Utc>,
    _day_end: DateTime<Utc>,
) -> Result<Option<SummaryNode>> {
    let start_ms = day_start.timestamp_millis();
    let opt_id: Option<String> = with_connection(config, |conn| {
        let id: Option<String> = conn
            .query_row(
                "SELECT id FROM mem_tree_summaries
                  WHERE tree_id = ?1
                    AND level = 0
                    AND time_range_start_ms = ?2
                    AND deleted = 0
                  LIMIT 1",
                rusqlite::params![global_tree_id, start_ms],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .context("query for existing daily node")?;
        Ok(id)
    })?;
    match opt_id {
        Some(id) => store::get_summary(config, &id),
        None => Ok(None),
    }
}

/// Pick the single best contribution from one source tree for the target
/// day. Priority:
///   1. The latest L1+ summary whose time range intersects the day.
///   2. The tree's current root summary (any level), as a fallback when no
///      summary intersects the exact day window.
///
/// Returns `None` when the tree has no sealed summaries at all — a
/// brand-new tree whose L0 buffer has not yet crossed the token budget.
/// Phase 3b intentionally skips such trees rather than plumbing the raw
/// L0 buffer into the digest; low-volume sources become visible once
/// either the token or time-based flush lands them in a summary.
fn pick_source_contribution(
    config: &Config,
    source_tree: &Tree,
    day_start: DateTime<Utc>,
    day_end: DateTime<Utc>,
) -> Result<Option<SummaryInput>> {
    let start_ms = day_start.timestamp_millis();
    let end_ms = day_end.timestamp_millis();
    let intersecting_id: Option<String> = with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id FROM mem_tree_summaries
              WHERE tree_id = ?1
                AND deleted = 0
                AND time_range_start_ms < ?3
                AND time_range_end_ms >= ?2
              ORDER BY level DESC, sealed_at_ms DESC
              LIMIT 1",
        )?;
        let row = stmt
            .query_row(rusqlite::params![&source_tree.id, start_ms, end_ms], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .context("query intersecting source summary")?;
        Ok(row)
    })?;

    let chosen_id = match intersecting_id {
        Some(id) => Some(id),
        None => source_tree.root_id.clone(),
    };

    let Some(id) = chosen_id else {
        return Ok(None);
    };

    let node = match store::get_summary(config, &id)? {
        Some(n) => n,
        None => {
            log::warn!(
                "[tree_global::digest] picked id={id} for tree={} but row missing — skipping",
                source_tree.scope
            );
            return Ok(None);
        }
    };

    // Read the full body from disk — `node.content` is a ≤500-char preview
    // after the MD-on-disk migration. The digest summariser must receive the
    // complete summary text so the daily recap is not assembled from previews.
    let body = match content_read::read_summary_body(config, &node.id) {
        Ok(b) => b,
        Err(e) => {
            log::warn!(
                "[tree_global::digest] read_summary_body failed for {} — using preview: {e:#}",
                node.id
            );
            // Non-fatal: fall back to preview for pre-MD-migration rows.
            node.content.clone()
        }
    };
    Ok(Some(SummaryInput {
        id: node.id,
        content: format!("[{}]\n{}", source_tree.scope, body),
        token_count: node.token_count,
        entities: node.entities,
        topics: node.topics,
        time_range_start: node.time_range_start,
        time_range_end: node.time_range_end,
        score: node.score,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::chat::{test_override, ChatProvider, StaticChatProvider};
    use crate::openhuman::memory::tree_source::registry::get_or_create_source_tree;
    use crate::openhuman::memory_store::chunks::store::upsert_chunks;
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    use crate::openhuman::memory_store::content as content_store;
    use crate::openhuman::memory_tree::tree::bucket_seal::{append_leaf, LabelStrategy, LeafRef};
    use std::sync::Arc;
    use tempfile::TempDir;

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    /// Stage chunk content files on disk so `read_summary_body` can find them
    /// during `pick_source_contribution`.
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

    /// Create a source tree for `scope`, upsert two 30 k-token chunks so that
    /// `append_leaf` triggers an L0→L1 seal (the 50 k-token threshold), and
    /// stage the chunk content files on disk.  The resulting source tree has
    /// at least one sealed summary that `pick_source_contribution` can return.
    async fn seed_source_l1(cfg: &Config, scope: &str, ts: DateTime<Utc>) {
        let tree = get_or_create_source_tree(cfg, scope).unwrap();
        let provider: Arc<dyn ChatProvider> =
            Arc::new(StaticChatProvider::new("test summary content"));

        let c1 = Chunk {
            id: chunk_id(SourceKind::Chat, scope, 0, "test-content"),
            content: format!("c1-{scope}"),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: scope.into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://x")),
            },
            token_count: 30_000,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        let c2 = Chunk {
            id: chunk_id(SourceKind::Chat, scope, 1, "test-content"),
            content: format!("c2-{scope}"),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: scope.into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://y")),
            },
            token_count: 30_000,
            seq_in_source: 1,
            created_at: ts,
            partial_message: false,
        };

        upsert_chunks(cfg, &[c1.clone(), c2.clone()]).unwrap();
        stage_test_chunks(cfg, &[c1.clone(), c2.clone()]);

        let leaf1 = LeafRef {
            chunk_id: c1.id.clone(),
            token_count: 30_000,
            timestamp: ts,
            content: c1.content.clone(),
            entities: vec![],
            topics: vec![],
            score: 0.5,
        };
        let leaf2 = LeafRef {
            chunk_id: c2.id.clone(),
            token_count: 30_000,
            timestamp: ts,
            content: c2.content.clone(),
            entities: vec![],
            topics: vec![],
            score: 0.5,
        };

        test_override::with_provider(Arc::clone(&provider), async {
            append_leaf(cfg, &tree, &leaf1, &LabelStrategy::Empty)
                .await
                .unwrap();
            append_leaf(cfg, &tree, &leaf2, &LabelStrategy::Empty)
                .await
                .unwrap();
        })
        .await;
    }

    // ---------------------------------------------------------------------------
    // Tests
    // ---------------------------------------------------------------------------

    /// When there are no source trees at all, `end_of_day_digest` must return
    /// `EmptyDay` without writing any rows.
    #[tokio::test]
    async fn empty_day_with_no_source_trees() {
        let (_tmp, cfg) = test_config();
        let provider: Arc<dyn ChatProvider> =
            Arc::new(StaticChatProvider::new("test summary content"));
        let day = Utc::now().date_naive();

        let outcome = test_override::with_provider(Arc::clone(&provider), async {
            end_of_day_digest(&cfg, day).await.unwrap()
        })
        .await;

        assert!(
            matches!(outcome, DigestOutcome::EmptyDay),
            "expected EmptyDay when no source trees exist, got {outcome:?}"
        );
    }

    /// A source tree exists but has no sealed summaries covering the target
    /// day.  A freshly-created tree whose L0 buffer has never crossed the
    /// token threshold has `root_id = None`, so `pick_source_contribution`
    /// returns `None` and the digest should be `EmptyDay`.
    #[tokio::test]
    async fn empty_day_no_contributions() {
        let (_tmp, cfg) = test_config();
        let provider: Arc<dyn ChatProvider> =
            Arc::new(StaticChatProvider::new("test summary content"));

        // Create the source tree but add no leaves — root_id stays None.
        get_or_create_source_tree(&cfg, "slack:#empty").unwrap();

        let day = Utc::now().date_naive();
        let outcome = test_override::with_provider(Arc::clone(&provider), async {
            end_of_day_digest(&cfg, day).await.unwrap()
        })
        .await;

        assert!(
            matches!(outcome, DigestOutcome::EmptyDay),
            "expected EmptyDay when source tree has no sealed summaries, got {outcome:?}"
        );
    }

    /// One source tree with sealed L1 material covering today should yield
    /// `DigestOutcome::Emitted` with `source_count == 1`.
    #[tokio::test]
    async fn emits_daily_node_from_single_source() {
        let (_tmp, cfg) = test_config();
        let provider: Arc<dyn ChatProvider> =
            Arc::new(StaticChatProvider::new("test summary content"));
        let day = Utc::now().date_naive();
        // Use a timestamp inside today so the intersecting-summary query hits.
        let ts = day.and_hms_opt(12, 0, 0).unwrap().and_utc();

        seed_source_l1(&cfg, "slack:#general", ts).await;

        let outcome = test_override::with_provider(Arc::clone(&provider), async {
            end_of_day_digest(&cfg, day).await.unwrap()
        })
        .await;

        match outcome {
            DigestOutcome::Emitted {
                source_count,
                daily_id,
                ..
            } => {
                assert_eq!(source_count, 1, "expected exactly one contributing source");
                assert!(!daily_id.is_empty(), "daily_id must be non-empty");
            }
            other => panic!("expected Emitted, got {other:?}"),
        }
    }

    /// Calling `end_of_day_digest` twice for the same calendar day must return
    /// `Skipped` on the second call and must NOT insert a second L0 node.
    #[tokio::test]
    async fn idempotent_skip_on_rerun() {
        let (_tmp, cfg) = test_config();
        let provider: Arc<dyn ChatProvider> =
            Arc::new(StaticChatProvider::new("test summary content"));
        let day = Utc::now().date_naive();
        let ts = day.and_hms_opt(9, 0, 0).unwrap().and_utc();

        seed_source_l1(&cfg, "slack:#idempotent", ts).await;

        // First call — should emit.
        let first = test_override::with_provider(Arc::clone(&provider), async {
            end_of_day_digest(&cfg, day).await.unwrap()
        })
        .await;
        assert!(
            matches!(first, DigestOutcome::Emitted { .. }),
            "first call must emit, got {first:?}"
        );

        // Second call — must skip.
        let second = test_override::with_provider(Arc::clone(&provider), async {
            end_of_day_digest(&cfg, day).await.unwrap()
        })
        .await;
        assert!(
            matches!(second, DigestOutcome::Skipped { .. }),
            "second call must be Skipped, got {second:?}"
        );
    }

    /// Two source trees with sealed L1 material covering today → `Emitted`
    /// with `source_count == 2`.
    #[tokio::test]
    async fn multiple_sources_contribute() {
        let (_tmp, cfg) = test_config();
        let provider: Arc<dyn ChatProvider> =
            Arc::new(StaticChatProvider::new("test summary content"));
        let day = Utc::now().date_naive();
        let ts = day.and_hms_opt(11, 0, 0).unwrap().and_utc();

        seed_source_l1(&cfg, "slack:#alpha", ts).await;
        seed_source_l1(&cfg, "slack:#beta", ts).await;

        let outcome = test_override::with_provider(Arc::clone(&provider), async {
            end_of_day_digest(&cfg, day).await.unwrap()
        })
        .await;

        match outcome {
            DigestOutcome::Emitted { source_count, .. } => {
                assert_eq!(
                    source_count, 2,
                    "both source trees must contribute; got source_count={source_count}"
                );
            }
            other => panic!("expected Emitted, got {other:?}"),
        }
    }
}
