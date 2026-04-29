//! Topic-tree backfill — hydrate a freshly-materialised topic tree with
//! recent leaves mentioning the entity (#709 Phase 3c).
//!
//! When the curator decides an entity has crossed the hotness threshold
//! for the first time, we create a fresh topic tree AND walk the
//! `mem_tree_entity_index` inverted index to append matching leaves into
//! its L0 buffer. Reusing `bucket_seal::append_leaf` means the cascade
//! fires automatically.
//!
//! ## Why bounded by hotness window
//!
//! Hotness uses a 30-day recency decay (see `topic_tree::hotness`). Leaves
//! older than 30 days contribute zero to current hotness, so by definition
//! they cannot be the reason a tree is spawning *now*. Including them
//! bloats the spawn latency, wastes summariser LLM calls, and amplifies
//! ancient signal that has already decayed away. We cap the backfill
//! window at [`BACKFILL_WINDOW_DAYS`] to align with the hotness math.
//!
//! Older content is still queryable through source-tree retrieval and the
//! entity index — it just doesn't get its own slot in the topic tree.
//!
//! Backfill is intentionally best-effort: missing chunks are skipped with
//! a warn log rather than failing the whole spawn, because Phase 3c is
//! additive — a partial topic tree is still useful.

use anyhow::{Context, Result};
use chrono::Utc;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::score::store::lookup_entity;
use crate::openhuman::memory::tree::source_tree::bucket_seal::{
    append_leaf, LabelStrategy, LeafRef,
};
use crate::openhuman::memory::tree::source_tree::summariser::Summariser;
use crate::openhuman::memory::tree::source_tree::types::Tree;
use crate::openhuman::memory::tree::store::get_chunk;
use crate::openhuman::memory::tree::util::redact::redact;

/// Max leaves to pull from the entity index during backfill. A hard cap
/// keeps initial spawn latency bounded even for very active entities.
const BACKFILL_LIMIT: usize = 500;

/// Backfill window in days — matches `topic_tree::hotness::recency_decay`'s
/// hard cliff. Leaves older than this contribute zero to current hotness
/// so they cannot have driven the spawn decision.
pub const BACKFILL_WINDOW_DAYS: i64 = 30;

const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

/// Walk the entity index for `entity_id` and append every discovered leaf
/// to `tree`. Returns the number of leaves appended (NOT the number of
/// summaries sealed). Idempotent: `append_leaf` itself is a no-op when a
/// leaf is already in the buffer, so re-running backfill is safe.
pub async fn backfill_topic_tree(
    config: &Config,
    tree: &Tree,
    entity_id: &str,
    summariser: &dyn Summariser,
) -> Result<usize> {
    backfill_topic_tree_at(
        config,
        tree,
        entity_id,
        summariser,
        Utc::now().timestamp_millis(),
    )
    .await
}

/// Deterministic variant — backfill against a caller-supplied `now_ms`
/// for the recency window. Used by tests so the 30-day cutoff doesn't
/// depend on the wall clock.
pub async fn backfill_topic_tree_at(
    config: &Config,
    tree: &Tree,
    entity_id: &str,
    summariser: &dyn Summariser,
    now_ms: i64,
) -> Result<usize> {
    let cutoff_ms = now_ms.saturating_sub(BACKFILL_WINDOW_DAYS.saturating_mul(DAY_MS));
    log::info!(
        "[topic_tree::backfill] start entity_id_hash={} tree_id={} window_days={} cutoff_ms={}",
        redact(entity_id),
        tree.id,
        BACKFILL_WINDOW_DAYS,
        cutoff_ms
    );

    let hits = lookup_entity(config, entity_id, Some(BACKFILL_LIMIT))
        .with_context(|| format!("failed to lookup entity {}", redact(entity_id)))?;

    if hits.is_empty() {
        log::debug!(
            "[topic_tree::backfill] no entity-index hits for entity_id_hash={} — empty backfill",
            redact(entity_id)
        );
        return Ok(0);
    }

    // Drop hits older than the hotness recency window — see module docs.
    let total_hits = hits.len();
    let mut hits: Vec<_> = hits
        .into_iter()
        .filter(|h| h.timestamp_ms >= cutoff_ms)
        .collect();
    let dropped = total_hits - hits.len();
    if dropped > 0 {
        log::debug!(
            "[topic_tree::backfill] dropped {dropped} hits older than {BACKFILL_WINDOW_DAYS}d \
             for entity_id_hash={}",
            redact(entity_id)
        );
    }
    if hits.is_empty() {
        log::debug!(
            "[topic_tree::backfill] all entity-index hits fell outside the {BACKFILL_WINDOW_DAYS}d \
             window for entity_id_hash={} — empty backfill",
            redact(entity_id)
        );
        return Ok(0);
    }

    // Sort by timestamp ASC so the buffer's `oldest_at` and the sealed
    // summary's `time_range_start` reflect the true historical order, not
    // the DESC ordering `lookup_entity` returns.
    hits.sort_by_key(|h| h.timestamp_ms);

    let mut appended = 0usize;
    for hit in hits {
        // Skip summary-node hits — Phase 3c backfill only routes raw leaves
        // into the topic tree. Including summary nodes would fold
        // summaries-of-summaries across unrelated sources, which defeats
        // the point.
        if hit.node_kind != "leaf" {
            log::debug!(
                "[topic_tree::backfill] skipping non-leaf hit node_id={} kind={}",
                hit.node_id,
                hit.node_kind
            );
            continue;
        }

        let chunk = match get_chunk(config, &hit.node_id)? {
            Some(c) => c,
            None => {
                log::warn!(
                    "[topic_tree::backfill] missing chunk {} for entity_id_hash={} — skipping",
                    hit.node_id,
                    redact(entity_id)
                );
                continue;
            }
        };

        let leaf = LeafRef {
            chunk_id: chunk.id.clone(),
            token_count: chunk.token_count,
            timestamp: chunk.metadata.timestamp,
            content: chunk.content.clone(),
            entities: vec![entity_id.to_string()],
            topics: chunk.metadata.tags.clone(),
            score: hit.score,
        };

        // Topic-tree backfill: empty labels for sealed summaries — the
        // tree's scope already pins the canonical id, so cross-pollinating
        // descendants' entities would noise the index. See LabelStrategy.
        append_leaf(config, tree, &leaf, summariser, &LabelStrategy::Empty)
            .await
            .with_context(|| {
                format!(
                    "backfill append_leaf failed entity_id_hash={} tree_id={} chunk_id={}",
                    redact(entity_id),
                    tree.id,
                    chunk.id
                )
            })?;
        appended += 1;
    }

    log::info!(
        "[topic_tree::backfill] done entity_id_hash={} tree_id={} appended={}",
        redact(entity_id),
        tree.id,
        appended
    );

    Ok(appended)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::score::extract::EntityKind;
    use crate::openhuman::memory::tree::score::resolver::CanonicalEntity;
    use crate::openhuman::memory::tree::score::store::index_entity;
    use crate::openhuman::memory::tree::source_tree::store as src_store;
    use crate::openhuman::memory::tree::source_tree::summariser::inert::InertSummariser;
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::topic_tree::registry::get_or_create_topic_tree;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Phase 4 (#710): backfill may trigger seal cascades.
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    fn mk_chunk(source_id: &str, seq: u32, ts_ms: i64, tokens: u32) -> Chunk {
        let ts = Utc.timestamp_millis_opt(ts_ms).unwrap();
        Chunk {
            id: chunk_id(SourceKind::Chat, source_id, seq, "test-content"),
            content: format!("substantive chunk mentioning alice {source_id}#{seq}"),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: source_id.to_string(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec!["eng".into()],
                source_ref: Some(SourceRef::new(format!("{source_id}://{seq}"))),
            },
            token_count: tokens,
            seq_in_source: seq,
            created_at: ts,
            partial_message: false,
        }
    }

    fn sample_entity(canonical: &str, surface: &str) -> CanonicalEntity {
        CanonicalEntity {
            canonical_id: canonical.to_string(),
            kind: EntityKind::Email,
            surface: surface.to_string(),
            span_start: 0,
            span_end: surface.len() as u32,
            score: 1.0,
        }
    }

    /// Deterministic "now" used by the windowed-backfill tests: 1 hour
    /// after the latest seeded leaf so all three sit inside the 30-day
    /// cutoff. Lets us keep the legacy 2023-era timestamps unchanged.
    const TEST_NOW_MS: i64 = 1_700_000_020_000 + 3_600_000;

    #[tokio::test]
    async fn backfill_appends_all_entity_leaves() {
        let (_tmp, cfg) = test_config();
        // Persist 3 chunks across 2 sources.
        let c1 = mk_chunk("slack:#eng", 0, 1_700_000_000_000, 100);
        let c2 = mk_chunk("gmail:alice", 0, 1_700_000_010_000, 100);
        let c3 = mk_chunk("slack:#eng", 1, 1_700_000_020_000, 100);
        upsert_chunks(&cfg, &[c1.clone(), c2.clone(), c3.clone()]).unwrap();

        let e = sample_entity("email:alice@example.com", "alice@example.com");
        index_entity(
            &cfg,
            &e,
            &c1.id,
            "leaf",
            1_700_000_000_000,
            Some("source:slack"),
        )
        .unwrap();
        index_entity(
            &cfg,
            &e,
            &c2.id,
            "leaf",
            1_700_000_010_000,
            Some("source:gmail"),
        )
        .unwrap();
        index_entity(
            &cfg,
            &e,
            &c3.id,
            "leaf",
            1_700_000_020_000,
            Some("source:slack"),
        )
        .unwrap();

        let tree = get_or_create_topic_tree(&cfg, "email:alice@example.com").unwrap();
        let summariser = InertSummariser::new();
        let n = backfill_topic_tree_at(
            &cfg,
            &tree,
            "email:alice@example.com",
            &summariser,
            TEST_NOW_MS,
        )
        .await
        .unwrap();
        assert_eq!(n, 3);

        // L0 buffer should hold all three leaves (combined tokens well
        // under the 10k seal budget).
        let buf = src_store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert_eq!(buf.item_ids.len(), 3);
        assert_eq!(buf.token_sum, 300);
        // Oldest item is c1.
        assert_eq!(buf.oldest_at.unwrap().timestamp_millis(), 1_700_000_000_000);
    }

    #[tokio::test]
    async fn backfill_drops_leaves_older_than_window() {
        let (_tmp, cfg) = test_config();
        // c_old is 60d before TEST_NOW_MS — outside the 30d cutoff.
        // c_new is 5d before TEST_NOW_MS — inside the window.
        let old_ts = TEST_NOW_MS - 60 * DAY_MS;
        let new_ts = TEST_NOW_MS - 5 * DAY_MS;
        let c_old = mk_chunk("slack:#eng", 0, old_ts, 100);
        let c_new = mk_chunk("slack:#eng", 1, new_ts, 100);
        upsert_chunks(&cfg, &[c_old.clone(), c_new.clone()]).unwrap();

        let e = sample_entity("email:alice@example.com", "alice@example.com");
        index_entity(&cfg, &e, &c_old.id, "leaf", old_ts, Some("source:slack")).unwrap();
        index_entity(&cfg, &e, &c_new.id, "leaf", new_ts, Some("source:slack")).unwrap();

        let tree = get_or_create_topic_tree(&cfg, "email:alice@example.com").unwrap();
        let summariser = InertSummariser::new();
        let n = backfill_topic_tree_at(
            &cfg,
            &tree,
            "email:alice@example.com",
            &summariser,
            TEST_NOW_MS,
        )
        .await
        .unwrap();
        assert_eq!(n, 1, "only the in-window leaf should be appended");
        let buf = src_store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert_eq!(buf.item_ids.len(), 1);
        assert_eq!(buf.item_ids[0], c_new.id);
    }

    #[tokio::test]
    async fn backfill_skips_missing_chunks_without_failing() {
        let (_tmp, cfg) = test_config();
        let e = sample_entity("email:alice@example.com", "alice@example.com");
        // Index a chunk that was never persisted.
        index_entity(&cfg, &e, "chunk:missing", "leaf", 1_700_000_000_000, None).unwrap();
        // And one that was.
        let c = mk_chunk("slack:#eng", 0, 1_700_000_010_000, 100);
        upsert_chunks(&cfg, &[c.clone()]).unwrap();
        index_entity(
            &cfg,
            &e,
            &c.id,
            "leaf",
            1_700_000_010_000,
            Some("source:slack"),
        )
        .unwrap();

        let tree = get_or_create_topic_tree(&cfg, "email:alice@example.com").unwrap();
        let summariser = InertSummariser::new();
        let n = backfill_topic_tree_at(
            &cfg,
            &tree,
            "email:alice@example.com",
            &summariser,
            TEST_NOW_MS,
        )
        .await
        .unwrap();
        assert_eq!(n, 1, "only the existing chunk should be appended");
        let buf = src_store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert_eq!(buf.item_ids.len(), 1);
    }

    #[tokio::test]
    async fn backfill_is_idempotent() {
        let (_tmp, cfg) = test_config();
        let c = mk_chunk("slack:#eng", 0, 1_700_000_000_000, 50);
        upsert_chunks(&cfg, &[c.clone()]).unwrap();
        let e = sample_entity("email:alice@example.com", "alice@example.com");
        index_entity(
            &cfg,
            &e,
            &c.id,
            "leaf",
            1_700_000_000_000,
            Some("source:slack"),
        )
        .unwrap();

        let tree = get_or_create_topic_tree(&cfg, "email:alice@example.com").unwrap();
        let summariser = InertSummariser::new();
        backfill_topic_tree_at(
            &cfg,
            &tree,
            "email:alice@example.com",
            &summariser,
            TEST_NOW_MS,
        )
        .await
        .unwrap();
        backfill_topic_tree_at(
            &cfg,
            &tree,
            "email:alice@example.com",
            &summariser,
            TEST_NOW_MS,
        )
        .await
        .unwrap();
        // append_leaf is idempotent so the buffer still has exactly one row.
        let buf = src_store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert_eq!(buf.item_ids.len(), 1);
    }

    #[tokio::test]
    async fn backfill_skips_summary_nodes() {
        let (_tmp, cfg) = test_config();
        let e = sample_entity("email:alice@example.com", "alice@example.com");
        // A summary-node hit in the entity index — should be skipped.
        index_entity(
            &cfg,
            &e,
            "summary:L1:abc",
            "summary",
            1_700_000_000_000,
            Some("source:slack"),
        )
        .unwrap();
        let tree = get_or_create_topic_tree(&cfg, "email:alice@example.com").unwrap();
        let summariser = InertSummariser::new();
        let n = backfill_topic_tree_at(
            &cfg,
            &tree,
            "email:alice@example.com",
            &summariser,
            TEST_NOW_MS,
        )
        .await
        .unwrap();
        assert_eq!(n, 0);
    }
}
