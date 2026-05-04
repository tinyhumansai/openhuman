use super::*;
use axum::{extract::Json, http::StatusCode, routing::post, Router};
use std::net::SocketAddr;

/// Spin up a local axum server and return its base URL.
async fn start_mock(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

// ── Constructor ──────────────────────────────────────────

#[test]
fn defaults() {
    let p = OllamaEmbedding::default();
    assert_eq!(p.base_url, DEFAULT_OLLAMA_URL);
    assert_eq!(p.model, DEFAULT_OLLAMA_MODEL);
    assert_eq!(p.dims, DEFAULT_OLLAMA_DIMENSIONS);
}

#[test]
fn name_is_ollama() {
    let p = OllamaEmbedding::default();
    assert_eq!(p.name(), "ollama");
}

#[test]
fn custom_values() {
    let p = OllamaEmbedding::new("http://gpu-box:11434/", "mxbai-embed-large", 1024);
    assert_eq!(p.base_url, "http://gpu-box:11434");
    assert_eq!(p.model, "mxbai-embed-large");
    assert_eq!(p.dims, 1024);
}

#[test]
fn empty_values_use_defaults() {
    let p = OllamaEmbedding::new("", "", 0);
    assert_eq!(p.base_url, DEFAULT_OLLAMA_URL);
    assert_eq!(p.model, DEFAULT_OLLAMA_MODEL);
    assert_eq!(p.dims, DEFAULT_OLLAMA_DIMENSIONS);
}

#[test]
fn whitespace_only_values_use_defaults() {
    let p = OllamaEmbedding::new("   ", "  ", 0);
    assert_eq!(p.base_url, DEFAULT_OLLAMA_URL);
    assert_eq!(p.model, DEFAULT_OLLAMA_MODEL);
}

#[test]
fn trailing_slash_stripped() {
    let p = OllamaEmbedding::new("http://host:1234/", "m", 1);
    assert_eq!(p.base_url, "http://host:1234");
}

#[test]
fn model_trimmed() {
    let p = OllamaEmbedding::new("", "  nomic-embed-text  ", 768);
    assert_eq!(p.model, "nomic-embed-text");
}

#[test]
fn embed_url_format() {
    let p = OllamaEmbedding::default();
    assert_eq!(p.embed_url(), "http://localhost:11434/api/embed");
}

#[test]
fn accessor_methods() {
    let p = OllamaEmbedding::new("http://x:1", "m", 42);
    assert_eq!(p.base_url(), "http://x:1");
    assert_eq!(p.model(), "m");
    assert_eq!(p.dimensions(), 42);
}

// ── embed — empty / whitespace ──────────────────────────

#[tokio::test]
async fn empty_input_returns_empty() {
    let p = OllamaEmbedding::default();
    let result = p.embed(&[]).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn whitespace_only_input_returns_zero_vecs() {
    let p = OllamaEmbedding::default();
    let result = p.embed(&["  ", "\t", "\n"]).await.unwrap();
    // Length preserved, all entries are empty zero-vectors.
    assert_eq!(result.len(), 3);
    assert!(result.iter().all(|v| v.is_empty()));
}

// ── embed — positional alignment ────────────────────────

#[tokio::test]
async fn embed_preserves_positions_for_blanks() {
    let app = Router::new().route(
        "/api/embed",
        post(|Json(body): Json<serde_json::Value>| async move {
            let inputs = body["input"].as_array().unwrap();
            // Server receives only non-blank texts.
            let embeddings: Vec<Vec<f32>> = inputs.iter().map(|_| vec![1.0, 2.0]).collect();
            Json(serde_json::json!({ "embeddings": embeddings }))
        }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "m", 2);

    // Mix of blank and real texts.
    let result = p.embed(&["hello", "", "  ", "world"]).await.unwrap();
    assert_eq!(result.len(), 4);
    assert_eq!(result[0], vec![1.0, 2.0]); // real
    assert!(result[1].is_empty()); // blank
    assert!(result[2].is_empty()); // blank
    assert_eq!(result[3], vec![1.0, 2.0]); // real
}

// ── embed — successful response ─────────────────────────

#[tokio::test]
async fn embed_success_single() {
    let app = Router::new().route(
        "/api/embed",
        post(|Json(_body): Json<serde_json::Value>| async {
            Json(serde_json::json!({
                "embeddings": [[0.1, 0.2, 0.3]]
            }))
        }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "test-model", 3);

    let result = p.embed(&["hello"]).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], vec![0.1, 0.2, 0.3]);
}

#[tokio::test]
async fn embed_success_batch() {
    let app = Router::new().route(
        "/api/embed",
        post(|Json(_body): Json<serde_json::Value>| async {
            Json(serde_json::json!({
                "embeddings": [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]]
            }))
        }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "test-model", 2);

    let result = p.embed(&["a", "b", "c"]).await.unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!(result[2], vec![5.0, 6.0]);
}

#[tokio::test]
async fn embed_verifies_request_body() {
    let app = Router::new().route(
        "/api/embed",
        post(|Json(body): Json<serde_json::Value>| async move {
            assert_eq!(body["model"], "my-model");
            let inputs = body["input"].as_array().unwrap();
            assert_eq!(inputs.len(), 1);
            assert_eq!(inputs[0], "test text");
            Json(serde_json::json!({ "embeddings": [[1.0]] }))
        }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "my-model", 1);

    p.embed(&["test text"]).await.unwrap();
}

// ── embed — error paths ─────────────────────────────────

#[tokio::test]
async fn embed_server_error_with_body() {
    let app = Router::new().route(
        "/api/embed",
        post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "model crashed") }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "m", 1);

    let err = p.embed(&["hi"]).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("500"), "should contain status code: {msg}");
    assert!(msg.contains("model crashed"), "should contain body: {msg}");
}

#[tokio::test]
async fn embed_server_error_empty_body() {
    let app = Router::new().route(
        "/api/embed",
        post(|| async { (StatusCode::BAD_REQUEST, "") }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "m", 1);

    let err = p.embed(&["hi"]).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("400"), "should contain status code: {msg}");
}

#[tokio::test]
async fn embed_count_mismatch() {
    let app = Router::new().route(
        "/api/embed",
        post(|| async {
            // Return 1 embedding even though 2 texts were sent.
            Json(serde_json::json!({ "embeddings": [[1.0]] }))
        }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "m", 1);

    let err = p.embed(&["a", "b"]).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("count mismatch"), "msg: {msg}");
}

#[tokio::test]
async fn embed_dimension_mismatch() {
    let app = Router::new().route(
        "/api/embed",
        post(|| async {
            // Return 3-dim vector when provider expects 2.
            Json(serde_json::json!({ "embeddings": [[1.0, 2.0, 3.0]] }))
        }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "m", 2);

    let err = p.embed(&["hi"]).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("dimension mismatch"), "msg: {msg}");
}

#[tokio::test]
async fn embed_empty_embeddings_array() {
    let app = Router::new().route(
        "/api/embed",
        post(|| async { Json(serde_json::json!({ "embeddings": [] })) }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "m", 1);

    let err = p.embed(&["hi"]).await.unwrap_err();
    assert!(err.to_string().contains("count mismatch"));
}

#[tokio::test]
async fn embed_malformed_json_response() {
    let app = Router::new().route(
        "/api/embed",
        post(|| async { (StatusCode::OK, "not json at all") }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "m", 1);

    let err = p.embed(&["hi"]).await.unwrap_err();
    assert!(err.to_string().contains("parse failed"));
}

#[tokio::test]
async fn embed_connection_refused() {
    let p = OllamaEmbedding::new("http://127.0.0.1:1", "m", 1);
    let err = p.embed(&["hi"]).await.unwrap_err();
    assert!(
        err.to_string().contains("is Ollama running"),
        "should mention Ollama: {}",
        err
    );
}

// ── embed_one (trait default) ───────────────────────────

#[tokio::test]
async fn embed_one_success() {
    let app = Router::new().route(
        "/api/embed",
        post(|| async { Json(serde_json::json!({ "embeddings": [[7.0, 8.0]] })) }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "m", 2);

    let vec = p.embed_one("test").await.unwrap();
    assert_eq!(vec, vec![7.0, 8.0]);
}
