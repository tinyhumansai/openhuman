//! Host HTTP inference and local-service integration coverage.
//!
//! This suite uses temp workspaces, fake binaries, and loopback HTTP servers
//! only. It must not call host Ollama, MLX, Python, Whisper, Piper, models, or
//! download endpoints, and OpenHuman itself must not launch any of them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use openhuman_core::config::schema::cloud_providers::{
    AuthStyle as CloudAuthStyle, CloudProviderCreds,
};
use openhuman_core::config::Config;
use openhuman_core::core::types::AppState;
use openhuman_core::inference::host_runtime::local_ai_status;
use openhuman_core::inference::http;
use openhuman_core::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<(String, Value)>>>,
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: mutation is serialized by `env_lock()` (see below).
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: mutation is serialized by `env_lock()` (see below).
        unsafe { std::env::remove_var(key) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => {
                // SAFETY: mutation is serialized by `env_lock()` (see below).
                unsafe { std::env::set_var(self.key, value) }
            }
            None => {
                // SAFETY: mutation is serialized by `env_lock()` (see below).
                unsafe { std::env::remove_var(self.key) }
            }
        }
    }
}

/// Serializes the whole suite's process-global env access.
///
/// `cargo test` and `cargo llvm-cov` run a binary's tests on multiple threads
/// by default. These tests mutate `OPENHUMAN_WORKSPACE`, `OPENHUMAN_OLLAMA_BASE_URL`,
/// and binary path env vars, so every test takes this guard before reading or
/// writing config that may be influenced by process env.
static ENV_LOCK: &OnceLock<tokio::sync::Mutex<()>> = &crate::SHARED_ENV_LOCK;

async fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK.get_or_init(|| tokio::sync::Mutex::new(())).lock().await
}

#[tokio::test]
async fn http_models_and_chat_use_mocked_ollama_without_real_runtime() {
    let _env = env_lock().await;
    let (base, state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.default_model = Some("reasoning-v1@0.9".to_string());
    config.chat_provider = Some("ollama:route-chat@0.3".to_string());
    config.reasoning_provider = Some("round23:cloud-chat@0.4".to_string());
    config.local_ai.provider = "ollama".to_string();
    config.local_ai.base_url = Some(base.clone());
    config.local_ai.chat_model_id = "configured-chat".to_string();
    config.cloud_providers = vec![CloudProviderCreds {
        id: "round23-id".to_string(),
        slug: "round23".to_string(),
        label: "Round 23".to_string(),
        endpoint: format!("{base}/cloud"),
        auth_style: CloudAuthStyle::None,
        default_model: Some("cloud-default@0.5".to_string()),
    }];
    config.save().await.expect("save config");

    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);
    store_app_session(&config).await;

    let app = http::router().with_state(AppState {
        core_version: "round23-test".to_string(),
    });
    let url = serve_app(app).await;
    let client = reqwest::Client::new();

    let models: Value = client
        .get(format!("{url}/models"))
        .send()
        .await
        .expect("models response")
        .json()
        .await
        .expect("models json");
    let ids = models["data"]
        .as_array()
        .expect("models array")
        .iter()
        .map(|item| item["id"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"openhuman".to_string()));
    assert!(ids.contains(&"reasoning-v1".to_string()));
    assert!(ids.contains(&"ollama:configured-chat".to_string()));
    assert!(ids.contains(&"ollama:route-chat".to_string()));
    assert!(ids.contains(&"round23:cloud-chat".to_string()));
    assert!(!ids.iter().any(|id| id.contains('@')));

    let chat: Value = client
        .post(format!("{url}/chat/completions"))
        .json(&json!({
            "model": "bare-chat",
            "messages": [{ "role": "user", "content": "hello http" }],
            "temperature": 0.2
        }))
        .send()
        .await
        .expect("chat response")
        .json()
        .await
        .expect("chat json");
    assert_eq!(
        chat["choices"][0]["message"]["content"],
        "round23 chat bare-chat"
    );
    assert_eq!(chat["model"], "bare-chat");

    let stream_text = client
        .post(format!("{url}/chat/completions"))
        .json(&json!({
            "model": "ollama:stream-chat",
            "stream": true,
            "messages": [{ "role": "user", "content": "stream please" }]
        }))
        .send()
        .await
        .expect("stream response")
        .text()
        .await
        .expect("stream text");
    assert!(
        stream_text.contains("round23 stream"),
        "stream_text={stream_text}"
    );
    assert!(stream_text.contains("[DONE]"));

    let bad: Value = client
        .post(format!("{url}/chat/completions"))
        .json(&json!({ "model": "ollama:", "messages": [] }))
        .send()
        .await
        .expect("bad response")
        .json()
        .await
        .expect("bad json");
    assert!(bad["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("empty model"));

    let seen = state.requests.lock().expect("requests").clone();
    assert!(seen
        .iter()
        .any(|(path, body)| path == "/v1/chat/completions" && body["model"] == "bare-chat"));
}

#[tokio::test]
async fn local_service_reports_endpoint_state_from_mocked_ollama_without_spawning() {
    let _env = env_lock().await;
    let (base, _state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    // Runtime binaries on PATH leave a marker if run: OpenHuman probes the
    // user's endpoint and must never launch a runtime itself.
    let scripts = tempdir().expect("scripts");
    let spawn_marker = scripts.path().join("spawned.marker");
    let marker_script = format!("#!/bin/sh\ntouch '{}'\nexit 42\n", spawn_marker.display());
    for name in ["ollama", "python", "python3", "mlx_lm.generate", "piper"] {
        write_stub_script(scripts.path(), name, &marker_script);
    }

    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = true;
    config.local_ai.opt_in_confirmed = true;
    config.local_ai.provider = "ollama".to_string();
    config.local_ai.base_url = Some(base.clone());
    config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
    config.local_ai.embedding_model_id = "bge-m3".to_string();
    config.local_ai.vision_model_id = "vision-ready".to_string();
    config.local_ai.tts_voice_id = "round23-voice".to_string();
    config.save().await.expect("save config");

    let _path = EnvVarGuard::set("PATH", scripts.path());
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);
    let _piper_bin = EnvVarGuard::unset("PIPER_BIN");
    let _ollama_bin = EnvVarGuard::unset("OLLAMA_BIN");

    // Exercise the host operation's config projection. It may probe this
    // loopback endpoint, but must not launch a local runtime binary itself.
    let ops_status = local_ai_status(&config).await.expect("ops status").value;
    assert_eq!(ops_status.provider, "ollama");
    assert!(
        !spawn_marker.exists(),
        "OpenHuman must never launch a local runtime binary"
    );
}

async fn serve_mock() -> (String, MockState) {
    let state = MockState::default();
    let app = Router::new()
        .route("/v1/chat/completions", post(ollama_chat_completions))
        .route("/api/tags", get(ollama_tags))
        .route("/api/show", post(ollama_show))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr = listener.local_addr().expect("mock addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });
    (format!("http://{addr}"), state)
}

async fn serve_app(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind app");
    let addr = listener.local_addr().expect("app addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve app");
    });
    format!("http://{addr}")
}

async fn ollama_chat_completions(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response<Body> {
    remember(&state, "/v1/chat/completions", body.clone());
    assert!(
        headers.get(header::AUTHORIZATION).is_none(),
        "ollama-compatible local requests should be authless"
    );
    let model = body["model"].as_str().unwrap_or_default();
    if body["stream"].as_bool().unwrap_or(false) {
        return sse_response([
            json!({"choices":[{"delta":{"content":"round23 stream"}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
        ]);
    }
    Json(json!({
        "id": "mock-chat",
        "object": "chat.completion",
        "choices": [{ "message": { "role": "assistant", "content": format!("round23 chat {model}") } }]
    }))
    .into_response()
}

async fn ollama_tags() -> impl IntoResponse {
    Json(json!({
        "models": [
            { "name": "configured-chat", "model": "configured-chat" },
            { "name": "gemma3:1b-it-qat", "model": "gemma3:1b-it-qat" },
            { "name": "bge-m3", "model": "bge-m3" },
            { "name": "vision-ready", "model": "vision-ready" }
        ]
    }))
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
    Json(json!({
        "model_info": {
            "general.context_length": 4096,
            "llama.context_length": 4096
        }
    }))
    .into_response()
}

fn sse_response<const N: usize>(events: [Value; N]) -> Response<Body> {
    let mut body = events
        .into_iter()
        .map(|event| format!("data: {}\n\n", event))
        .collect::<String>();
    body.push_str("data: [DONE]\n\n");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from(body))
        .expect("sse response")
}

fn remember(state: &MockState, path: &str, body: Value) {
    state
        .requests
        .lock()
        .expect("requests")
        .push((path.to_string(), body));
}

fn temp_config(tmp: &TempDir) -> Config {
    let root = tmp.path().join(".openhuman");
    std::fs::create_dir_all(root.join("workspace")).expect("workspace dir");
    let mut config = Config {
        config_path: root.join("config.toml"),
        workspace_dir: root.join("workspace"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    config.api_url = Some("http://127.0.0.1:9".to_string());
    config
}

async fn store_app_session(config: &Config) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "round23-session-token",
            HashMap::new(),
            true,
        )
        .await
        .expect("store app session");
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
