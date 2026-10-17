//! Round 16 raw/E2E coverage for inference local-admin branches.
//!
//! This suite uses temp workspaces, temp PATH scripts, and loopback HTTP mocks
//! only. It must not call host Ollama, Piper, Whisper, Python, or MLX binaries,
//! and asserts that OpenHuman itself never launches one either.

use crate::env_guard::EnvVarGuard;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::config::schema::cloud_providers::{
    AuthStyle as CloudAuthStyle, CloudProviderCreds,
};
use openhuman_core::config::Config;
use tinyinference_local::service::LocalAiService;
use openhuman_core::inference::provider::factory::auth_key_for_slug;
use openhuman_core::inference::provider::list_configured_models;
use openhuman_core::security::credentials::{AuthService, DEFAULT_AUTH_PROFILE_NAME};

/// One captured mock request: method/path, optional auth header, JSON body.
type RecordedRequest = (String, Option<String>, Value);

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    ollama_models: Arc<Mutex<Vec<String>>>,
}

/// Process-wide lock serializing tests that mutate global environment
/// variables through [`EnvVarGuard`]. `cargo llvm-cov` runs integration tests
/// multi-threaded (it does not pass `--test-threads=1`), so without this guard
/// concurrent tests clobber each other's env — e.g. one test points
/// `OPENHUMAN_OLLAMA_BASE_URL` at an unreachable port and asserts Ollama is
/// unavailable while another points it at a mock and asserts it is available.
/// Each env-mutating test holds this guard for its whole body; declaring it
/// before any `EnvVarGuard` makes it drop last, after the env is restored.
fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static ENV_LOCK: &OnceLock<tokio::sync::Mutex<()>> = &crate::SHARED_ENV_LOCK;
    ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .blocking_lock()
}

async fn env_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    static ENV_LOCK: &OnceLock<tokio::sync::Mutex<()>> = &crate::SHARED_ENV_LOCK;
    ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock().await
}

#[tokio::test]
async fn local_admin_covers_diagnostics_and_endpoint_probe_without_spawning() {
    let _env_guard = env_lock_async().await;
    let (base, state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = true;
    config.local_ai.opt_in_confirmed = true;
    config.local_ai.base_url = Some(base.clone());
    config.local_ai.chat_model_id = "gemma3n:e4b-it-q8_0".to_string();
    config.local_ai.embedding_model_id = "bge-m3".to_string();
    config.local_ai.vision_model_id = "missing-vision".to_string();
    config.local_ai.tts_voice_id = "round16-voice".to_string();

    // Every runtime binary on PATH is a stub that leaves a marker if run.
    // OpenHuman never launches a local runtime, so none may appear.
    let scripts = tempdir().expect("scripts");
    let spawn_marker = scripts.path().join("spawned.marker");
    let marker_script = format!("#!/bin/sh\ntouch '{}'\nexit 42\n", spawn_marker.display());
    write_stub_script(scripts.path(), "ollama", &marker_script);
    write_stub_script(scripts.path(), "python", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "python3", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "mlx_lm.generate", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "piper", "#!/bin/sh\nexit 42\n");
    let _path = EnvVarGuard::set("PATH", scripts.path());
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);
    let _ollama_bin = EnvVarGuard::unset("OLLAMA_BIN");
    let _piper_bin = EnvVarGuard::unset("PIPER_BIN");
    let _whisper_bin = EnvVarGuard::unset("WHISPER_BIN");

    let runtime = openhuman_core::inference::local_runtime_config(&config);
    let service = LocalAiService::new(&runtime);

    let diagnostics = service.diagnostics(&runtime).await.expect("diagnostics");
    assert_eq!(diagnostics["ollama_running"], true);
    assert_eq!(diagnostics["expected"]["chat_found"], false);
    assert_eq!(diagnostics["expected"]["embedding_found"], true);
    assert_eq!(diagnostics["expected"]["vision_found"], false);
    assert_eq!(diagnostics["ok"], false);
    assert!(diagnostics["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue.as_str().unwrap().contains("gemma3n:e4b-it-q8_0")));

    // Bootstrap is now a read-only endpoint probe: the mock answers
    // `/api/tags`, so the runtime is `ready` even though the chat model is
    // missing (the user pulls it themselves).
    service.bootstrap(&runtime).await;
    let status = service.status();
    assert_eq!(status.state, "ready");
    assert_eq!(status.chat_model_id, "gemma3n:e4b-it-q8_0");
    assert!(
        !state
            .requests
            .lock()
            .expect("requests")
            .iter()
            .any(|(path, _, _)| path.contains("pull")),
        "bootstrap must never pull a model"
    );
    assert!(
        !spawn_marker.exists(),
        "bootstrap must never launch a runtime binary"
    );
}

#[tokio::test]
async fn provider_model_listing_covers_local_synthesis_and_openrouter_failures() {
    let _env_guard = env_lock_async().await;
    let (base, _state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.base_url = Some(base.clone());
    config.cloud_providers = vec![
        CloudProviderCreds {
            id: "openrouter-id".to_string(),
            slug: "openrouter".to_string(),
            label: "OpenRouter".to_string(),
            endpoint: format!("{base}/openrouter-error"),
            auth_style: CloudAuthStyle::Bearer,
            headers: Default::default(),
            default_model: None,
        },
        CloudProviderCreds {
            id: "array-id".to_string(),
            slug: "array-body".to_string(),
            label: "Array Body".to_string(),
            endpoint: format!("{base}/array-body"),
            auth_style: CloudAuthStyle::None,
            headers: Default::default(),
            default_model: None,
        },
        CloudProviderCreds {
            id: "status-id".to_string(),
            slug: "status-body".to_string(),
            label: "Status Body".to_string(),
            endpoint: format!("{base}/status-body"),
            auth_style: CloudAuthStyle::None,
            headers: Default::default(),
            default_model: None,
        },
    ];
    config.save().await.expect("save config");
    AuthService::from_config(&config)
        .store_provider_token(
            &auth_key_for_slug("openrouter"),
            DEFAULT_AUTH_PROFILE_NAME,
            "sk-openrouter-secret",
            HashMap::new(),
            true,
        )
        .expect("store token");

    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);

    let local = list_configured_models("ollama")
        .await
        .expect("synthetic ollama")
        .value;
    assert_eq!(local["models"][0]["id"], "bge-m3");

    let array_err = list_configured_models("array-body")
        .await
        .expect_err("top-level array");
    assert!(array_err.contains("not a JSON object"));

    let status_err = list_configured_models("status-body")
        .await
        .expect_err("non-success");
    assert!(status_err.contains("provider returned 500"));
    assert!(!status_err.contains("sk-status-secret"));

    let openrouter_err = list_configured_models("openrouter")
        .await
        .expect_err("openrouter key validation error payload");
    assert!(openrouter_err.contains("OpenRouter key validation returned error payload"));
    assert!(!openrouter_err.contains("sk-openrouter-secret"));
}

#[tokio::test]
async fn local_admin_reports_unhealthy_runtime_and_lm_studio_issue_shapes() {
    let _env_guard = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = true;
    config.local_ai.opt_in_confirmed = true;
    config.local_ai.base_url = Some("http://127.0.0.1:9".to_string());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", "http://127.0.0.1:9");
    let runtime = openhuman_core::inference::local_runtime_config(&config);
    let service = LocalAiService::new(&runtime);

    let unhealthy = service.diagnostics(&runtime).await.expect("unhealthy diag");
    assert_eq!(unhealthy["ollama_running"], false);
    assert!(unhealthy["issues"][0]
        .as_str()
        .unwrap()
        .contains("not running or not reachable"));
    service.bootstrap(&runtime).await;
    assert_eq!(service.status().state, "unreachable");

    let (base, _state) = serve_mock().await;
    let mut lm_config = config.clone();
    lm_config.local_ai.provider = "lm-studio".to_string();
    lm_config.local_ai.base_url = Some(format!("{base}/lm-empty/v1"));
    lm_config.local_ai.chat_model_id = "loaded-chat".to_string();
    let mut lm_runtime = openhuman_core::inference::local_runtime_config(&lm_config);
    let lm_empty = service
        .diagnostics(&lm_runtime)
        .await
        .expect("lm studio empty");
    assert_eq!(lm_empty["provider"], "lm_studio");
    assert_eq!(lm_empty["lm_studio_running"], true);
    assert!(lm_empty["issues"][0]
        .as_str()
        .unwrap()
        .contains("no models are loaded"));

    lm_config.local_ai.base_url = Some(format!("{base}/lm-wrong/v1"));
    lm_runtime = openhuman_core::inference::local_runtime_config(&lm_config);
    let lm_wrong = service
        .diagnostics(&lm_runtime)
        .await
        .expect("lm studio wrong model");
    assert!(lm_wrong["issues"][0]
        .as_str()
        .unwrap()
        .contains("not loaded"));

    lm_config.local_ai.base_url = Some(format!("{base}/lm-error/v1"));
    lm_runtime = openhuman_core::inference::local_runtime_config(&lm_config);
    let lm_error = service
        .diagnostics(&lm_runtime)
        .await
        .expect("lm studio error payload");
    assert!(lm_error["issues"][0]
        .as_str()
        .unwrap()
        .contains("no models are loaded"));
}

async fn serve_mock() -> (String, MockState) {
    let state = MockState::default();
    *state.ollama_models.lock().expect("models") =
        vec!["bge-m3".to_string(), "loaded-chat".to_string()];
    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(responses))
        .route("/v1/models", get(models))
        .route("/array-body/models", get(array_body_models))
        .route("/status-body/models", get(status_body_models))
        .route("/openrouter-error/key", get(openrouter_error_key))
        .route("/openrouter-error/models", get(models))
        .route("/lm-empty/v1/models", get(empty_lm_models))
        .route("/lm-wrong/v1/models", get(wrong_lm_models))
        .route("/lm-error/v1/models", get(error_payload_models))
        .route("/api/tags", get(ollama_tags))
        .route("/api/show", post(ollama_show))
        .route("/api/pull", post(ollama_pull_recorded))
        .route("/api/generate", post(ollama_generate))
        .route("/api/chat", post(ollama_chat))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });
    (format!("http://{addr}"), state)
}

async fn chat_completions(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/v1/chat/completions", &headers, body.clone());
    let model = body["model"].as_str().unwrap_or_default();
    match model {
        "merge-model" => Json(json!({
            "choices": [{ "message": { "content": "merged response" } }]
        }))
        .into_response(),
        "stream-tools-unsupported" if body.get("tools").is_some() => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "model does not support tools"}})),
        )
            .into_response(),
        "stream-tools-unsupported" => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "choices": [{ "message": { "content": "json stream fallback" } }]
                })
                .to_string(),
            ))
            .expect("json response")
            .into_response(),
        "not-found-model" | "responses-empty-input" => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"message": "not found sk-should-redact"}})),
        )
            .into_response(),
        "empty-choices" => Json(json!({ "choices": [] })).into_response(),
        "auth-model" => Json(json!({
            "choices": [{ "message": { "content": "auth response" } }]
        }))
        .into_response(),
        _ => Json(json!({
            "choices": [{ "message": { "content": "default response" } }]
        }))
        .into_response(),
    }
}

async fn responses(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/v1/responses", &headers, body.clone());
    if body["input"]
        .as_str()
        .map(str::trim)
        .unwrap_or_default()
        .is_empty()
    {
        return Json(json!({ "output": [] })).into_response();
    }
    Json(json!({
        "output": [{
            "content": [{ "type": "output_text", "text": "responses fallback" }]
        }]
    }))
    .into_response()
}

async fn models(State(state): State<MockState>) -> impl IntoResponse {
    let models = state
        .ollama_models
        .lock()
        .expect("models")
        .iter()
        .map(|id| json!({ "id": id, "owned_by": "round16", "context_window": 8192 }))
        .collect::<Vec<_>>();
    Json(json!({ "object": "list", "data": models }))
}

async fn array_body_models() -> impl IntoResponse {
    Json(json!([{ "id": "bad" }]))
}

async fn status_body_models() -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "status failed sk-status-secret"})),
    )
}

async fn openrouter_error_key() -> impl IntoResponse {
    Json(json!({"error": {"message": "bad key sk-openrouter-secret"}}))
}

async fn empty_lm_models() -> impl IntoResponse {
    Json(json!({ "object": "list", "data": [] }))
}

async fn wrong_lm_models() -> impl IntoResponse {
    Json(json!({
        "object": "list",
        "data": [{ "id": "some-other-chat", "owned_by": "round16" }]
    }))
}

async fn error_payload_models() -> impl IntoResponse {
    Json(json!({ "error": { "message": "LM Studio endpoint error" } }))
}

async fn ollama_tags(State(state): State<MockState>) -> impl IntoResponse {
    let models = state
        .ollama_models
        .lock()
        .expect("models")
        .iter()
        .map(|name| json!({ "name": name, "model": name }))
        .collect::<Vec<_>>();
    Json(json!({ "models": models }))
}

async fn ollama_show(Json(body): Json<Value>) -> impl IntoResponse {
    let model = body
        .get("model")
        .or_else(|| body.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if model == "___nonexistent_probe___" {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "model not found"})),
        )
            .into_response();
    }
    let context = match model {
        "gemma3n:e4b-it-q8_0" => 1024,
        "bge-m3" => 8192,
        _ => 4096,
    };
    Json(json!({
        "model_info": {
            "general.context_length": context,
            "llama.context_length": context
        }
    }))
    .into_response()
}

/// Records any pull attempt so tests can assert none happens: OpenHuman no
/// longer pulls models, the user does.
async fn ollama_pull_recorded(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/api/pull", &headers, body);
    StatusCode::GONE
}

async fn ollama_generate() -> impl IntoResponse {
    Json(json!({
        "response": "generated",
        "done": true,
        "prompt_eval_count": 1,
        "prompt_eval_duration": 1000000,
        "eval_count": 1,
        "eval_duration": 1000000
    }))
}

async fn ollama_chat() -> impl IntoResponse {
    Json(json!({
        "message": { "role": "assistant", "content": "chat generated" },
        "done": true
    }))
}

fn remember(state: &MockState, path: &str, headers: &HeaderMap, body: Value) {
    state
        .requests
        .lock()
        .expect("requests")
        .push((path.to_string(), auth_header(headers), body));
}

fn auth_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .or_else(|| headers.get("x-api-key"))
        .or_else(|| headers.get("x-custom-auth"))
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn temp_config(tmp: &TempDir) -> Config {
    let root = tmp.path().join(".openhuman");
    std::fs::create_dir_all(root.join("workspace")).expect("workspace dir");
    let mut config = Config {
        config_path: root.join("config.toml"),
        workspace_dir: root.join("workspace"),
        ..Default::default()
    };
    config.secrets.encrypt = false;
    config.api_url = Some("http://127.0.0.1:9".to_string());
    config
}

fn write_stub_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }
    path
}
