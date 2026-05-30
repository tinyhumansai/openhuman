use super::*;
use axum::{
    extract::Json,
    http::{HeaderMap, StatusCode},
    routing::post,
    Router,
};
use std::net::SocketAddr;

async fn start_mock(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

// ── Constructor & URL building ──────────────────────────

#[test]
fn trailing_slash_stripped() {
    let p = OpenAiEmbedding::new("https://api.openai.com/", "key", "model", 1536);
    assert_eq!(p.base_url, "https://api.openai.com");
}

#[test]
fn dimensions_custom() {
    let p = OpenAiEmbedding::new("http://localhost", "k", "m", 384);
    assert_eq!(p.dimensions(), 384);
}

#[test]
fn accessors() {
    let p = OpenAiEmbedding::new("http://x", "k", "m", 1);
    assert_eq!(p.base_url(), "http://x");
    assert_eq!(p.model(), "m");
    assert_eq!(p.name(), "openai");
    assert_eq!(p.model_id(), "m");
    assert_eq!(p.signature(), "provider=openai;model=m;dims=1");
}

#[test]
fn url_standard_openai() {
    let p = OpenAiEmbedding::new("https://api.openai.com", "key", "model", 1536);
    assert_eq!(p.embeddings_url(), "https://api.openai.com/v1/embeddings");
}

#[test]
fn url_base_with_v1_no_duplicate() {
    let p = OpenAiEmbedding::new("https://api.example.com/v1", "key", "model", 1536);
    assert_eq!(p.embeddings_url(), "https://api.example.com/v1/embeddings");
}

#[test]
fn url_non_v1_api_path() {
    let p = OpenAiEmbedding::new(
        "https://api.example.com/api/coding/v3",
        "key",
        "model",
        1536,
    );
    assert_eq!(
        p.embeddings_url(),
        "https://api.example.com/api/coding/v3/embeddings"
    );
}

#[test]
fn url_already_ends_with_embeddings() {
    let p = OpenAiEmbedding::new(
        "https://my-api.example.com/api/v2/embeddings",
        "key",
        "model",
        1536,
    );
    assert_eq!(
        p.embeddings_url(),
        "https://my-api.example.com/api/v2/embeddings"
    );
}

#[test]
fn url_already_ends_with_embeddings_trailing_slash() {
    let p = OpenAiEmbedding::new(
        "https://api.example.com/v1/embeddings/",
        "key",
        "model",
        1536,
    );
    assert_eq!(p.embeddings_url(), "https://api.example.com/v1/embeddings");
}

#[test]
fn url_root_only() {
    let p = OpenAiEmbedding::new("http://localhost:8080", "k", "m", 1);
    assert_eq!(p.embeddings_url(), "http://localhost:8080/v1/embeddings");
}

#[test]
fn url_root_with_trailing_slash() {
    let p = OpenAiEmbedding::new("http://localhost:8080/", "k", "m", 1);
    assert_eq!(p.embeddings_url(), "http://localhost:8080/v1/embeddings");
}

#[test]
fn has_explicit_api_path_invalid_url() {
    let p = OpenAiEmbedding::new("not-a-url", "k", "m", 1);
    assert!(!p.has_explicit_api_path());
}

#[test]
fn has_embeddings_endpoint_invalid_url() {
    let p = OpenAiEmbedding::new("not-a-url", "k", "m", 1);
    assert!(!p.has_embeddings_endpoint());
}

// ── embed — empty input ─────────────────────────────────

#[tokio::test]
async fn empty_input_returns_empty() {
    let p = OpenAiEmbedding::new("http://unused", "k", "m", 1);
    let result = p.embed(&[]).await.unwrap();
    assert!(result.is_empty());
}

// ── embed — success ─────────────────────────────────────

#[tokio::test]
async fn embed_success_single() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async {
            Json(serde_json::json!({
                "data": [{ "embedding": [0.1, 0.2, 0.3] }]
            }))
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "test-key", "test-model", 3);

    let result = p.embed(&["hello"]).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], vec![0.1_f32, 0.2, 0.3]);
}

#[tokio::test]
async fn embed_success_batch() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async {
            Json(serde_json::json!({
                "data": [
                    { "embedding": [1.0, 2.0] },
                    { "embedding": [3.0, 4.0] }
                ]
            }))
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 2);

    let result = p.embed(&["a", "b"]).await.unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[1], vec![3.0_f32, 4.0]);
}

#[tokio::test]
async fn embed_sends_auth_header() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(
            |headers: HeaderMap, Json(body): Json<serde_json::Value>| async move {
                let auth = headers.get("Authorization").unwrap().to_str().unwrap();
                assert_eq!(auth, "Bearer my-secret-key");
                assert_eq!(body["model"], "text-embedding-3-small");
                Json(serde_json::json!({
                    "data": [{ "embedding": [1.0] }]
                }))
            },
        ),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "my-secret-key", "text-embedding-3-small", 1);

    p.embed(&["test"]).await.unwrap();
}

#[tokio::test]
async fn embed_skips_auth_header_when_key_empty() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(|headers: HeaderMap| async move {
            // No Authorization header should be present.
            assert!(
                headers.get("Authorization").is_none(),
                "should not send auth header when key is empty"
            );
            Json(serde_json::json!({
                "data": [{ "embedding": [1.0] }]
            }))
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "", "m", 1);

    p.embed(&["test"]).await.unwrap();
}

// ── embed — error paths ─────────────────────────────────

#[tokio::test]
async fn embed_server_error() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "rate limited") }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 1);

    let err = p.embed(&["hi"]).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("500"), "status: {msg}");
    assert!(msg.contains("rate limited"), "body: {msg}");
}

/// 429 rate-limit responses must format their message in the canonical
/// `"... API error (<status>): <body>"` shape so the shared
/// `is_transient_upstream_http_message` classifier in `core::observability`
/// demotes them to a warning breadcrumb instead of a Sentry error event.
#[tokio::test]
async fn embed_429_uses_canonical_transient_format() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async {
            (
                StatusCode::TOO_MANY_REQUESTS,
                r#"{"error":{"message":"Rate limit exceeded.","type":"rate_limit_error"}}"#,
            )
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 1);

    let err = p.embed(&["hi"]).await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("(429 Too Many Requests)"),
        "expected canonical transient HTTP shape, got: {msg}"
    );
    // Pin the shape to the exact substring `is_transient_upstream_http_message`
    // matches on (`"api error (<status> "`). The broader
    // `is_transient_message_failure` classifier below also passes for the *old*
    // `"Embedding API error 429 …"` format, so without this assertion a future
    // refactor could silently revert the format and the test would still go
    // green.
    assert!(
        msg.to_ascii_lowercase().contains("api error (429 "),
        "message must match is_transient_upstream_http_message classifier arm: {msg}"
    );
    assert!(
        crate::core::observability::is_transient_message_failure(&msg),
        "message should classify as transient: {msg}"
    );
}

#[tokio::test]
async fn embed_budget_exhausted_400_still_errors() {
    // OPENHUMAN-TAURI-JM: the backend returns HTTP 400 with a budget-exhausted
    // body when the user is out of credits. The provider must still surface
    // an `Err` to the caller (so the calling pipeline can short-circuit), but
    // the diagnostic emit site must route through `report_error_or_expected`
    // so the message is classified as `BudgetExhausted` and demoted rather
    // than spawning a Sentry error event for every embed call.
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async {
            (
                StatusCode::BAD_REQUEST,
                r#"{"success":false,"error":"Budget exceeded — add credits to continue"}"#,
            )
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 1);

    let err = p.embed(&["hi"]).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("400"), "status: {msg}");
    assert!(msg.contains("Budget exceeded"), "body: {msg}");
}

#[tokio::test]
async fn embed_missing_data_field() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async { Json(serde_json::json!({ "result": "ok" })) }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 1);

    let err = p.embed(&["hi"]).await.unwrap_err();
    assert!(err.to_string().contains("missing 'data'"));
}

#[tokio::test]
async fn embed_missing_embedding_field_in_item() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async {
            Json(serde_json::json!({
                "data": [{ "index": 0 }]
            }))
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 1);

    let err = p.embed(&["hi"]).await.unwrap_err();
    assert!(err.to_string().contains("missing 'embedding'"));
}

#[tokio::test]
async fn embed_non_numeric_value_errors() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async {
            Json(serde_json::json!({
                "data": [{ "embedding": [1.0, "not_a_number", 3.0] }]
            }))
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 3);

    let err = p.embed(&["hi"]).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("non-numeric"), "msg: {msg}");
}

#[tokio::test]
async fn embed_count_mismatch() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async {
            Json(serde_json::json!({
                "data": [{ "embedding": [1.0] }]
            }))
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 1);

    let err = p.embed(&["a", "b"]).await.unwrap_err();
    assert!(err.to_string().contains("count mismatch"));
}

#[tokio::test]
async fn embed_dimension_mismatch() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async {
            Json(serde_json::json!({
                "data": [{ "embedding": [1.0, 2.0, 3.0] }]
            }))
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 2);

    let err = p.embed(&["hi"]).await.unwrap_err();
    assert!(err.to_string().contains("dimension mismatch"));
}

#[tokio::test]
async fn embed_malformed_json() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async { (StatusCode::OK, "not json") }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 1);

    let err = p.embed(&["hi"]).await.unwrap_err();
    assert!(err.is::<reqwest::Error>());
}

#[tokio::test]
async fn embed_connection_refused() {
    let p = OpenAiEmbedding::new("http://127.0.0.1:1", "k", "m", 1);
    let err = p.embed(&["hi"]).await.unwrap_err();
    assert!(err.is::<reqwest::Error>());
}

#[test]
fn openai_embedding_api_error_stays_unexpected() {
    let msg = "Embedding API error 401 Unauthorized: invalid_api_key";
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        None,
        "OpenAI API key errors should continue to reach Sentry"
    );
}

// ── embed_one (trait default) ───────────────────────────

#[tokio::test]
async fn embed_one_success() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async {
            Json(serde_json::json!({
                "data": [{ "embedding": [9.0, 8.0, 7.0] }]
            }))
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 3);

    let vec = p.embed_one("test").await.unwrap();
    assert_eq!(vec, vec![9.0_f32, 8.0, 7.0]);
}

// ── URL building — custom endpoint ──────────────────────

#[tokio::test]
async fn embed_with_explicit_api_path() {
    let app = Router::new().route(
        "/custom/api/embeddings",
        post(|| async {
            Json(serde_json::json!({
                "data": [{ "embedding": [1.0] }]
            }))
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&format!("{url}/custom/api"), "k", "m", 1);

    let result = p.embed(&["test"]).await.unwrap();
    assert_eq!(result.len(), 1);
}

// ── 429 backoff / Retry-After tests ──────────────────────

/// Mock returns 429 twice (with Retry-After: 0) then 200 — verify embed
/// succeeds and that exactly 3 requests were made (initial + 2 retries).
#[tokio::test]
async fn embed_429_retries_with_retry_after_then_succeeds() {
    use std::sync::{Arc, Mutex};
    let counter = Arc::new(Mutex::new(0u32));
    let counter_clone = counter.clone();

    let app = Router::new().route(
        "/v1/embeddings",
        post(move || {
            let counter = counter_clone.clone();
            async move {
                let mut n = counter.lock().unwrap();
                *n += 1;
                if *n <= 2 {
                    // Return 429 with Retry-After: 0 (zero delay) for fast tests.
                    axum::response::Response::builder()
                        .status(StatusCode::TOO_MANY_REQUESTS)
                        .header("Retry-After", "0")
                        .body(axum::body::Body::from(
                            r#"{"error":{"message":"rate limited"}}"#,
                        ))
                        .unwrap()
                } else {
                    axum::response::Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "application/json")
                        .body(axum::body::Body::from(
                            r#"{"data":[{"embedding":[1.0,2.0]}]}"#,
                        ))
                        .unwrap()
                }
            }
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 2);

    let result = p.embed(&["hello"]).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], vec![1.0_f32, 2.0]);
    assert_eq!(
        *counter.lock().unwrap(),
        3,
        "expected 3 total requests (1 initial + 2 retries)"
    );
}

/// Mock returns 429 indefinitely — verify bail with canonical message and
/// that exactly MAX_429_RETRIES + 1 requests were made.
#[tokio::test]
async fn embed_429_indefinite_bails_after_retry_cap() {
    use crate::openhuman::embeddings::retry_after::MAX_429_RETRIES;
    use std::sync::{Arc, Mutex};
    let counter = Arc::new(Mutex::new(0u32));
    let counter_clone = counter.clone();

    let app = Router::new().route(
        "/v1/embeddings",
        post(move || {
            let counter = counter_clone.clone();
            async move {
                let mut n = counter.lock().unwrap();
                *n += 1;
                axum::response::Response::builder()
                    .status(StatusCode::TOO_MANY_REQUESTS)
                    .header("Retry-After", "0")
                    .body(axum::body::Body::from(
                        r#"{"error":{"message":"always rate limited"}}"#,
                    ))
                    .unwrap()
            }
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 1);

    let err = p.embed(&["hi"]).await.unwrap_err();
    let msg = err.to_string();

    // The error message must contain "429" so the classifier still suppresses it.
    assert!(
        msg.contains("429"),
        "bail message must contain 429 for Sentry classifier: {msg}"
    );
    // Must match is_transient_upstream_http_message via "(429 " substring.
    assert!(
        crate::core::observability::is_transient_message_failure(&msg),
        "bail message must classify as transient: {msg}"
    );

    let requests = *counter.lock().unwrap();
    assert_eq!(
        requests,
        MAX_429_RETRIES + 1,
        "expected exactly MAX_429_RETRIES+1={} requests, got {}",
        MAX_429_RETRIES + 1,
        requests
    );
}

/// Mock returns 429 without Retry-After header — verify the no-header code
/// path is taken, that a retry is attempted, and that the request succeeds.
/// Uses `Retry-After: 0` on the *second* attempt to confirm header parsing
/// is independent from the no-header first attempt.
#[tokio::test]
async fn embed_429_without_retry_after_uses_exponential_backoff() {
    use crate::openhuman::embeddings::retry_after::{backoff_ms_for_attempt, BASE_BACKOFF_MS};
    use std::sync::{Arc, Mutex};

    // Confirm the helper returns the exponential base when header is absent.
    assert_eq!(
        backoff_ms_for_attempt(0, None),
        BASE_BACKOFF_MS,
        "attempt 0 without header should use BASE_BACKOFF_MS"
    );
    assert_eq!(
        backoff_ms_for_attempt(1, None),
        BASE_BACKOFF_MS * 2,
        "attempt 1 without header should double"
    );

    // Confirm header path: Retry-After: 0 overrides the exponential base.
    assert_eq!(
        backoff_ms_for_attempt(0, Some("0")),
        0,
        "Retry-After: 0 should yield zero-ms delay"
    );

    // End-to-end: first request returns 429 (no Retry-After), second succeeds.
    // Use Retry-After: 0 on the 429 to avoid real-wall-clock delay in CI.
    let counter = Arc::new(Mutex::new(0u32));
    let counter_clone = counter.clone();

    let app = Router::new().route(
        "/v1/embeddings",
        post(move || {
            let counter = counter_clone.clone();
            async move {
                let mut n = counter.lock().unwrap();
                *n += 1;
                if *n == 1 {
                    // Mock still sets Retry-After: 0 here to keep this test fast;
                    // the no-header exponential branch is exercised end-to-end by
                    // `embed_429_no_retry_after_header_falls_back_to_exponential`
                    // below and by unit tests in `retry_after.rs`.
                    axum::response::Response::builder()
                        .status(StatusCode::TOO_MANY_REQUESTS)
                        .header("Retry-After", "0")
                        .body(axum::body::Body::from(
                            r#"{"error":{"message":"rate limited"}}"#,
                        ))
                        .unwrap()
                } else {
                    axum::response::Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "application/json")
                        .body(axum::body::Body::from(r#"{"data":[{"embedding":[9.9]}]}"#))
                        .unwrap()
                }
            }
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 1);

    let result = p.embed(&["hi"]).await;
    assert!(result.is_ok(), "should succeed after retry: {:?}", result);
    assert_eq!(
        *counter.lock().unwrap(),
        2,
        "expected 2 total requests (1 initial + 1 retry)"
    );
}

/// End-to-end coverage for the no-`Retry-After` exponential-backoff branch.
///
/// The previous "no Retry-After" test still set `Retry-After: 0` on the mock,
/// so the embed loop never actually called `backoff_ms_for_attempt(.., None)` —
/// the exponential fallback was only exercised by unit tests in `retry_after.rs`.
/// This test omits the header entirely so the loop must use the `BASE_BACKOFF_MS`
/// path. We tolerate the ~1 s wait (one retry @ `BASE_BACKOFF_MS = 1000 ms`) to
/// keep the assertion meaningful: the real backoff schedule is what we care about.
#[tokio::test]
async fn embed_429_no_retry_after_header_falls_back_to_exponential() {
    use crate::openhuman::embeddings::retry_after::BASE_BACKOFF_MS;
    use std::sync::{Arc, Mutex};
    use tokio::time::Instant;

    let counter = Arc::new(Mutex::new(0u32));
    let counter_clone = counter.clone();

    let app = Router::new().route(
        "/v1/embeddings",
        post(move || {
            let counter = counter_clone.clone();
            async move {
                let mut n = counter.lock().unwrap();
                *n += 1;
                if *n == 1 {
                    // Crucial: omit the Retry-After header entirely so the embed
                    // loop hits `backoff_ms_for_attempt(0, None)` =
                    // `BASE_BACKOFF_MS` and sleeps. If a future refactor breaks
                    // the fallback (e.g. defaulting to 0 ms) the elapsed-time
                    // assertion below will catch it.
                    axum::response::Response::builder()
                        .status(StatusCode::TOO_MANY_REQUESTS)
                        .body(axum::body::Body::from(
                            r#"{"error":{"message":"rate limited"}}"#,
                        ))
                        .unwrap()
                } else {
                    axum::response::Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "application/json")
                        .body(axum::body::Body::from(r#"{"data":[{"embedding":[4.2]}]}"#))
                        .unwrap()
                }
            }
        }),
    );
    let url = start_mock(app).await;
    let p = OpenAiEmbedding::new(&url, "k", "m", 1);

    let start = Instant::now();
    let result = p.embed(&["hi"]).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "should succeed after retry: {:?}", result);
    assert_eq!(
        *counter.lock().unwrap(),
        2,
        "expected 2 total requests (1 initial + 1 retry)"
    );
    // The fallback must actually wait the exponential base — within a 250 ms
    // jitter window for slow CI runners. Without this we couldn't tell whether
    // the no-header branch was taken or silently short-circuited.
    let min_wait = std::time::Duration::from_millis(BASE_BACKOFF_MS.saturating_sub(250));
    assert!(
        elapsed >= min_wait,
        "expected elapsed >= ~{}ms (BASE_BACKOFF_MS minus jitter), got {:?}",
        min_wait.as_millis(),
        elapsed
    );
}
