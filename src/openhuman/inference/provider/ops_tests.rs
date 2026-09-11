use super::*;
use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::openhuman::config::Config;
use crate::openhuman::security::credentials::AuthService;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering as AtomicOrdering},
    Arc, Mutex,
};
use tempfile::TempDir;

#[derive(Clone)]
struct ModelProbeState {
    key_status: StatusCode,
    key_calls: Arc<AtomicUsize>,
    model_calls: Arc<AtomicUsize>,
    key_authorization: Arc<Mutex<Vec<Option<String>>>>,
    model_authorization: Arc<Mutex<Vec<Option<String>>>>,
}

async fn openrouter_key_handler(
    State(state): State<ModelProbeState>,
    headers: HeaderMap,
) -> Response {
    state.key_calls.fetch_add(1, AtomicOrdering::SeqCst);
    state
        .key_authorization
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(authorization_header(&headers));
    if state.key_status.is_success() {
        Json(serde_json::json!({
            "data": {
                "label": "test-key",
                "usage": 0
            }
        }))
        .into_response()
    } else {
        (
            state.key_status,
            Json(serde_json::json!({
                "error": {
                    "message": "No auth credentials found"
                }
            })),
        )
            .into_response()
    }
}

async fn models_handler(State(state): State<ModelProbeState>, headers: HeaderMap) -> Response {
    state.model_calls.fetch_add(1, AtomicOrdering::SeqCst);
    state
        .model_authorization
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(authorization_header(&headers));
    Json(serde_json::json!({
        "data": [{
            "id": "openrouter/test-model",
            "owned_by": "openrouter",
            "context_length": 128000
        }]
    }))
    .into_response()
}

fn authorization_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
}

async fn spawn_openrouter_probe_server(key_status: StatusCode) -> (String, ModelProbeState) {
    let state = ModelProbeState {
        key_status,
        key_calls: Arc::new(AtomicUsize::new(0)),
        model_calls: Arc::new(AtomicUsize::new(0)),
        key_authorization: Arc::new(Mutex::new(Vec::new())),
        model_authorization: Arc::new(Mutex::new(Vec::new())),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let app = Router::new()
        .route("/key", get(openrouter_key_handler))
        .route("/models", get(models_handler))
        .with_state(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), state)
}

async fn configure_openrouter_workspace(tmp: &TempDir, endpoint: String, token: &str) -> Config {
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_openrouter_test".to_string(),
        slug: "openrouter".to_string(),
        label: "OpenRouter".to_string(),
        endpoint,
        auth_style: AuthStyle::Bearer,
        legacy_type: None,
        default_model: None,
    });
    config.save().await.expect("save config");

    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        &crate::openhuman::inference::provider::factory::auth_key_for_slug("openrouter"),
        "default",
        token,
        HashMap::new(),
        true,
    )
    .expect("store provider key");
    config
}

// ── TAURI-RUST-12: list_models JSON parse error must surface body ──────
//
// `response.json()` previously dropped the body when decoding failed, so
// Sentry saw `[providers][list_models] failed to parse JSON: error decoding
// response body` with no clue what the server actually returned. The fix
// reads the body as text first, parses with `serde_json::from_str`, and
// appends a sanitized + truncated snippet to the error string so the
// failure is diagnosable from the log line alone.

#[derive(Clone)]
struct StaticResponse {
    status: StatusCode,
    body: &'static str,
}

async fn static_models_handler(State(s): State<StaticResponse>) -> Response {
    (s.status, s.body).into_response()
}

async fn spawn_static_models_server(status: StatusCode, body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let app = Router::new()
        .route("/models", get(static_models_handler))
        .with_state(StaticResponse { status, body });
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}")
}

async fn configure_generic_workspace(tmp: &TempDir, endpoint: String) -> Config {
    // Non-`openrouter` slug so the OpenRouter pre-validation path is
    // skipped and the test hits `/models` directly.
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_generic_test".to_string(),
        slug: "generic-test".to_string(),
        label: "Generic".to_string(),
        endpoint,
        auth_style: AuthStyle::None,
        legacy_type: None,
        default_model: None,
    });
    config.save().await.expect("save config");
    config
}

#[path = "ops_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "ops_tests_part_02_tests.rs"]
mod part_02_tests;
