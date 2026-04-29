//! Topic-tree curator — the hotness gate (#709 Phase 3c).
//!
//! On every ingest that touches an entity we bump cheap counters
//! (`mention_count_30d`, `last_seen_ms`, `ingests_since_check`). Every
//! [`TOPIC_RECHECK_EVERY`] bumps we run the full hotness recompute:
//!
//! 1. Refresh `distinct_sources` from `mem_tree_entity_index`.
//! 2. Compute [`hotness`](super::hotness::hotness).
//! 3. If hotness ≥ [`TOPIC_CREATION_THRESHOLD`] and no topic tree exists
//!    yet → create one and kick off [`backfill_topic_tree`].
//! 4. Reset `ingests_since_check` to 0.
//!
//! The function is idempotent: if a topic tree already exists for the
//! entity it's a no-op at the creation step. Spawning is single-shot —
//! re-crossing the threshold after an archive would require explicit
//! unarchival (not Phase 3c).

use anyhow::Result;
use chrono::Utc;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::source_tree::store as src_store;
use crate::openhuman::memory::tree::source_tree::summariser::Summariser;
use crate::openhuman::memory::tree::source_tree::types::{Tree, TreeKind};
use crate::openhuman::memory::tree::topic_tree::backfill::backfill_topic_tree;
use crate::openhuman::memory::tree::topic_tree::hotness::hotness_at;
use crate::openhuman::memory::tree::topic_tree::registry::get_or_create_topic_tree;
use crate::openhuman::memory::tree::topic_tree::store::{
    distinct_sources_for, get_or_fresh, upsert,
};
use crate::openhuman::memory::tree::topic_tree::types::{
    HotnessCounters, TOPIC_CREATION_THRESHOLD, TOPIC_RECHECK_EVERY,
};

/// Outcome of one curator invocation. Surfaced so the caller (typically
/// the routing layer) can log / emit metrics.
#[derive(Clone, Debug, PartialEq)]
pub enum SpawnOutcome {
    /// Counters bumped; hotness not yet recomputed this round.
    CountersBumped,
    /// Full recompute ran; hotness below threshold, no tree spawned.
    BelowThreshold { hotness: f32 },
    /// Tree already existed — just bumped counters and refreshed hotness.
    TreeExists { hotness: f32, tree_id: String },
    /// Brand new topic tree materialised.
    Spawned {
        hotness: f32,
        tree_id: String,
        backfilled: usize,
    },
}

/// Record an ingest touching `entity_id` and, when the recheck cadence
/// fires, consider spawning a topic tree.
///
/// `summariser` is used only when a spawn + backfill happens; passing an
/// [`InertSummariser`](crate::openhuman::memory::tree::source_tree::summariser::inert::InertSummariser)
/// is fine for Phase 3c.
pub async fn maybe_spawn_topic_tree(
    config: &Config,
    entity_id: &str,
    summariser: &dyn Summariser,
) -> Result<SpawnOutcome> {
    let now_ms = Utc::now().timestamp_millis();

    // 1. Read existing counters (fresh row if first sighting).
    let mut counters = get_or_fresh(config, entity_id)?;

    // 2. Cheap per-ingest bumps.
    counters.mention_count_30d = counters.mention_count_30d.saturating_add(1);
    counters.last_seen_ms = Some(now_ms);
    counters.ingests_since_check = counters.ingests_since_check.saturating_add(1);
    counters.last_updated_ms = now_ms;

    // 3. Decide whether to run the full recompute.
    if counters.ingests_since_check < TOPIC_RECHECK_EVERY {
        upsert(config, &counters)?;
        log::debug!(
            "[topic_tree::curator] bumped counters entity={} mentions={} ingests_since_check={}",
            entity_id,
            counters.mention_count_30d,
            counters.ingests_since_check
        );
        return Ok(SpawnOutcome::CountersBumped);
    }

    // 4. Full recompute.
    run_full_recompute(config, entity_id, &mut counters, now_ms, summariser).await
}

/// Admin path: force a recompute + spawn-if-hot regardless of the
/// [`TOPIC_RECHECK_EVERY`] cadence. Used by (future) RPCs that want to
/// prod the curator without waiting for the next bump cycle.
pub async fn force_recompute(
    config: &Config,
    entity_id: &str,
    summariser: &dyn Summariser,
) -> Result<SpawnOutcome> {
    let now_ms = Utc::now().timestamp_millis();
    let mut counters = get_or_fresh(config, entity_id)?;
    counters.last_updated_ms = now_ms;
    run_full_recompute(config, entity_id, &mut counters, now_ms, summariser).await
}

async fn run_full_recompute(
    config: &Config,
    entity_id: &str,
    counters: &mut HotnessCounters,
    now_ms: i64,
    summariser: &dyn Summariser,
) -> Result<SpawnOutcome> {
    // Refresh distinct_sources from the entity index — the authoritative
    // source of cross-tree coverage.
    let distinct = distinct_sources_for(config, entity_id)?;
    counters.distinct_sources = distinct;

    // Compute hotness against the refreshed stats.
    let stats = counters.stats();
    let h = hotness_at(entity_id, &stats, now_ms);

    counters.last_hotness = Some(h);
    counters.ingests_since_check = 0;

    let outcome = if h < TOPIC_CREATION_THRESHOLD {
        log::debug!(
            "[topic_tree::curator] below threshold entity={} hotness={:.3} threshold={}",
            entity_id,
            h,
            TOPIC_CREATION_THRESHOLD
        );
        SpawnOutcome::BelowThreshold { hotness: h }
    } else if let Some(existing) = existing_topic_tree(config, entity_id)? {
        log::debug!(
            "[topic_tree::curator] tree already exists entity={} tree_id={} hotness={:.3}",
            entity_id,
            existing.id,
            h
        );
        SpawnOutcome::TreeExists {
            hotness: h,
            tree_id: existing.id,
        }
    } else {
        // Crossed threshold for the first time — materialise.
        log::info!(
            "[topic_tree::curator] spawning topic tree entity={} hotness={:.3}",
            entity_id,
            h
        );
        let tree = get_or_create_topic_tree(config, entity_id)?;
        let backfilled = backfill_topic_tree(config, &tree, entity_id, summariser).await?;
        SpawnOutcome::Spawned {
            hotness: h,
            tree_id: tree.id,
            backfilled,
        }
    };

    // Persist the refreshed counters regardless of outcome.
    upsert(config, counters)?;
    Ok(outcome)
}

fn existing_topic_tree(config: &Config, entity_id: &str) -> Result<Option<Tree>> {
    src_store::get_tree_by_scope(config, TreeKind::Topic, entity_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::score::extract::EntityKind;
    use crate::openhuman::memory::tree::score::resolver::CanonicalEntity;
    use crate::openhuman::memory::tree::score::store::index_entity;
    use crate::openhuman::memory::tree::source_tree::summariser::inert::InertSummariser;
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::topic_tree::store::get;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    fn seed_leaf_for_entity(cfg: &Config, entity_id: &str, source_tree: &str, seq: u32) {
        // Use a "now-anchored" timestamp so backfill's 30-day window
        // (see topic_tree::backfill::BACKFILL_WINDOW_DAYS) always
        // includes these seeded leaves. Spread by seq to keep ordering
        // deterministic.
        let ts_ms = Utc::now().timestamp_millis() - (seq as i64) * 1_000;
        let ts = Utc.timestamp_millis_opt(ts_ms).unwrap();
        let c = Chunk {
            id: chunk_id(SourceKind::Chat, source_tree, seq, "test-content"),
            content: format!("mentioning entity in {source_tree}#{seq}"),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: source_tree.to_string(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new(format!("{source_tree}://{seq}"))),
            },
            token_count: 50,
            seq_in_source: seq,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(cfg, &[c.clone()]).unwrap();
        let e = CanonicalEntity {
            canonical_id: entity_id.to_string(),
            kind: EntityKind::Email,
            surface: entity_id.to_string(),
            span_start: 0,
            span_end: entity_id.len() as u32,
            score: 1.0,
        };
        index_entity(cfg, &e, &c.id, "leaf", ts_ms, Some(source_tree)).unwrap();
    }

    #[tokio::test]
    async fn first_ingest_just_bumps_counters() {
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        let out = maybe_spawn_topic_tree(&cfg, "email:alice@example.com", &summariser)
            .await
            .unwrap();
        assert_eq!(out, SpawnOutcome::CountersBumped);
        let c = get(&cfg, "email:alice@example.com").unwrap().unwrap();
        assert_eq!(c.mention_count_30d, 1);
        assert_eq!(c.ingests_since_check, 1);
        assert!(c.last_hotness.is_none(), "no recompute yet");
    }

    #[tokio::test]
    async fn no_spawn_below_threshold_on_recompute() {
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        // Force a recompute on the very first call — but with no index data
        // the hotness comes out well below threshold.
        let out = force_recompute(&cfg, "email:alice@example.com", &summariser)
            .await
            .unwrap();
        match out {
            SpawnOutcome::BelowThreshold { hotness } => {
                assert!(hotness < TOPIC_CREATION_THRESHOLD);
            }
            other => panic!("expected BelowThreshold, got {other:?}"),
        }
        // No topic tree created.
        let t = existing_topic_tree(&cfg, "email:alice@example.com").unwrap();
        assert!(t.is_none());
    }

    #[tokio::test]
    async fn spawn_fires_exactly_once_when_threshold_crossed() {
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        // Seed substantial activity across several sources so hotness is
        // well above threshold.
        let mut counters = HotnessCounters::fresh("email:alice@example.com", 0);
        counters.mention_count_30d = 500;
        counters.distinct_sources = 4;
        counters.last_seen_ms = Some(Utc::now().timestamp_millis());
        counters.query_hits_30d = 5;
        upsert(&cfg, &counters).unwrap();
        // Seed leaves in the entity index so backfill has something to do.
        for i in 0..3 {
            seed_leaf_for_entity(&cfg, "email:alice@example.com", "slack:#eng", i);
        }
        for i in 0..2 {
            seed_leaf_for_entity(&cfg, "email:alice@example.com", "gmail:alice", i);
        }

        let out = force_recompute(&cfg, "email:alice@example.com", &summariser)
            .await
            .unwrap();
        match out {
            SpawnOutcome::Spawned {
                hotness,
                tree_id,
                backfilled,
            } => {
                assert!(hotness >= TOPIC_CREATION_THRESHOLD);
                assert!(tree_id.starts_with("topic:"));
                assert_eq!(backfilled, 5);
            }
            other => panic!("expected Spawned, got {other:?}"),
        }

        // Re-running should report TreeExists, NOT a second spawn.
        let out2 = force_recompute(&cfg, "email:alice@example.com", &summariser)
            .await
            .unwrap();
        match out2 {
            SpawnOutcome::TreeExists { tree_id, .. } => {
                assert!(tree_id.starts_with("topic:"));
            }
            other => panic!("expected TreeExists on retry, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn recompute_refreshes_distinct_sources_from_entity_index() {
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        // Counter says 0 distinct sources but the index has 3.
        let mut counters = HotnessCounters::fresh("email:alice@example.com", 0);
        counters.mention_count_30d = 1;
        counters.distinct_sources = 0;
        upsert(&cfg, &counters).unwrap();
        seed_leaf_for_entity(&cfg, "email:alice@example.com", "slack:#eng", 0);
        seed_leaf_for_entity(&cfg, "email:alice@example.com", "gmail:alice", 0);
        seed_leaf_for_entity(&cfg, "email:alice@example.com", "notion:abc", 0);

        force_recompute(&cfg, "email:alice@example.com", &summariser)
            .await
            .unwrap();
        let c = get(&cfg, "email:alice@example.com").unwrap().unwrap();
        assert_eq!(c.distinct_sources, 3);
        // ingests_since_check should also reset.
        assert_eq!(c.ingests_since_check, 0);
    }

    #[tokio::test]
    async fn cadence_only_recomputes_every_n_ingests() {
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        // Pre-seed entity index with enough cross-source signal that the
        // recompute (which refreshes `distinct_sources` from the index) will
        // still produce a hotness above threshold.
        for i in 0..4 {
            seed_leaf_for_entity(
                &cfg,
                "email:alice@example.com",
                &format!("slack:#eng-{i}"),
                i,
            );
        }

        let mut counters = HotnessCounters::fresh("email:alice@example.com", 0);
        counters.mention_count_30d = 500;
        counters.distinct_sources = 4;
        counters.last_seen_ms = Some(Utc::now().timestamp_millis());
        // Boost query_hits so hotness stays comfortably above threshold
        // after the distinct_sources refresh.
        counters.query_hits_30d = 5;
        // ingests_since_check just below the cadence: next call should
        // NOT yet recompute.
        counters.ingests_since_check = TOPIC_RECHECK_EVERY - 2;
        upsert(&cfg, &counters).unwrap();

        let out = maybe_spawn_topic_tree(&cfg, "email:alice@example.com", &summariser)
            .await
            .unwrap();
        assert_eq!(out, SpawnOutcome::CountersBumped);
        // No tree yet — cadence not crossed.
        assert!(existing_topic_tree(&cfg, "email:alice@example.com")
            .unwrap()
            .is_none());

        // One more bump — now ingests_since_check == TOPIC_RECHECK_EVERY
        // and the recompute fires.
        let out2 = maybe_spawn_topic_tree(&cfg, "email:alice@example.com", &summariser)
            .await
            .unwrap();
        match out2 {
            SpawnOutcome::Spawned { .. } | SpawnOutcome::TreeExists { .. } => {}
            other => panic!("expected Spawn/TreeExists after cadence, got {other:?}"),
        }
    }
}
