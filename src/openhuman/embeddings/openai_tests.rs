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
