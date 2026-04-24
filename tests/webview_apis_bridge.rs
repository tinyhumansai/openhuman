//! End-to-end test for the webview_apis bridge.
//!
//! Proves the full chain without the Tauri shell:
//!
//! ```text
//! client::request                                      ← core-side code we ship
//!   → ws://127.0.0.1:$OPENHUMAN_WEBVIEW_APIS_PORT
//!   → mock WS server (this test)                       ← stands in for Tauri
//!   → JSON response
//!   → decoded back into typed GmailLabel Vec
//! ```
//!
//! Tests are serial because they all mutate the `OPENHUMAN_WEBVIEW_APIS_PORT`
//! env var and share the lazy global `CLIENT` inside
//! `openhuman_core::openhuman::webview_apis::client`.

use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

use openhuman_core::openhuman::webview_apis::{client, types::GmailLabel};

/// Serialize the port-mutation so two tests don't race on the env var.
///
/// The client caches its connection in a process-global `OnceLock`, so
/// once a test picks up a port the others must use the same one for
/// the rest of the process. In practice this means we start ONE mock
/// server and funnel every test through it.
static TEST_SERVER: once_cell::sync::Lazy<Mutex<Option<u16>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));
static REQUEST_LOCK: once_cell::sync::Lazy<Mutex<()>> =
    once_cell::sync::Lazy::new(|| Mutex::new(()));

async fn ensure_mock_server() -> u16 {
    let mut guard = TEST_SERVER.lock().await;
    if let Some(port) = *guard {
        return port;
    }
    let listener = TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::env::set_var("OPENHUMAN_WEBVIEW_APIS_PORT", port.to_string());
    *guard = Some(port);
    tokio::spawn(async move {
        loop {
            tracing::debug!("[webview_apis_bridge:test] waiting for mock ws client");
            let (stream, _peer) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => continue,
            };
            tokio::spawn(async move {
                let ws = match tokio_tungstenite::accept_async(stream).await {
                    Ok(w) => w,
                    Err(_) => return,
                };
                let (mut sink, mut stream) = ws.split();
                tracing::debug!("[webview_apis_bridge:test] mock ws client connected");
                while let Some(msg) = stream.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            let req: Value = serde_json::from_str(&text).unwrap();
                            let id = req["id"].as_str().unwrap().to_string();
                            let method = req["method"].as_str().unwrap().to_string();
                            let redacted_id = if id.len() <= 4 {
                                "***".to_string()
                            } else {
                                format!("***{}", &id[id.len() - 4..])
                            };
                            tracing::debug!(
                                id = %redacted_id,
                                method = %method,
                                "[webview_apis_bridge:test] handling text rpc message"
                            );
                            let resp = match method.as_str() {
                                "gmail.list_labels" => json!({
                                    "kind": "response",
                                    "id": id,
                                    "ok": true,
                                    "result": [
                                        {"id": "INBOX", "name": "Inbox", "kind": "system", "unread": 3},
                                        {"id": "Receipts", "name": "Receipts", "kind": "user", "unread": null}
                                    ],
                                }),
                                "gmail.trash" => json!({
                                    "kind": "response",
                                    "id": id,
                                    "ok": false,
                                    "error": "simulated failure from mock bridge",
                                }),
                                _ => json!({
                                    "kind": "response",
                                    "id": id,
                                    "ok": false,
                                    "error": format!("mock bridge: unhandled method '{method}'"),
                                }),
                            };
                            if sink.send(Message::Text(resp.to_string())).await.is_err() {
                                tracing::warn!(
                                    id = %redacted_id,
                                    method = %method,
                                    "[webview_apis_bridge:test] failed to send mock response"
                                );
                                break;
                            }
                        }
                        Ok(Message::Close(_)) => {
                            tracing::debug!("[webview_apis_bridge:test] client closed connection");
                            break;
                        }
                        Ok(_) => {
                            tracing::debug!(
                                "[webview_apis_bridge:test] ignoring non-text ws message"
                            );
                            continue;
                        }
                        Err(err) => {
                            tracing::warn!(
                                error = %err,
                                "[webview_apis_bridge:test] websocket receive error"
                            );
                            break;
                        }
                    }
                }
            });
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_round_trips_and_surfaces_errors_through_mock_server() {
    let _request_guard = REQUEST_LOCK.lock().await;
    let _port = ensure_mock_server().await;
    let labels: Vec<GmailLabel> = client::request(
        "gmail.list_labels",
        serde_json::from_value(json!({"account_id": "gmail"})).unwrap(),
    )
    .await
    .expect("mock bridge call");
    assert_eq!(labels.len(), 2);
    assert_eq!(labels[0].id, "INBOX");
    assert_eq!(labels[0].unread, Some(3));
    assert_eq!(labels[1].kind, "user");
    let err: Result<Vec<GmailLabel>, String> = client::request(
        "gmail.trash",
        serde_json::from_value(json!({"account_id": "gmail", "message_id": "m1"})).unwrap(),
    )
    .await;
    let e = err.expect_err("expected bridge-side error");
    assert!(
        e.contains("simulated failure from mock bridge"),
        "unexpected error: {e}"
    );
}
