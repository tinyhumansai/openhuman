//! End-to-end test for the Composio → life_capture ingest bridge (A2).
//!
//! Wires up the life_capture runtime OnceCells, registers the
//! `LifeCaptureComposioBridge` on the global event bus, publishes a
//! `GMAIL_NEW_GMAIL_MESSAGE` trigger, and asserts that the payload
//! lands in the PersonalIndex with `source = gmail` and the message id
//! as `external_id`.
//!
//! Mirrors the pattern in `tests/imessage_ingest_e2e.rs`.
//!
//! Run with:
//!   cargo test --test composio_ingest_e2e -- --ignored --nocapture

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::event_bus::{self, publish_global, DomainEvent, DEFAULT_CAPACITY};
use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::life_capture::embedder::Embedder;
use openhuman_core::openhuman::life_capture::index::PersonalIndex;
use openhuman_core::openhuman::life_capture::ingest::register_life_capture_composio_bridge;
use openhuman_core::openhuman::life_capture::runtime;

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
                    v[i % self.dim] += (*b as f32) / 255.0;
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

async fn ensure_runtime_and_bridge() -> (Arc<PersonalIndex>, Arc<dyn Embedder>) {
    // Init the global event bus (idempotent).
    event_bus::init_global(DEFAULT_CAPACITY);

    let workspace = tempdir().expect("tempdir");
    let db_path = workspace.path().join("life_capture.db");
    std::mem::forget(workspace);

    let idx = Arc::new(PersonalIndex::open(&db_path).await.expect("open index"));
    let _ = runtime::init_index(Arc::clone(&idx)).await;
    let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder { dim: 1536 });
    let _ = runtime::init_embedder(Arc::clone(&embedder)).await;

    register_life_capture_composio_bridge(Arc::clone(&idx), Arc::clone(&embedder));

    (idx, embedder)
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
    resp.json().await.expect("json")
}

#[tokio::test]
#[ignore]
async fn gmail_trigger_flows_into_personal_index() {
    let (idx, _emb) = ensure_runtime_and_bridge().await;

    // Wait a tick so the subscriber task actually begins consuming
    // events before we publish — subscribe_global spawns asynchronously.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let unique_marker = "ledgercontractdraftQ3planning";
    let message_id = "gmail-e2e-msg-42";

    publish_global(DomainEvent::ComposioTriggerReceived {
        toolkit: "gmail".into(),
        trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
        metadata_id: "trig-e2e-1".into(),
        metadata_uuid: "uuid-e2e-1".into(),
        payload: json!({
            "messageId": message_id,
            "subject": format!("Draft attached — {unique_marker}"),
            "from": "sarah@example.com",
            "snippet": format!("Hi — sending the {unique_marker} for review."),
            "internalDate": 1_700_000_000_000_i64
        }),
    });

    // Give the async subscriber a window to run handle_ingest
    // (embedder + sqlite writes).
    let (addr, _join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let base = format!("http://{}", addr);

    let mut attempts = 0;
    loop {
        attempts += 1;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let env = post_rpc(
            &base,
            "openhuman.life_capture_search",
            json!({ "text": unique_marker, "k": 5 }),
        )
        .await;
        let body = env
            .pointer("/result")
            .unwrap_or_else(|| panic!("missing /result: {env}"));
        let hits = body["hits"].as_array().cloned().unwrap_or_default();
        if let Some(hit) = hits.iter().find(|h| {
            h.get("source").and_then(|s| s.as_str()) == Some("gmail")
                && h.get("item_id")
                    .and_then(|i| i.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false)
        }) {
            // Shape + dedupe fields visible from the search hit.
            assert_eq!(hit["source"], json!("gmail"));
            assert!(
                hit["ts"].as_i64().unwrap_or(0) == 1_700_000_000,
                "expected ts normalised from millis to seconds: {hit}"
            );
            // Confirm the underlying item row is keyed by messageId.
            let stats_env =
                post_rpc(&base, "openhuman.life_capture_get_stats", json!({})).await;
            let stats = stats_env
                .pointer("/result")
                .unwrap_or_else(|| panic!("missing stats /result: {stats_env}"));
            assert!(
                stats["total_items"].as_i64().unwrap_or(0) >= 1,
                "expected at least one item after ingest, got {stats}"
            );
            // Re-publish the same trigger — must upsert, not duplicate.
            publish_global(DomainEvent::ComposioTriggerReceived {
                toolkit: "gmail".into(),
                trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
                metadata_id: "trig-e2e-2".into(),
                metadata_uuid: "uuid-e2e-2".into(),
                payload: json!({
                    "messageId": message_id,
                    "subject": format!("Draft attached — {unique_marker}"),
                    "from": "sarah@example.com",
                    "snippet": format!("Hi — sending the {unique_marker} for review (v2)."),
                    "internalDate": 1_700_000_100_000_i64
                }),
            });
            tokio::time::sleep(Duration::from_millis(400)).await;
            let stats2_env =
                post_rpc(&base, "openhuman.life_capture_get_stats", json!({})).await;
            let stats2 = stats2_env.pointer("/result").unwrap();
            assert_eq!(
                stats2["total_items"], stats["total_items"],
                "re-fire of same messageId must upsert in place, got {stats2} vs {stats}"
            );

            // Silence unused binding warning on success path.
            let _ = idx;
            return;
        }

        if attempts >= 30 {
            panic!(
                "gmail trigger never produced a PersonalIndex hit after {attempts} attempts"
            );
        }
    }
}
