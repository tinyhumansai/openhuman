//! Time-based buffer flush for source trees (#709).
//!
//! The bucket-seal path only fires when a buffer crosses `TOKEN_BUDGET`.
//! Low-volume sources (e.g. an email account with two threads a week) can
//! otherwise leave leaves parked in the L0 buffer indefinitely, which
//! hurts recall. `flush_stale_buffers` force-seals any buffer whose
//! `oldest_at` is older than `max_age`, regardless of token count.
//!
//! This is meant to run on a cadence (e.g. daily cron). Phase 3a ships
//! the primitive; wiring into a scheduler is not required for merge.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::source_tree::bucket_seal::cascade_all_from;
use crate::openhuman::memory::tree::source_tree::store;
use crate::openhuman::memory::tree::source_tree::summariser::Summariser;
use crate::openhuman::memory::tree::source_tree::types::DEFAULT_FLUSH_AGE_SECS;

/// Seal every buffer whose oldest item is older than `max_age`. Returns
/// the number of individual seal calls (not trees) that fired. When the
/// same tree has multiple stale levels they're each sealed in order.
pub async fn flush_stale_buffers(
    config: &Config,
    max_age: Duration,
    summariser: &dyn Summariser,
) -> Result<usize> {
    let now = Utc::now();
    let cutoff = now - max_age;
    let stale = store::list_stale_buffers(config, cutoff)?;
    log::info!(
        "[source_tree::flush] found {} stale buffers (max_age={:?})",
        stale.len(),
        max_age
    );

    let mut seals: usize = 0;
    for buf in stale {
        let tree = match store::get_tree(config, &buf.tree_id)? {
            Some(t) => t,
            None => {
                log::warn!(
                    "[source_tree::flush] orphan buffer tree_id={} level={}",
                    buf.tree_id,
                    buf.level
                );
                continue;
            }
        };
        let sealed = cascade_all_from(config, &tree, buf.level, summariser, Some(now)).await?;
        seals += sealed.len();
    }
    Ok(seals)
}

/// Convenience wrapper that uses [`DEFAULT_FLUSH_AGE_SECS`].
pub async fn flush_stale_buffers_default(
    config: &Config,
    summariser: &dyn Summariser,
) -> Result<usize> {
    flush_stale_buffers(
        config,
        Duration::seconds(DEFAULT_FLUSH_AGE_SECS),
        summariser,
    )
    .await
}

/// Helper exposed for callers that want a single explicit force-seal (e.g.
/// "user disconnected this account, flush its buffer now").
pub async fn force_flush_tree(
    config: &Config,
    tree_id: &str,
    summariser: &dyn Summariser,
    now: Option<DateTime<Utc>>,
) -> Result<Vec<String>> {
    let tree = store::get_tree(config, tree_id)?
        .ok_or_else(|| anyhow::anyhow!("no tree with id {tree_id}"))?;
    cascade_all_from(config, &tree, 0, summariser, now).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::source_tree::bucket_seal::{append_leaf, LeafRef};
    use crate::openhuman::memory::tree::source_tree::registry::get_or_create_source_tree;
    use crate::openhuman::memory::tree::source_tree::summariser::inert::InertSummariser;
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    #[tokio::test]
    async fn flush_seals_old_buffer_even_under_budget() {
        let (_tmp, cfg) = test_config();
        let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        let summariser = InertSummariser::new();

        // Persist one chunk with an old timestamp (10 days ago).
        let old_ts = Utc::now() - Duration::days(10);
        let c = Chunk {
            id: chunk_id(SourceKind::Chat, "slack:#eng", 0),
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
        };
        upsert_chunks(&cfg, &[c.clone()]).unwrap();

        let leaf = LeafRef {
            chunk_id: c.id.clone(),
            token_count: 100, // way under the 10k budget
            timestamp: old_ts,
            content: c.content.clone(),
            entities: vec![],
            topics: vec![],
            score: 0.5,
        };
        append_leaf(&cfg, &tree, &leaf, &summariser).await.unwrap();
        assert_eq!(store::count_summaries(&cfg, &tree.id).unwrap(), 0);

        let seals = flush_stale_buffers(&cfg, Duration::days(7), &summariser)
            .await
            .unwrap();
        assert_eq!(seals, 1);
        assert_eq!(store::count_summaries(&cfg, &tree.id).unwrap(), 1);

        let l0 = store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert!(l0.is_empty());
    }

    #[tokio::test]
    async fn flush_noop_when_buffer_is_recent() {
        let (_tmp, cfg) = test_config();
        let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        let summariser = InertSummariser::new();

        // Persist a leaf stamped now so it's NOT stale.
        let now = Utc::now();
        let c = Chunk {
            id: chunk_id(SourceKind::Chat, "slack:#eng", 0),
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
        append_leaf(&cfg, &tree, &leaf, &summariser).await.unwrap();

        let seals = flush_stale_buffers(&cfg, Duration::days(7), &summariser)
            .await
            .unwrap();
        assert_eq!(seals, 0);
        assert_eq!(store::count_summaries(&cfg, &tree.id).unwrap(), 0);
    }
}
