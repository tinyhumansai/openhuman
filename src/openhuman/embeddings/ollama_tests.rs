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
fn base_url_edge_cases_build_embed_url() {
    let cases = [
        ("http://host:11434/", "http://host:11434/api/embed"),
        ("http://[::1]:11434", "http://[::1]:11434/api/embed"),
        ("http://host", "http://host/api/embed"),
    ];

    for (base_url, expected) in cases {
        let p = OllamaEmbedding::try_new(base_url, "m", 1).unwrap();
        assert_eq!(p.embed_url().unwrap(), expected);
    }
}

#[test]
fn rejects_api_endpoint_base_urls() {
    for base_url in [
        "http://host:11434/v1",
        "http://host:11434/api",
        "http://host:11434/api/embed",
        "http://host:11434/v1/chat/completions",
        "http://host:11434/chat/completions",
    ] {
        let err = OllamaEmbedding::try_new(base_url, "m", 1).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Ollama server root"),
            "should reject pre-suffixed base URL {base_url}: {msg}"
        );
    }
}

#[test]
fn rejects_credentialed_base_urls() {
    let err = OllamaEmbedding::try_new("http://user:pass@host:11434", "m", 1).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("without credentials"), "msg: {msg}");
}

#[test]
fn rejects_virtual_local_model_ids() {
    let err = OllamaEmbedding::try_new("http://host:11434", "local-v1", 768).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("local-*"), "msg: {msg}");
    assert!(msg.contains(DEFAULT_OLLAMA_MODEL), "msg: {msg}");
}

#[test]
fn model_trimmed() {
    let p = OllamaEmbedding::new("", "  nomic-embed-text  ", 768);
    assert_eq!(p.model, "nomic-embed-text");
}

#[test]
fn embed_url_format() {
    let p = OllamaEmbedding::default();
    assert_eq!(p.embed_url().unwrap(), "http://localhost:11434/api/embed");
}

#[test]
fn accessor_methods() {
    let p = OllamaEmbedding::new("http://x:1", "m", 42);
    assert_eq!(p.base_url(), "http://x:1");
    assert_eq!(p.model(), "m");
    assert_eq!(p.model_id(), "m");
    assert_eq!(p.dimensions(), 42);
    assert_eq!(p.signature(), "provider=ollama;model=m;dims=42");
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

// OPENHUMAN-TAURI-{GP,MA,KM,GX} + TAURI-RUST-XS wire shapes — routed through
// `report_error_or_expected` (Sentry classifier ladder) and demoted to
// `ProviderUserState` / `LocalAiCapabilityUnavailable` by the
// `is_ollama_user_config_rejection` matcher in `src/core/observability.rs`.
// GP catches the RAM-tier disable shape; XS/MA/KM/GX cover the four Ollama
// user-config rejection shapes (400 invalid-model-name, 404 model-not-found
// ×2, daemon-unreachable opt-in state).

#[test]
fn xs_wire_shape_classifies_as_provider_user_state() {
    // TAURI-RUST-XS (~376 events on self-hosted Sentry): user pointed
    // embedder at a chat / vision model with a temperature suffix
    // (`qwen3-vl:4b@0.7`) Ollama parses as malformed.
    let msg = r#"ollama embed failed with status 400 Bad Request: {"error":"invalid model name"}"#;
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        Some(crate::core::observability::ExpectedErrorKind::ProviderUserState),
        "XS — invalid model name 400 is pure user-config, must demote"
    );
}

#[test]
fn xs_temperature_suffix_model_classifies_as_provider_user_state() {
    // Same XS shape with the temperature-suffix model id embedded in the
    // body. Real wire shape from the field report.
    let msg = r#"ollama embed failed with status 400 Bad Request: {"error":"invalid model name: qwen3-vl:4b@0.7"}"#;
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        Some(crate::core::observability::ExpectedErrorKind::ProviderUserState),
        "XS — temperature-suffix model id must also demote"
    );
}

#[test]
fn ma_wire_shape_classifies_as_provider_user_state() {
    // OPENHUMAN-TAURI-MA: user configured an Ollama model id that the
    // local daemon hasn't pulled yet.
    let msg = r#"ollama embed failed with status 404 Not Found: {"error":"model \"bge-m3\" not found, try pulling it first"}"#;
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        Some(crate::core::observability::ExpectedErrorKind::ProviderUserState),
        "MA — model-not-found 404 is pure user-state (pull required), must demote"
    );
}

#[test]
fn km_wire_shape_classifies_as_provider_user_state() {
    // OPENHUMAN-TAURI-KM: same wire shape as MA with a different model
    // id + `:latest` tag.
    let msg = r#"ollama embed failed with status 404 Not Found: {"error":"model \"nomic-embed-text:latest\" not found, try pulling it first"}"#;
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        Some(crate::core::observability::ExpectedErrorKind::ProviderUserState),
        "KM — pull-required 404 must demote"
    );
}

#[test]
fn gp_wire_shape_classifies() {
    let msg =
        "Vision is disabled for this RAM tier. Switch to the 4-8 GB tier or above to enable it.";
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        Some(crate::core::observability::ExpectedErrorKind::LocalAiCapabilityUnavailable),
        "GP — LocalAiCapabilityUnavailable matcher must catch this"
    );
}

#[test]
fn gx_wire_shape_classifies_as_provider_user_state() {
    // OPENHUMAN-TAURI-GX: user opted into Ollama embeddings in Settings
    // but the daemon isn't running on localhost:11434.
    let msg = "ollama embeddings opted-in but daemon unreachable at http://localhost:11434; falling back to cloud embeddings for this session";
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        Some(crate::core::observability::ExpectedErrorKind::ProviderUserState),
        "GX — daemon-unreachable opt-in state is pure user-config (start daemon)"
    );
}

#[test]
fn ollama_500_wire_shape_stays_unexpected() {
    // Server-side ollama bug — must still reach Sentry.
    let msg = "ollama embed failed with status 500 Internal Server Error: model crashed";
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        None,
        "real ollama server errors must still reach Sentry"
    );
}

#[test]
fn ollama_parse_error_wire_shape_stays_unexpected() {
    let msg = "ollama embed response parse failed: invalid type: expected sequence";
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        None,
        "real parse bugs must still reach Sentry"
    );
}

#[test]
fn ollama_dimension_mismatch_stays_unexpected() {
    // Dimension mismatch — real bug (model dims don't match what we
    // recorded for the provider in Settings), must still reach Sentry
    // so we can investigate the desync.
    let msg = "ollama embed dimension mismatch at index 0: expected 768, got 1024";
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        None,
        "dimension-mismatch shape must still reach Sentry"
    );
}

// ── embed — NaN-encoding 500 recovery (TAURI-RUST-AZ) ───

#[test]
fn is_nan_encode_error_matches_ollama_wire_shape() {
    // Canonical wire shape from Sentry TAURI-RUST-AZ.
    assert!(is_nan_encode_error(
        r#"{"error":"failed to encode response: json: unsupported value: NaN"}"#
    ));
    // Case-insensitive on the "NaN" token.
    assert!(is_nan_encode_error("unsupported value: nan"));
    assert!(is_nan_encode_error("UNSUPPORTED VALUE: NAN"));
    // Negative: unrelated 500 bodies must not match.
    assert!(!is_nan_encode_error("model crashed"));
    assert!(!is_nan_encode_error(
        r#"{"error":"failed to encode response: json: unsupported value: +Inf"}"#
    ));
    assert!(!is_nan_encode_error(""));
}

#[tokio::test]
async fn embed_nan_batch_recovers_via_per_text() {
    // Ollama returns NaN-encoding 500 for any batch > 1, but succeeds on
    // single-text requests. Recovery should re-issue per-text and assemble
    // the full result.
    let app = Router::new().route(
        "/api/embed",
        post(|Json(body): Json<serde_json::Value>| async move {
            let n = body["input"].as_array().unwrap().len();
            if n > 1 {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    String::from(
                        r#"{"error":"failed to encode response: json: unsupported value: NaN"}"#,
                    ),
                )
            } else {
                (
                    StatusCode::OK,
                    serde_json::json!({ "embeddings": [[1.0, 2.0]] }).to_string(),
                )
            }
        }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "m", 2);

    let result = p.embed(&["a", "b", "c"]).await.unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!(result[0], vec![1.0, 2.0]);
    assert_eq!(result[1], vec![1.0, 2.0]);
    assert_eq!(result[2], vec![1.0, 2.0]);
}

#[tokio::test]
async fn embed_nan_persists_per_text_substitutes_empty() {
    // Every request — batch or single — returns NaN 500. Recovery
    // substitutes empty vectors for each input rather than bailing.
    let app = Router::new().route(
        "/api/embed",
        post(|| async {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                String::from(
                    r#"{"error":"failed to encode response: json: unsupported value: NaN"}"#,
                ),
            )
        }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "m", 2);

    let result = p.embed(&["a", "b"]).await.unwrap();
    assert_eq!(result.len(), 2);
    assert!(result[0].is_empty(), "NaN-failed entries must be empty vec");
    assert!(result[1].is_empty(), "NaN-failed entries must be empty vec");
}

#[tokio::test]
async fn embed_nan_single_text_short_circuits_to_empty() {
    // Single-input NaN should NOT trigger a wasted retry — short-circuit
    // straight to empty vector.
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let count = Arc::new(AtomicUsize::new(0));
    let count_clone = count.clone();
    let app = Router::new().route(
        "/api/embed",
        post(move || {
            let c = count_clone.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    String::from(
                        r#"{"error":"failed to encode response: json: unsupported value: NaN"}"#,
                    ),
                )
            }
        }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "m", 2);

    let result = p.embed(&["only-text"]).await.unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].is_empty());
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "single-text NaN must not retry — exactly one request expected"
    );
}

#[tokio::test]
async fn embed_nan_mixed_per_text_outcomes() {
    // Batch NaN-fails. Per-text retries: input "bad" returns NaN, others
    // succeed. Result vector has empty slot for "bad" and embeddings for
    // the rest, with positions preserved.
    let app = Router::new().route(
        "/api/embed",
        post(|Json(body): Json<serde_json::Value>| async move {
            let inputs = body["input"].as_array().unwrap();
            if inputs.len() > 1 {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    String::from(
                        r#"{"error":"failed to encode response: json: unsupported value: NaN"}"#,
                    ),
                );
            }
            let text = inputs[0].as_str().unwrap();
            if text == "bad" {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    String::from(
                        r#"{"error":"failed to encode response: json: unsupported value: NaN"}"#,
                    ),
                )
            } else {
                (
                    StatusCode::OK,
                    serde_json::json!({ "embeddings": [[9.0, 9.0]] }).to_string(),
                )
            }
        }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "m", 2);

    let result = p.embed(&["good1", "bad", "good2"]).await.unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!(result[0], vec![9.0, 9.0]);
    assert!(result[1].is_empty(), "NaN entry must be empty vec");
    assert_eq!(result[2], vec![9.0, 9.0]);
}

#[tokio::test]
async fn embed_non_nan_500_still_errors() {
    // Real ollama 500s (anything that isn't the NaN wire shape) must still
    // bail loudly — only NaN gets the recovery path.
    let app = Router::new().route(
        "/api/embed",
        post(|| async {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                String::from("model crashed"),
            )
        }),
    );
    let url = start_mock(app).await;
    let p = OllamaEmbedding::new(&url, "m", 2);

    let err = p.embed(&["a", "b"]).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("500"));
    assert!(msg.contains("model crashed"));
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
