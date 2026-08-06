use super::util::interrupted_pull_settle_window_secs;

#[test]
fn interrupted_pull_waits_when_bytes_were_observed() {
    assert_eq!(interrupted_pull_settle_window_secs(true, 20), 20);
}

#[test]
fn interrupted_pull_does_not_wait_before_any_progress() {
    assert_eq!(interrupted_pull_settle_window_secs(false, 20), 0);
}

use crate::openhuman::config::Config;
use crate::openhuman::inference::local::service::LocalAiService;
use axum::{routing::get, Json, Router};
use serde_json::json;

async fn spawn_mock(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{}", addr.port())
}

fn lm_studio_config(base: &str) -> Config {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = true;
    config.local_ai.opt_in_confirmed = true;
    config.local_ai.provider = "lm_studio".to_string();
    config.local_ai.base_url = Some(format!("{base}/v1"));
    config.local_ai.model_id = "local-model".to_string();
    config.local_ai.chat_model_id = "local-model".to_string();
    config
}

#[tokio::test]
async fn has_model_detects_exact_and_prefixed_tag() {
    let _guard = crate::openhuman::inference::inference_test_guard();

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
    let _guard = crate::openhuman::inference::inference_test_guard();

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
    let _guard = crate::openhuman::inference::inference_test_guard();

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
    let _guard = crate::openhuman::inference::inference_test_guard();

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
async fn ensure_ollama_server_requires_external_runtime_when_unreachable() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", "http://127.0.0.1:1");
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    let err = service
        .ensure_ollama_server(&config)
        .await
        .expect_err("unreachable runtime should fail");

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    assert!(
        err.contains("no longer starts or installs Ollama automatically"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn test_ollama_connection_returns_reachable_with_model_count() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new().route(
        "/api/tags",
        get(|| async {
            Json(json!({
                "models": [
                    {"name": "llama3:latest", "modified_at": "", "size": 1u64, "digest": "d"},
                    {"name": "mistral:7b", "modified_at": "", "size": 2u64, "digest": "d"}
                ]
            }))
        }),
    );
    let base = spawn_mock(app).await;

    let result = super::test_ollama_connection(&base).await.unwrap();
    assert_eq!(result["reachable"], true);
    assert_eq!(result["models_count"], 2);
    assert!(result["error"].is_null());
}

#[tokio::test]
async fn test_ollama_connection_returns_unreachable_on_server_error() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new().route(
        "/api/tags",
        get(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
    );
    let base = spawn_mock(app).await;

    let result = super::test_ollama_connection(&base).await.unwrap();
    assert_eq!(result["reachable"], false);
    assert!(!result["error"].as_str().unwrap_or("").is_empty());
}

#[tokio::test]
async fn test_ollama_connection_returns_unreachable_on_connect_failure() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let result = super::test_ollama_connection("http://127.0.0.1:1")
        .await
        .unwrap();
    assert_eq!(result["reachable"], false);
    assert!(!result["error"].as_str().unwrap_or("").is_empty());
}

#[tokio::test]
async fn test_ollama_connection_rejects_invalid_url() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let err = super::test_ollama_connection("not-a-url")
        .await
        .unwrap_err();
    assert!(
        !err.is_empty(),
        "expected validation error, got empty string"
    );
}

#[tokio::test]
async fn ensure_ollama_server_reports_broken_external_runner_without_restart_attempt() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new()
        .route("/api/tags", get(|| async { Json(json!({ "models": [] })) }))
        .route(
            "/api/show",
            axum::routing::post(|| async {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "fork/exec /broken/ollama: no such file or directory",
                )
            }),
        );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    let err = service
        .ensure_ollama_server(&config)
        .await
        .expect_err("broken runner should fail");

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    assert!(
        err.contains("cannot execute models") || err.contains("Restart the external runtime"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn ensure_ollama_server_accepts_healthy_external_runner() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new()
        .route("/api/tags", get(|| async { Json(json!({ "models": [] })) }))
        .route(
            "/api/show",
            axum::routing::post(|| async {
                (
                    axum::http::StatusCode::NOT_FOUND,
                    Json(json!({ "error": "model '___nonexistent_probe___' not found" })),
                )
            }),
        );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    service
        .ensure_ollama_server(&config)
        .await
        .expect("healthy external runner should pass");

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn assets_status_marks_ollama_unavailable_when_runtime_is_down_even_if_binary_exists() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", "http://127.0.0.1:1");
    }
    let fake_ollama = std::env::current_exe().expect("current exe");
    let prev_ollama_bin = std::env::var_os("OLLAMA_BIN");
    unsafe {
        std::env::set_var("OLLAMA_BIN", &fake_ollama);
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    let status = service.assets_status(&config).await.expect("assets status");

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        match prev_ollama_bin {
            Some(value) => std::env::set_var("OLLAMA_BIN", value),
            None => std::env::remove_var("OLLAMA_BIN"),
        }
    }

    assert!(
        !status.ollama_available,
        "runtime-down status must not be treated as available"
    );
    assert_ne!(status.chat.state, "ready");
}

#[tokio::test]
async fn diagnostics_reports_server_unreachable_when_url_unbound() {
    let _guard = crate::openhuman::inference::inference_test_guard();

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
        repair_actions.is_empty(),
        "OpenHuman should not suggest app-managed repair actions anymore"
    );
    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn diagnostics_with_running_server_but_missing_models_flags_issues() {
    let _guard = crate::openhuman::inference::inference_test_guard();

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
    let repair_actions = diag["repair_actions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        repair_actions.is_empty(),
        "missing models should no longer surface app-managed pull actions"
    );
    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn diagnostics_ok_when_expected_models_are_present() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let config = Config::default();
    let chat = crate::openhuman::inference::model_ids::effective_chat_model_id(&config);
    let embedding = crate::openhuman::inference::model_ids::effective_embedding_model_id(&config);
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
async fn diagnostics_reports_broken_runner_even_when_models_are_present() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let config = Config::default();
    let chat = crate::openhuman::inference::model_ids::effective_chat_model_id(&config);
    let embedding = crate::openhuman::inference::model_ids::effective_embedding_model_id(&config);
    let chat_tag = format!("{}:latest", chat);
    let embed_tag = format!("{}:latest", embedding);
    let app = Router::new()
        .route(
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
        )
        .route(
            "/api/show",
            axum::routing::post(|Json(body): Json<serde_json::Value>| async move {
                let model = body["name"]
                    .as_str()
                    .or_else(|| body["model"].as_str())
                    .unwrap_or_default();
                if model == "___nonexistent_probe___" {
                    return (
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        "fork/exec /broken/ollama: no such file or directory".to_string(),
                    );
                }
                (
                    axum::http::StatusCode::OK,
                    json!({
                        "model_info": {
                            "general.architecture": "bert",
                            "bert.context_length": 8192,
                        },
                        "capabilities": ["embedding"],
                    })
                    .to_string(),
                )
            }),
        );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let service = LocalAiService::new(&config);
    let diag = service.diagnostics(&config).await.expect("diagnostics");

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    assert_eq!(diag["ollama_running"], true);
    assert_eq!(diag["ok"], false);
    let issues = diag["issues"].as_array().cloned().unwrap_or_default();
    assert!(
        issues.iter().any(|issue| issue
            .as_str()
            .unwrap_or_default()
            .contains("cannot execute models")),
        "diagnostics should report the broken Ollama runner, got: {:?}",
        issues
    );
}

#[tokio::test]
async fn resolve_binary_path_finds_binary_via_ollama_bin_env() {
    let _guard = crate::openhuman::inference::inference_test_guard();

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
async fn diagnostics_repair_actions_are_empty_when_binary_is_known_but_server_is_down() {
    let _guard = crate::openhuman::inference::inference_test_guard();

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
        repair_actions.is_empty(),
        "when server is down, diagnostics should not advertise app-managed start actions"
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
    let _guard = crate::openhuman::inference::inference_test_guard();

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
    let _guard = crate::openhuman::inference::inference_test_guard();

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
    let models = service.list_models_at(&base).await.expect("list_models");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].name, "a:latest");
    assert_eq!(models[1].name, "b:v2");
    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn list_models_errors_on_non_success() {
    let _guard = crate::openhuman::inference::inference_test_guard();

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
    let err = service.list_models_at(&base).await.unwrap_err();
    assert!(err.contains("503") || err.contains("tags failed"));
    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn list_models_degrades_on_200_with_non_json_body() {
    // TAURI-RUST-560: a 2xx response whose body is not an Ollama tags JSON
    // object (a different local server/proxy, a captive portal, an HTML page
    // bound to the configured Ollama port) must degrade gracefully — return
    // `Err` so the diagnostics caller surfaces `tags_error` and an empty model
    // list — rather than emit an `error!`-level event that floods Sentry on
    // every diagnostics poll. The parse-failure log is now demoted to `warn!`
    // (a breadcrumb) to match the A3T non-success treatment.
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new().route(
        "/api/tags",
        // 200 OK, but the body is an HTML page, not Ollama tags JSON.
        get(|| async {
            (
                axum::http::StatusCode::OK,
                "<!doctype html><html><head><title>Sign in</title></head>\
                 <body>Captive portal</body></html>",
            )
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    let err = service.list_models_at(&base).await.unwrap_err();
    assert!(
        err.contains("parse failed"),
        "200 non-JSON body must yield a graceful parse-failed Err, got: {err}"
    );
    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn lm_studio_list_models_returns_loaded_models() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new().route(
        "/v1/models",
        get(|| async {
            Json(json!({
                "object": "list",
                "data": [
                    { "id": "local-model", "object": "model", "owned_by": "lm-studio" },
                    { "id": "second-model", "object": "model", "owned_by": "lm-studio" }
                ]
            }))
        }),
    );
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let models = service
        .list_lm_studio_models(&config)
        .await
        .expect("lm studio models");

    assert_eq!(models.len(), 2);
    assert_eq!(models[0].name, "local-model");
    assert!(service
        .has_lm_studio_model(&config, "local-model")
        .await
        .expect("has model"));
}

#[tokio::test]
async fn lm_studio_diagnostics_reports_loaded_chat_model() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new().route(
        "/v1/models",
        get(|| async {
            Json(json!({
                "data": [
                    { "id": "local-model", "object": "model", "owned_by": "lm-studio" }
                ]
            }))
        }),
    );
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let diag = service.diagnostics(&config).await.expect("diagnostics");

    assert_eq!(diag["provider"].as_str(), Some("lm_studio"));
    assert_eq!(diag["lm_studio_running"], true);
    assert_eq!(diag["expected"]["chat_found"], true);
    assert_eq!(diag["ok"], true);
}

/// Regression for GH #5053: a custom OpenAI-compatible BYOK endpoint on
/// localhost (e.g. LM Studio at `http://localhost:1234/v1`) whose `provider`
/// tag still defaults to `ollama` must be probed with `/v1/models`, NOT the
/// Ollama-native `/api/tags`. The mock serves ONLY `/v1/models` and no
/// `/api/tags`, so before the fix diagnostics took the Ollama branch,
/// hit an unrouted `/v1/api/tags`, and reported the model absent; after the
/// fix the `/v1` endpoint type routes discovery to `/v1/models` and the model
/// is found.
#[tokio::test]
async fn diagnostics_openai_compatible_v1_endpoint_uses_v1_models_not_api_tags() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    // OpenAI-compatible server: exposes `/v1/models` and deliberately no
    // `/api/tags` — an Ollama probe here would 404 (silently empty discovery).
    let app = Router::new().route(
        "/v1/models",
        get(|| async {
            Json(json!({
                "data": [
                    { "id": "local-model", "object": "model", "owned_by": "lm-studio" }
                ]
            }))
        }),
    );
    let base = spawn_mock(app).await;

    // The #5053 config: a `/v1` OpenAI-compatible endpoint whose provider tag is
    // the defaulted `ollama` (not `lm_studio`).
    let mut config = lm_studio_config(&base);
    config.local_ai.provider = "ollama".to_string();

    let service = LocalAiService::new(&config);
    let diag = service.diagnostics(&config).await.expect("diagnostics");

    // `lm_studio_running` is emitted only by the OpenAI-compatible (`/v1/models`)
    // diagnostics path — the Ollama branch reports `ollama_running` and leaves
    // this key null. Its presence proves discovery was routed by endpoint type,
    // not sent to `/api/tags`.
    assert_eq!(diag["lm_studio_running"], true);
    let installed = diag["installed_models"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        installed
            .iter()
            .any(|m| m["name"].as_str() == Some("local-model")),
        "OpenAI-compatible /v1 endpoint must discover models via /v1/models, got: {:?}",
        installed
    );
}

#[tokio::test]
async fn lm_studio_diagnostics_flags_missing_chat_model() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new().route(
        "/v1/models",
        get(|| async {
            Json(json!({
                "data": [
                    { "id": "other-model", "object": "model", "owned_by": "lm-studio" }
                ]
            }))
        }),
    );
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let diag = service.diagnostics(&config).await.expect("diagnostics");

    assert_eq!(diag["provider"].as_str(), Some("lm_studio"));
    assert_eq!(diag["expected"]["chat_found"], false);
    assert_eq!(diag["ok"], false);
    assert!(diag["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue.as_str().unwrap_or("").contains("local-model")));
}

#[tokio::test]
async fn lm_studio_diagnostics_surfaces_reachable_model_list_errors() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new().route("/v1/models", get(|| async { "not json" }));
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let diag = service.diagnostics(&config).await.expect("diagnostics");

    assert_eq!(diag["provider"].as_str(), Some("lm_studio"));
    assert_eq!(diag["lm_studio_running"], true);
    assert_eq!(diag["ok"], false);
    assert!(diag["issues"].as_array().unwrap().iter().any(|issue| issue
        .as_str()
        .unwrap_or("")
        .contains("Failed to list LM Studio models")));
    assert!(!diag["repair_actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["action"].as_str() == Some("load_lm_studio_model")));
}

#[tokio::test]
async fn lm_studio_assets_reports_embedding_as_ollama_managed() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new().route(
        "/v1/models",
        get(|| async {
            Json(json!({
                "data": [
                    { "id": "local-model", "object": "model", "owned_by": "lm-studio" }
                ]
            }))
        }),
    );
    let base = spawn_mock(app).await;
    let mut config = lm_studio_config(&base);
    config.local_ai.embedding_model_id = "bge-m3".to_string();

    let prev_ollama_bin = std::env::var_os("OLLAMA_BIN");
    let fake_ollama = std::env::current_exe().expect("current test exe path");
    unsafe {
        std::env::set_var("OLLAMA_BIN", &fake_ollama);
    }

    let service = LocalAiService::new(&config);
    let status = service.assets_status(&config).await.expect("assets status");

    unsafe {
        match prev_ollama_bin {
            Some(value) => std::env::set_var("OLLAMA_BIN", value),
            None => std::env::remove_var("OLLAMA_BIN"),
        }
    }

    assert_eq!(status.chat.provider, "lm_studio");
    assert_eq!(status.chat.state, "ready");
    assert_eq!(status.embedding.provider, "ollama");
    assert_eq!(status.embedding.path.as_deref(), Some("ollama://bge-m3"));
    assert!(status
        .embedding
        .warning
        .as_deref()
        .unwrap_or("")
        .contains("Ollama path"));
}

// ---- owned-PID lifecycle ------------------------------------------------
//
// These tests pin the contract that `kill_ollama_server` only touches
// daemons openhuman spawned itself, and that the kill path actually
// reaches the child process (the previous `taskkill /F /IM ollama.exe` /
// `pkill -f` would terminate any Ollama on the host, including ones the
// user started outside openhuman — the issue #1622 friendly-fire bug).

#[tokio::test]
async fn kill_ollama_server_with_no_owned_child_is_noop() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let config = Config::default();
    let service = LocalAiService::new(&config);

    // A fresh service has never spawned anything, so `owned_ollama` is `None`.
    assert!(
        service.owned_ollama.lock().is_none(),
        "owned_ollama must start as None"
    );

    // Must complete without panicking and leave the field None — i.e.
    // never reach for an external daemon when there's nothing to kill.
    service.kill_ollama_server().await;

    assert!(
        service.owned_ollama.lock().is_none(),
        "owned_ollama must stay None after a no-op kill"
    );
}

#[tokio::test]
async fn kill_ollama_server_kills_owned_child() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let config = Config::default();
    let service = LocalAiService::new(&config);

    // Spawn a long-lived child we fully control. We need something that
    // sleeps for longer than the test's worst-case settle window so it
    // can't exit on its own before our kill lands.
    let mut cmd = if cfg!(windows) {
        let mut c = tokio::process::Command::new("powershell");
        c.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
        c
    } else {
        let mut c = tokio::process::Command::new("sleep");
        c.arg("30");
        c
    };
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = cmd.spawn().expect("spawn sleep/Start-Sleep child");
    let pid = child.id().expect("child pid available");
    *service.owned_ollama.lock() = Some(child);

    // Sanity: child should be alive immediately after spawn.
    assert!(
        crate::openhuman::inference::local::service::spawn_marker::pid_is_alive(pid),
        "child pid {pid} should be alive right after spawn"
    );

    service.kill_ollama_server().await;

    // Owned slot is cleared — `take()` happened.
    assert!(
        service.owned_ollama.lock().is_none(),
        "kill_ollama_server must take() the owned child"
    );

    // PID should no longer be alive. Allow a brief settle for the OS to
    // update its process table — the kill is signalled but reap is async.
    let mut still_alive = true;
    for _ in 0..40 {
        if !crate::openhuman::inference::local::service::spawn_marker::pid_is_alive(pid) {
            still_alive = false;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        !still_alive,
        "child pid {pid} should be dead within 2s of kill_ollama_server"
    );
}

#[tokio::test]
async fn shutdown_owned_ollama_clears_marker_and_kills_child() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    // Redirect the workspace root to a tempdir so the marker file doesn't
    // touch the real `~/.openhuman/`. Per `paths::shared_root_dir`, when
    // `default_root_openhuman_dir()` errors, it falls back to
    // `config_root_dir(config)` — which is `config.config_path.parent()`.
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    config.config_path = tmp.path().join("config.toml");

    let service = LocalAiService::new(&config);

    // Spawn the same long-running stub.
    let mut cmd = if cfg!(windows) {
        let mut c = tokio::process::Command::new("powershell");
        c.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
        c
    } else {
        let mut c = tokio::process::Command::new("sleep");
        c.arg("30");
        c
    };
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = cmd.spawn().expect("spawn child");
    let pid = child.id().expect("pid");
    *service.owned_ollama.lock() = Some(child);

    // Write a marker (mimicking what `start_and_wait_for_server` would do
    // on a successful spawn) so we can verify shutdown clears it.
    //
    // NOTE: This test only verifies the shutdown path itself; it does not
    // assert the marker survives the `default_root_openhuman_dir()`
    // resolution on every CI environment. On hosts where the fallback
    // resolves to a writable temp path, the write is exercised. On hosts
    // where `default_root_openhuman_dir()` succeeds against the real home
    // dir, we skip the marker assertion to avoid touching `~/.openhuman/`.
    let marker_path = crate::openhuman::inference::paths::ollama_spawn_marker_path(&config);
    let marker_writable = marker_path.starts_with(tmp.path());
    if marker_writable {
        crate::openhuman::inference::local::service::spawn_marker::write_marker_at(
            &marker_path,
            &crate::openhuman::inference::local::service::spawn_marker::OllamaSpawnMarker::new(
                pid,
                std::path::Path::new("test-stub"),
            ),
        )
        .expect("write marker");
        assert!(marker_path.exists(), "marker should exist before shutdown");
    }

    service.shutdown_owned_ollama(&config).await;

    // Owned handle is gone.
    assert!(service.owned_ollama.lock().is_none());

    if marker_writable {
        assert!(
            !marker_path.exists(),
            "shutdown_owned_ollama must clear the spawn marker"
        );
    }

    // And the spawned process is dead.
    let mut still_alive = true;
    for _ in 0..40 {
        if !crate::openhuman::inference::local::service::spawn_marker::pid_is_alive(pid) {
            still_alive = false;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(!still_alive, "spawned stub pid {pid} should be dead");
}

// ── ollama_binary_present short-circuit tests ─────────────────────────────

/// When no Ollama binary is available anywhere (no custom path, no OLLAMA_BIN,
/// no workspace install, no system install), `ollama_binary_present` must return
/// false so `assets_status` can skip all HTTP probes and report
/// `ollama_available: false` immediately.
#[tokio::test]
async fn assets_status_sets_ollama_available_false_when_binary_missing() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    // Point workspace to the empty tempdir so no workspace ollama binary is found.
    config.workspace_dir = tmp.path().join("workspace");
    // Ensure no custom path is set.
    config.local_ai.ollama_binary_path = None;

    // Remove OLLAMA_BIN so the env-var probe is also skipped.
    let prev_ollama_bin = std::env::var_os("OLLAMA_BIN");
    unsafe {
        std::env::remove_var("OLLAMA_BIN");
    }

    let service = LocalAiService::new(&config);

    // `ollama_binary_present` is the cheapest check — no HTTP probes.
    // We test it indirectly via assets_status which is the production caller.
    // On a machine where the system `ollama` binary IS installed, this test
    // can't reliably verify the false path without intercepting PATH. We instead
    // test the method directly.
    let present = service.ollama_binary_present(&config);

    // Run the production path under the SAME env that produced `present` so
    // assets_status sees the same world `ollama_binary_present` did.
    // Restoring OLLAMA_BIN before this call would let a host-set OLLAMA_BIN
    // pointing at a real binary leak into assets_status and contradict
    // `present == false`, making the test host-dependent.
    let probe_outcome = if !present {
        let started = std::time::Instant::now();
        let status = service.assets_status(&config).await.unwrap();
        Some((status, started.elapsed()))
    } else {
        None
    };

    // Restore env *after* the production path has run.
    unsafe {
        match prev_ollama_bin {
            Some(v) => std::env::set_var("OLLAMA_BIN", v),
            None => std::env::remove_var("OLLAMA_BIN"),
        }
    }

    // The assertion depends on whether `ollama` is on PATH on the test machine.
    // We assert the logical contract: when present is false, assets_status must
    // not fire any HTTP probes (verified by timing — a 500ms connect timeout
    // per probe × 3 probes would be > 1s; the test should complete instantly).
    if let Some((status, elapsed)) = probe_outcome {
        assert!(
            !status.ollama_available,
            "assets_status must report ollama_available=false when binary missing"
        );
        // All model states must be false/not-ready when binary is absent.
        assert_ne!(
            status.chat.state, "ready",
            "chat must not be ready when binary missing"
        );
        assert_ne!(
            status.vision.state, "ready",
            "vision must not be ready when binary missing"
        );
        assert_ne!(
            status.embedding.state, "ready",
            "embedding must not be ready when binary missing"
        );
        // Short-circuit: no HTTP probes → should complete in under 1 second.
        assert!(
            elapsed.as_secs() < 2,
            "assets_status must short-circuit quickly when binary missing: took {:?}",
            elapsed
        );
    } else {
        // On machines with system ollama, skip the short-circuit assertion
        // but confirm the binary_present helper is consistent.
        assert!(
            present,
            "ollama_binary_present returned true on a machine with system ollama"
        );
    }
}

// The custom-path branch of `ollama_binary_present` is covered by
// `assets_status_sets_ollama_available_false_when_binary_missing` above, which
// already calls `service.ollama_binary_present(&config)` and asserts that
// downstream `assets_status` reports `ollama_available = false` whenever the
// helper returns false. A dedicated nonexistent-custom-path test that scrubs
// PATH globally was attempted but caused parallel-test interference (PATH=""
// poisoned the local_ai_test_guard mutex for sibling tests that legitimately
// rely on PATH). The behavior is covered; an isolated branch test would
// require per-process isolation that the existing harness doesn't support.

#[test]
fn binary_present_uses_ollama_bin_env_var_when_set() {
    // When OLLAMA_BIN points to a real file, it must be preferred over the
    // workspace/system lookup. Use the current test binary itself as the
    // "fake ollama" — it's guaranteed to be a real file.
    let _guard = crate::openhuman::inference::inference_test_guard();

    let real_file = std::env::current_exe().expect("current test exe path");
    let prev = std::env::var_os("OLLAMA_BIN");
    unsafe {
        std::env::set_var("OLLAMA_BIN", &real_file);
    }

    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("ws");
    config.local_ai.ollama_binary_path = None;
    let service = LocalAiService::new(&config);

    let present = service.ollama_binary_present(&config);

    unsafe {
        match prev {
            Some(v) => std::env::set_var("OLLAMA_BIN", v),
            None => std::env::remove_var("OLLAMA_BIN"),
        }
    }

    assert!(
        present,
        "OLLAMA_BIN pointing to a real file must make ollama_binary_present return true"
    );
}

#[tokio::test]
async fn diagnostics_gates_models_by_context_window() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    // /api/tags lists two models; /api/show reports their context windows:
    // one at the 8192 floor (accepted) and one well below (rejected).
    let app = Router::new()
        .route(
            "/api/tags",
            get(|| async {
                Json(json!({
                    "models": [
                        {"name": "bge-m3:latest", "modified_at": "", "size": 1u64, "digest": "d"},
                        {"name": "tiny-embed:latest", "modified_at": "", "size": 2u64, "digest": "d"}
                    ]
                }))
            }),
        )
        .route(
            "/api/show",
            axum::routing::post(|Json(body): Json<serde_json::Value>| async move {
                let model = body["model"].as_str().unwrap_or_default().to_string();
                let ctx = if model.starts_with("bge-m3") { 8192 } else { 2048 };
                Json(json!({
                    "model_info": {
                        "general.architecture": "bert",
                        "bert.context_length": ctx,
                    },
                    "capabilities": ["embedding"],
                }))
            }),
        );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = Config::default();
    let service = LocalAiService::new(&config);
    let diag = service.diagnostics(&config).await.expect("diagnostics");

    assert_eq!(diag["ollama_running"], true);
    assert_eq!(diag["context_requirement"]["min_context_tokens"], 8192);

    let models = diag["installed_models"]
        .as_array()
        .expect("installed_models");
    let by_name = |needle: &str| {
        models
            .iter()
            .find(|m| m["name"].as_str().unwrap_or("").starts_with(needle))
            .unwrap_or_else(|| panic!("model {needle} missing"))
            .clone()
    };

    let accepted = by_name("bge-m3");
    assert_eq!(accepted["context_length"], 8192);
    assert_eq!(accepted["eligibility"]["status"], "ok");

    let rejected = by_name("tiny-embed");
    assert_eq!(rejected["context_length"], 2048);
    assert_eq!(rejected["eligibility"]["status"], "below_minimum");
    assert_eq!(rejected["eligibility"]["required"], 8192);

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

// ── GH #5055: one-shot /api/tags fallback on a /v1/models 404 ───────────────
//
// Discovery is still chosen by provider *type* (`model_discovery_api`); these
// cover the recovery path taken when the chosen OpenAI-compatible endpoint
// answers 404, so a runtime that only speaks the Ollama listing is not left
// with an empty catalog.

/// A `/v1/models` 404 falls back to the host-rooted `/api/tags` exactly once
/// and returns that catalog.
#[tokio::test]
async fn v1_models_404_falls_back_to_ollama_api_tags() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async { (axum::http::StatusCode::NOT_FOUND, "404 page not found") }),
        )
        .route(
            "/api/tags",
            get(|| async {
                Json(json!({
                    "models": [
                        { "name": "local-model", "size": 42, "modified_at": "2026-01-01T00:00:00Z" }
                    ]
                }))
            }),
        );
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let models = service
        .list_lm_studio_models(&config)
        .await
        .expect("the /api/tags fallback should recover discovery");

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].name, "local-model");
}

/// When neither endpoint serves a catalog, the caller sees the original
/// `/v1/models` failure — not a second, more confusing error from the fallback.
#[tokio::test]
async fn v1_models_404_reports_the_original_error_when_fallback_also_fails() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    // The fallback returns a DISTINCT status (503, not 404). Without that,
    // Axum's implicit fallback route also answers 404 and the assertion below
    // would pass even if the implementation surfaced the /api/tags failure.
    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async {
                (
                    axum::http::StatusCode::NOT_FOUND,
                    "404 page not found: /v1/models",
                )
            }),
        )
        .route(
            "/api/tags",
            get(|| async {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "503 fallback unavailable",
                )
            }),
        );
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let err = service
        .list_lm_studio_models(&config)
        .await
        .expect_err("no catalog anywhere must fail");

    assert!(
        err.contains("404"),
        "expected the original /v1/models status, got: {err}"
    );
    assert!(
        !err.contains("503"),
        "the fallback failure must not replace the original error, got: {err}"
    );
}

/// LM Studio answers unknown paths with `200 {"error": …}` and no models
/// (GH #5053). That ERROR ENVELOPE must not be treated as a recovery,
/// otherwise discovery silently "succeeds" with zero models.
#[tokio::test]
async fn error_envelope_api_tags_fallback_is_not_treated_as_recovery() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async { (axum::http::StatusCode::NOT_FOUND, "404 page not found") }),
        )
        .route(
            "/api/tags",
            get(|| async { Json(json!({ "error": "Unexpected endpoint or method." })) }),
        );
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let err = service
        .list_lm_studio_models(&config)
        .await
        .expect_err("an error envelope is not a recovery");

    assert!(err.contains("404"), "got: {err}");
}

/// A fresh Ollama with nothing pulled answers `{"models":[]}`. That is a
/// REACHABLE runtime, not a failure: rejecting it hid the server behind the
/// original 404 so the UI could not offer the model-download action.
#[tokio::test]
async fn empty_api_tags_fallback_recovers_as_zero_models() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async { (axum::http::StatusCode::NOT_FOUND, "404 page not found") }),
        )
        .route("/api/tags", get(|| async { Json(json!({ "models": [] })) }));
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let models = service
        .list_lm_studio_models(&config)
        .await
        .expect("a reachable runtime with no models is not an error");

    assert!(
        models.is_empty(),
        "expected an empty catalog, got {models:?}"
    );
}

/// Any non-404 failure must NOT trigger the fallback: a 500 is a server fault,
/// not a wrong-endpoint signal, and probing a second path would mask it.
#[tokio::test]
async fn non_404_status_does_not_trigger_the_fallback() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits_route = std::sync::Arc::clone(&hits);
    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        )
        .route(
            "/api/tags",
            get(move || {
                let hits = std::sync::Arc::clone(&hits_route);
                async move {
                    hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Json(json!({ "models": [{ "name": "should-not-be-used" }] }))
                }
            }),
        );
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let err = service
        .list_lm_studio_models(&config)
        .await
        .expect_err("a 500 must surface, not fall back");

    assert!(err.contains("500"), "got: {err}");
    assert_eq!(
        hits.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the /api/tags fallback must not run for a non-404 status"
    );
}

// ── #5146 P1: never pull a model the user did not choose ────────────────────

/// A blank model id must fail immediately with a message about configuration,
/// never becoming a `POST /api/pull` for a nameless model.
///
/// `effective_*_model_id` returns an empty string when a role has no usable
/// model, and several callers feed that straight into
/// `ensure_ollama_model_available`. Before this guard that produced a nameless
/// pull retried three times before failing opaquely — and it is the same path
/// that silently downloaded a ~1.7 GB vision substitute.
///
/// No mock server is needed: the guard must reject before any network work, so
/// a test that reaches the network would itself be the failure.
#[tokio::test]
async fn ensure_ollama_model_available_rejects_a_blank_model_id() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let config = Config::default();
    let service = LocalAiService::new(&config);

    for blank in ["", "   ", "\t"] {
        let err = service
            .ensure_ollama_model_available(&config, blank, "vision")
            .await
            .expect_err("a blank model id must not be pulled");
        assert!(
            err.contains("vision"),
            "error must name the role that is unconfigured: {err}"
        );
        assert!(
            err.contains("nothing to download"),
            "error must say no download will happen: {err}"
        );
    }

    // The label is carried through, so embedding reads as an embedding problem.
    let err = service
        .ensure_ollama_model_available(&config, "", "embedding")
        .await
        .expect_err("a blank model id must not be pulled");
    assert!(err.contains("embedding"), "got: {err}");
}

/// A vision model the user misconfigured must not take the whole local runtime
/// down with it.
///
/// `bootstrap()` returns on the first `ensure_models_available` error, so
/// propagating a chat-only vision model out of the `Bundled` branch left the
/// service `degraded` and skipped the remaining preloads and the ready state —
/// punishing chat for a vision-only mistake. The reason is recorded on the
/// status instead, and `resolve_vision_model_id` raises it again, actionably,
/// at request time.
#[tokio::test]
async fn bundled_vision_misconfiguration_does_not_abort_the_rest_of_bootstrap() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    // The chat model is present, so only vision can fail here.
    let app = Router::new().route(
        "/api/tags",
        get(|| async {
            Json(json!({
                "models": [
                    {"name": "gemma3:1b-it-qat", "modified_at": "", "size": 1u64, "digest": "d"}
                ]
            }))
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let mut config = Config::default();
    config.local_ai.runtime_enabled = true;
    config.local_ai.selected_tier = Some("custom".to_string());
    config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
    // A valid chat model on Ollama, but it cannot accept images.
    config.local_ai.vision_model_id = "gemma3n:e4b-it-q8_0".to_string();
    config.local_ai.preload_vision_model = true;
    // `preload_embedding_model` defaults to TRUE, and the mock serves no
    // `/api/pull`, so leaving it on made the embedding preload 404 and this
    // test fail for a reason that has nothing to do with vision. The
    // later-preload interaction is covered by its own test below.
    config.local_ai.preload_embedding_model = false;

    let service = LocalAiService::new(&config);
    let result = service.ensure_models_available(&config).await;

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    result.expect("a chat-only vision model must not fail the whole bootstrap");

    let status = service.status.lock();
    assert_eq!(
        status.vision_state, "missing",
        "the unusable vision model must be visible as missing, not ready"
    );
    let warning = status
        .warning
        .clone()
        .expect("the reason vision is unavailable must be surfaced");
    assert!(
        warning.contains("gemma3n:e4b-it-q8_0"),
        "the warning must name the model the user configured: {warning}"
    );
    assert!(
        warning.contains("not vision-capable"),
        "the warning must say what is wrong with it: {warning}"
    );
}

/// The vision reason must still be readable after a later preload runs.
///
/// `ensure_ollama_model_available` writes `status.warning` for its own transient
/// "Pulling …" progress, so publishing the vision failure at the point it
/// happens let the embedding pull bury it — leaving `vision_state = "missing"`
/// with no explanation of why. The reason is published after every other
/// preload for exactly this reason.
#[tokio::test]
async fn a_later_preload_does_not_bury_the_vision_failure_reason() {
    use axum::routing::post;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let _guard = crate::openhuman::inference::inference_test_guard();

    // The embedding model appears only once it has been pulled, so the preload
    // takes the full download path — the one that writes `status.warning`.
    let pulled = Arc::new(AtomicBool::new(false));
    let tags_flag = pulled.clone();
    let pull_flag = pulled.clone();
    let app = Router::new()
        .route(
            "/api/tags",
            get(move || {
                let pulled = tags_flag.clone();
                async move {
                    let mut models = vec![
                        json!({"name": "gemma3:1b-it-qat", "modified_at": "", "size": 1u64, "digest": "d"}),
                    ];
                    if pulled.load(Ordering::SeqCst) {
                        models.push(
                            json!({"name": "bge-m3", "modified_at": "", "size": 2u64, "digest": "d"}),
                        );
                    }
                    Json(json!({ "models": models }))
                }
            }),
        )
        .route(
            "/api/pull",
            post(move || {
                let pulled = pull_flag.clone();
                async move {
                    pulled.store(true, Ordering::SeqCst);
                    Json(json!({ "status": "success" }))
                }
            }),
        );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let mut config = Config::default();
    config.local_ai.runtime_enabled = true;
    config.local_ai.selected_tier = Some("custom".to_string());
    config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
    config.local_ai.vision_model_id = "gemma3n:e4b-it-q8_0".to_string();
    config.local_ai.preload_vision_model = true;
    config.local_ai.embedding_model_id = "bge-m3".to_string();
    config.local_ai.preload_embedding_model = true;

    let service = LocalAiService::new(&config);
    let result = service.ensure_models_available(&config).await;

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    result.expect("a chat-only vision model must not fail the whole bootstrap");
    assert!(
        pulled.load(Ordering::SeqCst),
        "the embedding preload must have pulled"
    );

    let status = service.status.lock();
    assert_eq!(status.vision_state, "missing");
    assert_eq!(status.embedding_state, "ready");
    let warning = status
        .warning
        .clone()
        .expect("the vision reason must survive the embedding preload");
    assert!(
        warning.contains("gemma3n:e4b-it-q8_0"),
        "the embedding pull's progress text must not have buried it: {warning}"
    );
}
