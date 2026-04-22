//! Per-leaf routing into topic trees (#709 Phase 3c).
//!
//! This is the hook point the ingest path calls after it has finished
//! appending a leaf to its source tree. For each canonical entity on the
//! chunk we:
//!
//! 1. Append the leaf to that entity's topic tree *if* one already exists
//!    (active status only — archived topic trees don't receive new
//!    leaves).
//! 2. Notify the curator that this entity was just mentioned, which may
//!    cross the hotness threshold and spawn a new topic tree.
//!
//! Steps 1 and 2 are independent — if an entity's topic tree already
//! exists, step 2 just bumps counters; if it doesn't, step 1 is skipped
//! and step 2 may materialise it on this ingest.
//!
//! Failures are logged at warn level but never bubble up: Phase 3c is
//! additive and must not poison the ingest path. The source-tree append
//! has already succeeded by the time we get here.

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::source_tree::bucket_seal::{append_leaf, LeafRef};
use crate::openhuman::memory::tree::source_tree::store as src_store;
use crate::openhuman::memory::tree::source_tree::summariser::Summariser;
use crate::openhuman::memory::tree::source_tree::types::{TreeKind, TreeStatus};
use crate::openhuman::memory::tree::topic_tree::curator::maybe_spawn_topic_tree;

/// Route `leaf` into every active topic tree matching one of
/// `canonical_entities`. Also ticks the curator for each entity so the
/// next cadence-aligned ingest may spawn a new tree.
///
/// Returns `Ok(())` even if individual entities fail — per-entity errors
/// are logged. A hard DB failure early in the process is surfaced so the
/// caller can decide how loud to be in logs.
pub async fn route_leaf_to_topic_trees(
    config: &Config,
    leaf: &LeafRef,
    canonical_entities: &[String],
    summariser: &dyn Summariser,
) -> Result<()> {
    if canonical_entities.is_empty() {
        return Ok(());
    }

    log::debug!(
        "[topic_tree::routing] leaf={} entities={}",
        leaf.chunk_id,
        canonical_entities.len()
    );

    for entity_id in canonical_entities {
        if let Err(e) = route_one_entity(config, leaf, entity_id, summariser).await {
            log::warn!(
                "[topic_tree::routing] failed routing leaf={} entity={} err={:#}",
                leaf.chunk_id,
                entity_id,
                e
            );
        }
    }
    Ok(())
}

async fn route_one_entity(
    config: &Config,
    leaf: &LeafRef,
    entity_id: &str,
    summariser: &dyn Summariser,
) -> Result<()> {
    // Step 1: if a topic tree already exists and is active, append the leaf.
    // We intentionally do this BEFORE asking the curator to spawn — a
    // same-call spawn would also include this leaf via backfill
    // (`lookup_entity` was just updated by the ingest's score persist) but
    // keeping the existing-tree fast path separate keeps the common case
    // (hot entity already has a tree) clean.
    if let Some(tree) = src_store::get_tree_by_scope(config, TreeKind::Topic, entity_id)? {
        if tree.status == TreeStatus::Active {
            log::debug!(
                "[topic_tree::routing] appending leaf={} → topic_tree={}",
                leaf.chunk_id,
                tree.id
            );
            // Rebuild the leaf with this entity-id stamped on so the seal
            // path sees the topic membership. The source-tree append used
            // the full entity list; here we scope to just this entity so
            // the curated summariser (future) can prompt accordingly.
            let topic_leaf = LeafRef {
                entities: vec![entity_id.to_string()],
                ..leaf.clone()
            };
            append_leaf(config, &tree, &topic_leaf, summariser).await?;
        } else {
            log::debug!(
                "[topic_tree::routing] skip archived topic tree id={} entity={}",
                tree.id,
                entity_id
            );
        }
    }

    // Step 2: curator tick — may spawn a new tree on cadence.
    maybe_spawn_topic_tree(config, entity_id, summariser).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::score::extract::EntityKind;
    use crate::openhuman::memory::tree::score::resolver::CanonicalEntity;
    use crate::openhuman::memory::tree::score::store::index_entity;
    use crate::openhuman::memory::tree::source_tree::summariser::inert::InertSummariser;
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::topic_tree::registry::{
        archive_topic_tree, get_or_create_topic_tree,
    };
    use crate::openhuman::memory::tree::topic_tree::store::get as get_hotness;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    fn mk_leaf(chunk_id_s: &str, tokens: u32, ts_ms: i64) -> LeafRef {
        LeafRef {
            chunk_id: chunk_id_s.to_string(),
            token_count: tokens,
            timestamp: Utc.timestamp_millis_opt(ts_ms).unwrap(),
            content: format!("content for {chunk_id_s}"),
            entities: vec!["email:alice@example.com".into()],
            topics: vec![],
            score: 0.5,
        }
    }

    fn persist_chunk(cfg: &Config, source_id: &str, seq: u32, ts_ms: i64, tokens: u32) -> String {
        let ts = Utc.timestamp_millis_opt(ts_ms).unwrap();
        let c = Chunk {
            id: chunk_id(SourceKind::Chat, source_id, seq),
            content: format!("chunk content {source_id} {seq}"),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: source_id.to_string(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new(format!("{source_id}://{seq}"))),
            },
            token_count: tokens,
            seq_in_source: seq,
            created_at: ts,
        };
        let id = c.id.clone();
        upsert_chunks(cfg, &[c]).unwrap();
        id
    }

    #[tokio::test]
    async fn empty_entities_is_noop() {
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        let leaf = mk_leaf("c1", 10, 1_700_000_000_000);
        route_leaf_to_topic_trees(&cfg, &leaf, &[], &summariser)
            .await
            .unwrap();
        // No hotness rows were created.
        assert_eq!(
            crate::openhuman::memory::tree::topic_tree::store::count(&cfg).unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn appends_to_existing_topic_tree() {
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        // Pre-create the topic tree so the hot-path append fires.
        let tree = get_or_create_topic_tree(&cfg, "email:alice@example.com").unwrap();
        // Persist the backing chunk so hydrate can read it on seal.
        let chunk_id_s = persist_chunk(&cfg, "slack:#eng", 0, 1_700_000_000_000, 100);
        let leaf = mk_leaf(&chunk_id_s, 100, 1_700_000_000_000);

        route_leaf_to_topic_trees(
            &cfg,
            &leaf,
            &["email:alice@example.com".to_string()],
            &summariser,
        )
        .await
        .unwrap();

        let buf = src_store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert_eq!(buf.item_ids.len(), 1);
        assert_eq!(buf.item_ids[0], chunk_id_s);
        // Counter should also be bumped.
        let c = get_hotness(&cfg, "email:alice@example.com")
            .unwrap()
            .unwrap();
        assert_eq!(c.mention_count_30d, 1);
    }

    #[tokio::test]
    async fn archived_topic_tree_does_not_receive_new_leaves() {
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        let tree = get_or_create_topic_tree(&cfg, "email:alice@example.com").unwrap();
        archive_topic_tree(&cfg, &tree.id).unwrap();

        let chunk_id_s = persist_chunk(&cfg, "slack:#eng", 0, 1_700_000_000_000, 100);
        let leaf = mk_leaf(&chunk_id_s, 100, 1_700_000_000_000);
        route_leaf_to_topic_trees(
            &cfg,
            &leaf,
            &["email:alice@example.com".to_string()],
            &summariser,
        )
        .await
        .unwrap();

        let buf = src_store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert!(
            buf.is_empty(),
            "archived topic tree should not receive new leaves"
        );
        // Counter should still be bumped — archiving doesn't freeze hotness.
        let c = get_hotness(&cfg, "email:alice@example.com")
            .unwrap()
            .unwrap();
        assert_eq!(c.mention_count_30d, 1);
    }

    #[tokio::test]
    async fn one_leaf_multiple_entities_fans_out() {
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        let t1 = get_or_create_topic_tree(&cfg, "email:alice@example.com").unwrap();
        let t2 = get_or_create_topic_tree(&cfg, "hashtag:launch").unwrap();

        let chunk_id_s = persist_chunk(&cfg, "slack:#eng", 0, 1_700_000_000_000, 100);
        let leaf = mk_leaf(&chunk_id_s, 100, 1_700_000_000_000);
        route_leaf_to_topic_trees(
            &cfg,
            &leaf,
            &[
                "email:alice@example.com".to_string(),
                "hashtag:launch".to_string(),
            ],
            &summariser,
        )
        .await
        .unwrap();

        // Both topic trees' L0 buffers hold the leaf.
        let b1 = src_store::get_buffer(&cfg, &t1.id, 0).unwrap();
        let b2 = src_store::get_buffer(&cfg, &t2.id, 0).unwrap();
        assert_eq!(b1.item_ids.len(), 1);
        assert_eq!(b2.item_ids.len(), 1);
    }

    #[tokio::test]
    async fn integration_two_sources_mentioning_alice_materialise_topic_tree() {
        // Phase 3c acceptance scenario: ingest across 2 sources mentioning
        // Alice → hotness crosses threshold → topic tree materialised →
        // new Alice-mentioning leaf routes into both the source tree AND
        // the topic tree.
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        let entity_id = "email:alice@example.com";

        // Pre-seed counters / index so the next call crosses threshold.
        // Note: the curator refreshes `distinct_sources` from the entity
        // index during recompute, so we also need enough `query_hits_30d`
        // to keep hotness above `TOPIC_CREATION_THRESHOLD` once the index
        // is queried (two indexed sources below → distinct_sources → 2).
        let mut counters =
            crate::openhuman::memory::tree::topic_tree::types::HotnessCounters::fresh(entity_id, 0);
        counters.mention_count_30d = 1_000;
        counters.distinct_sources = 2;
        counters.last_seen_ms = Some(Utc::now().timestamp_millis());
        counters.query_hits_30d = 5;
        counters.ingests_since_check =
            crate::openhuman::memory::tree::topic_tree::types::TOPIC_RECHECK_EVERY - 1;
        crate::openhuman::memory::tree::topic_tree::store::upsert(&cfg, &counters).unwrap();

        // Seed a leaf in slack and gmail referencing Alice.
        let c1 = persist_chunk(&cfg, "slack:#eng", 0, 1_700_000_000_000, 100);
        let c2 = persist_chunk(&cfg, "gmail:alice", 0, 1_700_000_010_000, 100);
        let e = CanonicalEntity {
            canonical_id: entity_id.into(),
            kind: EntityKind::Email,
            surface: entity_id.into(),
            span_start: 0,
            span_end: entity_id.len() as u32,
            score: 1.0,
        };
        index_entity(&cfg, &e, &c1, "leaf", 1_700_000_000_000, Some("slack:#eng")).unwrap();
        index_entity(
            &cfg,
            &e,
            &c2,
            "leaf",
            1_700_000_010_000,
            Some("gmail:alice"),
        )
        .unwrap();

        // A third leaf arrives — should both fan out to (future) topic tree
        // and push the curator over the recheck cadence, materialising it.
        let c3 = persist_chunk(&cfg, "slack:#eng", 1, 1_700_000_020_000, 100);
        let leaf = LeafRef {
            chunk_id: c3.clone(),
            token_count: 100,
            timestamp: Utc.timestamp_millis_opt(1_700_000_020_000).unwrap(),
            content: "new mention".into(),
            entities: vec![entity_id.into()],
            topics: vec![],
            score: 0.5,
        };

        route_leaf_to_topic_trees(&cfg, &leaf, &[entity_id.to_string()], &summariser)
            .await
            .unwrap();

        // Topic tree now exists.
        let tree = src_store::get_tree_by_scope(&cfg, TreeKind::Topic, entity_id)
            .unwrap()
            .expect("topic tree should be materialised");
        assert_eq!(tree.kind, TreeKind::Topic);
        assert_eq!(tree.scope, entity_id);
        // Backfill pulled c1 + c2 into the buffer. (c3 didn't get into the
        // entity index during this test since we didn't run the full ingest
        // path — we're exercising routing in isolation.)
        let buf = src_store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert!(
            buf.item_ids.len() >= 2,
            "backfill should pull historic leaves"
        );
    }
}
