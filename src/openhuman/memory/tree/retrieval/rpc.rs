//! JSON-RPC handler bodies for Phase 4 retrieval tools (#710).
//!
//! Each handler is a thin wrapper around its `retrieval::<tool>` function.
//! Shapes mirror the internal API — in particular, `QueryResponse` and
//! `Vec<RetrievalHit>` / `Vec<EntityMatch>` all serialise directly without
//! an extra envelope.

use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::retrieval::{
    drill_down::drill_down,
    fetch::fetch_leaves,
    global::query_global,
    search::search_entities,
    source::query_source,
    topic::query_topic,
    types::{EntityMatch, QueryResponse, RetrievalHit},
};
use crate::openhuman::memory::tree::score::extract::EntityKind;
use crate::openhuman::memory::tree::types::SourceKind;
use crate::rpc::RpcOutcome;

// ── query_source ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QuerySourceRequest {
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub time_window_days: Option<u32>,
    /// Phase 4 (#710) — optional natural-language query string. When
    /// provided, candidates are reranked by cosine similarity to the
    /// query's embedding rather than sorted by recency. Legacy rows
    /// with no stored embedding fall to the bottom.
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub async fn query_source_rpc(
    config: &Config,
    req: QuerySourceRequest,
) -> Result<RpcOutcome<QueryResponse>, String> {
    let source_kind = match req.source_kind.as_deref() {
        Some(s) => Some(SourceKind::parse(s)?),
        None => None,
    };
    let limit = req.limit.unwrap_or(0);
    let resp = query_source(
        config,
        req.source_id.as_deref(),
        source_kind,
        req.time_window_days,
        req.query.as_deref(),
        limit,
    )
    .await
    .map_err(|e| format!("query_source: {e}"))?;
    let n = resp.hits.len();
    Ok(RpcOutcome::single_log(
        resp,
        format!("memory_tree: query_source hits={n}"),
    ))
}

// ── query_global ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryGlobalRequest {
    pub window_days: u32,
}

pub async fn query_global_rpc(
    config: &Config,
    req: QueryGlobalRequest,
) -> Result<RpcOutcome<QueryResponse>, String> {
    let resp = query_global(config, req.window_days)
        .await
        .map_err(|e| format!("query_global: {e}"))?;
    let n = resp.hits.len();
    Ok(RpcOutcome::single_log(
        resp,
        format!("memory_tree: query_global hits={n}"),
    ))
}

// ── query_topic ───────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryTopicRequest {
    pub entity_id: String,
    #[serde(default)]
    pub time_window_days: Option<u32>,
    /// Phase 4 (#710) — optional natural-language query for semantic
    /// rerank. When unset, falls back to the classic score DESC order.
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub async fn query_topic_rpc(
    config: &Config,
    req: QueryTopicRequest,
) -> Result<RpcOutcome<QueryResponse>, String> {
    let limit = req.limit.unwrap_or(0);
    let resp = query_topic(
        config,
        &req.entity_id,
        req.time_window_days,
        req.query.as_deref(),
        limit,
    )
    .await
    .map_err(|e| format!("query_topic: {e}"))?;
    let n = resp.hits.len();
    Ok(RpcOutcome::single_log(
        resp,
        format!(
            "memory_tree: query_topic entity_id={} hits={}",
            req.entity_id, n
        ),
    ))
}

// ── search_entities ───────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchEntitiesRequest {
    pub query: String,
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchEntitiesResponse {
    pub matches: Vec<EntityMatch>,
}

pub async fn search_entities_rpc(
    config: &Config,
    req: SearchEntitiesRequest,
) -> Result<RpcOutcome<SearchEntitiesResponse>, String> {
    let kinds = match req.kinds {
        None => None,
        Some(list) => {
            let parsed: Result<Vec<EntityKind>, String> =
                list.iter().map(|s| EntityKind::parse(s)).collect();
            Some(parsed?)
        }
    };
    let limit = req.limit.unwrap_or(0);
    let matches = search_entities(config, &req.query, kinds, limit)
        .await
        .map_err(|e| format!("search_entities: {e}"))?;
    let n = matches.len();
    Ok(RpcOutcome::single_log(
        SearchEntitiesResponse { matches },
        format!("memory_tree: search_entities query={:?} n={}", req.query, n),
    ))
}

// ── drill_down ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrillDownRequest {
    pub node_id: String,
    #[serde(default)]
    pub max_depth: Option<u32>,
    /// When set, visited children are reranked by cosine similarity between
    /// the query embedding and each child's stored embedding. Legacy children
    /// without an embedding sort to the bottom.
    #[serde(default)]
    pub query: Option<String>,
    /// Optional cap on the returned hit count, applied AFTER rerank so the
    /// top-K is relevance-based when `query` is provided.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrillDownResponse {
    pub hits: Vec<RetrievalHit>,
}

pub async fn drill_down_rpc(
    config: &Config,
    req: DrillDownRequest,
) -> Result<RpcOutcome<DrillDownResponse>, String> {
    let depth = req.max_depth.unwrap_or(1);
    let hits = drill_down(config, &req.node_id, depth, req.query.as_deref(), req.limit)
        .await
        .map_err(|e| format!("drill_down: {e}"))?;
    let n = hits.len();
    Ok(RpcOutcome::single_log(
        DrillDownResponse { hits },
        format!(
            "memory_tree: drill_down node_id={} depth={} query={} limit={:?} n={}",
            req.node_id,
            depth,
            req.query.is_some(),
            req.limit,
            n
        ),
    ))
}

// ── fetch_leaves ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FetchLeavesRequest {
    pub chunk_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FetchLeavesResponse {
    pub hits: Vec<RetrievalHit>,
}

pub async fn fetch_leaves_rpc(
    config: &Config,
    req: FetchLeavesRequest,
) -> Result<RpcOutcome<FetchLeavesResponse>, String> {
    let hits = fetch_leaves(config, &req.chunk_ids)
        .await
        .map_err(|e| format!("fetch_leaves: {e}"))?;
    let n = hits.len();
    Ok(RpcOutcome::single_log(
        FetchLeavesResponse { hits },
        format!("memory_tree: fetch_leaves n={n}"),
    ))
}
