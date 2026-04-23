//! `memory_tree_drill_down` — walk `child_ids` from a summary node (Phase 4
//! / #710).
//!
//! Primary use case: the LLM gets a summary hit back from `query_source` or
//! `query_topic` and wants to look at the next level down — either more
//! summaries (for L2+ nodes) or the raw chunks (for L1 nodes). This is
//! deliberately a one-step expansion; for multi-step walks the caller
//! passes `max_depth > 1`.
//!
//! When `query` is `Some`, visited children are reranked by cosine similarity
//! against the query embedding so a deep summary with many children can surface
//! the relevant ones to the top. When `query` is `None`, children are returned
//! in BFS order (same as before).
//!
//! Behaviour:
//! - Unknown `node_id` → empty vec (not an error — the LLM can recover).
//! - `max_depth == 0` → empty vec (documented as "no-op").
//! - Leaves have no children; drilling into a leaf id returns empty.
//! - `limit` is optional; when set, it truncates the final (reranked) output.

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::retrieval::types::{
    hit_from_chunk, hit_from_summary, RetrievalHit,
};
use crate::openhuman::memory::tree::score::embed::{build_embedder_from_config, cosine_similarity};
use crate::openhuman::memory::tree::source_tree::store;
use crate::openhuman::memory::tree::store::{get_chunk, get_chunk_embedding};

/// Walk the summary hierarchy down one step (or more if `max_depth > 1`)
/// and return the hydrated child hits. Children at level 1 are raw chunks;
/// deeper children are summaries.
///
/// When `query` is `Some`, the returned hits are reranked by cosine similarity
/// to the query embedding; hits without a stored embedding (legacy rows) sort
/// to the bottom. When `None`, BFS order is preserved.
pub async fn drill_down(
    config: &Config,
    node_id: &str,
    max_depth: u32,
    query: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<RetrievalHit>> {
    log::info!(
        "[retrieval::drill_down] drill_down node_id={} max_depth={} query={} limit={:?}",
        node_id,
        max_depth,
        query.is_some(),
        limit
    );
    if max_depth == 0 {
        log::debug!("[retrieval::drill_down] max_depth=0 — returning empty vec");
        return Ok(Vec::new());
    }

    // Phase 1 — blocking walk produces hits + the per-hit embedding so the
    // async rerank pass can avoid a second trip through the DB.
    let node_id_owned = node_id.to_string();
    let config_owned = config.clone();
    let (hits, embeddings) = tokio::task::spawn_blocking(
        move || -> Result<(Vec<RetrievalHit>, Vec<Option<Vec<f32>>>)> {
            walk_with_embeddings(&config_owned, &node_id_owned, max_depth)
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("drill_down join error: {e}"))??;

    // Phase 2 — optional query rerank.
    let hits = if let Some(q) = query {
        rerank_by_semantic_similarity(config, q, hits, embeddings).await?
    } else {
        hits
    };

    // Phase 3 — apply optional limit AFTER rerank so the top-K is relevance-
    // based when `query` is Some, BFS-based otherwise.
    let hits = match limit {
        Some(n) if hits.len() > n => hits.into_iter().take(n).collect(),
        _ => hits,
    };

    log::debug!("[retrieval::drill_down] returning hits={}", hits.len());
    Ok(hits)
}

/// Rerank hits by cosine similarity to the query embedding. Mirrors the
/// pattern used by `query_source` / `query_topic`. Legacy rows without
/// embeddings land at the end in BFS order.
async fn rerank_by_semantic_similarity(
    config: &Config,
    query: &str,
    hits: Vec<RetrievalHit>,
    embeddings: Vec<Option<Vec<f32>>>,
) -> Result<Vec<RetrievalHit>> {
    debug_assert_eq!(hits.len(), embeddings.len());
    let embedder = build_embedder_from_config(config)?;
    let query_vec = embedder.embed(query).await?;
    log::debug!(
        "[retrieval::drill_down] query embedded provider={} hits_to_rerank={}",
        embedder.name(),
        hits.len()
    );

    let mut decorated: Vec<(f32, bool, RetrievalHit)> = hits
        .into_iter()
        .zip(embeddings.into_iter())
        .map(|(h, emb)| match emb {
            Some(v) if v.len() == query_vec.len() => {
                let sim = cosine_similarity(&query_vec, &v);
                (sim, true, h)
            }
            _ => (f32::NEG_INFINITY, false, h),
        })
        .collect();

    decorated.sort_by(|a, b| match (a.1, b.1) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        // Both ranked (or both unranked): similarity DESC, then by time.
        _ => {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.2.time_range_end.cmp(&a.2.time_range_end))
        }
    });

    Ok(decorated.into_iter().map(|(_, _, h)| h).collect())
}

/// Blocking walker. BFS-style expansion up to `max_depth` levels. Returns
/// each hit paired with its stored embedding (if any), so the async rerank
/// pass doesn't have to round-trip through the DB again.
fn walk_with_embeddings(
    config: &Config,
    start_id: &str,
    max_depth: u32,
) -> Result<(Vec<RetrievalHit>, Vec<Option<Vec<f32>>>)> {
    // Fetch the root. If it's a summary we expand its child_ids; if it's a
    // chunk it has no children. If it's neither we return empty.
    let root_summary = store::get_summary(config, start_id)?;
    let root_tree_scope = match root_summary.as_ref().map(|s| s.tree_id.clone()) {
        Some(tid) => store::get_tree(config, &tid)?
            .map(|t| t.scope)
            .unwrap_or_default(),
        None => String::new(),
    };

    let mut out: Vec<RetrievalHit> = Vec::new();
    let mut embeddings: Vec<Option<Vec<f32>>> = Vec::new();

    let start_children: Vec<String> = match root_summary {
        Some(s) => s.child_ids.clone(),
        None => {
            if let Some(_c) = get_chunk(config, start_id)? {
                return Ok((out, embeddings));
            }
            log::debug!(
                "[retrieval::drill_down] node_id={start_id} not found in summaries or chunks"
            );
            return Ok((out, embeddings));
        }
    };

    // BFS frontier: (child_id, depth_from_start)
    let mut frontier: Vec<(String, u32)> =
        start_children.into_iter().map(|id| (id, 1u32)).collect();

    while let Some((id, depth)) = frontier.pop() {
        if depth > max_depth {
            continue;
        }
        // Is it a summary?
        if let Some(summary) = store::get_summary(config, &id)? {
            let scope = store::get_tree(config, &summary.tree_id)?
                .map(|t| t.scope)
                .unwrap_or_else(|| root_tree_scope.clone());
            // Summary embeddings live on the struct directly (Phase 4 amend).
            embeddings.push(summary.embedding.clone());
            out.push(hit_from_summary(&summary, &scope));
            if depth < max_depth {
                for next in summary.child_ids {
                    frontier.push((next, depth + 1));
                }
            }
            continue;
        }
        // Else try as a chunk (leaf). Chunk embeddings live in a separate
        // blob column — fetch via the existing accessor.
        if let Some(chunk) = get_chunk(config, &id)? {
            let emb = get_chunk_embedding(config, &chunk.id).unwrap_or(None);
            embeddings.push(emb);
            // Score unknown here; 0.0 neutral placeholder.
            out.push(hit_from_chunk(&chunk, "", &chunk.metadata.source_id, 0.0));
            continue;
        }
        log::warn!("[retrieval::drill_down] child id={id} points at nothing — skipping");
    }
    Ok((out, embeddings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::source_tree::bucket_seal::{append_leaf, LeafRef};
    use crate::openhuman::memory::tree::source_tree::registry::get_or_create_source_tree;
    use crate::openhuman::memory::tree::source_tree::summariser::inert::InertSummariser;
    use crate::openhuman::memory::tree::source_tree::types::TreeKind;
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
    use chrono::Utc;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Phase 4 (#710): seeding requires seals which embed.
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    async fn seed_sealed_tree(cfg: &Config) -> (String, String) {
        // Seed two 6k-token leaves so the L0 buffer seals into an L1 node.
        let ts = Utc::now();
        let tree = get_or_create_source_tree(cfg, "slack:#eng").unwrap();
        let summariser = InertSummariser::new();
        let mut leaf_ids: Vec<String> = Vec::new();
        for seq in 0..2u32 {
            let c = Chunk {
                id: chunk_id(SourceKind::Chat, "slack:#eng", seq),
                content: format!("content-{seq}"),
                metadata: Metadata {
                    source_kind: SourceKind::Chat,
                    source_id: "slack:#eng".into(),
                    owner: "alice".into(),
                    timestamp: ts,
                    time_range: (ts, ts),
                    tags: vec![],
                    source_ref: Some(SourceRef::new("slack://x")),
                },
                token_count: 6_000,
                seq_in_source: seq,
                created_at: ts,
            };
            upsert_chunks(cfg, &[c.clone()]).unwrap();
            leaf_ids.push(c.id.clone());
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
            )
            .await
            .unwrap();
        }
        // Fetch the sealed L1 summary id from the tree row.
        let refreshed = store::get_tree(cfg, &tree.id).unwrap().unwrap();
        assert_eq!(refreshed.kind, TreeKind::Source);
        let root_id = refreshed.root_id.unwrap();
        (root_id, leaf_ids.remove(0))
    }

    #[tokio::test]
    async fn depth_zero_returns_empty() {
        let (_tmp, cfg) = test_config();
        let (root_id, _) = seed_sealed_tree(&cfg).await;
        let out = drill_down(&cfg, &root_id, 0, None, None).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn invalid_id_returns_empty() {
        let (_tmp, cfg) = test_config();
        let out = drill_down(&cfg, "nonexistent:id", 1, None, None)
            .await
            .unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn summary_drills_to_leaves_at_depth_one() {
        let (_tmp, cfg) = test_config();
        let (root_id, _) = seed_sealed_tree(&cfg).await;
        let out = drill_down(&cfg, &root_id, 1, None, None).await.unwrap();
        assert_eq!(out.len(), 2, "L1 has 2 leaf children");
        for hit in &out {
            assert_eq!(hit.level, 0, "direct children of L1 are leaves");
        }
    }

    #[tokio::test]
    async fn leaf_drill_down_returns_empty() {
        let (_tmp, cfg) = test_config();
        let (_root_id, leaf_id) = seed_sealed_tree(&cfg).await;
        let out = drill_down(&cfg, &leaf_id, 3, None, None).await.unwrap();
        assert!(out.is_empty(), "leaves have no children");
    }

    #[tokio::test]
    async fn deeper_max_depth_does_not_break_on_shallow_tree() {
        // Only one summary level exists; asking for max_depth=5 is fine.
        let (_tmp, cfg) = test_config();
        let (root_id, _) = seed_sealed_tree(&cfg).await;
        let out = drill_down(&cfg, &root_id, 5, None, None).await.unwrap();
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn query_with_limit_truncates_after_rerank() {
        // Verifies the plumbing for the query param: embedder is invoked
        // (InertEmbedder under this test config — all-zero vectors so
        // cosine is 0 for every candidate), limit truncates the output,
        // and the function completes without error.
        let (_tmp, cfg) = test_config();
        let (root_id, _) = seed_sealed_tree(&cfg).await;
        let out = drill_down(&cfg, &root_id, 1, Some("phoenix migration timing"), Some(1))
            .await
            .unwrap();
        assert_eq!(out.len(), 1, "limit=1 truncates 2 children to 1");
    }

    #[tokio::test]
    async fn query_without_limit_returns_all_children() {
        let (_tmp, cfg) = test_config();
        let (root_id, _) = seed_sealed_tree(&cfg).await;
        let out = drill_down(&cfg, &root_id, 1, Some("phoenix"), None)
            .await
            .unwrap();
        assert_eq!(out.len(), 2, "no limit — both children returned");
    }
}
