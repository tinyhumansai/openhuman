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
    let mut config = Config::default();
    config.local_ai.enabled = true;
    config
}

/// Build a LocalAiService pre-seeded to `ready` so inference calls skip
/// `bootstrap()` and hit the HTTP path directly.
fn ready_service(config: &Config) -> LocalAiService {
    let service = LocalAiService::new(config);
    {
        let mut guard = service.status.lock();
        guard.state = "ready".to_string();
    }
    service
}

#[tokio::test]
async fn inference_hits_ollama_generate_and_returns_response() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai test mutex");

    let app = Router::new().route(
        "/api/generate",
        post(|Json(_body): Json<serde_json::Value>| async move {
            Json(json!({
                "model": "test",
                "response": "hello from mock",
                "done": true,
                "total_duration": 1_000_000u64,
                "prompt_eval_count": 5,
                "prompt_eval_duration": 100_000u64,
                "eval_count": 3,
                "eval_duration": 500_000u64
            }))
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = enabled_config();
    let service = ready_service(&config);
    let reply = service
        .prompt(&config, "hi", Some(16), true)
        .await
        .expect("ollama prompt");
    assert_eq!(reply, "hello from mock");

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn inference_errors_on_non_success_status() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai test mutex");

    let app = Router::new().route(
        "/api/generate",
        post(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = enabled_config();
    let service = ready_service(&config);
    let err = service.prompt(&config, "hi", None, true).await.unwrap_err();
    assert!(err.contains("500"));

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn inference_errors_on_empty_response_when_allow_empty_false() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai test mutex");

    let app = Router::new().route(
        "/api/generate",
        post(|| async {
            Json(json!({
                "model": "test",
                "response": "   ",
                "done": true
            }))
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = enabled_config();
    let service = ready_service(&config);
    // `inference()` is the lower-level entry that hard-codes
    // allow_empty=false, so a whitespace-only mock response must
    // surface as the "empty content" error.
    let res = service.inference(&config, "", "hi", None, false).await;

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    let err = res.expect_err("whitespace response must be rejected when allow_empty=false");
    assert!(
        err.contains("empty"),
        "expected an empty-content error, got: {err}"
    );
}

#[tokio::test]
async fn suggest_questions_parses_line_separated_output() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai test mutex");

    let app = Router::new().route(
        "/api/generate",
        post(|| async {
            Json(json!({
                "model": "test",
                "response": "What next?\nHow about this?\nTell me more.",
                "done": true
            }))
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let mut config = enabled_config();
    config.local_ai.max_suggestions = 3;
    let service = ready_service(&config);
    let suggestions = service
        .suggest_questions(&config, "prior context")
        .await
        .expect("suggest_questions");
    assert!(!suggestions.is_empty());

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn summarize_disabled_returns_error() {
    // When local_ai is disabled the summarize fn should short-circuit.
    let mut config = Config::default();
    config.local_ai.enabled = false;
    let service = LocalAiService::new(&config);
    let err = service.summarize(&config, "text", None).await.unwrap_err();
    assert!(err.contains("local ai is disabled"));
}

#[tokio::test]
async fn prompt_disabled_returns_error() {
    let mut config = Config::default();
    config.local_ai.enabled = false;
    let service = LocalAiService::new(&config);
    let err = service
        .prompt(&config, "text", None, false)
        .await
        .unwrap_err();
    assert!(err.contains("local ai is disabled"));
}

#[tokio::test]
async fn suggest_questions_disabled_returns_empty() {
    let mut config = Config::default();
    config.local_ai.enabled = false;
    let service = LocalAiService::new(&config);
    let out = service.suggest_questions(&config, "ctx").await.unwrap();
    assert!(out.is_empty());
}

#[tokio::test]
async fn inline_complete_disabled_returns_empty_string() {
    let mut config = Config::default();
    config.local_ai.enabled = false;
    let service = LocalAiService::new(&config);
    let out = service
        .inline_complete(&config, "ctx", "casual", None, &[], None)
        .await
        .unwrap();
    assert!(out.is_empty());
}
