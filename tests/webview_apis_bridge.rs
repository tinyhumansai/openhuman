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
use std::sync::Mutex;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio_tungstenite::tungstenite::Message;

use openhuman_core::openhuman::webview_apis::{client, types::GmailLabel};

/// The webview_apis client caches its WebSocket connection in a
/// process-global `OnceLock`, so all tests must funnel through ONE mock
/// server for the life of the process.
///
/// Critically, that mock server cannot live on a per-test tokio runtime:
/// each `#[tokio::test]` spins up its own runtime and drops it when the
/// test returns, which would kill the accept loop and every open
/// connection. We run it on a dedicated OS thread with its own
/// `Runtime` that is leaked for the duration of the test binary.
static TEST_SERVER: once_cell::sync::Lazy<Mutex<Option<u16>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

fn ensure_mock_server() -> u16 {
    let mut guard = TEST_SERVER.lock().unwrap();
    if let Some(port) = *guard {
        return port;
    }
    let (tx, rx) = std::sync::mpsc::channel::<u16>();
    std::thread::Builder::new()
        .name("webview-apis-mock-bridge".into())
        .spawn(move || {
            let rt = Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("build mock-server runtime");
            rt.block_on(async move {
                let listener = TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse().unwrap())
                    .await
                    .expect("bind");
                let port = listener.local_addr().unwrap().port();
                tx.send(port).expect("port channel");
                loop {
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
                        while let Some(msg) = stream.next().await {
                            match msg {
                                Ok(Message::Text(text)) => {
                                    let req: Value = serde_json::from_str(&text).unwrap();
                                    let id = req["id"].as_str().unwrap().to_string();
                                    let method = req["method"].as_str().unwrap().to_string();
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
                                        break;
                                    }
                                }
                                Ok(Message::Close(_)) => break,
                                Ok(_) => continue,
                                Err(_) => break,
                            }
                        }
                    });
                }
            });
        })
        .expect("spawn mock-server thread");
    let port = rx.recv().expect("mock-server port");
    std::env::set_var("OPENHUMAN_WEBVIEW_APIS_PORT", port.to_string());
    *guard = Some(port);
    port
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_round_trips_list_labels_through_mock_server() {
    let _port = ensure_mock_server();
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_surfaces_bridge_error_verbatim() {
    let _port = ensure_mock_server();
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
