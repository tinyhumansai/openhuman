//! Tests for the shared integrations HTTP client.
//!
//! Focus: backend error body propagation. Pre-fix, non-2xx responses
//! discarded the body (`let _body_text = …`) leaving callers with a
//! generic `"Backend returned 400 …"` message — see #1296. These tests
//! lock in the new behaviour where `extract_error_detail` pulls the
//! envelope's `error` field (or falls back to truncated raw text) and
//! the bail message includes it.

use super::*;
use axum::{
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;

// ── Integration: HTTP error propagation through `post`/`get` ──────

async fn start_mock_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn client_for(base: String) -> IntegrationClient {
    IntegrationClient::new(base, "test-token".into())
}

/// The `x-sdk-name` each of `IntegrationClient`'s two transports sent.
///
/// The verb methods go out through the SDK and are tagged. `get_bytes` drives
/// the separate `download_client`, which deliberately is NOT tagged: it follows
/// a 302 to presigned S3 storage, and reqwest preserves non-sensitive headers
/// across a cross-host redirect, so tagging it would hand the product identity
/// to the storage provider. Two independent transports with deliberately
/// opposite expectations, so both are asserted.
struct ProductIdentitySeen {
    sdk: Option<String>,
    download: Option<String>,
}

async fn product_identity_seen_by_backend() -> ProductIdentitySeen {
    use crate::backend::product::PRODUCT_IDENTITY_HEADER;

    fn sink_header(sink: &Arc<std::sync::Mutex<Option<String>>>, headers: &axum::http::HeaderMap) {
        *sink.lock().unwrap() = headers
            .get(PRODUCT_IDENTITY_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
    }

    let sdk_seen: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let download_seen: Arc<std::sync::Mutex<Option<String>>> =
        Arc::new(std::sync::Mutex::new(None));
    let sdk_sink = sdk_seen.clone();
    let download_sink = download_seen.clone();

    let app = Router::new()
        .route(
            "/agent-integrations/composio/execute",
            post(move |headers: axum::http::HeaderMap| {
                let sink = sdk_sink.clone();
                async move {
                    sink_header(&sink, &headers);
                    Json(json!({ "success": true, "data": {} })).into_response()
                }
            }),
        )
        .route(
            "/agent-integrations/file-storage/files/f1/download",
            get(move |headers: axum::http::HeaderMap| {
                let sink = download_sink.clone();
                async move {
                    sink_header(&sink, &headers);
                    "file-bytes".into_response()
                }
            }),
        );
    let base = start_mock_backend(app).await;

    let client = client_for(base);
    let sdk_result = client
        .post::<serde_json::Value>(
            &Default::default(),
            "/agent-integrations/composio/execute",
            &json!({}),
        )
        .await;
    let download_result = client
        .get_bytes(
            &Default::default(),
            "/agent-integrations/file-storage/files/f1/download",
        )
        .await;

    sdk_result.expect("mock backend returns a success envelope");
    download_result.expect("mock backend returns file bytes");

    let sdk = sdk_seen.lock().unwrap().clone();
    let download = download_seen.lock().unwrap().clone();
    ProductIdentitySeen { sdk, download }
}

#[path = "client_error_propagation_tests.rs"]
mod error_propagation_tests;
#[path = "client_session_expiry_tests.rs"]
mod session_expiry_tests;

#[path = "client_api_key_tests.rs"]
mod api_key_tests;
