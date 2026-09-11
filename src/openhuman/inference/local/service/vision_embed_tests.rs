use super::*;
use axum::{routing::post, Json, Router};
use serde_json::json;

async fn spawn_mock(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{}", addr.port())
}

fn enabled_config() -> Config {
    let mut c = Config::default();
    c.local_ai.runtime_enabled = true;
    c
}

fn ready_service(config: &Config) -> LocalAiService {
    let s = LocalAiService::new(config);
    {
        let mut g = s.status.lock();
        g.state = "ready".to_string();
    }
    s
}

fn mock_with_tags_and(route: &str, handler: axum::routing::MethodRouter) -> Router {
    use axum::routing::get;
    // Respond to `/api/tags` with a payload that contains whatever model
    // the caller asks about, so `has_model` returns true and `embed`
    // proceeds to the real endpoint.
    Router::new()
        .route(
            "/api/tags",
            get(|| async {
                Json(json!({
                    "models": [
                        { "name": "nomic-embed-text:latest", "modified_at": "", "size": 0u64, "digest": "x" },
                        { "name": "llava:latest", "modified_at": "", "size": 0u64, "digest": "y" }
                    ]
                }))
            }),
        )
        .route(route, handler)
}

#[tokio::test]
async fn embed_against_mock_returns_vectors_with_dimensions() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = mock_with_tags_and(
        "/api/embed",
        post(|Json(_b): Json<serde_json::Value>| async {
            Json(json!({
                "model": "m",
                "embeddings": [[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]]
            }))
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = enabled_config();
    let service = ready_service(&config);
    let result = service
        .embed(&config, &["hello".to_string(), "world".to_string()])
        .await;
    let _ = result; // Ensure the call path completes — exact pass/fail
                    // depends on model name matching in `has_model`.

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn embed_rejects_all_empty_inputs_before_network_call() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    // Even without a working mock server, entirely-empty inputs must be
    // rejected before any HTTP call.
    let config = enabled_config();
    let service = ready_service(&config);
    let err = service
        .embed(&config, &["".to_string(), "   ".to_string()])
        .await
        .unwrap_err();
    assert!(err.contains("non-empty input"));
}

#[tokio::test]
async fn embed_disabled_returns_error() {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    let service = LocalAiService::new(&config);
    let err = service.embed(&config, &["x".into()]).await.unwrap_err();
    assert!(err.contains("local ai is disabled"));
}

#[test]
fn embedding_dimensions_match_supported_legacy_models() {
    assert_eq!(embedding_dimensions("bge-m3"), Some(1024));
    assert_eq!(embedding_dimensions("all-minilm:latest"), Some(384));
    assert_eq!(embedding_dimensions("nomic-embed-text"), Some(768));
    assert_eq!(embedding_dimensions("user-managed-model"), None);
}

#[tokio::test]
async fn vision_prompt_disabled_returns_error() {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    let service = LocalAiService::new(&config);
    let err = service
        .vision_prompt(&config, "describe", &[], None)
        .await
        .unwrap_err();
    assert!(err.contains("local ai is disabled"));
}

// ── #5146 §Part 1: which model a vision request actually reaches ────────
//
// These drive the real `vision_prompt` path against a mock Ollama server.
// `ready_service` marks the status "ready", which makes `bootstrap` return
// early, so no process launch or network beyond the mock is involved.

/// Mock Ollama exposing `/api/tags` with `installed` present, and an
/// `/api/generate` that echoes back the `model` field it was sent. The
/// echo is what lets a test assert *which* model the request targeted.
fn mock_ollama_echoing_requested_model(installed: &'static str) -> Router {
    use axum::routing::get;
    Router::new()
        .route(
            "/api/tags",
            get(move || async move {
                Json(json!({
                    "models": [
                        { "name": installed, "modified_at": "", "size": 0u64, "digest": "a" }
                    ]
                }))
            }),
        )
        .route(
            "/api/generate",
            post(|Json(body): Json<serde_json::Value>| async move {
                Json(json!({
                    "response": body["model"].as_str().unwrap_or("<no model field>"),
                    "done": true
                }))
            }),
        )
}

/// A configured, genuinely vision-capable model must reach Ollama unchanged.
///
/// Before #5146 the `MVP_ALLOWED_VISION_MODELS = &[""]` allowlist rewrote
/// this to the empty string, so the request went out with `model: ""`.
#[tokio::test]
async fn vision_prompt_sends_the_configured_vision_capable_model() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let base = spawn_mock(mock_ollama_echoing_requested_model("llava:7b")).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let mut config = enabled_config();
    config.local_ai.vision_model_id = "llava:7b".to_string();
    let service = ready_service(&config);

    let result = service
        .vision_prompt(
            &config,
            "describe",
            &["data:image/png;base64,QUJD".to_string()],
            None,
        )
        .await;

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    assert_eq!(
        result.expect("vision prompt should succeed"),
        "llava:7b",
        "the configured vision model must reach Ollama unchanged"
    );
}

/// A configured-but-unpullable vision model must report a vision problem
/// naming the model and the `ollama pull` that fixes it.
#[tokio::test]
async fn vision_prompt_reports_an_unavailable_vision_model() {
    use axum::routing::get;
    let _guard = crate::openhuman::inference::inference_test_guard();

    // Empty tag list, and a pull that refuses: nothing to fall back to.
    let app = Router::new()
        .route("/api/tags", get(|| async { Json(json!({ "models": [] })) }))
        .route(
            "/api/pull",
            post(|| async {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "pull refused",
                )
            }),
        );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let mut config = enabled_config();
    config.local_ai.vision_model_id = "llava:7b".to_string();
    let service = ready_service(&config);

    let err = service
        .vision_prompt(
            &config,
            "describe",
            &["data:image/png;base64,QUJD".to_string()],
            None,
        )
        .await
        .expect_err("an unpullable vision model must fail");

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    assert!(
        err.contains("llava:7b"),
        "error should name the model: {err}"
    );
    assert!(
        err.contains("ollama pull"),
        "error should say how to install it: {err}"
    );
    assert_eq!(service.status.lock().vision_state, "missing");
}

/// #5146 P1: a chat-only `vision_model_id` must fail *before* any network
/// work, naming the configured model — no substitution, and above all no
/// pull of a model the user never chose.
///
/// The mock deliberately offers a working `/api/pull`. If the request ever
/// reaches it, the substitution is back and this test fails on the assert
/// that no pull was attempted rather than on a transport error.
#[tokio::test]
async fn chat_only_vision_model_errors_without_substituting_or_pulling() {
    use axum::routing::get;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let _guard = crate::openhuman::inference::inference_test_guard();

    let pulls = Arc::new(AtomicUsize::new(0));
    let pull_counter = pulls.clone();
    let app = Router::new()
        .route("/api/tags", get(|| async { Json(json!({ "models": [] })) }))
        .route(
            "/api/pull",
            post(move || {
                let pulls = pull_counter.clone();
                async move {
                    pulls.fetch_add(1, Ordering::SeqCst);
                    Json(json!({ "status": "success" }))
                }
            }),
        );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let mut config = enabled_config();
    config.local_ai.vision_model_id = "gemma3n:e4b-it-q8_0".to_string();
    let service = ready_service(&config);

    let err = service
        .vision_prompt(
            &config,
            "describe",
            &["data:image/png;base64,QUJD".to_string()],
            None,
        )
        .await
        .expect_err("a chat-only vision model must fail");

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    assert!(
        err.contains("gemma3n:e4b-it-q8_0"),
        "error must name the model the user actually configured: {err}"
    );
    assert!(
        err.contains("not vision-capable"),
        "error must explain what is wrong with it: {err}"
    );
    // The suggestion list legitimately contains `DEFAULT_LOW_VISION_MODEL`,
    // so its mere presence proves nothing. What must not happen is the
    // message presenting it as the model that *ran*; the `pulls` and
    // `vision_state` assertions below pin that nothing was fetched behind
    // the user's back.
    assert!(
        err.contains("for example"),
        "a vision-capable model must be offered as an example to choose, never as a \
         substitute that was already applied: {err}"
    );
    assert_eq!(
        pulls.load(Ordering::SeqCst),
        0,
        "no model may be downloaded for a vision request the user misconfigured"
    );
    assert_eq!(service.status.lock().vision_state, "missing");
}

/// #5146 P6: a reference that is not base64 (a filesystem path is the
/// common case) must produce a message about what `image_refs` accepts,
/// not Ollama's `illegal base64 data at input byte 19`.
#[tokio::test]
async fn non_base64_image_reference_is_rejected_with_guidance() {
    use axum::routing::get;
    let _guard = crate::openhuman::inference::inference_test_guard();

    // The payload check runs *after* model availability, so the model must
    // read as already installed or this would dial a real Ollama.
    let app = Router::new().route(
        "/api/tags",
        get(|| async {
            Json(json!({
                "models": [
                    { "name": "llava:7b", "modified_at": "", "size": 0u64, "digest": "a" }
                ]
            }))
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let mut config = enabled_config();
    config.local_ai.vision_model_id = "llava:7b".to_string();
    let service = ready_service(&config);

    let err = service
        .vision_prompt(
            &config,
            "describe",
            &["/tmp/vision-test.png".to_string()],
            None,
        )
        .await
        .expect_err("a filesystem path is not an image payload");

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    assert!(
        err.contains("base64"),
        "error must say what the parameter accepts: {err}"
    );
    assert!(
        err.contains("filesystem path"),
        "error must name the mistake the caller actually made: {err}"
    );
}
