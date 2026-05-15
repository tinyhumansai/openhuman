//! Integration tests for the agentmemory `Memory` backend.
//!
//! Spins up a small axum mock server that mimics the agentmemory REST
//! contract (matches the v0.9.12 wire shapes) and exercises every trait
//! method end-to-end. Tests are kept in `tests/` so they share the public
//! crate surface — they do not reach into private modules.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use openhuman_core::openhuman::config::MemoryConfig;
use openhuman_core::openhuman::memory::store::AgentMemoryBackend;
use openhuman_core::openhuman::memory::traits::{Memory, MemoryCategory, RecallOpts};
use serde::{Deserialize, Serialize};

#[derive(Default, Clone)]
struct MockState {
    memories: Arc<Mutex<Vec<MockMemory>>>,
    next_id: Arc<Mutex<usize>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MockMemory {
    id: String,
    project: Option<String>,
    title: Option<String>,
    content: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    concepts: Vec<String>,
    #[serde(default, rename = "sessionIds")]
    session_ids: Vec<String>,
    #[serde(rename = "updatedAt")]
    updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    score: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RememberRequest {
    project: String,
    title: String,
    content: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    concepts: Vec<String>,
    #[serde(default, rename = "sessionIds")]
    session_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SmartSearchRequest {
    query: String,
    limit: usize,
    #[serde(default)]
    project: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ForgetRequest {
    id: String,
}

#[derive(Debug, Deserialize)]
struct MemoriesQuery {
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    latest: Option<String>,
}

async fn start_mock_server() -> (SocketAddr, MockState) {
    let state = MockState::default();
    let app = Router::new()
        .route(
            "/agentmemory/livez",
            get(|| async { Json(serde_json::json!({"service":"agentmemory","status":"ok"})) }),
        )
        .route(
            "/agentmemory/health",
            get(handle_health).with_state(state.clone()),
        )
        .route(
            "/agentmemory/remember",
            post(handle_remember).with_state(state.clone()),
        )
        .route(
            "/agentmemory/smart-search",
            post(handle_smart_search).with_state(state.clone()),
        )
        .route(
            "/agentmemory/forget",
            post(handle_forget).with_state(state.clone()),
        )
        .route(
            "/agentmemory/memories",
            get(handle_memories).with_state(state.clone()),
        )
        .route(
            "/agentmemory/projects",
            get(handle_projects).with_state(state.clone()),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, state)
}

async fn handle_health(State(state): State<MockState>) -> Json<serde_json::Value> {
    let n = state.memories.lock().unwrap().len();
    Json(serde_json::json!({"memories": n, "status": "ok"}))
}

async fn handle_remember(
    State(state): State<MockState>,
    Json(req): Json<RememberRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut next_id = state.next_id.lock().unwrap();
    *next_id += 1;
    let id = format!("mem_{}", *next_id);
    state.memories.lock().unwrap().push(MockMemory {
        id: id.clone(),
        project: Some(req.project),
        title: Some(req.title),
        content: Some(req.content),
        kind: Some(req.kind),
        concepts: req.concepts,
        session_ids: req.session_ids,
        updated_at: Some("2026-05-14T00:00:00Z".to_string()),
        score: None,
    });
    (StatusCode::CREATED, Json(serde_json::json!({ "id": id })))
}

async fn handle_smart_search(
    State(state): State<MockState>,
    Json(req): Json<SmartSearchRequest>,
) -> Json<serde_json::Value> {
    let memories = state.memories.lock().unwrap();
    let q = req.query.to_lowercase();
    let project = req.project.as_deref();
    let hits: Vec<MockMemory> = memories
        .iter()
        .filter(|m| project.is_none_or(|p| m.project.as_deref() == Some(p)))
        .filter(|m| {
            m.title
                .as_deref()
                .map(|t| t.to_lowercase().contains(&q))
                .unwrap_or(false)
                || m.content
                    .as_deref()
                    .map(|c| c.to_lowercase().contains(&q))
                    .unwrap_or(false)
                || m.concepts.iter().any(|c| c.to_lowercase().contains(&q))
        })
        .take(req.limit)
        .cloned()
        .map(|mut m| {
            m.score = Some(0.9);
            m
        })
        .collect();
    Json(serde_json::json!({ "mode": "compact", "results": hits }))
}

async fn handle_forget(
    State(state): State<MockState>,
    Json(req): Json<ForgetRequest>,
) -> Json<serde_json::Value> {
    let mut memories = state.memories.lock().unwrap();
    let before = memories.len();
    memories.retain(|m| m.id != req.id);
    let forgotten = memories.len() < before;
    Json(serde_json::json!({ "forgotten": forgotten }))
}

async fn handle_memories(
    State(state): State<MockState>,
    Query(q): Query<MemoriesQuery>,
) -> Json<serde_json::Value> {
    let memories = state.memories.lock().unwrap();
    let filtered: Vec<MockMemory> = memories
        .iter()
        .filter(|m| {
            q.project
                .as_deref()
                .is_none_or(|p| m.project.as_deref() == Some(p))
        })
        .cloned()
        .collect();
    Json(serde_json::json!({ "memories": filtered }))
}

async fn handle_projects(State(state): State<MockState>) -> Json<serde_json::Value> {
    use std::collections::BTreeMap;
    let memories = state.memories.lock().unwrap();
    let mut by_project: BTreeMap<String, (usize, Option<String>)> = BTreeMap::new();
    for m in memories.iter() {
        let ns = m.project.clone().unwrap_or_else(|| "default".to_string());
        let entry = by_project.entry(ns).or_insert((0, None));
        entry.0 += 1;
        if entry.1.is_none() {
            entry.1 = m.updated_at.clone();
        }
    }
    let projects: Vec<serde_json::Value> = by_project
        .into_iter()
        .map(|(name, (count, last_updated))| {
            serde_json::json!({
                "name": name,
                "count": count,
                "lastUpdated": last_updated,
            })
        })
        .collect();
    Json(serde_json::json!({ "projects": projects }))
}

fn make_config(addr: SocketAddr) -> MemoryConfig {
    MemoryConfig {
        backend: "agentmemory".to_string(),
        agentmemory_url: Some(format!("http://{addr}")),
        agentmemory_timeout_ms: Some(2_000),
        ..MemoryConfig::default()
    }
}

#[tokio::test]
async fn health_check_passes_against_running_daemon() {
    let (addr, _state) = start_mock_server().await;
    let backend = AgentMemoryBackend::from_config(&make_config(addr)).unwrap();
    assert!(backend.health_check().await);
}

#[tokio::test]
async fn health_check_fails_when_daemon_is_unreachable() {
    let cfg = MemoryConfig {
        backend: "agentmemory".to_string(),
        agentmemory_url: Some("http://127.0.0.1:1".to_string()),
        agentmemory_timeout_ms: Some(500),
        ..MemoryConfig::default()
    };
    let backend = AgentMemoryBackend::from_config(&cfg).unwrap();
    assert!(!backend.health_check().await);
}

#[tokio::test]
async fn store_then_get_round_trip() {
    let (addr, _state) = start_mock_server().await;
    let backend = AgentMemoryBackend::from_config(&make_config(addr)).unwrap();

    backend
        .store(
            "demo",
            "auth-stack",
            "Service uses HMAC bearer tokens; refresh every 24h.",
            MemoryCategory::Core,
            None,
        )
        .await
        .unwrap();

    let entry = backend
        .get("demo", "auth-stack")
        .await
        .unwrap()
        .expect("expected to recall the stored memory");
    assert_eq!(entry.key, "auth-stack");
    assert_eq!(entry.namespace.as_deref(), Some("demo"));
    assert!(entry.content.contains("HMAC"));
    assert_eq!(entry.category, MemoryCategory::Core);
}

#[tokio::test]
async fn store_then_recall_finds_matching_memory() {
    let (addr, _state) = start_mock_server().await;
    let backend = AgentMemoryBackend::from_config(&make_config(addr)).unwrap();

    backend
        .store(
            "demo",
            "k1",
            "hmac bearer auth refresh",
            MemoryCategory::Core,
            None,
        )
        .await
        .unwrap();
    backend
        .store(
            "demo",
            "k2",
            "stripe webhook handling",
            MemoryCategory::Core,
            None,
        )
        .await
        .unwrap();

    let opts = RecallOpts {
        namespace: Some("demo"),
        ..RecallOpts::default()
    };
    let hits = backend.recall("hmac", 10, opts).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].key, "k1");
    assert!(hits[0].score.unwrap() > 0.5);
}

#[tokio::test]
async fn recall_filters_by_session_id_client_side() {
    let (addr, _state) = start_mock_server().await;
    let backend = AgentMemoryBackend::from_config(&make_config(addr)).unwrap();

    backend
        .store(
            "demo",
            "k1",
            "hmac one",
            MemoryCategory::Core,
            Some("ses-1"),
        )
        .await
        .unwrap();
    backend
        .store(
            "demo",
            "k2",
            "hmac two",
            MemoryCategory::Core,
            Some("ses-2"),
        )
        .await
        .unwrap();

    let opts = RecallOpts {
        namespace: Some("demo"),
        session_id: Some("ses-1"),
        ..RecallOpts::default()
    };
    let hits = backend.recall("hmac", 10, opts).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].session_id.as_deref(), Some("ses-1"));
}

#[tokio::test]
async fn recall_min_score_drops_scoreless_and_below_threshold_hits() {
    let (addr, _state) = start_mock_server().await;
    let backend = AgentMemoryBackend::from_config(&make_config(addr)).unwrap();

    backend
        .store("demo", "k", "hmac auth refresh", MemoryCategory::Core, None)
        .await
        .unwrap();

    // Mock always returns score=0.9; a threshold above that should
    // drop the hit. Scoreless rows are not relevant on this path
    // (smart-search hits always carry a score in the mock).
    let opts = RecallOpts {
        namespace: Some("demo"),
        min_score: Some(0.95),
        ..RecallOpts::default()
    };
    let hits = backend.recall("hmac", 10, opts).await.unwrap();
    assert!(
        hits.is_empty(),
        "min_score = 0.95 should drop the 0.9 hit, got {hits:?}"
    );

    let opts_loose = RecallOpts {
        namespace: Some("demo"),
        min_score: Some(0.5),
        ..RecallOpts::default()
    };
    let hits_loose = backend.recall("hmac", 10, opts_loose).await.unwrap();
    assert_eq!(hits_loose.len(), 1);
}

#[tokio::test]
async fn list_with_no_namespace_returns_every_project() {
    let (addr, _state) = start_mock_server().await;
    let backend = AgentMemoryBackend::from_config(&make_config(addr)).unwrap();

    backend
        .store("alpha", "a1", "x", MemoryCategory::Core, None)
        .await
        .unwrap();
    backend
        .store("beta", "b1", "y", MemoryCategory::Core, None)
        .await
        .unwrap();

    let all = backend.list(None, None, None).await.unwrap();
    assert_eq!(all.len(), 2);
    let mut ns: Vec<_> = all
        .iter()
        .map(|e| e.namespace.clone().unwrap_or_default())
        .collect();
    ns.sort();
    assert_eq!(ns, vec!["alpha", "beta"]);
}

#[tokio::test]
async fn list_filters_by_namespace_and_category() {
    let (addr, _state) = start_mock_server().await;
    let backend = AgentMemoryBackend::from_config(&make_config(addr)).unwrap();

    backend
        .store("alpha", "a1", "fact", MemoryCategory::Core, None)
        .await
        .unwrap();
    backend
        .store("alpha", "a2", "convo", MemoryCategory::Conversation, None)
        .await
        .unwrap();
    backend
        .store("beta", "b1", "fact", MemoryCategory::Core, None)
        .await
        .unwrap();

    let all_alpha = backend.list(Some("alpha"), None, None).await.unwrap();
    assert_eq!(all_alpha.len(), 2);

    let only_facts = backend
        .list(Some("alpha"), Some(&MemoryCategory::Core), None)
        .await
        .unwrap();
    assert_eq!(only_facts.len(), 1);
    assert_eq!(only_facts[0].key, "a1");
}

#[tokio::test]
async fn forget_removes_existing_memory_and_reports_missing() {
    let (addr, _state) = start_mock_server().await;
    let backend = AgentMemoryBackend::from_config(&make_config(addr)).unwrap();

    backend
        .store("demo", "doomed", "delete me", MemoryCategory::Core, None)
        .await
        .unwrap();

    assert!(backend.forget("demo", "doomed").await.unwrap());
    // Second time around the key is gone.
    assert!(!backend.forget("demo", "doomed").await.unwrap());
    // Unknown key reports missing without an error.
    assert!(!backend.forget("demo", "never-existed").await.unwrap());
}

#[tokio::test]
async fn namespace_summaries_aggregate_per_project() {
    let (addr, _state) = start_mock_server().await;
    let backend = AgentMemoryBackend::from_config(&make_config(addr)).unwrap();

    backend
        .store("alpha", "a1", "x", MemoryCategory::Core, None)
        .await
        .unwrap();
    backend
        .store("alpha", "a2", "y", MemoryCategory::Core, None)
        .await
        .unwrap();
    backend
        .store("beta", "b1", "z", MemoryCategory::Core, None)
        .await
        .unwrap();

    let mut summaries = backend.namespace_summaries().await.unwrap();
    summaries.sort_by(|a, b| a.namespace.cmp(&b.namespace));
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].namespace, "alpha");
    assert_eq!(summaries[0].count, 2);
    assert_eq!(summaries[1].namespace, "beta");
    assert_eq!(summaries[1].count, 1);
}

#[tokio::test]
async fn count_reads_total_from_health_endpoint() {
    let (addr, _state) = start_mock_server().await;
    let backend = AgentMemoryBackend::from_config(&make_config(addr)).unwrap();

    backend
        .store("demo", "k1", "x", MemoryCategory::Core, None)
        .await
        .unwrap();
    backend
        .store("demo", "k2", "y", MemoryCategory::Core, None)
        .await
        .unwrap();

    assert_eq!(backend.count().await.unwrap(), 2);
}

#[tokio::test]
async fn name_returns_agentmemory_string() {
    let (addr, _state) = start_mock_server().await;
    let backend = AgentMemoryBackend::from_config(&make_config(addr)).unwrap();
    assert_eq!(backend.name(), "agentmemory");
}

#[test]
fn from_config_rejects_empty_url() {
    let cfg = MemoryConfig {
        backend: "agentmemory".to_string(),
        agentmemory_url: Some("   ".to_string()),
        ..MemoryConfig::default()
    };
    // `AgentMemoryBackend` does not derive `Debug` (its inner `reqwest::Client`
    // is opaque), so use a `match` instead of `.unwrap_err()`.
    match AgentMemoryBackend::from_config(&cfg) {
        Ok(_) => panic!("expected error for empty url"),
        Err(err) => assert!(
            err.to_string().contains("cannot be empty"),
            "unexpected error: {err}"
        ),
    }
}

#[test]
fn from_config_rejects_invalid_url() {
    let cfg = MemoryConfig {
        backend: "agentmemory".to_string(),
        agentmemory_url: Some("not a url".to_string()),
        ..MemoryConfig::default()
    };
    match AgentMemoryBackend::from_config(&cfg) {
        Ok(_) => panic!("expected error for invalid url"),
        Err(err) => assert!(
            err.to_string().contains("not a valid URL"),
            "expected URL error, got: {err}"
        ),
    }
}
