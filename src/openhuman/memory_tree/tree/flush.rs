//! Time-based buffer flush for source trees (#709).
//!
//! The bucket-seal path only fires when a buffer crosses
//! `INPUT_TOKEN_BUDGET` (token volume) or `SUMMARY_FANOUT` (item count
//! — the L0 fallback gate). Low-volume sources (e.g. an email account
//! with two threads a week) can still park a buffer below both
//! thresholds indefinitely, which hurts recall.
//! `flush_stale_buffers` force-seals any buffer whose `oldest_at` is
//! older than `max_age`, regardless of token count or item count.
//!
//! This is meant to run on a cadence (e.g. daily cron). Phase 3a ships
//! the primitive; wiring into a scheduler is not required for merge.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::trees::types::DEFAULT_FLUSH_AGE_SECS;
use crate::openhuman::memory_tree::tree::bucket_seal::{cascade_all_from, LabelStrategy};
use crate::openhuman::memory_tree::tree::store;

/// Seal every buffer whose oldest item is older than `max_age`. Returns
/// the number of individual seal calls (not trees) that fired. When the
/// same tree has multiple stale levels they're each sealed in order.
pub async fn flush_stale_buffers(
    config: &Config,
    max_age: Duration,
    strategy: &LabelStrategy,
) -> Result<usize> {
    use crate::openhuman::memory_store::trees::store::get_trees_batch;

    let now = Utc::now();
    let cutoff = now - max_age;
    let stale = store::list_stale_buffers(config, cutoff)?;
    log::info!(
        "[tree::flush] found {} stale buffers (max_age={:?})",
        stale.len(),
        max_age
    );

    // One batched `SELECT … WHERE id IN (?,?,…)` over the distinct
    // tree_ids instead of N per-buffer `get_tree` round-trips. `stale`
    // is filtered to L0 by `list_stale_buffers` so the same tree_id
    // typically appears at most once, but we still de-dup defensively
    // before hitting the DB — future widenings of the filter (e.g.
    // dropping the `level = 0` clause) would otherwise re-pay the cost.
    // Missing rows stay silently absent from the map; the
    // `Some(t) => ... / None => warn-and-skip` orphan-buffer path below
    // preserves the per-id `get_tree` `Ok(None)` semantics.
    // Preserve list_stale_buffers order so the HashMap lookup below stays
    // predictable; dedup is purely to avoid redundant DB params (not for
    // semantic ordering — the per-buffer loop walks `stale` directly).
    let distinct_tree_ids: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for buf in &stale {
            if seen.insert(buf.tree_id.clone()) {
                out.push(buf.tree_id.clone());
            }
        }
        out
    };
    let tree_by_id = get_trees_batch(config, &distinct_tree_ids)?;

    let mut seals: usize = 0;
    for buf in stale {
        let Some(tree) = tree_by_id.get(&buf.tree_id) else {
            log::warn!(
                "[tree::flush] orphan buffer tree_id={} level={}",
                buf.tree_id,
                buf.level
            );
            continue;
        };
        let sealed = cascade_all_from(config, tree, buf.level, Some(now), strategy).await?;
        seals += sealed.len();
    }
    Ok(seals)
}

/// Convenience wrapper that uses [`DEFAULT_FLUSH_AGE_SECS`].
pub async fn flush_stale_buffers_default(
    config: &Config,
    strategy: &LabelStrategy,
) -> Result<usize> {
    flush_stale_buffers(config, Duration::seconds(DEFAULT_FLUSH_AGE_SECS), strategy).await
}

/// Helper exposed for callers that want a single explicit force-seal (e.g.
/// "user disconnected this account, flush its buffer now").
pub async fn force_flush_tree(
    config: &Config,
    tree_id: &str,
    now: Option<DateTime<Utc>>,
    strategy: &LabelStrategy,
) -> Result<Vec<String>> {
    let tree = store::get_tree(config, tree_id)?
        .ok_or_else(|| anyhow::anyhow!("no tree with id {tree_id}"))?;
    cascade_all_from(config, &tree, 0, now, strategy).await
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
    use crate::openhuman::memory_tree::tree::bucket_seal::{append_leaf, LeafRef};
    use std::sync::Arc;
    use tempfile::TempDir;

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

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Phase 4 (#710): flush triggers seals which embed — force inert.
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    #[tokio::test]
    async fn flush_seals_old_buffer_even_under_budget() {
        let (_tmp, cfg) = test_config();
        let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        let provider: Arc<dyn ChatProvider> =
            Arc::new(StaticChatProvider::new("test summary content"));

        // Persist one chunk with an old timestamp (10 days ago).
        let old_ts = Utc::now() - Duration::days(10);
        let c = Chunk {
            id: chunk_id(SourceKind::Chat, "slack:#eng", 0, "test-content"),
            content: "old content that should get sealed".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice".into(),
                timestamp: old_ts,
                time_range: (old_ts, old_ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://x")),
            },
            token_count: 100,
            seq_in_source: 0,
            created_at: old_ts,
            partial_message: false,
        };
        upsert_chunks(&cfg, &[c.clone()]).unwrap();
        stage_test_chunks(&cfg, &[c.clone()]);

        let leaf = LeafRef {
            chunk_id: c.id.clone(),
            token_count: 100, // way under the 10k budget
            timestamp: old_ts,
            content: c.content.clone(),
            entities: vec![],
            topics: vec![],
            score: 0.5,
        };
        test_override::with_provider(Arc::clone(&provider), async {
            append_leaf(&cfg, &tree, &leaf, &LabelStrategy::Empty)
                .await
                .unwrap();
        })
        .await;
        assert_eq!(store::count_summaries(&cfg, &tree.id).unwrap(), 0);

        let seals = test_override::with_provider(Arc::clone(&provider), async {
            flush_stale_buffers(&cfg, Duration::days(7), &LabelStrategy::Empty)
                .await
                .unwrap()
        })
        .await;
        assert_eq!(seals, 1);
        assert_eq!(store::count_summaries(&cfg, &tree.id).unwrap(), 1);

        let l0 = store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert!(l0.is_empty());
    }

    #[tokio::test]
    async fn flush_does_not_force_seal_under_fanout_upper_buffer() {
        // Regression: previously `list_stale_buffers` returned every level,
        // and `cascade_all_from` force-sealed the first iteration regardless
        // of `should_seal`. A stale L1 buffer with one child would seal into
        // a degenerate L2 summary that wraps a single L1 — repeating across
        // flush cycles produced the L7→L13 1:1:1 chain in real workspaces.
        // Flush must restrict force-seals to L0 and let upper levels gate
        // on `SUMMARY_FANOUT` naturally.
        let (_tmp, cfg) = test_config();
        let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        let provider: Arc<dyn ChatProvider> =
            Arc::new(StaticChatProvider::new("test summary content"));

        // Plant a stale L1 buffer holding a single (synthetic) child id.
        // No L0 chunks — the only thing flush could touch is the L1 buffer.
        let old_ts = Utc::now() - Duration::days(10);
        crate::openhuman::memory_store::chunks::store::with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            crate::openhuman::memory_store::trees::store::upsert_buffer_tx(
                &tx,
                &crate::openhuman::memory_store::trees::types::Buffer {
                    tree_id: tree.id.clone(),
                    level: 1,
                    item_ids: vec!["fake-l1-child".into()],
                    token_sum: 100,
                    oldest_at: Some(old_ts),
                },
            )?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();

        let seals = test_override::with_provider(provider, async {
            flush_stale_buffers(&cfg, Duration::days(7), &LabelStrategy::Empty)
                .await
                .unwrap()
        })
        .await;
        assert_eq!(seals, 0, "L1 stale buffer must not be force-sealed");
        assert_eq!(store::count_summaries(&cfg, &tree.id).unwrap(), 0);

        // The L1 buffer must still be intact — flush cannot touch it.
        let l1 = store::get_buffer(&cfg, &tree.id, 1).unwrap();
        assert_eq!(l1.item_ids, vec!["fake-l1-child".to_string()]);
    }

    #[tokio::test]
    async fn flush_noop_when_buffer_is_recent() {
        let (_tmp, cfg) = test_config();
        let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        let provider: Arc<dyn ChatProvider> =
            Arc::new(StaticChatProvider::new("test summary content"));

        // Persist a leaf stamped now so it's NOT stale.
        let now = Utc::now();
        let c = Chunk {
            id: chunk_id(SourceKind::Chat, "slack:#eng", 0, "test-content"),
            content: "fresh".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice".into(),
                timestamp: now,
                time_range: (now, now),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://x")),
            },
            token_count: 50,
            seq_in_source: 0,
            created_at: now,
            partial_message: false,
        };
        upsert_chunks(&cfg, &[c.clone()]).unwrap();
        let leaf = LeafRef {
            chunk_id: c.id.clone(),
            token_count: 50,
            timestamp: now,
            content: c.content.clone(),
            entities: vec![],
            topics: vec![],
            score: 0.5,
        };
        test_override::with_provider(Arc::clone(&provider), async {
            append_leaf(&cfg, &tree, &leaf, &LabelStrategy::Empty)
                .await
                .unwrap();
        })
        .await;

        let seals = test_override::with_provider(provider, async {
            flush_stale_buffers(&cfg, Duration::days(7), &LabelStrategy::Empty)
                .await
                .unwrap()
        })
        .await;
        assert_eq!(seals, 0);
        assert_eq!(store::count_summaries(&cfg, &tree.id).unwrap(), 0);
    }

    /// Regression for the `get_trees_batch` refactor: two distinct
    /// trees each carry a stale L0 buffer; `flush_stale_buffers` must
    /// seal both via a single batched tree-fetch. Pre-refactor this
    /// path issued N per-buffer `get_tree(...)` round-trips; post-
    /// refactor it's one `SELECT … WHERE id IN (?,?)` and a per-id
    /// HashMap lookup. The interesting bit is the HashMap lookup
    /// keying by tree_id (not by `enumerate()` position over the input
    /// slice) — with two trees, swapping the lookup key would leak one
    /// tree's `Tree` into the other tree's seal and either error out
    /// or write summaries against the wrong `root_id`.
    #[tokio::test]
    async fn flush_seals_multiple_distinct_trees_via_batched_lookup() {
        let (_tmp, cfg) = test_config();
        let tree_a = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        let tree_b = get_or_create_source_tree(&cfg, "slack:#design").unwrap();
        assert_ne!(
            tree_a.id, tree_b.id,
            "test prerequisite: distinct source_ids must yield distinct tree_ids"
        );
        let provider: Arc<dyn ChatProvider> =
            Arc::new(StaticChatProvider::new("test summary content"));

        let old_ts = Utc::now() - Duration::days(10);

        // Plant a stale L0 leaf in each tree, backed by real chunks so
        // the cascade can seal end-to-end.
        for (src_id, content) in [
            ("slack:#eng", "engineering content"),
            ("slack:#design", "design content"),
        ] {
            let c = Chunk {
                id: chunk_id(SourceKind::Chat, src_id, 0, content),
                content: content.into(),
                metadata: Metadata {
                    source_kind: SourceKind::Chat,
                    source_id: src_id.into(),
                    owner: "alice".into(),
                    timestamp: old_ts,
                    time_range: (old_ts, old_ts),
                    tags: vec![],
                    source_ref: Some(SourceRef::new(&format!("slack://{src_id}"))),
                },
                token_count: 100,
                seq_in_source: 0,
                created_at: old_ts,
                partial_message: false,
            };
            upsert_chunks(&cfg, &[c.clone()]).unwrap();
            stage_test_chunks(&cfg, &[c.clone()]);
            let leaf = LeafRef {
                chunk_id: c.id.clone(),
                token_count: 100,
                timestamp: old_ts,
                content: c.content.clone(),
                entities: vec![],
                topics: vec![],
                score: 0.5,
            };
            let target_tree = if src_id == "slack:#eng" {
                &tree_a
            } else {
                &tree_b
            };
            test_override::with_provider(Arc::clone(&provider), async {
                append_leaf(&cfg, target_tree, &leaf, &LabelStrategy::Empty)
                    .await
                    .unwrap();
            })
            .await;
        }

        let seals = test_override::with_provider(provider, async {
            flush_stale_buffers(&cfg, Duration::days(7), &LabelStrategy::Empty)
                .await
                .unwrap()
        })
        .await;

        // Both trees sealed via a single batched fetch — the HashMap
        // lookup correctly routed each buffer to its own `Tree`.
        assert_eq!(seals, 2, "both stale trees must seal in one flush pass");
        assert_eq!(
            store::count_summaries(&cfg, &tree_a.id).unwrap(),
            1,
            "tree_a must own its summary (HashMap keyed by id, not position)"
        );
        assert_eq!(
            store::count_summaries(&cfg, &tree_b.id).unwrap(),
            1,
            "tree_b must own its summary (HashMap keyed by id, not position)"
        );
    }
}
