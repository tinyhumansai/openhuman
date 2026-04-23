//! End-to-end test for the iMessage → life_capture ingest bridge.
//!
//! This is Step 6 of the iMessage live-harness plan: prove that an iMessage
//! transcript flowing through `openhuman.life_capture_ingest` is (a) findable
//! via `openhuman.life_capture_search`, and (b) idempotent — re-ingesting the
//! same `(source, external_id)` does not create a duplicate.
//!
//! Each test in this binary uses its own ephemeral HTTP router but shares the
//! same process-global `life_capture::runtime` OnceCells (index + embedder).
//! That's by design: the OnceCells model the production startup contract, and
//! the tests run serially so a single shared init is correct.
//!
//! Run with: `cargo test --test imessage_ingest_e2e -- --ignored --nocapture`

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::life_capture::embedder::Embedder;
use openhuman_core::openhuman::life_capture::index::PersonalIndex;
use openhuman_core::openhuman::life_capture::runtime;

/// Deterministic embedder for tests. Returns a fixed-dim vector seeded from
/// the input string's bytes. Same input → same vector, so re-ingest paths
/// match the production "embedder is stable for repeats" expectation without
/// requiring a real API key.
struct DeterministicEmbedder {
    dim: usize,
}

#[async_trait]
impl Embedder for DeterministicEmbedder {
    async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; self.dim];
                for (i, b) in t.as_bytes().iter().enumerate() {
                    let slot = i % self.dim;
                    v[slot] += (*b as f32) / 255.0;
                }
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for x in v.iter_mut() {
                        *x /= norm;
                    }
                }
                v
            })
            .collect())
    }
    fn dim(&self) -> usize {
        self.dim
    }
}

async fn ensure_runtime_initialised() {
    // Each call is idempotent: OnceCell::set returns Err on the second attempt
    // and we just ignore it. The first test that runs wins; that's fine because
    // every test in this binary uses the same shared index + embedder.
    let workspace = tempdir().expect("tempdir");
    let db_path = workspace.path().join("life_capture.db");
    // Leak the tempdir so the path stays valid for the whole test run.
    std::mem::forget(workspace);

    let idx = Arc::new(PersonalIndex::open(&db_path).await.expect("open index"));
    let _ = runtime::init_index(idx).await;
    let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder { dim: 1536 });
    let _ = runtime::init_embedder(embedder).await;
}

async fn serve_on_ephemeral(
    app: Router,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, handle)
}

async fn post_rpc(base: &str, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("client");
    let url = format!("{}/rpc", base.trim_end_matches('/'));
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {url} {method}: {e}"));
    assert!(
        resp.status().is_success(),
        "HTTP {} for {method}",
        resp.status()
    );
    resp.json().await.expect("json")
}

fn rpc_result_body(envelope: &Value) -> &Value {
    // HTTP JSON-RPC envelope: { "id":1, "jsonrpc":"2.0", "result": <body> }
    // (Double-wrapping `/result/result` only appears in CLI-compatible JSON
    // for some controllers — life_capture's controllers use `to_json` which
    // unwraps the RpcOutcome's logs once.)
    envelope
        .pointer("/result")
        .unwrap_or_else(|| panic!("missing /result in envelope: {envelope}"))
}

#[tokio::test]
#[ignore]
async fn ingest_then_search_then_reingest_is_idempotent() {
    ensure_runtime_initialised().await;
    let (addr, _join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let base = format!("http://{}", addr);

    let unique_marker = "ledgercontractdraft9f3a2b";
    let transcript = format!(
        "[1700000000] +15555550100: Hey — sending the {unique_marker} now.\n\
         [1700000060] me: Got it, will review tonight.\n"
    );
    let ext_id = "+15555550100:2026-04-22";

    // First ingest — should INSERT.
    let env1 = post_rpc(
        &base,
        "openhuman.life_capture_ingest",
        json!({
            "source": "imessage",
            "external_id": ext_id,
            "ts": 1_700_000_000_i64,
            "subject": "Messages — +15555550100 — 2026-04-22",
            "text": transcript,
            "metadata": { "chat_identifier": "+15555550100", "day": "2026-04-22" }
        }),
    )
    .await;
    let body1 = rpc_result_body(&env1);
    let item_id_1 = body1["item_id"].as_str().expect("item_id").to_string();
    assert_eq!(
        body1["replaced"],
        json!(false),
        "first ingest must be an insert, got: {body1}"
    );

    // Stats — should show exactly one item.
    let env_stats1 = post_rpc(&base, "openhuman.life_capture_get_stats", json!({})).await;
    let stats1 = rpc_result_body(&env_stats1);
    assert_eq!(
        stats1["total_items"],
        json!(1),
        "expected total_items=1 after first ingest, got {stats1}"
    );

    // Search — must surface the marker.
    let env_search = post_rpc(
        &base,
        "openhuman.life_capture_search",
        json!({ "text": unique_marker, "k": 5 }),
    )
    .await;
    let search_body = rpc_result_body(&env_search);
    let hits = search_body["hits"].as_array().unwrap_or_else(|| {
        panic!("search response must wrap hits in a 'hits' array, got: {search_body}")
    });
    assert!(!hits.is_empty(), "expected at least one hit");
    assert_eq!(
        hits[0]["source"],
        json!("imessage"),
        "top hit must come from imessage source"
    );
    assert_eq!(hits[0]["item_id"].as_str(), Some(item_id_1.as_str()));

    // Re-ingest with same (source, external_id) — must UPDATE in place,
    // not insert a duplicate.
    let env2 = post_rpc(
        &base,
        "openhuman.life_capture_ingest",
        json!({
            "source": "imessage",
            "external_id": ext_id,
            "ts": 1_700_000_120_i64,
            "subject": "Messages — +15555550100 — 2026-04-22",
            "text": format!("{transcript}[1700000120] +15555550100: One more thing.\n"),
            "metadata": { "chat_identifier": "+15555550100", "day": "2026-04-22" }
        }),
    )
    .await;
    let body2 = rpc_result_body(&env2);
    assert_eq!(
        body2["replaced"],
        json!(true),
        "re-ingest with same external_id must report replaced=true: {body2}"
    );
    assert_eq!(
        body2["item_id"].as_str(),
        Some(item_id_1.as_str()),
        "canonical item_id must be stable across re-ingests"
    );

    // Stats unchanged — process-once guarantee.
    let env_stats2 = post_rpc(&base, "openhuman.life_capture_get_stats", json!({})).await;
    let stats2 = rpc_result_body(&env_stats2);
    assert_eq!(
        stats2["total_items"], stats1["total_items"],
        "total_items must NOT increase on re-ingest of the same key — got before={stats1}, after={stats2}"
    );
}

#[tokio::test]
#[ignore]
async fn search_response_shape_matches_schema() {
    // Regression guard for obs 3165: search RPC must return `{"hits": [...]}`,
    // not a bare array. Schema declares wrapping, callers depend on it.
    ensure_runtime_initialised().await;
    let (addr, _join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let base = format!("http://{}", addr);

    // Seed one item so search has something to look at; query an unrelated
    // term so we still hit the wrapping codepath even if hits is empty.
    let _ = post_rpc(
        &base,
        "openhuman.life_capture_ingest",
        json!({
            "source": "imessage",
            "external_id": "shape-guard:2026-04-22",
            "ts": 1_700_000_000_i64,
            "text": "shape guard sentinel text"
        }),
    )
    .await;

    let env = post_rpc(
        &base,
        "openhuman.life_capture_search",
        json!({ "text": "shape guard", "k": 3 }),
    )
    .await;
    let body = rpc_result_body(&env);
    assert!(
        body.get("hits").is_some(),
        "search body must contain 'hits' field per schema, got: {body}"
    );
    assert!(
        body["hits"].is_array(),
        "'hits' field must be an array, got: {body}"
    );
}
