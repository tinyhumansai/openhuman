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

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use rusqlite::OptionalExtension;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::global_tree::registry::get_or_create_global_tree;
use crate::openhuman::memory::tree::global_tree::seal::append_daily_and_cascade;
use crate::openhuman::memory::tree::global_tree::GLOBAL_TOKEN_BUDGET;
use crate::openhuman::memory::tree::score::embed::build_embedder_from_config;
use crate::openhuman::memory::tree::source_tree::registry::new_summary_id;
use crate::openhuman::memory::tree::source_tree::store;
use crate::openhuman::memory::tree::source_tree::summariser::{
    Summariser, SummaryContext, SummaryInput,
};
use crate::openhuman::memory::tree::source_tree::types::{SummaryNode, Tree, TreeKind};
use crate::openhuman::memory::tree::store::with_connection;

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
/// summariser is called once to fold the per-source material into a single
/// cross-source recap.
///
/// `day` is the calendar date in UTC the digest should cover. Callers that
/// simply want "yesterday" can pass `Utc::now().date_naive() - Duration::days(1)`.
pub async fn end_of_day_digest(
    config: &Config,
    day: NaiveDate,
    summariser: &dyn Summariser,
) -> Result<DigestOutcome> {
    let (day_start, day_end) = day_bounds_utc(day)?;
    log::info!(
        "[global_tree::digest] end_of_day_digest day={} window=[{}, {})",
        day,
        day_start,
        day_end
    );

    let global = get_or_create_global_tree(config)?;

    // Idempotency: check for an existing L0 daily node whose time range
    // matches this day.
    if let Some(existing) = find_existing_daily(config, &global.id, day_start, day_end)? {
        log::info!(
            "[global_tree::digest] daily already exists for {day} id={} — skipping",
            existing.id
        );
        return Ok(DigestOutcome::Skipped {
            existing_id: existing.id,
        });
    }

    // Gather one contribution per active source tree.
    let source_trees = store::list_trees_by_kind(config, TreeKind::Source)?;
    log::debug!(
        "[global_tree::digest] scanning {} source trees",
        source_trees.len()
    );
    let mut inputs: Vec<SummaryInput> = Vec::with_capacity(source_trees.len());
    for source_tree in &source_trees {
        match pick_source_contribution(config, source_tree, day_start, day_end)? {
            Some(inp) => {
                log::debug!(
                    "[global_tree::digest] source={} contributed id={} tokens={}",
                    source_tree.scope,
                    inp.id,
                    inp.token_count
                );
                inputs.push(inp);
            }
            None => {
                log::debug!(
                    "[global_tree::digest] source={} had no material for {day}",
                    source_tree.scope
                );
            }
        }
    }

    if inputs.is_empty() {
        log::info!(
            "[global_tree::digest] empty day — no source trees contributed material for {day}"
        );
        return Ok(DigestOutcome::EmptyDay);
    }

    // Fold cross-source material into one daily recap.
    let ctx = SummaryContext {
        tree_id: &global.id,
        tree_kind: TreeKind::Global,
        target_level: 0, // daily node lives at L0 on the global tree
        token_budget: GLOBAL_TOKEN_BUDGET,
    };
    let output = summariser
        .summarise(&inputs, &ctx)
        .await
        .context("summariser failed during end-of-day digest")?;

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
        entities: output.entities,
        topics: output.topics,
        time_range_start: day_start,
        time_range_end: day_end,
        score,
        sealed_at: now,
        deleted: false,
        embedding: Some(embedding),
    };

    // Persist the daily node. Note: we do NOT backlink parent_id on the
    // child summaries here — their parents are their own source trees, not
    // the global tree. The global-tree child_ids are cross-source
    // *references*, not ownership.
    let daily_clone = daily.clone();
    let tree_id_clone = global.id.clone();
    with_connection(config, move |conn| {
        let tx = conn.unchecked_transaction()?;
        store::insert_summary_tx(&tx, &daily_clone)?;
        // Index any entities the summariser emitted (no-op under inert).
        crate::openhuman::memory::tree::score::store::index_summary_entity_ids_tx(
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
        "[global_tree::digest] emitted daily id={} sources={} tokens={}",
        daily.id,
        inputs.len(),
        daily.token_count
    );

    // Append into L0 buffer + cascade-seal if thresholds crossed.
    let sealed_ids = append_daily_and_cascade(config, &global, &daily, summariser).await?;

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
                "[global_tree::digest] picked id={id} for tree={} but row missing — skipping",
                source_tree.scope
            );
            return Ok(None);
        }
    };

    Ok(Some(SummaryInput {
        id: node.id,
        content: format!("[{}]\n{}", source_tree.scope, node.content),
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
    use crate::openhuman::memory::tree::source_tree::bucket_seal::{append_leaf, LeafRef};
    use crate::openhuman::memory::tree::source_tree::registry::get_or_create_source_tree;
    use crate::openhuman::memory::tree::source_tree::summariser::inert::InertSummariser;
    use crate::openhuman::memory::tree::source_tree::types::TreeStatus;
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Phase 4 (#710): digest embeds before committing — inert in tests.
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    async fn seed_source_tree_with_sealed_l1(cfg: &Config, scope: &str, ts: DateTime<Utc>) {
        // Use chunks with the source_tree bucket-seal mechanics so we get a
        // real L1 summary persisted that intersects the target day.
        let tree = get_or_create_source_tree(cfg, scope).unwrap();
        let summariser = InertSummariser::new();

        let c1 = Chunk {
            id: chunk_id(SourceKind::Chat, scope, 0, "test-content"),
            content: format!("chunk 1 in {scope}"),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: scope.into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://x")),
            },
            token_count: 6_000,
            seq_in_source: 0,
            created_at: ts,
        };
        let c2 = Chunk {
            id: chunk_id(SourceKind::Chat, scope, 1, "test-content"),
            content: format!("chunk 2 in {scope}"),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: scope.into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://y")),
            },
            token_count: 6_000,
            seq_in_source: 1,
            created_at: ts,
        };
        upsert_chunks(cfg, &[c1.clone(), c2.clone()]).unwrap();

        let leaf1 = LeafRef {
            chunk_id: c1.id.clone(),
            token_count: 6_000,
            timestamp: ts,
            content: c1.content.clone(),
            entities: vec![],
            topics: vec![],
            score: 0.5,
        };
        let leaf2 = LeafRef {
            chunk_id: c2.id.clone(),
            token_count: 6_000,
            timestamp: ts,
            content: c2.content.clone(),
            entities: vec![],
            topics: vec![],
            score: 0.5,
        };
        append_leaf(cfg, &tree, &leaf1, &summariser).await.unwrap();
        append_leaf(cfg, &tree, &leaf2, &summariser).await.unwrap();
        // 12k tokens > 10k budget → one L1 summary covering `ts`.
    }

    #[tokio::test]
    async fn empty_day_returns_empty_day_outcome() {
        // No source trees exist yet — digest should no-op.
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        let day = NaiveDate::from_ymd_opt(2025, 1, 15).unwrap();
        let outcome = end_of_day_digest(&cfg, day, &summariser).await.unwrap();
        assert!(matches!(outcome, DigestOutcome::EmptyDay));

        // No L0 nodes emitted on the global tree.
        let global = get_or_create_global_tree(&cfg).unwrap();
        assert_eq!(store::count_summaries(&cfg, &global.id).unwrap(), 0);
    }

    #[tokio::test]
    async fn populated_day_emits_one_daily_leaf() {
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();

        // Seed 3 source trees with sealed L1s on the target day. This
        // exercises the main cross-source path end to end.
        let day = NaiveDate::from_ymd_opt(2025, 1, 15).unwrap();
        let ts = day.and_hms_opt(12, 0, 0).unwrap().and_utc();
        seed_source_tree_with_sealed_l1(&cfg, "slack:#eng", ts).await;
        seed_source_tree_with_sealed_l1(&cfg, "email:alice", ts).await;
        seed_source_tree_with_sealed_l1(&cfg, "notion:workspace", ts).await;

        let outcome = end_of_day_digest(&cfg, day, &summariser).await.unwrap();
        let (daily_id, source_count) = match outcome {
            DigestOutcome::Emitted {
                daily_id,
                source_count,
                sealed_ids,
            } => {
                assert!(sealed_ids.is_empty(), "one day ≠ weekly seal yet");
                (daily_id, source_count)
            }
            other => panic!("expected Emitted, got {other:?}"),
        };
        assert_eq!(source_count, 3);

        let global = get_or_create_global_tree(&cfg).unwrap();
        // Exactly one L0 daily node on the global tree.
        let daily_nodes = store::list_summaries_at_level(&cfg, &global.id, 0).unwrap();
        assert_eq!(daily_nodes.len(), 1);
        assert_eq!(daily_nodes[0].id, daily_id);
        assert_eq!(daily_nodes[0].tree_kind, TreeKind::Global);

        // Time range matches the target day exactly.
        let (expected_start, expected_end) = day_bounds_utc(day).unwrap();
        assert_eq!(daily_nodes[0].time_range_start, expected_start);
        assert_eq!(daily_nodes[0].time_range_end, expected_end);
        assert_eq!(daily_nodes[0].child_ids.len(), 3);

        // L0 buffer now carries this daily id (≠ empty).
        let l0 = store::get_buffer(&cfg, &global.id, 0).unwrap();
        assert_eq!(l0.item_ids, vec![daily_id]);
    }

    #[tokio::test]
    async fn rerun_on_same_day_is_idempotent() {
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        let day = NaiveDate::from_ymd_opt(2025, 2, 3).unwrap();
        let ts = day.and_hms_opt(9, 0, 0).unwrap().and_utc();
        seed_source_tree_with_sealed_l1(&cfg, "slack:#eng", ts).await;

        let first = end_of_day_digest(&cfg, day, &summariser).await.unwrap();
        let first_id = match first {
            DigestOutcome::Emitted { daily_id, .. } => daily_id,
            other => panic!("expected Emitted, got {other:?}"),
        };

        let second = end_of_day_digest(&cfg, day, &summariser).await.unwrap();
        match second {
            DigestOutcome::Skipped { existing_id } => assert_eq!(existing_id, first_id),
            other => panic!("expected Skipped on rerun, got {other:?}"),
        }

        let global = get_or_create_global_tree(&cfg).unwrap();
        let daily_nodes = store::list_summaries_at_level(&cfg, &global.id, 0).unwrap();
        assert_eq!(daily_nodes.len(), 1, "rerun must not duplicate daily node");
    }

    #[tokio::test]
    async fn seven_days_cascade_to_weekly_seal() {
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();

        // Emit 7 daily nodes across 7 consecutive days. The 7th should
        // cascade to seal an L1 weekly node.
        let base = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
        let mut emitted_days = 0;
        for i in 0..7 {
            let day = base + Duration::days(i);
            let ts = day.and_hms_opt(10, 0, 0).unwrap().and_utc();
            // Fresh source scope per day keeps L1s day-specific.
            seed_source_tree_with_sealed_l1(&cfg, &format!("slack:#day{i}"), ts).await;

            let outcome = end_of_day_digest(&cfg, day, &summariser).await.unwrap();
            match outcome {
                DigestOutcome::Emitted {
                    sealed_ids,
                    source_count: _,
                    ..
                } => {
                    emitted_days += 1;
                    if emitted_days < 7 {
                        assert!(
                            sealed_ids.is_empty(),
                            "no weekly seal until 7 daily nodes accumulate"
                        );
                    } else {
                        assert_eq!(sealed_ids.len(), 1, "weekly seal should fire on day 7");
                    }
                }
                other => panic!("expected Emitted on day {i}, got {other:?}"),
            }
        }
        assert_eq!(emitted_days, 7);

        let global = get_or_create_global_tree(&cfg).unwrap();
        let l0 = store::get_buffer(&cfg, &global.id, 0).unwrap();
        assert!(l0.is_empty(), "L0 buffer cleared after weekly seal");
        let l1 = store::get_buffer(&cfg, &global.id, 1).unwrap();
        assert_eq!(l1.item_ids.len(), 1, "one weekly node parked at L1");

        let weekly = store::get_summary(&cfg, &l1.item_ids[0]).unwrap().unwrap();
        assert_eq!(weekly.level, 1);
        assert_eq!(weekly.child_ids.len(), 7);

        let t = store::get_tree(&cfg, &global.id).unwrap().unwrap();
        assert_eq!(t.max_level, 1);
        assert_eq!(t.status, TreeStatus::Active);
    }
}
