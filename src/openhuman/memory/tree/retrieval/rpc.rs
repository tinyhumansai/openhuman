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
        Some(s) => Some(SourceKind::parse(s).map_err(|e| format!("query_source: {e}"))?),
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
    // Omit scope / source_id from the log — can carry PII. Log counts only.
    Ok(RpcOutcome::single_log(
        resp,
        format!(
            "memory_tree: query_source has_source_id={} source_kind={:?} has_query={} hits={}",
            req.source_id.is_some(),
            req.source_kind,
            req.query.is_some(),
            n
        ),
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
    // entity_id can be an email or handle — log only the kind prefix
    // ("email:", "handle:", etc.) not the full value.
    let entity_kind_prefix = req
        .entity_id
        .split_once(':')
        .map(|(k, _)| k)
        .unwrap_or("unknown");
    Ok(RpcOutcome::single_log(
        resp,
        format!(
            "memory_tree: query_topic entity_kind={} has_query={} hits={}",
            entity_kind_prefix,
            req.query.is_some(),
            n
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
    // Capture logging-friendly summary BEFORE we move fields out of `req`.
    let query_len = req.query.len();
    let has_kinds = req.kinds.is_some();
    let kinds = match req.kinds {
        None => None,
        Some(list) => {
            let parsed: Result<Vec<EntityKind>, String> = list
                .iter()
                .map(|s| EntityKind::parse(s).map_err(|e| format!("search_entities: {e}")))
                .collect();
            Some(parsed?)
        }
    };
    let limit = req.limit.unwrap_or(0);
    let matches = search_entities(config, &req.query, kinds, limit)
        .await
        .map_err(|e| format!("search_entities: {e}"))?;
    let n = matches.len();
    // Don't log the raw search query — can be an email, handle, etc. Log
    // only its length and the kind filter.
    Ok(RpcOutcome::single_log(
        SearchEntitiesResponse { matches },
        format!("memory_tree: search_entities query_len={query_len} has_kinds={has_kinds} n={n}"),
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
    // node_id can embed source scope (e.g. "chat:slack:#eng:0") which may
    // carry workspace hints — log only the structural prefix.
    let node_kind_prefix = req
        .node_id
        .split_once(':')
        .map(|(k, _)| k)
        .unwrap_or("unknown");
    Ok(RpcOutcome::single_log(
        DrillDownResponse { hits },
        format!(
            "memory_tree: drill_down node_kind={} depth={} has_query={} limit={:?} n={}",
            node_kind_prefix,
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

#[cfg(test)]
mod tests {
    //! Unit tests for the Phase 4 retrieval RPC handlers.
    //!
    //! Scope: the handler layer specifically — param parsing, default
    //! fallbacks, `SourceKind` / `EntityKind` validation, `RpcOutcome`
    //! envelope shape, and PII-redacted log formatting. Deeper domain
    //! behaviour is already covered by the per-module tests in
    //! `source.rs`, `topic.rs`, `drill_down.rs`, etc. — these tests
    //! intentionally do NOT re-verify retrieval correctness.
    //!
    //! All tests run against a fresh empty workspace. `with_connection`
    //! initialises the schema idempotently on first access, so read-only
    //! calls return empty responses rather than erroring.
    use super::*;
    use crate::openhuman::memory::tree::content_store;
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceRef};
    use chrono::{TimeZone, Utc};
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
        // Phase 4 (#710): inert embedder keeps tests deterministic and
        // avoids any real Ollama call.
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    fn sample_chunk(source: &str, seq: u32) -> Chunk {
        let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        Chunk {
            id: chunk_id(SourceKind::Chat, source, seq, "test-content"),
            content: format!("content-{source}-{seq}"),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: source.into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new(format!("slack://{source}/{seq}"))),
            },
            token_count: 20,
            seq_in_source: seq,
            created_at: ts,
            partial_message: false,
        }
    }

    // ── query_source_rpc ──────────────────────────────────────────────

    #[tokio::test]
    async fn query_source_rpc_returns_hits_with_no_filters() {
        let (_tmp, cfg) = test_config();
        let outcome = query_source_rpc(&cfg, QuerySourceRequest::default())
            .await
            .unwrap();
        assert!(outcome.value.hits.is_empty());
        assert_eq!(outcome.value.total, 0);
        assert_eq!(outcome.logs.len(), 1);
        let log = &outcome.logs[0];
        assert!(log.contains("has_source_id=false"), "log: {log}");
        assert!(log.contains("source_kind=None"), "log: {log}");
        assert!(log.contains("has_query=false"), "log: {log}");
        assert!(log.contains("hits=0"), "log: {log}");
    }

    #[tokio::test]
    async fn query_source_rpc_parses_valid_source_kind_and_limit() {
        let (_tmp, cfg) = test_config();
        let req = QuerySourceRequest {
            source_id: Some("slack:#eng".into()),
            source_kind: Some("chat".into()),
            time_window_days: None,
            query: None,
            limit: Some(5),
        };
        let outcome = query_source_rpc(&cfg, req).await.unwrap();
        assert!(outcome.value.hits.is_empty());
        let log = &outcome.logs[0];
        assert!(log.contains("has_source_id=true"), "log: {log}");
        assert!(log.contains("source_kind=Some(\"chat\")"), "log: {log}");
        // PII redaction: the raw source_id must NOT leak into the log.
        assert!(!log.contains("slack:#eng"), "log leaked source_id: {log}");
    }

    #[tokio::test]
    async fn query_source_rpc_rejects_invalid_source_kind() {
        let (_tmp, cfg) = test_config();
        let req = QuerySourceRequest {
            source_id: None,
            source_kind: Some("bogus".into()),
            time_window_days: None,
            query: None,
            limit: None,
        };
        let err = query_source_rpc(&cfg, req).await.unwrap_err();
        assert!(err.contains("unknown source kind: bogus"), "got {err}");
    }

    // ── query_global_rpc ──────────────────────────────────────────────

    #[tokio::test]
    async fn query_global_rpc_returns_response_for_valid_window() {
        let (_tmp, cfg) = test_config();
        let req = QueryGlobalRequest { window_days: 7 };
        let outcome = query_global_rpc(&cfg, req).await.unwrap();
        assert!(outcome.value.hits.is_empty());
        assert_eq!(outcome.logs.len(), 1);
        assert!(
            outcome.logs[0].contains("query_global hits=0"),
            "log: {}",
            outcome.logs[0]
        );
    }

    // ── query_topic_rpc ───────────────────────────────────────────────

    #[tokio::test]
    async fn query_topic_rpc_logs_entity_kind_prefix_for_colon_separated_id() {
        let (_tmp, cfg) = test_config();
        let req = QueryTopicRequest {
            entity_id: "email:alice@example.com".into(),
            time_window_days: None,
            query: None,
            limit: None,
        };
        let outcome = query_topic_rpc(&cfg, req).await.unwrap();
        let log = &outcome.logs[0];
        assert!(log.contains("entity_kind=email"), "log: {log}");
        // PII redaction — the raw email must NOT appear anywhere in the log.
        assert!(!log.contains("alice@example.com"), "log leaked PII: {log}");
    }

    #[tokio::test]
    async fn query_topic_rpc_logs_unknown_when_entity_id_has_no_colon() {
        let (_tmp, cfg) = test_config();
        let req = QueryTopicRequest {
            entity_id: "nocolonhere".into(),
            time_window_days: None,
            query: None,
            limit: None,
        };
        let outcome = query_topic_rpc(&cfg, req).await.unwrap();
        assert!(
            outcome.logs[0].contains("entity_kind=unknown"),
            "log: {}",
            outcome.logs[0]
        );
    }

    // ── search_entities_rpc ───────────────────────────────────────────

    #[tokio::test]
    async fn search_entities_rpc_passes_through_kinds_none() {
        let (_tmp, cfg) = test_config();
        let req = SearchEntitiesRequest {
            query: "alice".into(),
            kinds: None,
            limit: None,
        };
        let outcome = search_entities_rpc(&cfg, req).await.unwrap();
        assert!(outcome.value.matches.is_empty());
        let log = &outcome.logs[0];
        assert!(log.contains("query_len=5"), "log: {log}");
        assert!(log.contains("has_kinds=false"), "log: {log}");
        // PII redaction — the raw query value must NOT appear in the log.
        assert!(!log.contains("alice"), "log leaked raw query: {log}");
    }

    #[tokio::test]
    async fn search_entities_rpc_parses_valid_kinds_list() {
        let (_tmp, cfg) = test_config();
        let req = SearchEntitiesRequest {
            query: "x".into(),
            kinds: Some(vec!["email".into(), "topic".into()]),
            limit: Some(10),
        };
        let outcome = search_entities_rpc(&cfg, req).await.unwrap();
        assert!(outcome.value.matches.is_empty());
        assert!(
            outcome.logs[0].contains("has_kinds=true"),
            "log: {}",
            outcome.logs[0]
        );
    }

    #[tokio::test]
    async fn search_entities_rpc_rejects_unknown_entity_kind() {
        let (_tmp, cfg) = test_config();
        let req = SearchEntitiesRequest {
            query: "x".into(),
            kinds: Some(vec!["email".into(), "bogus".into()]),
            limit: None,
        };
        let err = search_entities_rpc(&cfg, req).await.unwrap_err();
        assert!(err.contains("unknown entity kind: bogus"), "got {err}");
    }

    // ── drill_down_rpc ────────────────────────────────────────────────

    #[tokio::test]
    async fn drill_down_rpc_defaults_max_depth_to_one_when_unset() {
        let (_tmp, cfg) = test_config();
        let req = DrillDownRequest {
            node_id: "chat:missing".into(),
            max_depth: None,
            query: None,
            limit: None,
        };
        let outcome = drill_down_rpc(&cfg, req).await.unwrap();
        assert!(
            outcome.logs[0].contains("depth=1"),
            "log: {}",
            outcome.logs[0]
        );
    }

    #[tokio::test]
    async fn drill_down_rpc_logs_node_kind_prefix_for_colon_separated_id() {
        let (_tmp, cfg) = test_config();
        let req = DrillDownRequest {
            node_id: "chat:slack:#eng:0".into(),
            max_depth: Some(2),
            query: None,
            limit: None,
        };
        let outcome = drill_down_rpc(&cfg, req).await.unwrap();
        let log = &outcome.logs[0];
        assert!(log.contains("node_kind=chat"), "log: {log}");
        // PII redaction — scope segments beyond the kind prefix must not leak.
        assert!(!log.contains("slack"), "log leaked scope: {log}");
        assert!(!log.contains("#eng"), "log leaked scope: {log}");
    }

    #[tokio::test]
    async fn drill_down_rpc_logs_unknown_when_node_id_has_no_colon() {
        let (_tmp, cfg) = test_config();
        let req = DrillDownRequest {
            node_id: "rootnode".into(),
            max_depth: None,
            query: None,
            limit: None,
        };
        let outcome = drill_down_rpc(&cfg, req).await.unwrap();
        assert!(
            outcome.logs[0].contains("node_kind=unknown"),
            "log: {}",
            outcome.logs[0]
        );
    }

    // ── fetch_leaves_rpc ──────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_leaves_rpc_returns_empty_response_for_empty_input() {
        let (_tmp, cfg) = test_config();
        let req = FetchLeavesRequest { chunk_ids: vec![] };
        let outcome = fetch_leaves_rpc(&cfg, req).await.unwrap();
        assert!(outcome.value.hits.is_empty());
        assert!(outcome.logs[0].contains("n=0"), "log: {}", outcome.logs[0]);
    }

    #[tokio::test]
    async fn fetch_leaves_rpc_hydrates_valid_ids() {
        let (_tmp, cfg) = test_config();
        let c1 = sample_chunk("slack:#eng", 0);
        let c2 = sample_chunk("slack:#eng", 1);
        upsert_chunks(&cfg, &[c1.clone(), c2.clone()]).unwrap();
        stage_test_chunks(&cfg, &[c1.clone(), c2.clone()]);
        let req = FetchLeavesRequest {
            chunk_ids: vec![c1.id.clone(), c2.id.clone()],
        };
        let outcome = fetch_leaves_rpc(&cfg, req).await.unwrap();
        assert_eq!(outcome.value.hits.len(), 2);
        assert!(outcome.logs[0].contains("n=2"), "log: {}", outcome.logs[0]);
    }

    #[tokio::test]
    async fn fetch_leaves_rpc_skips_missing_ids_silently() {
        let (_tmp, cfg) = test_config();
        let c1 = sample_chunk("slack:#eng", 0);
        upsert_chunks(&cfg, &[c1.clone()]).unwrap();
        stage_test_chunks(&cfg, &[c1.clone()]);
        let req = FetchLeavesRequest {
            chunk_ids: vec![c1.id.clone(), "ghost:nonexistent".into()],
        };
        let outcome = fetch_leaves_rpc(&cfg, req).await.unwrap();
        assert_eq!(outcome.value.hits.len(), 1);
        assert!(outcome.logs[0].contains("n=1"), "log: {}", outcome.logs[0]);
    }
}
