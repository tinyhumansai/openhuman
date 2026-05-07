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
    assert!(
        diag["ollama_base_url"].as_str().is_some(),
        "diagnostics must include ollama_base_url"
    );
    let issues = diag["issues"].as_array().cloned().unwrap_or_default();
    assert!(
        !issues.is_empty(),
        "unreachable server must surface an issue"
    );
    assert!(issues
        .iter()
        .any(|v| v.as_str().unwrap_or("").contains("not running")));
    let repair_actions = diag["repair_actions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !repair_actions.is_empty(),
        "unreachable server must produce at least one repair action"
    );
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
    assert_eq!(
        diag["ollama_base_url"].as_str(),
        Some(base.as_str()),
        "diagnostics must echo back the base url being checked"
    );
    // No models are installed → expected chat model issue surfaces.
    let issues = diag["issues"].as_array().cloned().unwrap_or_default();
    assert!(!issues.is_empty());
    // Missing chat model should produce a pull_model repair action.
    let repair_actions = diag["repair_actions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        repair_actions
            .iter()
            .any(|a| a["action"].as_str() == Some("pull_model")),
        "missing models must produce pull_model repair action"
    );
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
    let embedding = crate::openhuman::local_ai::model_ids::effective_embedding_model_id(&config);
    let chat_tag = format!("{}:latest", chat);
    let embed_tag = format!("{}:latest", embedding);
    let app = Router::new().route(
        "/api/tags",
        get(move || {
            let chat_tag = chat_tag.clone();
            let embed_tag = embed_tag.clone();
            async move {
                Json(json!({
                    "models": [
                        { "name": chat_tag, "modified_at": "", "size": 1u64, "digest": "d" },
                        { "name": embed_tag, "modified_at": "", "size": 2u64, "digest": "e" },
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
    assert_eq!(diag["expected"]["embedding_found"], true);
    assert!(diag["ollama_base_url"].as_str().is_some());
    // All required models present → no issues and no repair actions.
    let issues = diag["issues"].as_array().cloned().unwrap_or_default();
    assert!(
        issues.is_empty(),
        "all models present should produce no issues, got: {:?}",
        issues
    );
    let repair_actions = diag["repair_actions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        repair_actions.is_empty(),
        "no issues should produce no repair actions"
    );
    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn resolve_binary_path_finds_binary_via_ollama_bin_env() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai mutex");

    let tmp = tempfile::tempdir().unwrap();
    let fake_bin = tmp.path().join(if cfg!(windows) {
        "ollama.exe"
    } else {
        "ollama"
    });
    std::fs::write(&fake_bin, b"stub").unwrap();

    unsafe {
        std::env::set_var("OLLAMA_BIN", fake_bin.to_str().unwrap());
        // Point the base URL at a dead port so we don't depend on a real server.
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", "http://127.0.0.1:1");
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    let diag = service.diagnostics(&config).await.expect("diagnostics");
    assert_eq!(
        diag["ollama_binary_path"].as_str(),
        Some(fake_bin.to_str().unwrap()),
        "diagnostics should resolve binary via OLLAMA_BIN"
    );

    unsafe {
        std::env::remove_var("OLLAMA_BIN");
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn diagnostics_repair_actions_include_start_server_when_binary_known() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai mutex");

    let tmp = tempfile::tempdir().unwrap();
    let fake_bin = tmp.path().join(if cfg!(windows) {
        "ollama.exe"
    } else {
        "ollama"
    });
    std::fs::write(&fake_bin, b"stub").unwrap();

    unsafe {
        std::env::set_var("OLLAMA_BIN", fake_bin.to_str().unwrap());
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", "http://127.0.0.1:1");
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    let diag = service.diagnostics(&config).await.expect("diagnostics");

    assert_eq!(diag["ollama_running"], false);
    let repair_actions = diag["repair_actions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        repair_actions
            .iter()
            .any(|a| a["action"].as_str() == Some("start_server")),
        "when binary is known but server is down, repair action should be start_server"
    );

    unsafe {
        std::env::remove_var("OLLAMA_BIN");
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn diagnostics_repair_actions_field_always_present() {
    // Verifies that the "repair_actions" key is always present in the diagnostics
    // JSON, regardless of the server state, so the UI can always iterate over it.
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai mutex");

    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", "http://127.0.0.1:1");
    }
    let config = Config::default();
    let service = LocalAiService::new(&config);
    let diag = service.diagnostics(&config).await.expect("diagnostics");

    assert!(
        diag["repair_actions"].is_array(),
        "repair_actions must always be a JSON array"
    );

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
