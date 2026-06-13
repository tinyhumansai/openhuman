//! Round 25 raw/E2E coverage for inference compatible/admin cold paths.
//!
//! This suite uses loopback HTTP mocks and temp workspaces only. It must not
//! call host Ollama, MLX, Python, whisper, piper, local AI binaries, models, or
//! downloads.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use openhuman_core::core::all::RegisteredController;
use openhuman_core::openhuman::config::schema::cloud_providers::{
    AuthStyle as CloudAuthStyle, CloudProviderCreds,
};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::{AuthService, DEFAULT_AUTH_PROFILE_NAME};
use openhuman_core::openhuman::inference::local::all_local_inference_registered_controllers;
use openhuman_core::openhuman::inference::ops::inference_test_provider_model;
use openhuman_core::openhuman::inference::provider::compatible::{
    AuthStyle as CompatibleAuthStyle, OpenAiCompatibleProvider,
};
use openhuman_core::openhuman::inference::provider::factory::auth_key_for_slug;
use openhuman_core::openhuman::inference::provider::traits::{StreamError, StreamOptions};
use openhuman_core::openhuman::inference::provider::{
    list_configured_models, ChatMessage, Provider,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<SeenRequest>>>,
}

#[derive(Clone, Debug)]
struct SeenRequest {
    path: String,
    auth: Option<String>,
    body: Value,
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: validation runs this integration test with --test-threads=1.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => {
                // SAFETY: validation runs this integration test with --test-threads=1.
                unsafe { std::env::set_var(self.key, value) }
            }
            None => {
                // SAFETY: validation runs this integration test with --test-threads=1.
                unsafe { std::env::remove_var(self.key) }
            }
        }
    }
}

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[tokio::test]
async fn compatible_provider_cold_paths_cover_auth_url_temperature_and_stream_errors() {
    let (base, state) = serve_mock().await;

    let missing_key = OpenAiCompatibleProvider::new(
        "round25-missing-key",
        &format!("{base}/v1"),
        None,
        CompatibleAuthStyle::Bearer,
    );
    let err = missing_key
        .chat_with_system(None, "must fail before network", "missing-key", 0.1)
        .await
        .expect_err("credential guard");
    assert!(err.to_string().contains("API key not set"));
    let stream_errs = missing_key
        .stream_chat_with_history(
            &[ChatMessage::user("no key stream")],
            "missing-key",
            0.1,
            StreamOptions::new(true),
        )
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        &stream_errs[0],
        Err(StreamError::Provider(message)) if message.contains("API key not set")
    ));

    let full_endpoint = OpenAiCompatibleProvider::new(
        "round25-full-endpoint",
        &format!("{base}/direct/chat/completions"),
        Some("sk-full"),
        CompatibleAuthStyle::Bearer,
    )
    .with_temperature_override(Some(0.12))
    .with_temperature_unsupported_models(vec!["cold-*".to_string()]);
    assert_eq!(
        full_endpoint
            .chat_with_system(Some("policy"), "hello", "cold-model", 0.99)
            .await
            .expect("full endpoint chat"),
        "full endpoint ok"
    );

    let tools_empty = full_endpoint
        .chat_with_tools(&[ChatMessage::user("tools empty")], &[], "hot-model", 0.77)
        .await
        .expect("chat_with_tools empty tools");
    assert_eq!(tools_empty.text.as_deref(), Some("tools empty ok"));

    let chunks = full_endpoint
        .stream_chat_with_system(
            None,
            "stream denied",
            "stream-policy-denied",
            0.2,
            StreamOptions::new(true),
        )
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        &chunks[0],
        Err(StreamError::Provider(message))
            if message.contains("403") && !message.contains("sk-stream-secret")
    ));

    let seen = state.requests.lock().expect("requests");
    let cold = seen
        .iter()
        .find(|req| req.body["model"] == "cold-model")
        .expect("cold request");
    assert_eq!(cold.path, "/direct/chat/completions");
    assert_eq!(cold.auth.as_deref(), Some("Bearer sk-full"));
    assert!(cold.body.get("temperature").is_none());

    let hot = seen
        .iter()
        .find(|req| req.body["model"] == "hot-model")
        .expect("hot request");
    assert_eq!(hot.body["temperature"], 0.12);
    assert!(hot.body.get("tools").is_none());
    assert!(hot.body.get("tool_choice").is_none());
}

#[tokio::test]
async fn provider_admin_cold_paths_cover_model_errors_local_factory_and_connection_controller() {
    let _lock = env_lock();
    let (base, _state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.base_url = Some(base.clone());
    config.cloud_providers = vec![
        provider_entry(
            "array-id",
            "array-body",
            &format!("{base}/array-body"),
            CloudAuthStyle::None,
            None,
        ),
        provider_entry(
            "status-id",
            "status-secret",
            &format!("{base}/status-secret"),
            CloudAuthStyle::Bearer,
            None,
        ),
    ];
    config.save().await.expect("save config");
    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        &auth_key_for_slug("status-secret"),
        DEFAULT_AUTH_PROFILE_NAME,
        "sk-status-secret",
        HashMap::new(),
        true,
    )
    .expect("store provider key");

    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());

    let array_err = list_configured_models("array-body")
        .await
        .expect_err("top-level array body");
    assert!(array_err.contains("not a JSON object"));
    assert!(array_err.contains("array"));

    let status_err = list_configured_models("status-secret")
        .await
        .expect_err("non-2xx provider response");
    assert!(status_err.contains("provider returned 500"));
    assert!(!status_err.contains("sk-status-secret"));

    let empty_lmstudio = inference_test_provider_model(
        &config,
        "chat",
        "lmstudio:   ",
        "should fail before network",
    )
    .await
    .expect_err("empty lmstudio model");
    assert!(empty_lmstudio.contains("empty model"));

    let controllers = all_local_inference_registered_controllers();
    let test_connection = controller(&controllers, "test_connection");
    let reachable = call(test_connection, json!({"url": base}))
        .await
        .expect("reachable connection");
    assert_eq!(reachable["reachable"], true);
    assert_eq!(reachable["models_count"], 1);

    let bad_json_base = serve_bad_ollama_json_mock().await;
    let bad_json = call(test_connection, json!({"url": bad_json_base}))
        .await
        .expect("bad json still returns structured unreachable-ish result");
    assert_eq!(bad_json["reachable"], true);
    assert_eq!(bad_json["models_count"], 0);

    let invalid = call(test_connection, json!({"url": "not-a-url"}))
        .await
        .expect_err("invalid url rejected");
    assert!(invalid.contains("URL must start with http:// or https://"));
}

async fn serve_mock() -> (String, MockState) {
    let state = MockState::default();
    let app = Router::new()
        .route("/direct/chat/completions", post(direct_chat))
        .route("/array-body/models", get(array_models))
        .route("/status-secret/models", get(status_secret_models))
        .route("/api/tags", get(ollama_tags))
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

async fn serve_bad_ollama_json_mock() -> String {
    let app = Router::new().route("/api/tags", get(bad_ollama_json));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind bad json mock");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve bad json mock");
    });
    format!("http://{addr}")
}

async fn direct_chat(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/direct/chat/completions", &headers, body.clone());
    match body["model"].as_str().unwrap_or_default() {
        "stream-policy-denied" => (
            StatusCode::FORBIDDEN,
            "provider denied access for sk-stream-secret",
        )
            .into_response(),
        "hot-model" => {
            Json(json!({"choices":[{"message":{"content":"tools empty ok"}}]})).into_response()
        }
        _ => Json(json!({"choices":[{"message":{"content":"full endpoint ok"}}]})).into_response(),
    }
}

async fn array_models() -> impl IntoResponse {
    Json(json!([{"id":"not-envelope"}]))
}

async fn status_secret_models() -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "upstream exploded with sk-status-secret",
    )
        .into_response()
}

async fn ollama_tags() -> impl IntoResponse {
    Json(json!({"models":[{"name":"round25-model","model":"round25-model","size":1}]}))
}

async fn bad_ollama_json() -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("<html>not ollama json</html>"))
        .expect("bad json response")
}

fn remember(state: &MockState, path: &str, headers: &HeaderMap, body: Value) {
    state.requests.lock().expect("requests").push(SeenRequest {
        path: path.to_string(),
        auth: auth_header(headers),
        body,
    });
}

fn auth_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .or_else(|| headers.get("x-api-key"))
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

fn controller<'a>(
    controllers: &'a [RegisteredController],
    function: &str,
) -> &'a RegisteredController {
    controllers
        .iter()
        .find(|controller| controller.schema.function == function)
        .unwrap_or_else(|| panic!("controller {function} registered"))
}

async fn call(controller: &RegisteredController, params: Value) -> Result<Value, String> {
    let params = params.as_object().cloned().unwrap_or_default();
    (controller.handler)(params).await
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
