//! `fast_retrieve` — deterministic, LLM-free memory retrieval (E2GraphRAG).
//!
//! This replaces the agentic `walk` / `smart_walk` loops. Instead of an LLM
//! navigating the summary tree turn-by-turn, retrieval routing is decided
//! purely by spaCy query entities + co-occurrence-graph hop distance:
//!
//! 1. Extract query entities `Eq` (spaCy, with regex fallback).
//! 2. `Eq` empty → **global**: dense rerank over the summary tree.
//! 3. Otherwise compute related entity pairs `Ph` within `h` hops.
//!    - `Ph` empty → **global with occurrence ranking**: dense top-2k, then
//!      re-rank by how many `Eq` entities each summary mentions.
//!    - `Ph` non-empty → **local (index mapping)**: intersect the entity-index
//!      node sets of each related pair; tighten `h` while the candidate set is
//!      larger than `k`; rank survivors by entity coverage then recency.
//!
//! Output is a structured [`QueryResponse`] of [`RetrievalHit`]s (no
//! synthesized prose) for a higher-level context agent to consume.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory::source_scope::current_source_scope;
use crate::openhuman::memory_store::content::read as content_read;
use crate::openhuman::memory_store::trees::store as tree_store;
use crate::openhuman::memory_tree::graph;
use crate::openhuman::memory_tree::nlp;
use crate::openhuman::memory_tree::retrieval::fetch::{self, fetch_leaves};
use crate::openhuman::memory_tree::retrieval::source::query_source;
use crate::openhuman::memory_tree::retrieval::types::{
    hit_from_summary, QueryResponse, RetrievalHit,
};
use crate::openhuman::memory_tree::score::store::lookup_entity;

/// Tunables for [`fast_retrieve`]. Defaults match the E2GraphRAG paper's
/// small-`k` regime and a 2-hop relatedness threshold.
#[derive(Clone, Debug)]
pub struct FastRetrieveOptions {
    /// `k` — max hits returned.
    pub limit: usize,
    /// `h` — initial hop threshold for relatedness. Tightened (decremented)
    /// during the local branch when too many candidates match.
    pub max_hops: u32,
    /// Look-back window (days) applied on the global/dense branch. `None` =
    /// unbounded.
    pub time_window_days: Option<u32>,
}

impl Default for FastRetrieveOptions {
    fn default() -> Self {
        Self {
            limit: 10,
            max_hops: 2,
            time_window_days: None,
        }
    }
}

/// Per-node-entity lookup cap. Popular entities can touch many nodes; this
/// bounds the intersection work while staying well above any realistic `k`.
const LOOKUP_LIMIT: usize = 500;

/// Default / ceiling for `limit` (`k`). The tool and RPC paths expose this, so
/// a huge value must not be able to request oversized dense/local result sets.
const DEFAULT_LIMIT: usize = 10;
const MAX_RETRIEVE_LIMIT: usize = 100;
/// Default / ceiling for the hop threshold. A large `max_hops` would force many
/// bounded-BFS passes through the tightening loop; cap it.
const DEFAULT_MAX_HOPS: u32 = 2;
const MAX_GRAPH_HOPS: u32 = 4;

/// Run deterministic retrieval for `query`. Never invokes an LLM.
pub async fn fast_retrieve(
    config: &Config,
    query: &str,
    opts: FastRetrieveOptions,
) -> Result<QueryResponse> {
    let k = match opts.limit {
        0 => DEFAULT_LIMIT,
        n => n.min(MAX_RETRIEVE_LIMIT),
    };
    let max_hops = match opts.max_hops {
        0 => DEFAULT_MAX_HOPS,
        n => n.min(MAX_GRAPH_HOPS),
    };
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(QueryResponse::empty());
    }

    // 1. Query entities.
    let entities = nlp::extract_query_entities(config, trimmed).await;
    let eq: Vec<String> = dedup_ids(entities.iter().map(|e| e.canonical_id.clone()));
    log::debug!(
        "[retrieval::fast] query_len={} eq={} k={} h={}",
        trimmed.len(),
        eq.len(),
        k,
        max_hops
    );

    // 2. No entities → pure global dense retrieval.
    if eq.is_empty() {
        log::debug!("[retrieval::fast] branch=global_dense (no query entities)");
        return query_source(config, None, None, opts.time_window_days, Some(trimmed), k).await;
    }

    // 3. Graph filter: related entity pairs within `h` hops.
    let pairs = graph::pair_distances(config, &eq, max_hops)?;
    if pairs.is_empty() {
        log::debug!("[retrieval::fast] branch=global_occurrence (no related pairs)");
        return global_occurrence(config, trimmed, &eq, k, opts.time_window_days).await;
    }

    // 4. Local branch: index-mapping intersection with `h` tightening.
    let mut h = max_hops;
    let mut cands = local_candidates(config, &eq, &pairs)?;
    let mut last_non_empty = cands.clone();
    while cands.len() > k && h > 1 {
        h -= 1;
        let tightened = graph::pair_distances(config, &eq, h)?;
        let next = local_candidates(config, &eq, &tightened)?;
        if next.is_empty() {
            // Tightening removed everything — keep the looser, non-empty set.
            break;
        }
        last_non_empty = next.clone();
        cands = next;
    }
    if cands.is_empty() {
        cands = last_non_empty;
    }

    if cands.is_empty() {
        // Related pairs existed but never co-occurred in an indexed node
        // (e.g. only 2-hop links). Fall back to global occurrence ranking.
        log::debug!("[retrieval::fast] branch=local->global_occurrence (empty intersection)");
        return global_occurrence(config, trimmed, &eq, k, opts.time_window_days).await;
    }

    log::debug!(
        "[retrieval::fast] branch=local candidates={} final_h={}",
        cands.len(),
        h
    );
    resolve_local(config, cands, k).await
}

/// Candidate node accumulated during the local branch.
#[derive(Clone, Debug)]
struct Candidate {
    node_kind: String,
    /// Distinct query entities that landed on this node (coverage signal).
    matched: HashSet<String>,
    latest_ts: i64,
}

/// Intersect the entity-index node sets of each related pair and union the
/// results. A node enters the candidate set when it is indexed against *both*
/// members of some related pair.
fn local_candidates(
    config: &Config,
    eq: &[String],
    pairs: &[graph::PairDistance],
) -> Result<HashMap<String, Candidate>> {
    let _ = eq; // `eq` documents intent; matched entities come from the pairs.
    let mut out: HashMap<String, Candidate> = HashMap::new();
    for pair in pairs {
        let hits_a = lookup_entity(config, &pair.a, Some(LOOKUP_LIMIT))?;
        if hits_a.is_empty() {
            continue;
        }
        let hits_b = lookup_entity(config, &pair.b, Some(LOOKUP_LIMIT))?;
        if hits_b.is_empty() {
            continue;
        }
        let b_nodes: HashMap<&str, i64> = hits_b
            .iter()
            .map(|h| (h.node_id.as_str(), h.timestamp_ms))
            .collect();
        for ha in &hits_a {
            let Some(&b_ts) = b_nodes.get(ha.node_id.as_str()) else {
                continue;
            };
            let entry = out.entry(ha.node_id.clone()).or_insert_with(|| Candidate {
                node_kind: ha.node_kind.clone(),
                matched: HashSet::new(),
                latest_ts: 0,
            });
            entry.matched.insert(pair.a.clone());
            entry.matched.insert(pair.b.clone());
            entry.latest_ts = entry.latest_ts.max(ha.timestamp_ms).max(b_ts);
        }
    }
    Ok(out)
}

/// Rank candidates by entity coverage (desc) then recency (desc), resolve to
/// hits, apply the profile source-scope gate **before** counting/truncating
/// (so an out-of-scope top hit never displaces an allowed lower-ranked one,
/// and `total` never reveals out-of-scope match counts), then truncate to `k`.
async fn resolve_local(
    config: &Config,
    cands: HashMap<String, Candidate>,
    k: usize,
) -> Result<QueryResponse> {
    let mut ordered: Vec<(String, Candidate)> = cands.into_iter().collect();
    ordered.sort_by(|a, b| {
        b.1.matched
            .len()
            .cmp(&a.1.matched.len())
            .then_with(|| b.1.latest_ts.cmp(&a.1.latest_ts))
            .then_with(|| a.0.cmp(&b.0))
    });

    // Coverage score per node id so resolved hits carry the ranking signal.
    let coverage: HashMap<String, f32> = ordered
        .iter()
        .map(|(id, c)| (id.clone(), c.matched.len() as f32))
        .collect();

    let leaf_ids: Vec<String> = ordered
        .iter()
        .filter(|(_, c)| c.node_kind == "leaf")
        .map(|(id, _)| id.clone())
        .collect();
    let summary_ids: Vec<String> = ordered
        .iter()
        .filter(|(_, c)| c.node_kind != "leaf")
        .map(|(id, _)| id.clone())
        .collect();

    let mut by_id: HashMap<String, RetrievalHit> = HashMap::new();
    // `fetch_leaves` caps each batch at MAX_BATCH and would silently drop the
    // rest, so chunk to resolve every candidate leaf before the scope gate.
    for chunk in leaf_ids.chunks(fetch::MAX_BATCH) {
        for hit in fetch_leaves(config, chunk).await? {
            by_id.insert(hit.node_id.clone(), hit);
        }
    }
    if !summary_ids.is_empty() {
        for hit in resolve_summaries(config, &summary_ids)? {
            by_id.insert(hit.node_id.clone(), hit);
        }
    }

    // Scope-gate the full ranked set first; `total` reflects only in-scope hits.
    let scope = current_source_scope();
    let mut hits: Vec<RetrievalHit> = Vec::with_capacity(ordered.len());
    for (id, _) in &ordered {
        if let Some(mut hit) = by_id.remove(id) {
            if !scope_allows(scope.as_ref(), &hit.tree_scope) {
                continue;
            }
            if let Some(score) = coverage.get(id) {
                hit.score = *score;
            }
            hits.push(hit);
        }
    }
    let total = hits.len();
    hits.truncate(k);
    Ok(QueryResponse::new(hits, total))
}

/// Resolve summary node ids to hits, hydrating the full body from disk and
/// threading the owning tree's scope for provenance.
fn resolve_summaries(config: &Config, summary_ids: &[String]) -> Result<Vec<RetrievalHit>> {
    let nodes = tree_store::get_summaries_batch(config, summary_ids)?;
    let mut scope_cache: HashMap<String, String> = HashMap::new();
    let mut out = Vec::with_capacity(nodes.len());
    for (_, mut node) in nodes {
        // Hydrate full body (the `content` column is a ≤500-char preview).
        if let Ok(body) = content_read::read_summary_body(config, &node.id) {
            node.content = body;
        }
        let scope = match scope_cache.get(&node.tree_id) {
            Some(s) => s.clone(),
            None => {
                let s = tree_store::get_tree(config, &node.tree_id)
                    .ok()
                    .flatten()
                    .map(|t| t.scope)
                    .unwrap_or_default();
                scope_cache.insert(node.tree_id.clone(), s.clone());
                s
            }
        };
        out.push(hit_from_summary(&node, &scope));
    }
    Ok(out)
}

/// Global branch with occurrence ranking: dense-retrieve top-`2k`, then re-rank
/// by how many `Eq` entities each summary mentions (occurrence). Stable sort
/// preserves the semantic order on ties.
async fn global_occurrence(
    config: &Config,
    query: &str,
    eq: &[String],
    k: usize,
    window: Option<u32>,
) -> Result<QueryResponse> {
    let resp = query_source(config, None, None, window, Some(query), k.saturating_mul(2)).await?;
    let eq_set: HashSet<&str> = eq.iter().map(|s| s.as_str()).collect();
    let mut hits = resp.hits;
    let total = resp.total;
    hits.sort_by_key(|h| {
        let occ = h
            .entities
            .iter()
            .filter(|e| eq_set.contains(e.as_str()))
            .count();
        Reverse(occ)
    });
    hits.truncate(k);
    Ok(QueryResponse::new(hits, total))
}

/// Deduplicate ids preserving first-seen order.
fn dedup_ids(ids: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for id in ids {
        if seen.insert(id.clone()) {
            out.push(id);
        }
    }
    out
}

/// Profile source-scope gate: `None` = unrestricted; otherwise the scope must
/// be on the allowlist. Empty scopes (unknown provenance) are allowed through
/// only when unrestricted.
fn scope_allows(scope: Option<&HashSet<String>>, tree_scope: &str) -> bool {
    match scope {
        None => true,
        Some(set) => set.contains(tree_scope),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::ingest_pipeline::ingest_chat;
    use crate::openhuman::memory_sync::canonicalize::chat::{ChatBatch, ChatMessage};
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        // spaCy off in CI — exercise the regex fallback + graph routing
        // deterministically without a Python runtime.
        cfg.memory_tree.spacy_enabled = false;
        (tmp, cfg)
    }

    async fn seed_chat(cfg: &Config, source: &str, text: &str) {
        let batch = ChatBatch {
            platform: "slack".into(),
            channel_label: source.into(),
            messages: vec![ChatMessage {
                author: "alice".into(),
                timestamp: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
                text: text.into(),
                source_ref: Some("slack://x".into()),
            }],
        };
        ingest_chat(cfg, source, "alice", vec![], batch)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn empty_query_returns_empty() {
        let (_tmp, cfg) = test_config();
        let resp = fast_retrieve(&cfg, "  ", FastRetrieveOptions::default())
            .await
            .unwrap();
        assert!(resp.hits.is_empty());
    }

    #[tokio::test]
    async fn no_entities_routes_global_without_panicking() {
        let (_tmp, cfg) = test_config();
        // A query with no mechanical entities (regex fallback finds nothing)
        // must take the global branch and return cleanly on an empty store.
        let resp = fast_retrieve(
            &cfg,
            "what happened recently",
            FastRetrieveOptions::default(),
        )
        .await
        .unwrap();
        assert!(resp.hits.is_empty());
    }

    #[tokio::test]
    async fn local_branch_finds_cooccurring_entities() {
        let (_tmp, cfg) = test_config();
        // Two emails co-occur in one message → an edge + both indexed on the
        // same leaf. A query mentioning both routes local and returns the leaf.
        seed_chat(
            &cfg,
            "slack:#eng",
            "Sync between alice@example.com and bob@example.com about the runbook.",
        )
        .await;

        let resp = fast_retrieve(
            &cfg,
            "alice@example.com and bob@example.com",
            FastRetrieveOptions::default(),
        )
        .await
        .unwrap();
        assert!(
            !resp.hits.is_empty(),
            "co-occurring entities should yield a local hit; got {resp:?}"
        );
        // Coverage score = 2 distinct query entities matched the leaf.
        assert!(resp.hits.iter().any(|h| h.score >= 2.0));
    }

    #[test]
    fn scope_allows_respects_allowlist() {
        assert!(scope_allows(None, "slack:#eng"));
        let mut set = HashSet::new();
        set.insert("slack:#eng".to_string());
        assert!(scope_allows(Some(&set), "slack:#eng"));
        assert!(!scope_allows(Some(&set), "gmail:alice@example.com"));
    }

    #[test]
    fn dedup_ids_preserves_first_seen_order() {
        let out = dedup_ids(["a".to_string(), "b".into(), "a".into(), "c".into()].into_iter());
        assert_eq!(out, vec!["a", "b", "c"]);
    }
}
