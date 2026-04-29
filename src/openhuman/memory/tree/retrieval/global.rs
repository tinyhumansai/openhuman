//! `memory_tree_query_global` — window-scoped recap from the global digest
//! (Phase 4 / #710).
//!
//! Thin wrapper on [`global_tree::recap::recap`]. The recap function does
//! the heavy lifting (level selection + time-range filter); we convert its
//! output into the uniform [`RetrievalHit`] shape.
//!
//! When no global summaries exist yet (e.g. early in a workspace's life),
//! we return an empty [`QueryResponse`] rather than an error so the LLM can
//! surface "no digest yet" naturally.

use anyhow::Result;
use chrono::Duration;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::global_tree::recap::{recap, RecapOutput};
use crate::openhuman::memory::tree::global_tree::registry::get_or_create_global_tree;
use crate::openhuman::memory::tree::retrieval::types::{NodeKind, QueryResponse, RetrievalHit};
use crate::openhuman::memory::tree::source_tree::types::TreeKind;

/// Return the global digest for the given window in days. Always returns a
/// [`QueryResponse`]; the response is empty if the global tree has no
/// sealed summaries yet.
pub async fn query_global(config: &Config, window_days: u32) -> Result<QueryResponse> {
    log::info!(
        "[retrieval::global] query_global window_days={}",
        window_days
    );

    let window = Duration::days(window_days as i64);
    let recap_out = match recap(config, window).await? {
        Some(r) => r,
        None => {
            log::debug!("[retrieval::global] no recap available — returning empty response");
            return Ok(QueryResponse::empty());
        }
    };

    let tree = get_or_create_global_tree(config)?;
    let hits = recap_to_hits(recap_out, &tree.id, &tree.scope);
    let total = hits.len();
    log::debug!(
        "[retrieval::global] returning hits={} total={}",
        hits.len(),
        total
    );
    Ok(QueryResponse::new(hits, total))
}

/// Convert a [`RecapOutput`] into one synthetic summary hit per fold. We
/// emit one [`RetrievalHit`] covering the assembled recap content — the
/// per-summary provenance lives in `recap.summary_ids`, threaded through as
/// `child_ids` so the LLM can drill into a specific folded day/week/month.
fn recap_to_hits(recap: RecapOutput, tree_id: &str, tree_scope: &str) -> Vec<RetrievalHit> {
    let RecapOutput {
        content,
        time_range,
        level_used,
        summary_ids,
    } = recap;
    // We emit ONE hit summarising the whole recap. Drill-down into
    // `child_ids` (the individual summary node ids) is available via
    // `memory_tree_drill_down`. This keeps the shape consistent with the
    // other query tools (which also return summary-level hits).
    let node_id = summary_ids
        .first()
        .cloned()
        .unwrap_or_else(|| format!("recap:L{level_used}"));
    vec![RetrievalHit {
        node_id,
        node_kind: NodeKind::Summary,
        tree_id: tree_id.to_string(),
        tree_kind: TreeKind::Global,
        tree_scope: tree_scope.to_string(),
        level: level_used,
        content,
        entities: Vec::new(),
        topics: Vec::new(),
        time_range_start: time_range.0,
        time_range_end: time_range.1,
        score: 0.0,
        child_ids: summary_ids,
        source_ref: None,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::content_store;
    use crate::openhuman::memory::tree::global_tree::digest::{end_of_day_digest, DigestOutcome};
    use crate::openhuman::memory::tree::source_tree::bucket_seal::{
        append_leaf, LabelStrategy, LeafRef,
    };
    use crate::openhuman::memory::tree::source_tree::registry::get_or_create_source_tree;
    use crate::openhuman::memory::tree::source_tree::summariser::inert::InertSummariser;
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
    use chrono::{DateTime, Utc};
    use tempfile::TempDir;

    fn stage_test_chunks(cfg: &Config, chunks: &[Chunk]) {
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).expect("create content_root for test");
        let staged = content_store::stage_chunks(&content_root, chunks)
            .expect("stage_chunks for test chunks");
        crate::openhuman::memory::tree::store::with_connection(cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            crate::openhuman::memory::tree::store::upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .expect("persist staged chunk pointers");
    }

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Phase 4 (#710): digest embeds — inert in tests.
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    async fn seed_daily_digest(cfg: &Config) {
        let summariser = InertSummariser::new();
        let day = Utc::now().date_naive();
        let ts = day.and_hms_opt(12, 0, 0).unwrap().and_utc();
        seed_source_for_day(cfg, "slack:#eng", ts).await;
        end_of_day_digest(cfg, day, &summariser).await.unwrap();
    }

    async fn seed_source_for_day(cfg: &Config, scope: &str, ts: DateTime<Utc>) {
        let tree = get_or_create_source_tree(cfg, scope).unwrap();
        let summariser = InertSummariser::new();
        for seq in 0..2u32 {
            let c = Chunk {
                id: chunk_id(SourceKind::Chat, scope, seq, "test-content"),
                content: format!("daily-{scope}-{seq}"),
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
                seq_in_source: seq,
                created_at: ts,
                partial_message: false,
            };
            upsert_chunks(cfg, &[c.clone()]).unwrap();
            stage_test_chunks(cfg, &[c.clone()]);
            append_leaf(
                cfg,
                &tree,
                &LeafRef {
                    chunk_id: c.id.clone(),
                    token_count: 6_000,
                    timestamp: ts,
                    content: c.content.clone(),
                    entities: vec![],
                    topics: vec![],
                    score: 0.5,
                },
                &summariser,
                &LabelStrategy::Empty,
            )
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn empty_tree_returns_empty_response() {
        let (_tmp, cfg) = test_config();
        let resp = query_global(&cfg, 7).await.unwrap();
        assert!(resp.hits.is_empty());
        assert_eq!(resp.total, 0);
        assert!(!resp.truncated);
    }

    #[tokio::test]
    async fn wraps_daily_recap_into_a_hit() {
        let (_tmp, cfg) = test_config();
        seed_daily_digest(&cfg).await;
        let resp = query_global(&cfg, 1).await.unwrap();
        assert_eq!(resp.hits.len(), 1);
        assert_eq!(resp.hits[0].tree_kind, TreeKind::Global);
        assert_eq!(resp.hits[0].level, 0);
        assert!(!resp.hits[0].content.is_empty());
        assert!(
            !resp.hits[0].child_ids.is_empty(),
            "child_ids must expose the folded summary ids for drill-down"
        );
    }

    #[tokio::test]
    async fn digest_outcome_sanity_check() {
        // Sanity: make sure the test helper fixture actually emits a digest;
        // if this ever returned Skipped the rest of the suite would trivially
        // pass which would be misleading.
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        let day = Utc::now().date_naive();
        let ts = day.and_hms_opt(12, 0, 0).unwrap().and_utc();
        seed_source_for_day(&cfg, "slack:#eng", ts).await;
        let outcome = end_of_day_digest(&cfg, day, &summariser).await.unwrap();
        assert!(matches!(outcome, DigestOutcome::Emitted { .. }));
    }
}
