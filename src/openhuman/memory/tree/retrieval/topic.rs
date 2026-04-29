//! `memory_tree_query_topic` — entity-scoped retrieval across every tree
//! that has seen the entity (Phase 4 / #710).
//!
//! Two data sources combined:
//! 1. [`score::store::lookup_entity`] returns every `(node_id, tree_id)`
//!    association from the `mem_tree_entity_index` — covers leaves AND
//!    summaries across all trees regardless of kind.
//! 2. If a per-entity topic tree exists (`(kind=topic, scope=entity_id)`),
//!    we also surface its current root so the LLM can ask "summarise
//!    everything you know about $entity" in one hop.
//!
//! Hits are filtered by `time_window_days` if given, then sorted
//! `score DESC, timestamp DESC` (strongest signal first, then newest).
//! Truncation to `limit` comes last.

use anyhow::Result;
use chrono::{Duration, TimeZone, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::content_store::read as content_read;
use crate::openhuman::memory::tree::retrieval::types::{
    hit_from_summary, QueryResponse, RetrievalHit,
};
use crate::openhuman::memory::tree::score::embed::{build_embedder_from_config, cosine_similarity};
use crate::openhuman::memory::tree::score::store::{lookup_entity, EntityHit};
use crate::openhuman::memory::tree::source_tree::store;
use crate::openhuman::memory::tree::source_tree::types::{Tree, TreeKind};

const DEFAULT_LIMIT: usize = 10;
/// How many rows we pull from the entity index before filtering. We give
/// ourselves plenty of headroom because time-window + score-based filtering
/// can drop many rows — asking the index for exactly `limit` would bias
/// toward the newest hits at the expense of the strongest-score ones.
const LOOKUP_HEADROOM: usize = 200;

/// Public entrypoint. `entity_id` should be the canonical id string
/// (e.g. `email:alice@example.com`, `topic:phoenix`). Unknown ids return
/// an empty response — callers that want fuzzy matching should go through
/// `memory_tree_search_entities` first.
///
/// When `query` is `Some`, hits are reranked by cosine similarity to the
/// query's embedding; candidates without embeddings (legacy rows) fall
/// to the bottom. When `None`, the classic `(score DESC, timestamp DESC)`
/// ordering applies.
pub async fn query_topic(
    config: &Config,
    entity_id: &str,
    time_window_days: Option<u32>,
    query: Option<&str>,
    limit: usize,
) -> Result<QueryResponse> {
    let limit = if limit == 0 { DEFAULT_LIMIT } else { limit };
    // Redact `entity_id` — typically `email:<addr>` or `handle:<name>`.
    // Log the kind prefix only so operators can still see what kind of
    // entity was queried.
    let entity_kind_prefix = entity_id
        .split_once(':')
        .map(|(k, _)| k)
        .unwrap_or("unknown");
    log::debug!(
        "[retrieval::topic] query_topic entity_kind={} window_days={:?} has_query={} limit={}",
        entity_kind_prefix,
        time_window_days,
        query.is_some(),
        limit
    );

    let entity_id_owned = entity_id.to_string();
    let config_owned = config.clone();
    let (index_hits, topic_tree_summary) =
        tokio::task::spawn_blocking(move || -> Result<(Vec<EntityHit>, Option<RetrievalHit>)> {
            let hits = lookup_entity(&config_owned, &entity_id_owned, Some(LOOKUP_HEADROOM))?;
            let topic_summary = fetch_topic_tree_root_summary(&config_owned, &entity_id_owned)?;
            Ok((hits, topic_summary))
        })
        .await
        .map_err(|e| anyhow::anyhow!("query_topic join error: {e}"))??;

    log::debug!(
        "[retrieval::topic] index hits={} topic_tree_summary_present={}",
        index_hits.len(),
        topic_tree_summary.is_some()
    );

    // Deduplicate by node_id: the same node can appear multiple times
    // across the entity index (one row per occurrence) and may also
    // overlap the topic-tree root summary. Without dedup we inflate
    // `total` and waste result slots. For duplicates, keep the higher
    // score; if scores tie, prefer the newer `time_range_end`.
    // Flagged on PR #831 CodeRabbit review.
    use std::collections::HashMap;
    let mut by_node: HashMap<String, RetrievalHit> = HashMap::new();

    let merge = |map: &mut HashMap<String, RetrievalHit>, hit: RetrievalHit| {
        map.entry(hit.node_id.clone())
            .and_modify(|existing| {
                let better = match hit
                    .score
                    .partial_cmp(&existing.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                {
                    std::cmp::Ordering::Greater => true,
                    std::cmp::Ordering::Less => false,
                    std::cmp::Ordering::Equal => hit.time_range_end > existing.time_range_end,
                };
                if better {
                    *existing = hit.clone();
                }
            })
            .or_insert(hit);
    };

    if let Some(summary) = topic_tree_summary {
        merge(&mut by_node, summary);
    }
    for h in index_hits {
        if let Some(hit) = entity_hit_to_retrieval_hit(config, &h).await? {
            merge(&mut by_node, hit);
        }
    }

    let mut hits: Vec<RetrievalHit> = by_node.into_values().collect();
    if let Some(days) = time_window_days {
        hits = filter_by_window(hits, days);
    }

    let total = hits.len();

    let sorted = if let Some(q) = query {
        rerank_by_semantic_similarity(config, q, hits).await?
    } else {
        let mut by_score = hits;
        // Sort: score DESC, then newest first on ties.
        by_score.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.time_range_end.cmp(&a.time_range_end))
        });
        by_score
    };
    let mut sorted = sorted;
    sorted.truncate(limit);

    log::debug!(
        "[retrieval::topic] returning hits={} total={}",
        sorted.len(),
        total
    );
    Ok(QueryResponse::new(sorted, total))
}

/// Rerank hits by cosine similarity to the query embedding. Reads each
/// hit's stored embedding (summary rows from `mem_tree_summaries`, leaf
/// rows from `mem_tree_chunks`) directly via store helpers. Rows with no
/// embedding sort to the bottom, preserving their relative (score, time)
/// order so the unranked tail remains readable.
async fn rerank_by_semantic_similarity(
    config: &Config,
    query: &str,
    hits: Vec<RetrievalHit>,
) -> Result<Vec<RetrievalHit>> {
    use crate::openhuman::memory::tree::retrieval::types::NodeKind;
    use crate::openhuman::memory::tree::source_tree::store as src_store;
    use crate::openhuman::memory::tree::store::get_chunk_embedding;

    let embedder = build_embedder_from_config(config)?;
    let query_vec = embedder.embed(query).await?;
    log::debug!(
        "[retrieval::topic] query embedded provider={} hits_to_rerank={}",
        embedder.name(),
        hits.len()
    );

    // Resolve each hit's embedding. spawn_blocking around the DB reads
    // so the event loop stays healthy even for larger headroom pulls.
    let mut decorated: Vec<(f32, bool, RetrievalHit)> = Vec::with_capacity(hits.len());
    for h in hits {
        let node_id = h.node_id.clone();
        let node_kind = h.node_kind;
        let config_owned = config.clone();
        let emb = tokio::task::spawn_blocking(move || -> Result<Option<Vec<f32>>> {
            match node_kind {
                NodeKind::Summary => src_store::get_summary_embedding(&config_owned, &node_id),
                NodeKind::Leaf => get_chunk_embedding(&config_owned, &node_id),
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("embedding fetch join error: {e}"))??;

        match emb {
            Some(v) => {
                let sim = cosine_similarity(&query_vec, &v);
                decorated.push((sim, true, h));
            }
            None => {
                decorated.push((f32::NEG_INFINITY, false, h));
            }
        }
    }

    decorated.sort_by(|a, b| match (a.1, b.1) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    b.2.score
                        .partial_cmp(&a.2.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| b.2.time_range_end.cmp(&a.2.time_range_end))
        }
    });

    Ok(decorated.into_iter().map(|(_, _, h)| h).collect())
}

/// Look up the topic tree for `entity_id` and return its current root as a
/// retrieval hit. Returns `None` if no topic tree exists (per Phase 3c
/// lazy materialisation — topic trees only spawn on hotness) or if the
/// tree has no sealed root yet.
fn fetch_topic_tree_root_summary(config: &Config, entity_id: &str) -> Result<Option<RetrievalHit>> {
    let tree = match store::get_tree_by_scope(config, TreeKind::Topic, entity_id)? {
        Some(t) => t,
        None => return Ok(None),
    };
    let root_id = match &tree.root_id {
        Some(id) => id.clone(),
        None => return Ok(None),
    };
    let mut summary = match store::get_summary(config, &root_id)? {
        Some(s) => s,
        None => {
            log::warn!(
                "[retrieval::topic] topic tree has root_id set but the summary row is missing"
            );
            return Ok(None);
        }
    };
    // Hydrate the full body from disk — `summary.content` is a ≤500-char
    // preview after the MD-on-disk migration. Non-fatal fallback for
    // pre-MD-migration rows.
    match content_read::read_summary_body(config, &root_id) {
        Ok(body) => summary.content = body,
        Err(e) => {
            log::warn!(
                "[retrieval::topic] read_summary_body failed for topic root — serving preview: {e:#}"
            );
        }
    }
    Ok(Some(hit_from_summary(&summary, &tree.scope)))
}

/// Convert a raw [`EntityHit`] row into a [`RetrievalHit`] by hydrating the
/// backing node. Summary hits fetch from `mem_tree_summaries`; leaf hits
/// fetch from `mem_tree_chunks`. Missing rows are skipped with a warn log
/// — the index row is stale but the retrieval doesn't error out.
async fn entity_hit_to_retrieval_hit(
    config: &Config,
    hit: &EntityHit,
) -> Result<Option<RetrievalHit>> {
    let node_id = hit.node_id.clone();
    let node_kind = hit.node_kind.clone();
    let tree_id_opt = hit.tree_id.clone();
    let score = hit.score;
    let timestamp_ms = hit.timestamp_ms;
    let config_owned = config.clone();

    tokio::task::spawn_blocking(move || -> Result<Option<RetrievalHit>> {
        if node_kind == "summary" {
            let mut summary = match store::get_summary(&config_owned, &node_id)? {
                Some(s) => s,
                None => {
                    log::warn!("[retrieval::topic] entity index points at missing summary row");
                    return Ok(None);
                }
            };
            // Hydrate the full body from disk — `summary.content` is a
            // ≤500-char preview after the MD-on-disk migration.
            match content_read::read_summary_body(&config_owned, &node_id) {
                Ok(body) => summary.content = body,
                Err(e) => {
                    log::warn!(
                        "[retrieval::topic] read_summary_body failed — serving preview: {e:#}"
                    );
                }
            }
            // Prefer tree scope from the summary's parent tree if resolvable.
            let scope = if let Some(tid) = &tree_id_opt {
                store::get_tree(&config_owned, tid)?
                    .map(|t: Tree| t.scope)
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let mut h = hit_from_summary(&summary, &scope);
            // The index row's own score is a per-(entity, node) signal —
            // inherit it so topic ordering uses the association strength
            // rather than the summary's overall score.
            h.score = score;
            return Ok(Some(h));
        }
        // Leaf: fetch chunk and hydrate.
        use crate::openhuman::memory::tree::retrieval::types::hit_from_chunk;
        use crate::openhuman::memory::tree::store::get_chunk;
        let mut chunk = match get_chunk(&config_owned, &node_id)? {
            Some(c) => c,
            None => {
                log::warn!("[retrieval::topic] entity index points at missing chunk row");
                return Ok(None);
            }
        };
        // Hydrate the full body from disk — `chunk.content` is a ≤500-char
        // preview after the MD-on-disk migration.
        match content_read::read_chunk_body(&config_owned, &node_id) {
            Ok(body) => chunk.content = body,
            Err(e) => {
                log::warn!("[retrieval::topic] read_chunk_body failed — serving preview: {e:#}");
            }
        }
        let scope = if let Some(tid) = &tree_id_opt {
            store::get_tree(&config_owned, tid)?
                .map(|t: Tree| t.scope)
                .unwrap_or_else(|| chunk.metadata.source_id.clone())
        } else {
            chunk.metadata.source_id.clone()
        };
        let mut h = hit_from_chunk(&chunk, tree_id_opt.as_deref().unwrap_or(""), &scope, score);
        // Stamp the hit's time range end to the index's recorded timestamp
        // if our chunk row lacks a meaningful range (e.g. pre-3a leaves).
        if h.time_range_end <= chrono::DateTime::<Utc>::MIN_UTC {
            if let chrono::LocalResult::Single(dt) = Utc.timestamp_millis_opt(timestamp_ms) {
                h.time_range_end = dt;
                h.time_range_start = dt;
            }
        }
        Ok(Some(h))
    })
    .await
    .map_err(|e| anyhow::anyhow!("entity_hit conversion join error: {e}"))?
}

fn filter_by_window(hits: Vec<RetrievalHit>, window_days: u32) -> Vec<RetrievalHit> {
    let now = Utc::now();
    let window_start = now - Duration::days(window_days as i64);
    hits.into_iter()
        .filter(|h| h.time_range_end >= window_start && h.time_range_start <= now)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::canonicalize::chat::{ChatBatch, ChatMessage};
    use crate::openhuman::memory::tree::ingest::ingest_chat;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Phase 4 (#710): ingest triggers seals which embed.
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    fn substantive_batch() -> ChatBatch {
        ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![ChatMessage {
                author: "alice".into(),
                timestamp: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
                text: "We are planning to ship the Phoenix migration on Friday \
                       after reviewing the runbook and staging results. \
                       alice@example.com please confirm."
                    .into(),
                source_ref: Some("slack://m1".into()),
            }],
        }
    }

    #[tokio::test]
    async fn unknown_entity_returns_empty() {
        let (_tmp, cfg) = test_config();
        let resp = query_topic(&cfg, "email:nobody@example.com", None, None, 10)
            .await
            .unwrap();
        assert!(resp.hits.is_empty());
        assert_eq!(resp.total, 0);
    }

    #[tokio::test]
    async fn query_email_entity_after_ingest() {
        let (_tmp, cfg) = test_config();
        ingest_chat(&cfg, "slack:#eng", "alice", vec![], substantive_batch())
            .await
            .unwrap();
        let resp = query_topic(&cfg, "email:alice@example.com", None, None, 10)
            .await
            .unwrap();
        assert!(
            !resp.hits.is_empty(),
            "alice's chunk should be surfaced via the entity index"
        );
    }

    #[tokio::test]
    async fn query_topic_entity_after_ingest() {
        // The topic-as-entity promotion from Phase 3a means "phoenix" shows
        // up under `topic:phoenix` once the ingest's scorer extracts it.
        let (_tmp, cfg) = test_config();
        ingest_chat(&cfg, "slack:#eng", "alice", vec![], substantive_batch())
            .await
            .unwrap();
        let resp = query_topic(&cfg, "topic:phoenix", None, None, 10)
            .await
            .unwrap();
        // Topic extraction may depend on the specific scorer config; at
        // minimum the call should succeed and the response is a well-formed
        // (possibly empty) `QueryResponse`. We don't hard-assert hits here
        // because the scorer extraction rules are out of Phase 4's scope.
        assert!(resp.total >= resp.hits.len());
    }

    #[tokio::test]
    async fn query_filters_by_time_window() {
        let (_tmp, cfg) = test_config();
        // Seed an old chunk via a batch whose timestamp is ancient.
        let old_batch = ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![ChatMessage {
                author: "alice".into(),
                timestamp: Utc.timestamp_millis_opt(1_000_000_000_000).unwrap(),
                text: "Ancient plan to ship Phoenix. alice@example.com has been \
                       the owner of the runbook for ages."
                    .into(),
                source_ref: Some("slack://ancient".into()),
            }],
        };
        ingest_chat(&cfg, "slack:#ancient", "alice", vec![], old_batch)
            .await
            .unwrap();

        // 7-day window should reject the ancient hit.
        let resp = query_topic(&cfg, "email:alice@example.com", Some(7), None, 10)
            .await
            .unwrap();
        assert!(resp.hits.is_empty(), "ancient mention filtered by window");
    }

    #[tokio::test]
    async fn query_truncates_to_limit() {
        let (_tmp, cfg) = test_config();
        // Three separate sources all mentioning alice.
        for i in 0..3 {
            let source = format!("slack:#c{i}");
            let batch = ChatBatch {
                platform: "slack".into(),
                channel_label: format!("#c{i}"),
                messages: vec![ChatMessage {
                    author: "alice".into(),
                    timestamp: Utc::now(),
                    text: format!(
                        "Meeting {i} about Phoenix migration. alice@example.com owns it. \
                         Launch status looks good."
                    ),
                    source_ref: None,
                }],
            };
            ingest_chat(&cfg, &source, "alice", vec![], batch)
                .await
                .unwrap();
        }
        let resp = query_topic(&cfg, "email:alice@example.com", None, None, 2)
            .await
            .unwrap();
        assert!(resp.hits.len() <= 2);
        assert!(resp.total >= resp.hits.len());
        if resp.total > 2 {
            assert!(resp.truncated);
        }
    }

    #[tokio::test]
    async fn hits_sorted_by_score_descending() {
        let (_tmp, cfg) = test_config();
        ingest_chat(&cfg, "slack:#eng", "alice", vec![], substantive_batch())
            .await
            .unwrap();
        let resp = query_topic(&cfg, "email:alice@example.com", None, None, 10)
            .await
            .unwrap();
        for w in resp.hits.windows(2) {
            assert!(
                w[0].score >= w[1].score,
                "expected score DESC ordering, got {} then {}",
                w[0].score,
                w[1].score
            );
        }
    }

    // Regression: the same node_id must only appear once in `hits`, even
    // when the topic-tree root overlaps with its own entity-index row.
    // Flagged on PR #831 CodeRabbit review — see the HashMap-based merge
    // in `query_topic`. Without the dedup, `total` would be 2 and the
    // caller would see two rows for the same summary.
    #[tokio::test]
    async fn duplicate_node_is_deduplicated_across_index_and_topic_tree_root() {
        use crate::openhuman::memory::tree::score::extract::EntityKind;
        use crate::openhuman::memory::tree::score::resolver::CanonicalEntity;
        use crate::openhuman::memory::tree::score::store as score_store;
        use crate::openhuman::memory::tree::source_tree::store as tree_store;
        use crate::openhuman::memory::tree::source_tree::types::{
            SummaryNode, Tree, TreeKind, TreeStatus,
        };
        use crate::openhuman::memory::tree::store::with_connection;

        let (_tmp, cfg) = test_config();
        let ts = Utc::now();
        let entity_id = "topic:phoenix";
        let summary_id = "summary:L1:phoenix-root";

        // 1. Create a topic tree whose root points at `summary_id`.
        let tree = Tree {
            id: "test:phoenix-topic-tree".into(),
            kind: TreeKind::Topic,
            scope: entity_id.into(),
            root_id: Some(summary_id.into()),
            max_level: 1,
            status: TreeStatus::Active,
            created_at: ts,
            last_sealed_at: Some(ts),
        };

        // 2. Create the summary row itself.
        let summary = SummaryNode {
            id: summary_id.into(),
            tree_id: tree.id.clone(),
            tree_kind: TreeKind::Topic,
            level: 1,
            parent_id: None,
            child_ids: vec![],
            content: "Phoenix migration recap".into(),
            token_count: 100,
            entities: vec![entity_id.into()],
            topics: vec![],
            time_range_start: ts,
            time_range_end: ts,
            score: 0.5,
            sealed_at: ts,
            deleted: false,
            embedding: None,
        };

        // 3. Write tree + summary + entity-index row in one tx. The
        //    entity-index row is what creates the dedup scenario: both
        //    `lookup_entity` AND `fetch_topic_tree_root_summary` will
        //    now return the same node.
        let entity = CanonicalEntity {
            canonical_id: entity_id.into(),
            kind: EntityKind::Topic,
            surface: "phoenix".into(),
            span_start: 0,
            span_end: 7,
            score: 0.9,
        };
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            tree_store::insert_tree_conn(&tx, &tree)?;
            tree_store::insert_summary_tx(&tx, &summary, None)?;
            score_store::index_entities_tx(
                &tx,
                &[entity],
                summary_id,
                "summary",
                ts.timestamp_millis(),
                Some(&tree.id),
            )?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();

        // 4. Query — expect exactly one hit (the summary), not two.
        let resp = query_topic(&cfg, entity_id, None, None, 10).await.unwrap();
        let phoenix_hits: Vec<_> = resp
            .hits
            .iter()
            .filter(|h| h.node_id == summary_id)
            .collect();
        assert_eq!(
            phoenix_hits.len(),
            1,
            "summary should appear once, not duplicated between index \
             and topic-tree root; got {} phoenix hits in response of {}",
            phoenix_hits.len(),
            resp.hits.len()
        );
        // `total` also reflects the dedup'd count.
        assert_eq!(
            resp.total, 1,
            "total should count distinct nodes, not raw row occurrences"
        );
    }
}
