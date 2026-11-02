//! Focused JSON-RPC E2E coverage for config, auth/credentials, app_state,
//! and connectivity controller surfaces.

pub use crate::env_guard::{env_lock, env_lock_async, EnvVarGuard};
pub use crate::rpc_auth::ensure_rpc_auth;
pub use crate::rpc_harness::{ok, payload, rpc, schema};
pub use openhuman_core::core::runtime::DispatchCtx;
pub use std::net::SocketAddr;
pub use std::path::{Path, PathBuf};
pub use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
pub use std::time::Duration;

pub use axum::extract::{Path as AxumPath, State};
pub use axum::http::{header::AUTHORIZATION, HeaderMap};
pub use axum::routing::{delete, get, post};
pub use axum::{Json, Router};
pub use serde_json::{json, Value};
pub use tempfile::{tempdir, TempDir};

pub use openhuman_core::config::schema::{
    AuditConfig, CapabilityProviderConfig, CapabilityProviderTrustState, DashboardConfig,
    DingTalkConfig, DiscordConfig, EventStreamConfig, IrcConfig, LarkConfig, MatrixConfig,
    MemoryConfig, ModelHealthConfig, ProxyConfig, ProxyScope, QQConfig, ResourceLimitsConfig,
    SandboxConfig, SecurityConfig, SlackConfig, TelegramConfig, WebhookConfig, WhatsAppConfig,
};
pub use openhuman_core::config::{
    clear_active_user, default_projects_dir, pre_login_user_dir, read_active_user_id,
    user_openhuman_dir, write_active_user_id, Config, DaemonConfig, UpdateRestartStrategy,
};
pub use openhuman_core::core::events::DomainEvent;
pub use openhuman_core::credentials::bus::SessionExpiredSubscriber;
pub use openhuman_core::credentials::profiles::{AuthProfile, AuthProfilesStore, TokenSet};
pub use openhuman_core::credentials::{
    decrypt_secret, encrypt_secret, list_provider_credentials_by_prefix, normalize_provider,
    AuthService, APP_SESSION_PROVIDER,
};
pub use openhuman_core::desktop::app_state::app_state_schemas;
pub use openhuman_core::integrations::composio::{
    clear_composio_api_key, get_composio_api_key, rpc_store_composio_api_key,
    store_composio_api_key, COMPOSIO_DIRECT_PROVIDER,
};
pub use openhuman_rpc::server::build_core_http_router;
pub use openhuman_tinyhumans::backend::url::{
    api_base_from_env, app_env_from_env, default_api_base_url_for_env, effective_api_url,
    effective_backend_api_url, effective_inference_url, join_url as api_url,
    looks_like_local_ai_endpoint, normalize_api_base_url, APP_ENV_VAR, DEFAULT_API_BASE_URL,
    DEFAULT_STAGING_API_BASE_URL, OPENHUMAN_INFERENCE_PATH, VITE_APP_ENV_VAR,
};
pub use tinybus::EventHandler;

pub(crate) async fn serve_rpc() -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    // The core carries no backend client; the `auth_*` remote paths below reach
    // the mock backend through the `openhuman-tinyhumans` transport.
    crate::tinyhumans_boot::boot();
    ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr = listener.local_addr().expect("rpc listener addr");
    let router = build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });
    (addr, join)
}

#[derive(Clone, Default)]
pub(crate) struct MockBackendState {
    pub(crate) auth_me_hits: Arc<AtomicUsize>,
}

pub(crate) async fn serve_mock_backend() -> (
    String,
    MockBackendState,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let state = MockBackendState::default();
    let app = Router::new()
        .route("/auth/me", get(mock_auth_me))
        .route("/api/auth/me", get(mock_auth_me))
        .route(
            "/auth/channels/{channel}/link-token",
            post(mock_channel_link_token),
        )
        .route(
            "/api/auth/channels/{channel}/link-token",
            post(mock_channel_link_token),
        )
        .route("/auth/integrations", get(mock_integrations))
        .route("/api/auth/integrations", get(mock_integrations))
        .route("/auth/github/connect", get(mock_oauth_connect))
        .route("/api/auth/github/connect", get(mock_oauth_connect))
        .route(
            "/auth/integrations/{integration_id}/tokens",
            post(mock_integration_tokens),
        )
        .route(
            "/api/auth/integrations/{integration_id}/tokens",
            post(mock_integration_tokens),
        )
        .route(
            "/auth/integrations/{integration_id}/client-key",
            post(mock_client_key),
        )
        .route(
            "/api/auth/integrations/{integration_id}/client-key",
            post(mock_client_key),
        )
        .route(
            "/auth/integrations/{integration_id}",
            delete(mock_revoke_integration),
        )
        .route(
            "/api/auth/integrations/{integration_id}",
            delete(mock_revoke_integration),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock backend");
    let addr = listener.local_addr().expect("mock backend addr");
    let join = tokio::spawn(async move { axum::serve(listener, app).await });
    (format!("http://{addr}"), state, join)
}

pub(crate) fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
}

pub(crate) async fn mock_auth_me(
    State(state): State<MockBackendState>,
    headers: HeaderMap,
) -> Json<Value> {
    state.auth_me_hits.fetch_add(1, Ordering::SeqCst);
    let auth = bearer(&headers).unwrap_or_default();
    Json(json!({
        "success": true,
        "data": {
            "id": "remote-user-1",
            "_id": "remote-user-1",
            "name": "Remote Worker",
            "email": "remote-worker@example.test",
            "authHeader": auth
        }
    }))
}

pub(crate) async fn mock_channel_link_token(AxumPath(channel): AxumPath<String>) -> Json<Value> {
    Json(json!({
        "success": true,
        "data": {
            "channel": channel,
            "linkToken": "link-token-123",
            "expiresIn": 300
        }
    }))
}

pub(crate) async fn mock_integrations() -> Json<Value> {
    Json(json!({
        "success": true,
        "data": {
            "integrations": [{
                "id": "0123456789abcdef01234567",
                "provider": "github",
                "createdAt": "2026-01-01T00:00:00Z"
            }]
        }
    }))
}

pub(crate) async fn mock_oauth_connect() -> Json<Value> {
    Json(json!({
        "success": true,
        "oauthUrl": "https://github.example.test/oauth?state=worker-a-state",
        "state": "worker-a-state"
    }))
}

pub(crate) async fn mock_integration_tokens() -> Json<Value> {
    Json(json!({
        "success": true,
        "data": {
            "encrypted": encrypt_handoff_blob(
                "0123456789abcdef0123456789abcdef",
                &json!({
                    "accessToken": "gh-access-token",
                    "refreshToken": "gh-refresh-token",
                    "expiresAt": "2026-01-01T00:00:00Z"
                }).to_string(),
            )
        }
    }))
}

pub(crate) async fn mock_client_key(AxumPath(integration_id): AxumPath<String>) -> Json<Value> {
    Json(json!({
        "success": true,
        "data": {
            "integrationId": integration_id,
            "clientKey": "client-key-share"
        }
    }))
}

pub(crate) fn encrypt_handoff_blob(key: &str, plaintext: &str) -> String {
    use aes_gcm::aead::generic_array::typenum::U16;
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::aes::Aes256;
    use aes_gcm::AesGcm;
    use base64::Engine;

    type Aes256Gcm16 = AesGcm<Aes256, U16>;

    let cipher = Aes256Gcm16::new_from_slice(key.as_bytes()).expect("valid handoff key");
    let iv = [7_u8; 16];
    let nonce = aes_gcm::aead::generic_array::GenericArray::from_slice(&iv);
    let encrypted = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .expect("encrypt handoff payload");
    let (ciphertext, tag) = encrypted.split_at(encrypted.len() - 16);
    let mut combined = Vec::with_capacity(16 + 16 + ciphertext.len());
    combined.extend_from_slice(&iv);
    combined.extend_from_slice(tag);
    combined.extend_from_slice(ciphertext);
    base64::engine::general_purpose::STANDARD.encode(combined)
}

pub(crate) async fn mock_revoke_integration(
    AxumPath(_integration_id): AxumPath<String>,
) -> Json<Value> {
    Json(json!({ "success": true, "data": { "revoked": true } }))
}

pub(crate) fn write_min_config(openhuman_dir: &Path) {
    std::fs::create_dir_all(openhuman_dir).expect("create .openhuman");
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "worker-a-model"
default_temperature = 0.2

[secrets]
encrypt = false

[local_ai]
enabled = false
runtime_enabled = false
opt_in_confirmed = false

[memory]
engine = "tinyhumans"
"#;
    std::fs::write(openhuman_dir.join("config.toml"), cfg).expect("write config.toml");
    let _: openhuman_core::config::Config =
        toml::from_str(cfg).expect("test config must match schema");
}

pub(crate) struct TestHarness {
    pub(crate) _tmp: TempDir,
    pub(crate) home: std::path::PathBuf,
    pub(crate) _guards: Vec<EnvVarGuard>,
    pub(crate) rpc_base: String,
    pub(crate) join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

pub(crate) async fn setup() -> TestHarness {
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let openhuman_home = home.join(".openhuman");
    write_min_config(&openhuman_home);

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", &home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::unset("OPENHUMAN_TAVILY_API_KEY"),
        EnvVarGuard::unset("TAVILY_API_KEY"),
        EnvVarGuard::unset("OPENHUMAN_CORE_RPC_URL"),
        EnvVarGuard::unset("OPENHUMAN_CORE_PORT"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
        EnvVarGuard::set("OPENHUMAN_BROWSER_ALLOW_ALL_RPC_ENABLE", ""),
    ];

    let (addr, join) = serve_rpc().await;
    TestHarness {
        _tmp: tmp,
        home,
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        join,
    }
}

pub(crate) fn err<'a>(value: &'a Value, context: &str) -> &'a Value {
    value
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected JSON-RPC error, got: {value}"))
}

pub(crate) fn assert_error_contains(value: &Value, context: &str, needle: &str) {
    let message = err(value, context)
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        message.contains(needle),
        "{context}: expected error containing {needle:?}, got {message:?}"
    );
}

pub(crate) fn schema_method_names(value: &Value, namespace: &str) -> Vec<String> {
    let mut methods = value
        .get("methods")
        .and_then(Value::as_array)
        .expect("schema methods array")
        .iter()
        .filter(|method| method.get("namespace").and_then(Value::as_str) == Some(namespace))
        .map(|method| {
            method
                .get("method")
                .and_then(Value::as_str)
                .expect("method name")
                .to_string()
        })
        .collect::<Vec<_>>();
    methods.sort();
    methods
}
