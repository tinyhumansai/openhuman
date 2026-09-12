//! Round 22 raw/E2E-style coverage for inference provider/admin branches.
//!
//! All external inference/admin surfaces are mocked with loopback HTTP servers
//! and temp PATH binaries. This suite must not invoke real Ollama, MLX, Python,
//! whisper, piper, local AI binaries, models, or downloads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tinyinference::message::Message;
use tinyinference::model::ModelRequest;

use openhuman_core::openhuman::config::schema::cloud_providers::{
    AuthStyle as CloudAuthStyle, CloudProviderCreds,
};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::inference::local::LocalAiService;
use openhuman_core::openhuman::inference::provider::factory::{
    auth_key_for_slug, create_chat_model_from_string_with_model_id,
};
use openhuman_core::openhuman::inference::provider::list_configured_models;

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<SeenRequest>>>,
}

#[derive(Debug, Clone)]
struct SeenRequest {
    path: String,
    auth: Option<String>,
    user_agent: Option<String>,
    body: Value,
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: this integration test is validated with --test-threads=1.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: this integration test is validated with --test-threads=1.
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
/// Several tests mutate `OPENHUMAN_WORKSPACE` / `OPENHUMAN_OLLAMA_BASE_URL` /
/// `PATH` via [`EnvVarGuard`]. `cargo test` (and `cargo llvm-cov`) run a
/// binary's tests on multiple threads by default, so without this lock those
/// mutations race and a test reads another test's workspace/config — observed
/// as a flaky failure under `cargo llvm-cov` (the coverage job does not pass
/// `--test-threads=1`). Every test takes this guard up front so the suite is
/// effectively serialized regardless of the runner's thread count.
static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[tokio::test]
async fn provider_admin_model_listing_covers_openrouter_validation_and_local_synthesis() {
    let _env = env_lock();
    let (base, state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.base_url = Some(base.clone());
    config.cloud_providers = vec![
        provider_entry(
            "openrouter-id",
            "openrouter",
            &format!("{base}/openrouter/api/v1"),
            CloudAuthStyle::Bearer,
            None,
        ),
        provider_entry(
            "object-error-id",
            "object-error",
            &format!("{base}/object-error"),
            CloudAuthStyle::None,
            None,
        ),
    ];
    config.save().await.expect("save config");
    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        &auth_key_for_slug("openrouter"),
        DEFAULT_AUTH_PROFILE_NAME,
        "sk-openrouter",
        HashMap::new(),
        true,
    )
    .expect("store openrouter key");
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);

    let openrouter = list_configured_models("openrouter")
        .await
        .expect("openrouter models")
        .value;
    assert_eq!(openrouter["models"][0]["id"], "or-model");

    let object_error = list_configured_models("object-error")
        .await
        .expect_err("object error payload");
    assert!(object_error.contains("nested provider failure"));

    let synthetic_ollama = list_configured_models("ollama")
        .await
        .expect("synthetic ollama /v1 models")
        .value;
    assert_eq!(synthetic_ollama["models"][0]["id"], "ollama-synth");

    config.cloud_providers = vec![provider_entry(
        "openrouter-id",
        "openrouter",
        &format!("{base}/openrouter-bad/api/v1"),
        CloudAuthStyle::Bearer,
        None,
    )];
    config.save().await.expect("save bad openrouter config");
    auth.store_provider_token(
        &auth_key_for_slug("openrouter"),
        DEFAULT_AUTH_PROFILE_NAME,
        "sk-openrouter-bad",
        HashMap::new(),
        true,
    )
    .expect("store bad openrouter key");
    let bad_key = list_configured_models("openrouter")
        .await
        .expect_err("openrouter key validation body");
    assert!(bad_key.contains("OpenRouter key validation returned error payload"));
    assert!(!bad_key.contains("sk-openrouter-bad"));

    let seen = state.requests.lock().expect("requests");
    assert!(seen.iter().any(|req| req.path == "/openrouter/api/v1/key"
        && req.auth.as_deref() == Some("Bearer sk-openrouter")));
    assert!(seen
        .iter()
        .any(|req| req.path == "/v1/models" && req.auth.is_none()));
}

#[tokio::test]
async fn factory_covers_legacy_api_key_scoping_and_abstract_model_errors() {
    let _env = env_lock();
    let (base, state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.api_key = Some("sk-legacy-direct".to_string());
    config.inference_url = Some(format!("{base}/legacy/v1"));
    config.cloud_providers = vec![
        provider_entry(
            "legacy-id",
            "legacy",
            &format!("{base}/legacy/v1/"),
            CloudAuthStyle::Bearer,
            Some("legacy-default"),
        ),
        provider_entry(
            "other-id",
            "other",
            &format!("{base}/other/v1"),
            CloudAuthStyle::Bearer,
            Some("other-default"),
        ),
        provider_entry(
            "abstract-id",
            "abstract",
            &format!("{base}/abstract/v1"),
            CloudAuthStyle::Bearer,
            None,
        ),
    ];
    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        APP_SESSION_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
        "session-token",
        HashMap::new(),
        true,
    )
    .expect("store app session");
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());

    let (legacy, legacy_model) = create_chat_model_from_string_with_model_id(
        "chat",
        "legacy:requested-model",
        &config,
        0.4,
    )
    .expect("legacy direct model");
    assert_eq!(legacy_model, "requested-model");
    let legacy_response = legacy
        .invoke(
            &(),
            ModelRequest::new(vec![Message::user("hello")]).with_model(&legacy_model),
        )
        .await
        .expect("legacy chat");
    assert_eq!(
        legacy_response.text(),
        "legacy direct ok"
    );

    let (other, other_model) = create_chat_model_from_string_with_model_id(
        "chat",
        "other:other-model",
        &config,
        0.4,
    )
    .expect("other model");
    let other_text = other
        .invoke(
            &(),
            ModelRequest::new(vec![Message::user("hello")]).with_model(&other_model),
        )
        .await
        .expect("other model dispatches without inheriting the legacy key");
    assert_eq!(other_text.text(), "other no key ok");

    let abstract_err = match create_chat_model_from_string_with_model_id(
        "reasoning",
        "abstract:reasoning-v1",
        &config,
        0.4,
    ) {
            Ok(_) => panic!("expected abstract tier error"),
            Err(err) => err,
        };
    assert!(abstract_err
        .to_string()
        .contains("has no concrete default_model configured"));

    let seen = state.requests.lock().expect("requests");
    assert!(seen
        .iter()
        .any(|req| req.path == "/legacy/v1/chat/completions"
            && req.auth.as_deref() == Some("Bearer sk-legacy-direct")));
    assert!(seen
        .iter()
        .any(|req| req.path == "/other/v1/chat/completions"
            && !req
                .auth
                .as_deref()
                .is_some_and(|auth| auth.contains("sk-legacy-direct"))));
}

#[tokio::test]
async fn local_admin_covers_diagnostics_errors_assets_status_and_shutdown_with_fake_bins() {
    let _env = env_lock();
    let (base, _state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = true;
    config.local_ai.opt_in_confirmed = true;
    config.local_ai.base_url = Some(base.clone());
    config.local_ai.chat_model_id = "gemma3n:e4b-it-q8_0".to_string();
    config.local_ai.embedding_model_id = "all-minilm:latest".to_string();
    config.local_ai.selected_tier = Some("custom".to_string());
    config.local_ai.preload_embedding_model = true;
    config.local_ai.preload_stt_model = true;
    config.local_ai.preload_tts_voice = true;
    config.local_ai.stt_model_id = "round22-stt".to_string();
    config.local_ai.tts_voice_id = "round22-voice".to_string();

    let scripts = tempdir().expect("scripts");
    let ollama = write_stub_script(&scripts, "ollama", "#!/bin/sh\nprintf 'fake ollama\\n'\n");
    write_stub_script(&scripts, "python", "#!/bin/sh\nexit 42\n");
    write_stub_script(&scripts, "python3", "#!/bin/sh\nexit 42\n");
    write_stub_script(&scripts, "mlx_lm.generate", "#!/bin/sh\nexit 42\n");
    write_stub_script(&scripts, "piper", "#!/bin/sh\nexit 42\n");

    let _path = EnvVarGuard::set("PATH", scripts.path());
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);
    let _ollama_bin = EnvVarGuard::set("OLLAMA_BIN", &ollama);
    let _piper_bin = EnvVarGuard::unset("PIPER_BIN");
    let _whisper_bin = EnvVarGuard::unset("WHISPER_BIN");

    let service = LocalAiService::new(&config);
    let diag = service.diagnostics(&config).await.expect("diagnostics");
    assert_eq!(diag["ollama_running"], true);
    let issues = diag["issues"].as_array().expect("issues");
    assert!(issues.iter().any(|issue| issue
        .as_str()
        .unwrap()
        .contains("Chat model `gemma3n:e4b-it-q8_0`")));
    assert!(issues.iter().any(|issue| issue
        .as_str()
        .unwrap()
        .contains("Embedding model `all-minilm:latest`")));

    let mut tags_500 = config.clone();
    tags_500.local_ai.base_url = Some(format!("{base}/tags-500"));
    let diag_500 = service
        .diagnostics(&tags_500)
        .await
        .expect("500 diagnostics");
    assert_eq!(diag_500["ollama_running"], false);
    assert!(diag_500["issues"][0]
        .as_str()
        .unwrap()
        .contains("not running or not reachable"));

    let assets = service.assets_status(&config).await.expect("assets status");
    assert!(assets.ollama_available);
    assert_eq!(assets.chat.state, "missing");
    assert_eq!(assets.embedding.state, "missing");
    assert_ne!(assets.tts.state, "ready");

    let child = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("sleep 30")
        .spawn()
        .expect("spawn fake owned ollama child");
    service.inject_owned_ollama(child);
    assert!(service.has_owned_ollama());
    service.shutdown_owned_ollama(&config).await;
    assert!(!service.has_owned_ollama());
}

async fn serve_mock() -> (String, MockState) {
    let state = MockState::default();
    let app = Router::new()
        .route("/fallback/v1/chat/completions", post(always_404))
        .route("/fallback/v1/responses", post(responses_fallback))
        .route("/merge/v1/chat/completions", post(merge_chat))
        .route("/custom-auth/v1/chat/completions", post(custom_auth_chat))
        .route("/openrouter/api/v1/key", get(openrouter_key_ok))
        .route("/openrouter/api/v1/models", get(openrouter_models))
        .route("/openrouter-bad/api/v1/key", get(openrouter_key_bad))
        .route("/object-error/models", get(object_error_models))
        .route("/v1/models", get(synthetic_ollama_models))
        .route("/legacy/v1/chat/completions", post(legacy_chat))
        .route("/other/v1/chat/completions", post(other_chat))
        .route("/api/tags", get(ollama_tags))
        .route("/api/show", post(ollama_show))
        .route("/tags-500/api/tags", get(tags_500))
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

async fn always_404(State(state): State<MockState>, headers: HeaderMap) -> impl IntoResponse {
    remember(
        &state,
        "/fallback/v1/chat/completions",
        &headers,
        Value::Null,
    );
    (StatusCode::NOT_FOUND, "missing chat endpoint").into_response()
}

async fn responses_fallback(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/fallback/v1/responses", &headers, body);
    Json(json!({"output_text": "round22 responses text"})).into_response()
}

async fn merge_chat(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/merge/v1/chat/completions", &headers, body);
    Json(json!({"choices":[{"message":{"content":"merged ok"}}]})).into_response()
}

async fn custom_auth_chat(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/custom-auth/v1/chat/completions", &headers, body);
    Json(json!({"choices":[{"message":{"content":"custom auth ok"}}]})).into_response()
}

async fn openrouter_key_ok(
    State(state): State<MockState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    remember(&state, "/openrouter/api/v1/key", &headers, Value::Null);
    Json(json!({"data": {"label": "ok"}})).into_response()
}

async fn openrouter_models() -> impl IntoResponse {
    Json(json!({"object":"list","data":[{"id":"or-model","owned_by":"openrouter"}]}))
}

async fn openrouter_key_bad(
    State(state): State<MockState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    remember(&state, "/openrouter-bad/api/v1/key", &headers, Value::Null);
    Json(json!({"error": {"message": "bad key sk-openrouter-bad"}})).into_response()
}

async fn object_error_models() -> impl IntoResponse {
    Json(json!({"error": {"message": "nested provider failure"}}))
}

async fn synthetic_ollama_models(
    State(state): State<MockState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    remember(&state, "/v1/models", &headers, Value::Null);
    Json(json!({"object":"list","data":[{"id":"ollama-synth","context_length":4096}]}))
}

async fn legacy_chat(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/legacy/v1/chat/completions", &headers, body);
    Json(json!({"choices":[{"message":{"content":"legacy direct ok"}}]})).into_response()
}

async fn other_chat(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/other/v1/chat/completions", &headers, body);
    Json(json!({"choices":[{"message":{"content":"other no key ok"}}]})).into_response()
}

async fn ollama_tags() -> impl IntoResponse {
    Json(json!({
        "models": [
            {"name": "round22-existing", "model": "round22-existing", "size": 1}
        ]
    }))
}

async fn ollama_show() -> impl IntoResponse {
    Json(json!({"model_info": {"general.context_length": 8192}}))
}

async fn tags_500() -> impl IntoResponse {
    (StatusCode::INTERNAL_SERVER_ERROR, "tags failed").into_response()
}

fn remember(state: &MockState, path: &str, headers: &HeaderMap, body: Value) {
    state.requests.lock().expect("requests").push(SeenRequest {
        path: path.to_string(),
        auth: auth_header(headers),
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned),
        body,
    });
}

fn auth_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .or_else(|| headers.get("x-api-key"))
        .or_else(|| headers.get("x-custom-auth"))
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn provider_entry(
    id: &str,
    slug: &str,
    endpoint: &str,
    auth_style: CloudAuthStyle,
    default_model: Option<&str>,
) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: slug.to_string(),
        label: slug.to_string(),
        endpoint: endpoint.to_string(),
        auth_style,
        legacy_type: None,
        default_model: default_model.map(ToString::to_string),
    }
}

fn temp_config(tmp: &TempDir) -> Config {
    let root = tmp.path().join(".openhuman");
    std::fs::create_dir_all(root.join("workspace")).expect("workspace dir");
    let mut config = Config::default();
    config.config_path = root.join("config.toml");
    config.workspace_dir = root.join("workspace");
    config.secrets.encrypt = false;
    config.api_url = Some("http://127.0.0.1:9".to_string());
    config
}

fn write_stub_script(tmp: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = tmp.path().join(name);
    std::fs::write(&path, body).expect("write stub");
    make_executable(&path);
    path
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod");
    }
}
