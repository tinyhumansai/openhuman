use super::interrupted_pull_settle_window_secs;

#[test]
fn interrupted_pull_waits_when_bytes_were_observed() {
    assert_eq!(interrupted_pull_settle_window_secs(true, 20), 20);
}

#[test]
fn interrupted_pull_does_not_wait_before_any_progress() {
    assert_eq!(interrupted_pull_settle_window_secs(false, 20), 0);
}

use crate::openhuman::config::Config;
use crate::openhuman::local_ai::service::LocalAiService;
use axum::{routing::get, Json, Router};
use serde_json::json;

async fn spawn_mock(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{}", addr.port())
}

#[tokio::test]
async fn has_model_detects_exact_and_prefixed_tag() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai mutex");

    let app = Router::new().route(
        "/api/tags",
        get(|| async {
            Json(json!({
                "models": [
                    {"name": "llama3:latest", "modified_at": "", "size": 1u64, "digest": "d"},
                    {"name": "nomic-embed-text:v1", "modified_at": "", "size": 2u64, "digest": "d"}
                ]
            }))
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    assert!(service.has_model("llama3").await.unwrap());
    assert!(service.has_model("llama3:latest").await.unwrap());
    assert!(service.has_model("nomic-embed-text").await.unwrap());
    assert!(!service.has_model("__missing__").await.unwrap());

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn has_model_errors_on_non_success_tags_response() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai mutex");

    let app = Router::new().route(
        "/api/tags",
        get(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    let err = service.has_model("any").await.unwrap_err();
    assert!(err.contains("500") || err.contains("tags failed"));

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn ollama_healthy_returns_true_on_200_tags_response() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai mutex");

    let app = Router::new().route("/api/tags", get(|| async { Json(json!({ "models": [] })) }));
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    assert!(service.ollama_healthy().await);

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn ollama_healthy_returns_false_on_unreachable_url() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai mutex");

    // Point at a port we never bind → connect fails → healthy = false.
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", "http://127.0.0.1:1");
    }
    let config = Config::default();
    let service = LocalAiService::new(&config);
    assert!(!service.ollama_healthy().await);
    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn diagnostics_reports_server_unreachable_when_url_unbound() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai mutex");

    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", "http://127.0.0.1:1");
    }
    let config = Config::default();
    let service = LocalAiService::new(&config);
    let diag = service.diagnostics(&config).await.expect("diagnostics");
    assert_eq!(diag["ollama_running"], false);
    let issues = diag["issues"].as_array().cloned().unwrap_or_default();
    assert!(
        !issues.is_empty(),
        "unreachable server must surface an issue"
    );
    assert!(issues
        .iter()
        .any(|v| v.as_str().unwrap_or("").contains("not running")));
    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn diagnostics_with_running_server_but_missing_models_flags_issues() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai mutex");

    let app = Router::new().route("/api/tags", get(|| async { Json(json!({ "models": [] })) }));
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    let diag = service.diagnostics(&config).await.expect("diagnostics");
    assert_eq!(diag["ollama_running"], true);
    // No models are installed → expected chat model issue surfaces.
    let issues = diag["issues"].as_array().cloned().unwrap_or_default();
    assert!(!issues.is_empty());
    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn diagnostics_ok_when_expected_models_are_present() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai mutex");

    let config = Config::default();
    let chat = crate::openhuman::local_ai::model_ids::effective_chat_model_id(&config);
    let chat_tag = format!("{}:latest", chat);
    let app = Router::new().route(
        "/api/tags",
        get(move || {
            let chat_tag = chat_tag.clone();
            async move {
                Json(json!({
                    "models": [
                        { "name": chat_tag, "modified_at": "", "size": 1u64, "digest": "d" }
                    ]
                }))
            }
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let service = LocalAiService::new(&config);
    let diag = service.diagnostics(&config).await.expect("diagnostics");
    assert_eq!(diag["ollama_running"], true);
    assert_eq!(diag["expected"]["chat_found"], true);
    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn list_models_returns_parsed_payload() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai mutex");

    let app = Router::new().route(
        "/api/tags",
        get(|| async {
            Json(json!({
                "models": [
                    { "name": "a:latest", "modified_at": "t", "size": 1u64, "digest": "d1" },
                    { "name": "b:v2", "modified_at": "t", "size": 2u64, "digest": "d2" }
                ]
            }))
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    let models = service.list_models().await.expect("list_models");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].name, "a:latest");
    assert_eq!(models[1].name, "b:v2");
    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn list_models_errors_on_non_success() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai mutex");

    let app = Router::new().route(
        "/api/tags",
        get(|| async { (axum::http::StatusCode::SERVICE_UNAVAILABLE, "down") }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    let err = service.list_models().await.unwrap_err();
    assert!(err.contains("503") || err.contains("tags failed"));
    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}
