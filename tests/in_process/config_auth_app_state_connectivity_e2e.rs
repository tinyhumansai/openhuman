//! Focused JSON-RPC E2E coverage for config, auth/credentials, app_state,
//! and connectivity controller surfaces.

use crate::env_guard::{env_lock, env_lock_async, EnvVarGuard};
use crate::rpc_auth::ensure_rpc_auth;
use crate::rpc_harness::{ok, payload, rpc, schema};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use axum::extract::{Path as AxumPath, State};
use axum::http::{header::AUTHORIZATION, HeaderMap};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::config::schema::{
    AuditConfig, CapabilityProviderConfig, CapabilityProviderTrustState, DashboardConfig,
    DingTalkConfig, DiscordConfig, EventStreamConfig, IrcConfig, LarkConfig, MatrixConfig,
    MemoryConfig, ModelHealthConfig, ProxyConfig, ProxyScope, QQConfig, ResourceLimitsConfig,
    SandboxConfig, SecurityConfig, SlackConfig, TelegramConfig, WebhookConfig, WhatsAppConfig,
};
use openhuman_core::config::{
    clear_active_user, default_projects_dir, pre_login_user_dir, read_active_user_id,
    user_openhuman_dir, write_active_user_id, Config, DaemonConfig, DictationActivationMode,
    UpdateRestartStrategy,
};
use openhuman_core::core::events::DomainEvent;
use openhuman_core::desktop::app_state::app_state_schemas;
use openhuman_core::security::credentials::bus::SessionExpiredSubscriber;
use openhuman_core::security::credentials::profiles::{AuthProfile, AuthProfilesStore, TokenSet};
use openhuman_core::security::credentials::{
    clear_composio_api_key, decrypt_secret, encrypt_secret, get_composio_api_key,
    list_provider_credentials_by_prefix, normalize_provider, rpc_store_composio_api_key,
    store_composio_api_key, AuthService, APP_SESSION_PROVIDER, COMPOSIO_DIRECT_PROVIDER,
};
use openhuman_rpc::server::build_core_http_router;
use openhuman_tinyhumans::backend::url::{
    api_base_from_env, app_env_from_env, default_api_base_url_for_env, effective_api_url,
    effective_backend_api_url, effective_inference_url, join_url as api_url,
    looks_like_local_ai_endpoint, normalize_api_base_url, APP_ENV_VAR, DEFAULT_API_BASE_URL,
    DEFAULT_STAGING_API_BASE_URL, OPENHUMAN_INFERENCE_PATH, VITE_APP_ENV_VAR,
};
use tinybus::EventHandler;

async fn serve_rpc() -> (
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
struct MockBackendState {
    auth_me_hits: Arc<AtomicUsize>,
}

async fn serve_mock_backend() -> (
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

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
}

async fn mock_auth_me(State(state): State<MockBackendState>, headers: HeaderMap) -> Json<Value> {
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

async fn mock_channel_link_token(AxumPath(channel): AxumPath<String>) -> Json<Value> {
    Json(json!({
        "success": true,
        "data": {
            "channel": channel,
            "linkToken": "link-token-123",
            "expiresIn": 300
        }
    }))
}

async fn mock_integrations() -> Json<Value> {
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

async fn mock_oauth_connect() -> Json<Value> {
    Json(json!({
        "success": true,
        "oauthUrl": "https://github.example.test/oauth?state=worker-a-state",
        "state": "worker-a-state"
    }))
}

async fn mock_integration_tokens() -> Json<Value> {
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

async fn mock_client_key(AxumPath(integration_id): AxumPath<String>) -> Json<Value> {
    Json(json!({
        "success": true,
        "data": {
            "integrationId": integration_id,
            "clientKey": "client-key-share"
        }
    }))
}

fn encrypt_handoff_blob(key: &str, plaintext: &str) -> String {
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

async fn mock_revoke_integration(AxumPath(_integration_id): AxumPath<String>) -> Json<Value> {
    Json(json!({ "success": true, "data": { "revoked": true } }))
}

fn write_min_config(openhuman_dir: &Path) {
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

struct TestHarness {
    _tmp: TempDir,
    home: std::path::PathBuf,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

async fn setup() -> TestHarness {
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

fn err<'a>(value: &'a Value, context: &str) -> &'a Value {
    value
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected JSON-RPC error, got: {value}"))
}

fn assert_error_contains(value: &Value, context: &str, needle: &str) {
    let message = err(value, context)
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        message.contains(needle),
        "{context}: expected error containing {needle:?}, got {message:?}"
    );
}

fn schema_method_names(value: &Value, namespace: &str) -> Vec<String> {
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

#[test]
fn config_schema_defaults_cover_dashboard_capability_memory_and_security_shapes() {
    let capability = CapabilityProviderConfig::default();
    assert_eq!(
        capability.trust_state,
        CapabilityProviderTrustState::Untrusted
    );
    assert!(!capability.enabled);
    let trusted_capability: CapabilityProviderConfig = serde_json::from_value(json!({
        "id": "external-mcp",
        "display_name": "External MCP",
        "source_uri": "https://example.test/catalog.json",
        "source_digest": "sha256:abc123",
        "trust_state": "trusted",
        "enabled": true
    }))
    .expect("capability provider config should deserialize");
    assert_eq!(
        trusted_capability.trust_state,
        CapabilityProviderTrustState::Trusted
    );

    let dashboard = DashboardConfig::default();
    assert!(dashboard.event_stream.enabled);
    assert_eq!(dashboard.event_stream.max_entries, 200);
    assert_eq!(dashboard.event_stream.new_entries, "top");
    assert!(dashboard.model_health.enabled);
    assert_eq!(dashboard.model_health.min_tasks_for_rating, 10);
    assert_eq!(dashboard.model_health.evaluation_window_tasks, 50);
    assert!(dashboard.diagram_viewer.enabled);
    assert_eq!(
        dashboard.diagram_viewer.source_url,
        "http://localhost:8787/workspace/diagrams/latest.png"
    );
    assert_eq!(dashboard.diagram_viewer.refresh_interval_seconds, 10);
    let partial_dashboard: DashboardConfig = serde_json::from_value(json!({
        "event_stream": {},
        "model_health": {},
        "diagram_viewer": {}
    }))
    .expect("partial dashboard config should fill serde defaults");
    assert!(partial_dashboard.event_stream.enabled);
    assert_eq!(partial_dashboard.model_health.hallucination_threshold, 0.10);
    assert_eq!(
        partial_dashboard.diagram_viewer.refresh_interval_seconds,
        10
    );
    let event_stream: EventStreamConfig =
        serde_json::from_value(json!({})).expect("event stream defaults");
    assert_eq!(event_stream.new_entries, "top");
    let model_health: ModelHealthConfig =
        serde_json::from_value(json!({})).expect("model health defaults");
    assert_eq!(model_health.evaluation_window_tasks, 50);

    let memory = MemoryConfig::default();
    assert_eq!(memory.engine, "tinyhumans");
    assert!(memory.conversations.enabled);
    assert!(memory.context.enabled);

    let telegram: TelegramConfig = serde_json::from_value(json!({
        "bot_token": "bot-token",
        "allowed_users": ["alice"]
    }))
    .expect("telegram serde defaults");
    assert_eq!(telegram.draft_update_interval_ms, 1000);
    assert!(telegram.silent_streaming);
    assert!(!telegram.mention_only);

    let sandbox = SandboxConfig::default();
    assert!(sandbox.enabled.is_none());
    assert!(sandbox.firejail_args.is_empty());
    let security = SecurityConfig::default();
    assert!(security.audit.enabled);
    let resources = ResourceLimitsConfig::default();
    assert_eq!(
        serde_json::to_value(resources).expect("resource limits to json"),
        json!({})
    );
    let audit = AuditConfig::default();
    assert_eq!(audit.log_path, "audit.log");
    assert_eq!(audit.max_size_mb, 100);

    let observability: openhuman_core::config::schema::ObservabilityConfig =
        serde_json::from_value(json!({})).expect("observability defaults");
    assert!(observability.analytics_enabled);
    assert!(observability.sentry_dsn.is_none());
    let scheduler_gate: openhuman_core::config::schema::SchedulerGateConfig =
        serde_json::from_value(json!({})).expect("scheduler gate defaults");
    assert_eq!(
        scheduler_gate.mode,
        openhuman_core::config::schema::SchedulerGateMode::Auto
    );
    assert_eq!(
        openhuman_core::config::schema::SchedulerGateMode::AlwaysOn.as_str(),
        "always_on"
    );
    assert_eq!(
        openhuman_core::config::schema::SchedulerGateMode::Off.as_str(),
        "off"
    );

    let multimodal = openhuman_core::config::schema::MultimodalConfig {
        max_images: 99,
        max_image_size_mb: 0,
        allow_remote_fetch: true,
    };
    assert_eq!(multimodal.effective_limits(), (16, 1));
    assert_eq!(multimodal.clamp_image_count(120), 99);

    let mut local_ai = openhuman_core::config::schema::LocalAiConfig {
        runtime_enabled: false,
        usage: openhuman_core::config::schema::LocalAiUsage {
            embeddings: true,
            heartbeat: true,
            subconscious: true,
        },
        ..Default::default()
    };
    assert!(!local_ai.is_active());
    #[allow(deprecated)]
    {
        assert!(!local_ai.use_local_for_embeddings());
        local_ai.runtime_enabled = true;
        assert!(local_ai.is_active());
        assert!(local_ai.use_local_for_embeddings());
        assert!(local_ai.use_local_for_heartbeat());
        assert!(local_ai.use_local_for_subconscious());
    }

    let mut search = openhuman_core::config::schema::SearchConfig {
        brave: openhuman_core::config::schema::SearchEngineCredentials {
            api_key: Some(" brave-key ".into()),
        },
        ..Default::default()
    };
    assert_eq!(
        search.brave.key(),
        Some("brave-key"),
        "search credential keys should be trimmed at read time"
    );
    search
        .apply_legacy_engine(" Brave ")
        .expect("legacy engine applies");
    assert_eq!(
        search.roles.get("search"),
        Some(&vec!["brave".to_string(), "exa".to_string()])
    );
    assert!(search.apply_legacy_engine("bing").is_err());

    let integration = openhuman_core::config::schema::IntegrationToggle {
        enabled: true,
        mode: "byo".into(),
        api_key: Some("   ".into()),
    };
    assert!(!integration.is_active());
    let managed_integration = openhuman_core::config::schema::IntegrationToggle {
        enabled: true,
        mode: "managed".into(),
        api_key: None,
    };
    assert!(managed_integration.is_active());

    let mcp_default = openhuman_core::config::schema::McpServerConfig::default();
    assert!(mcp_default.enabled);
    assert_eq!(mcp_default.timeout_secs, 30);
    assert!(matches!(
        mcp_default.auth,
        openhuman_core::config::schema::McpAuthConfig::None
    ));
    let mcp_with_auth: openhuman_core::config::schema::McpServerConfig =
        serde_json::from_value(json!({
            "name": "worker-a-mcp",
            "endpoint": "https://mcp.example.test",
            "auth": {
                "kind": "header",
                "name": "x-api-key",
                "value": "secret"
            }
        }))
        .expect("mcp server auth config");
    assert!(matches!(
        mcp_with_auth.auth,
        openhuman_core::config::schema::McpAuthConfig::Header { .. }
    ));
    for auth in [
        json!({ "kind": "bearer_token", "token": "bearer" }),
        json!({ "kind": "basic", "username": "u", "password": "p" }),
        json!({ "kind": "query_param", "name": "api_key", "value": "secret" }),
    ] {
        let _: openhuman_core::config::schema::McpAuthConfig =
            serde_json::from_value(auth).expect("mcp auth variant should deserialize");
    }
}

#[test]
fn config_active_user_and_daemon_public_helpers_cover_path_branches() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join(".openhuman");

    assert_eq!(read_active_user_id(&root), None);
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("active_user.toml"), "user_id = \"   \"\n")
        .expect("write blank active user");
    assert_eq!(read_active_user_id(&root), None);
    std::fs::write(root.join("active_user.toml"), "not = [toml\n")
        .expect("write malformed active user");
    assert_eq!(read_active_user_id(&root), None);

    write_active_user_id(&root, "user-77").expect("write active user");
    assert_eq!(read_active_user_id(&root).as_deref(), Some("user-77"));
    assert_eq!(
        user_openhuman_dir(&root, "user-77"),
        root.join("users").join("user-77")
    );
    assert_eq!(pre_login_user_dir(&root), root.join("users").join("local"));

    clear_active_user(&root).expect("clear active user");
    clear_active_user(&root).expect("clearing missing active user is idempotent");
    assert_eq!(read_active_user_id(&root), None);

    let daemon = DaemonConfig::from_app_data_dir(tmp.path());
    assert_eq!(daemon.data_dir, tmp.path().join("openhuman"));
    assert_eq!(
        daemon.workspace_dir,
        tmp.path().join("openhuman").join("workspace")
    );
    assert!(daemon.security.audit.enabled);
}

#[test]
fn config_proxy_public_paths_normalize_validate_and_apply_scope() {
    let _lock = env_lock();
    let _http = EnvVarGuard::unset("HTTP_PROXY");
    let _https = EnvVarGuard::unset("HTTPS_PROXY");
    let _all = EnvVarGuard::unset("ALL_PROXY");
    let _no = EnvVarGuard::unset("NO_PROXY");
    let _http_lower = EnvVarGuard::unset("http_proxy");
    let _https_lower = EnvVarGuard::unset("https_proxy");
    let _all_lower = EnvVarGuard::unset("all_proxy");
    let _no_lower = EnvVarGuard::unset("no_proxy");

    assert!(ProxyConfig::supported_service_keys().contains(&"memory.embeddings"));
    assert!(ProxyConfig::supported_service_selectors().contains(&"tool.*"));

    let services = ProxyConfig {
        enabled: true,
        http_proxy: Some(" http://proxy.example:8080 ".into()),
        https_proxy: Some("https://secure-proxy.example".into()),
        all_proxy: None,
        no_proxy: vec![" localhost, 127.0.0.1 ".into(), "example.test".into()],
        scope: ProxyScope::Services,
        services: vec![
            " Tool.* ".into(),
            "tool.browser".into(),
            "memory.embeddings".into(),
        ],
    };
    services.validate().expect("valid services proxy");
    assert_eq!(
        services.normalized_services(),
        vec!["memory.embeddings", "tool.*", "tool.browser"]
    );
    assert_eq!(
        services.normalized_no_proxy(),
        vec!["127.0.0.1", "example.test", "localhost"]
    );
    assert!(services.should_apply_to_service("tool.http_request"));
    assert!(services.should_apply_to_service("memory.embeddings"));
    assert!(!services.should_apply_to_service("provider.openai"));
    assert!(!services.should_apply_to_service("   "));
    let _client = services
        .apply_to_reqwest_builder(reqwest::Client::builder(), "tool.browser")
        .build()
        .expect("proxied client builds");

    let env_scope = ProxyConfig {
        enabled: true,
        scope: ProxyScope::Environment,
        all_proxy: Some("socks5h://proxy.example:1080".into()),
        ..ProxyConfig::default()
    };
    env_scope.validate().expect("valid env proxy");
    assert!(!env_scope.should_apply_to_service("tool.browser"));
    env_scope.apply_to_process_env();
    assert_eq!(
        std::env::var("ALL_PROXY").as_deref(),
        Ok("socks5h://proxy.example:1080")
    );
    assert_eq!(
        std::env::var("all_proxy").as_deref(),
        Ok("socks5h://proxy.example:1080")
    );
    assert!(std::env::var("NO_PROXY").is_err());

    ProxyConfig::clear_process_env();
    assert!(std::env::var("ALL_PROXY").is_err());
    assert!(std::env::var("all_proxy").is_err());

    let openhuman_scope = ProxyConfig {
        enabled: true,
        scope: ProxyScope::OpenHuman,
        http_proxy: Some("https://proxy.example".into()),
        no_proxy: vec![" local.test ".into()],
        ..ProxyConfig::default()
    };
    assert!(openhuman_scope.has_any_proxy_url());
    assert!(openhuman_scope.should_apply_to_service("provider.openai"));
    assert_eq!(openhuman_scope.normalized_no_proxy(), vec!["local.test"]);
    openhuman_scope.apply_to_process_env();
    assert_eq!(
        std::env::var("HTTP_PROXY").as_deref(),
        Ok("https://proxy.example")
    );
    assert_eq!(std::env::var("NO_PROXY").as_deref(), Ok("local.test"));
    ProxyConfig::clear_process_env();

    for mut invalid in [
        ProxyConfig {
            enabled: true,
            http_proxy: Some("ftp://proxy.example".into()),
            ..ProxyConfig::default()
        },
        ProxyConfig {
            enabled: true,
            scope: ProxyScope::Services,
            services: vec![],
            http_proxy: Some("http://proxy.example".into()),
            ..ProxyConfig::default()
        },
        ProxyConfig {
            enabled: true,
            http_proxy: None,
            https_proxy: None,
            all_proxy: None,
            ..ProxyConfig::default()
        },
        ProxyConfig {
            enabled: false,
            services: vec!["unknown.service".into()],
            ..ProxyConfig::default()
        },
    ] {
        assert!(
            invalid.validate().is_err(),
            "invalid proxy config should fail: {invalid:?}"
        );
        invalid.enabled = false;
    }

    openhuman_core::config::set_runtime_proxy_config(services.clone());
    assert!(openhuman_core::config::runtime_proxy_config().should_apply_to_service("tool.browser"));
    let _cached = openhuman_core::config::build_runtime_proxy_client("tool.browser");
    let _cached_again = openhuman_core::config::build_runtime_proxy_client("tool.browser");
    let _timeout_client =
        openhuman_core::config::build_runtime_proxy_client_with_timeouts("memory.embeddings", 1, 1);
    let _builder = openhuman_core::config::apply_runtime_proxy_to_builder(
        reqwest::Client::builder(),
        "tool.http_request",
    );
    openhuman_core::config::set_runtime_proxy_config(ProxyConfig::default());
}

#[test]
fn api_config_url_resolution_classifies_backend_and_inference_paths() {
    let _lock = env_lock();
    // Integration-test binaries link the library compiled WITHOUT `cfg(test)`,
    // so `compile_time_api_base_env_values()` resolves `option_env!("BACKEND_URL")`
    // / `option_env!("VITE_BACKEND_URL")`. The mock test harness bakes
    // `BACKEND_URL` at build time, which would make the blank / local-AI override
    // fall-throughs below resolve to the baked URL instead of the compile
    // default. Pin `BACKEND_URL` at runtime — runtime resolution wins over the
    // compile-time bake — so env/default resolution is deterministic regardless
    // of what CI baked into the binary.
    let _backend = EnvVarGuard::set("BACKEND_URL", DEFAULT_API_BASE_URL);
    let _vite_backend = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _app_env = EnvVarGuard::unset(APP_ENV_VAR);
    let _vite_app_env = EnvVarGuard::unset(VITE_APP_ENV_VAR);

    assert_eq!(
        normalize_api_base_url(" https://api.example.test/// "),
        "https://api.example.test"
    );
    assert_eq!(
        api_url(
            "https://api.tinyhumans.ai/openai/v1/chat/completions",
            "/auth/me"
        ),
        "https://api.tinyhumans.ai/auth/me"
    );
    assert_eq!(api_url("not a url/", "auth/me"), "not a url/auth/me");
    assert_eq!(api_url("not a url/", "/auth/me"), "not a url/auth/me");
    assert_eq!(
        api_url(" https://api.tinyhumans.ai/ ", ""),
        "https://api.tinyhumans.ai"
    );

    assert!(!looks_like_local_ai_endpoint(""));
    assert!(looks_like_local_ai_endpoint("http://localhost:11434"));
    assert!(looks_like_local_ai_endpoint(
        "http://10.0.0.2/v1/chat/completions"
    ));
    assert!(looks_like_local_ai_endpoint(
        "https://api.openai.com/v1/completions"
    ));
    assert!(looks_like_local_ai_endpoint("http://0.0.0.0:8000"));
    assert!(looks_like_local_ai_endpoint("http://service.localhost/v1"));
    assert!(looks_like_local_ai_endpoint("http://192.168.1.7:9000/v1"));
    assert!(!looks_like_local_ai_endpoint("http://127.0.0.1:45678"));
    assert!(!looks_like_local_ai_endpoint("https://api.example.test/v1"));
    assert!(!looks_like_local_ai_endpoint(
        "https://api.example.test/audit/v1/chat/completions-logs"
    ));
    assert!(!looks_like_local_ai_endpoint("not a url"));

    assert_eq!(effective_api_url(&Some("   ".into())), DEFAULT_API_BASE_URL);
    assert_eq!(
        effective_api_url(&Some(" http://127.0.0.1:11434/ ".into())),
        "http://127.0.0.1:11434"
    );
    assert_eq!(
        effective_inference_url(&Some("https://api.tinyhumans.ai".into()), &None),
        format!("https://api.tinyhumans.ai{OPENHUMAN_INFERENCE_PATH}")
    );
    assert_eq!(
        effective_inference_url(
            &Some("https://api.tinyhumans.ai".into()),
            &Some(" http://127.0.0.1:11434/v1/chat/completions ".into())
        ),
        "http://127.0.0.1:11434/v1/chat/completions"
    );

    assert_eq!(
        effective_backend_api_url(&Some(" http://127.0.0.1:11434/v1 ".into())),
        DEFAULT_API_BASE_URL
    );
    assert_eq!(
        effective_backend_api_url(&Some(
            " https://api.tinyhumans.ai/openai/v1/chat/completions?x=1#frag ".into()
        )),
        "https://api.tinyhumans.ai"
    );
    assert_eq!(
        effective_backend_api_url(&Some("api.tinyhumans.ai/openai/v1/chat/completions".into())),
        "https://api.tinyhumans.ai"
    );
    assert_eq!(
        effective_backend_api_url(&Some(" http://backend.example.test/path?q=1#frag ".into())),
        "http://backend.example.test"
    );

    std::env::set_var("BACKEND_URL", "");
    std::env::set_var(
        "VITE_BACKEND_URL",
        " https://backend.example.test/openai/v1/chat/completions ",
    );
    assert_eq!(
        api_base_from_env().as_deref(),
        Some("https://backend.example.test/openai/v1/chat/completions")
    );
    assert_eq!(
        effective_backend_api_url(&None),
        "https://backend.example.test"
    );

    std::env::set_var(APP_ENV_VAR, " Staging ");
    assert_eq!(app_env_from_env().as_deref(), Some("staging"));
    assert_eq!(
        default_api_base_url_for_env(app_env_from_env().as_deref()),
        DEFAULT_STAGING_API_BASE_URL
    );

    std::env::remove_var(APP_ENV_VAR);
    std::env::set_var(VITE_APP_ENV_VAR, " Production ");
    assert_eq!(app_env_from_env().as_deref(), Some("production"));
}

#[tokio::test]
async fn credentials_session_expired_subscriber_ignores_unrelated_events() {
    let subscriber = SessionExpiredSubscriber::new();
    assert_eq!(subscriber.name(), "credentials::session_expired_handler");
    assert_eq!(subscriber.domains(), Some(&["auth"][..]));

    subscriber
        .handle(&DomainEvent::AgentTurnStarted {
            session_id: "worker-a-session".to_string(),
            channel: "e2e".to_string(),
        })
        .await;
}

/// Sentry 36649: an offline local session ("Continue locally") has no
/// TinyHumans account, so every hosted RPC — the ones that moved into
/// `openhuman-tinyhumans` included — must answer the `BACKEND_UNAVAILABLE:`
/// sentinel the JSON-RPC layer demotes, without a backend request.
#[tokio::test]
async fn hosted_rpcs_answer_backend_unavailable_for_a_local_session() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let local_session = rpc(
        &harness.rpc_base,
        18_201,
        "openhuman.auth_store_session",
        json!({
            "token": "header.payload.local",
            "user": { "id": "local-hosted-36649", "name": "Local Hosted Worker" }
        }),
    )
    .await;
    assert_eq!(
        payload(&local_session, "auth_store_session local")
            .get("credential")
            .and_then(Value::as_str),
        Some("local")
    );

    for (id, method, params) in [
        (18_202, "openhuman.team_get_usage", json!({})),
        (18_203, "openhuman.announcements_get_latest", json!({})),
        (18_204, "openhuman.billing_get_current_plan", json!({})),
        (18_205, "openhuman.webhooks_list_tunnels", json!({})),
        (18_206, "openhuman.channels_telegram_login_start", json!({})),
        (18_207, "openhuman.auth_oauth_list_integrations", json!({})),
        (
            18_208,
            "openhuman.auth_create_channel_link_token",
            json!({ "channel": "telegram" }),
        ),
    ] {
        assert_error_contains(
            &rpc(&harness.rpc_base, id, method, params).await,
            method,
            "BACKEND_UNAVAILABLE:",
        );
    }

    harness.join.abort();
}

#[tokio::test]
async fn credentials_session_expired_subscriber_clears_remote_session_but_keeps_local_session() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    // The host hands the core an already-obtained session; no backend is
    // consulted to store it.
    let remote_session = rpc(
        &harness.rpc_base,
        18_101,
        "openhuman.auth_store_session",
        json!({
            "token": "session-expired-remote-jwt",
            "user_id": "session-expired-user",
            "user": {
                "id": "session-expired-user",
                "name": "Session Expired Worker",
                "email": "session-expired@example.test"
            }
        }),
    )
    .await;
    assert_eq!(
        payload(&remote_session, "auth_store_session before SessionExpired")
            .get("credential")
            .and_then(Value::as_str),
        Some("session")
    );

    let subscriber = SessionExpiredSubscriber::new();
    subscriber
        .handle(&DomainEvent::SessionExpired {
            source: "coverage-test".to_string(),
            reason: "remote token rejected".to_string(),
        })
        .await;

    let cleared_state = rpc(
        &harness.rpc_base,
        18_102,
        "openhuman.auth_get_state",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&cleared_state, "auth_get_state after remote SessionExpired")
            .get("isAuthenticated")
            .and_then(Value::as_bool),
        Some(false),
        "remote SessionExpired events should clear the stored session"
    );

    let local_session = rpc(
        &harness.rpc_base,
        18_103,
        "openhuman.auth_store_session",
        json!({
            "token": "header.payload.local",
            "user": {
                "id": "renderer-local-session-expired",
                "name": "Local Session Expired Worker",
                "email": "local-session-expired@example.test"
            }
        }),
    )
    .await;
    assert_eq!(
        payload(
            &local_session,
            "auth_store_session local before SessionExpired"
        )
        .get("credential")
        .and_then(Value::as_str),
        Some("local")
    );

    subscriber
        .handle(&DomainEvent::SessionExpired {
            source: "coverage-test".to_string(),
            reason: "local token should survive".to_string(),
        })
        .await;

    let local_state = rpc(
        &harness.rpc_base,
        18_104,
        "openhuman.auth_get_state",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&local_state, "auth_get_state after local SessionExpired")
            .get("isAuthenticated")
            .and_then(Value::as_bool),
        Some(true),
        "local offline sessions should not be cleared by SessionExpired events"
    );

    harness.join.abort();
}

#[tokio::test]
async fn config_loaders_resolve_user_workspace_markers_and_ignore_workspace_when_scoped() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let root = home.join(".openhuman");
    let user_dir = root.join("users").join("user-42");
    let explicit_config_dir = tmp.path().join("explicit");
    let explicit_workspace = tmp.path().join("explicit-workspace");
    let env_workspace = tmp.path().join("env-workspace");
    let legacy_parent = tmp.path().join("legacy-parent");
    let legacy_config_dir = legacy_parent.join(".openhuman");
    let legacy_workspace = legacy_parent.join("workspace");

    let _guards = vec![
        EnvVarGuard::set_to_path("HOME", &home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset("OPENHUMAN_MODEL"),
        EnvVarGuard::unset(APP_ENV_VAR),
        EnvVarGuard::unset(VITE_APP_ENV_VAR),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];

    std::fs::create_dir_all(&root).expect("create root config dir");
    write_active_user_id(&root, "user-42").expect("write active user marker");
    write_min_config(&user_dir);

    let active_user_config = Config::load_or_init()
        .await
        .expect("load active user config");
    assert_eq!(active_user_config.config_path, user_dir.join("config.toml"));
    assert_eq!(active_user_config.workspace_dir, user_dir.join("workspace"));

    {
        write_min_config(&env_workspace);
        let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &env_workspace);
        let env_config = Config::load_or_init()
            .await
            .expect("load env workspace config");
        assert_eq!(env_config.config_path, env_workspace.join("config.toml"));
        assert_eq!(env_config.workspace_dir, env_workspace.join("workspace"));
    }

    {
        write_min_config(&legacy_config_dir);
        std::fs::create_dir_all(&legacy_workspace).expect("create legacy workspace");
        let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &legacy_workspace);
        let legacy_config = Config::load_or_init()
            .await
            .expect("load legacy workspace config");
        assert_eq!(
            legacy_config.config_path,
            legacy_config_dir.join("config.toml")
        );
        assert_eq!(legacy_config.workspace_dir, legacy_workspace);
    }

    clear_active_user(&root).expect("clear active user marker");
    std::fs::write(root.join("active_workspace.toml"), "config_dir = [\n")
        .expect("write malformed active workspace marker");
    let malformed_marker_config = Config::load_or_init()
        .await
        .expect("malformed active workspace marker should be ignored");
    assert_eq!(
        malformed_marker_config.config_path,
        root.join("users").join("local").join("config.toml")
    );

    std::fs::remove_file(root.join("active_workspace.toml"))
        .expect("remove malformed active workspace marker");
    std::fs::create_dir(root.join("active_workspace.toml"))
        .expect("create unreadable active workspace marker");
    let unreadable_marker_config = Config::load_or_init()
        .await
        .expect("unreadable active workspace marker should be ignored");
    assert_eq!(
        unreadable_marker_config.config_path,
        root.join("users").join("local").join("config.toml")
    );
    std::fs::remove_dir(root.join("active_workspace.toml"))
        .expect("remove unreadable active workspace marker");

    let active_marker_dir = root.join("relative-active");
    write_min_config(&active_marker_dir);
    std::fs::write(
        root.join("active_workspace.toml"),
        "config_dir = \"relative-active\"\n",
    )
    .expect("write active workspace marker");
    let marker_config = Config::load_or_init()
        .await
        .expect("load active workspace marker config");
    assert_eq!(
        marker_config.config_path,
        active_marker_dir.join("config.toml")
    );
    assert_eq!(
        marker_config.workspace_dir,
        active_marker_dir.join("workspace")
    );

    write_min_config(&explicit_config_dir);
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &env_workspace);
    let _model_guard = EnvVarGuard::set("OPENHUMAN_MODEL", " scoped-env-model ");
    let explicit = Config::load_from_config_path(
        &explicit_config_dir.join("config.toml"),
        &explicit_workspace,
    )
    .await
    .expect("load explicit config path");
    assert_eq!(explicit.workspace_dir, explicit_workspace);
    assert_eq!(explicit.default_model.as_deref(), Some("scoped-env-model"));
}

#[tokio::test]
async fn config_default_path_loader_ignores_workspace_override_and_projects_dir_trims() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let root = home.join(".openhuman");
    let user_dir = root.join("users").join("default-loader-user");
    let workspace_override = tmp.path().join("workspace-override");
    let _guards = [EnvVarGuard::set_to_path("HOME", &home),
        EnvVarGuard::unset(APP_ENV_VAR),
        EnvVarGuard::unset(VITE_APP_ENV_VAR),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace_override),
        EnvVarGuard::set("OPENHUMAN_MODEL", " default-loader-model ")];

    let missing = Config::load_from_default_paths()
        .await
        .expect("load default paths without an existing config");
    assert_eq!(
        missing.config_path,
        root.join("users").join("local").join("config.toml")
    );
    assert_eq!(missing.workspace_dir, workspace_override.join("workspace"));
    assert_eq!(
        missing.default_model.as_deref(),
        Some("default-loader-model")
    );

    write_active_user_id(&root, "default-loader-user").expect("write active user");
    write_min_config(&user_dir);
    write_min_config(&workspace_override);
    let loaded = Config::load_from_default_paths()
        .await
        .expect("load default active-user config while workspace override is set");
    assert_eq!(loaded.config_path, user_dir.join("config.toml"));
    assert_eq!(loaded.workspace_dir, workspace_override.join("workspace"));
    assert_eq!(
        loaded.default_model.as_deref(),
        Some("default-loader-model")
    );

    let custom_projects = tmp.path().join("OpenHuman Projects");
    {
        let _projects_guard = EnvVarGuard::set_to_path("OPENHUMAN_PROJECTS_DIR", &custom_projects);
        assert_eq!(default_projects_dir(), custom_projects);
    }
    let _blank_projects_guard = EnvVarGuard::set("OPENHUMAN_PROJECTS_DIR", "   ");
    assert_eq!(
        default_projects_dir(),
        home.join("OpenHuman").join("projects")
    );
}

#[tokio::test]
async fn config_env_overlay_public_loader_applies_runtime_and_tool_overrides() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let config_dir = tmp.path().join("config");
    let workspace_dir = tmp.path().join("workspace");
    write_min_config(&config_dir);

    let _guards = vec![
        EnvVarGuard::set_to_path("HOME", tmp.path()),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::set("OPENHUMAN_MODEL", " env-model "),
        EnvVarGuard::set("OPENHUMAN_TEMPERATURE", "1.25"),
        EnvVarGuard::set("OPENHUMAN_MAX_ACTIONS_PER_HOUR", "17"),
        EnvVarGuard::set("OPENHUMAN_OUTPUT_LANGUAGE", " ja "),
        EnvVarGuard::set("OPENHUMAN_REASONING_ENABLED", "yes"),
        EnvVarGuard::set("OPENHUMAN_SELTZ_API_KEY", "seltz-key"),
        EnvVarGuard::set("OPENHUMAN_SELTZ_API_URL", "https://seltz.example/v1"),
        EnvVarGuard::set("OPENHUMAN_SELTZ_MAX_RESULTS", "13"),
        EnvVarGuard::set("OPENHUMAN_SEARXNG_ENABLED", "on"),
        EnvVarGuard::set("OPENHUMAN_SEARXNG_BASE_URL", "https://searx.example"),
        EnvVarGuard::set("OPENHUMAN_SEARXNG_MAX_RESULTS", "31"),
        EnvVarGuard::set("OPENHUMAN_SEARXNG_DEFAULT_LANGUAGE", "de"),
        EnvVarGuard::set("OPENHUMAN_SEARXNG_TIMEOUT_SECS", "9"),
        EnvVarGuard::set("OPENHUMAN_SEARCH_ENGINE", "brave"),
        EnvVarGuard::set("OPENHUMAN_PARALLEL_API_KEY", "parallel-key"),
        EnvVarGuard::set("OPENHUMAN_BRAVE_API_KEY", "brave-key"),
        EnvVarGuard::set("OPENHUMAN_QUERIT_API_KEY", "querit-key"),
        EnvVarGuard::set("OPENHUMAN_EXA_API_KEY", "exa-key"),
        EnvVarGuard::set("OPENHUMAN_SEARCH_MAX_RESULTS", "11"),
        EnvVarGuard::set("OPENHUMAN_SEARCH_TIMEOUT_SECS", "8"),
        EnvVarGuard::set("OPENHUMAN_WEB_SEARCH_ENABLED", "0"),
        EnvVarGuard::set("OPENHUMAN_WEB_SEARCH_MAX_RESULTS", "7"),
        EnvVarGuard::set("OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS", "6"),
        EnvVarGuard::set("OPENHUMAN_PROXY_ENABLED", "true"),
        EnvVarGuard::set("OPENHUMAN_HTTP_PROXY", " http://proxy.example:8080 "),
        EnvVarGuard::set("OPENHUMAN_NO_PROXY", " localhost,example.test "),
        EnvVarGuard::set("OPENHUMAN_PROXY_SCOPE", "services"),
        EnvVarGuard::set("OPENHUMAN_PROXY_SERVICES", "tool.browser,memory.embeddings"),
        EnvVarGuard::set("OPENHUMAN_NODE_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_NODE_VERSION", "v24.0.0"),
        EnvVarGuard::set("OPENHUMAN_NODE_CACHE_DIR", "/tmp/openhuman-node-cache"),
        EnvVarGuard::set("OPENHUMAN_NODE_PREFER_SYSTEM", "false"),
        EnvVarGuard::set("OPENHUMAN_RUNTIME_PYTHON_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_RUNTIME_PYTHON_MINIMUM_VERSION", "3.13.0"),
        EnvVarGuard::set(
            "OPENHUMAN_RUNTIME_PYTHON_CACHE_DIR",
            "/tmp/openhuman-python-cache",
        ),
        EnvVarGuard::set("OPENHUMAN_RUNTIME_PYTHON_MANAGED_RELEASE_TAG", "20260401"),
        EnvVarGuard::set("OPENHUMAN_RUNTIME_PYTHON_PREFER_SYSTEM", "true"),
        EnvVarGuard::set("OPENHUMAN_RUNTIME_PYTHON_PREFERRED_COMMAND", "python3.13"),
        EnvVarGuard::set("OPENHUMAN_CORE_SENTRY_DSN", "https://dsn.example/1"),
        EnvVarGuard::set("OPENHUMAN_ANALYTICS_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_AUTO_UPDATE_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_AUTO_UPDATE_INTERVAL_MINUTES", "1440"),
        EnvVarGuard::set("OPENHUMAN_AUTO_UPDATE_RESTART_STRATEGY", "supervisor"),
        EnvVarGuard::set("OPENHUMAN_AUTO_UPDATE_RPC_MUTATIONS_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_DICTATION_ENABLED", "true"),
        EnvVarGuard::set("OPENHUMAN_DICTATION_HOTKEY", "CmdOrCtrl+Shift+D"),
        EnvVarGuard::set("OPENHUMAN_DICTATION_ACTIVATION_MODE", "toggle"),
        EnvVarGuard::set("OPENHUMAN_DICTATION_LLM_REFINEMENT", "false"),
        EnvVarGuard::set("OPENHUMAN_DICTATION_STREAMING", "false"),
        EnvVarGuard::set("OPENHUMAN_DICTATION_STREAMING_INTERVAL_MS", "333"),
        EnvVarGuard::set("OPENHUMAN_CONTEXT_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_CONTEXT_MICROCOMPACT_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_CONTEXT_AUTOCOMPACT_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_CONTEXT_TOOL_RESULT_BUDGET_BYTES", "12345"),
        EnvVarGuard::set("OPENHUMAN_CONTEXT_SUMMARIZER_MODEL", "summary-env"),
    ];

    let config = Config::load_from_config_path(&config_dir.join("config.toml"), &workspace_dir)
        .await
        .expect("load config with env overlay");

    assert_eq!(config.default_model.as_deref(), Some("env-model"));
    assert_eq!(config.default_temperature, 1.25);
    assert_eq!(config.autonomy.max_actions_per_hour, 17);
    assert_eq!(config.output_language.as_deref(), Some("ja"));
    assert_eq!(config.runtime.reasoning_enabled, Some(true));
    assert!(config.seltz.enabled);
    assert_eq!(config.seltz.api_key.as_deref(), Some("seltz-key"));
    assert_eq!(config.seltz.max_results, 13);
    assert!(config.searxng.enabled);
    assert_eq!(config.searxng.base_url, "https://searx.example");
    assert_eq!(config.searxng.max_results, 31);
    assert_eq!(
        config
            .search
            .roles
            .get("search")
            .and_then(|order| order.first()),
        Some(&"brave".to_string())
    );
    assert!(config.search.parallel.has_key());
    assert!(config.search.brave.has_key());
    assert!(config.search.querit.has_key());
    assert!(config.search.exa.has_key());
    assert_eq!(config.search.max_results, 11);
    assert_eq!(config.web_search.max_results, 7);
    assert!(config.proxy.enabled);
    assert_eq!(config.proxy.scope, ProxyScope::Services);
    assert!(config.proxy.should_apply_to_service("tool.browser"));
    assert!(!config.node.enabled);
    assert_eq!(config.node.version, "v24.0.0");
    assert!(!config.node.prefer_system);
    assert!(!config.runtime_python.enabled);
    assert_eq!(config.runtime_python.minimum_version, "3.13.0");
    assert!(config.runtime_python.prefer_system);
    assert!(!config.observability.analytics_enabled);
    assert_eq!(
        config.observability.sentry_dsn.as_deref(),
        Some("https://dsn.example/1")
    );
    assert!(!config.update.enabled);
    assert_eq!(config.update.interval_minutes, 1440);
    assert_eq!(
        config.update.restart_strategy,
        UpdateRestartStrategy::Supervisor
    );
    assert!(!config.update.rpc_mutations_enabled);
    assert!(config.dictation.enabled);
    assert_eq!(config.dictation.hotkey, "CmdOrCtrl+Shift+D");
    assert_eq!(
        config.dictation.activation_mode,
        DictationActivationMode::Toggle
    );
    assert!(!config.dictation.llm_refinement);
    assert!(!config.dictation.streaming);
    assert_eq!(config.dictation.streaming_interval_ms, 333);
    assert!(!config.context.enabled);
    assert!(!config.context.microcompact_enabled);
    assert!(!config.context.autocompact_enabled);
    assert_eq!(config.context.tool_result_budget_bytes, 12345);
    assert_eq!(
        config.context.summarizer_model.as_deref(),
        Some("summary-env")
    );
}

#[tokio::test]
async fn config_save_and_load_encrypts_channel_secret_fields() {
    let _lock = env_lock_async().await;
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let _guards = vec![
        EnvVarGuard::set_to_path("HOME", &home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset(APP_ENV_VAR),
        EnvVarGuard::unset(VITE_APP_ENV_VAR),
        EnvVarGuard::unset("OPENHUMAN_TAVILY_API_KEY"),
        EnvVarGuard::unset("TAVILY_API_KEY"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];
    let config_path = home
        .join(".openhuman")
        .join("users")
        .join("local")
        .join("config.toml");
    let workspace_dir = config_path
        .parent()
        .expect("config parent")
        .join("workspace");

    let mut config = Config {
        config_path: config_path.clone(),
        workspace_dir: workspace_dir.clone(),
        ..Default::default()
    };
    config.secrets.encrypt = true;
    config.api_key = Some("api-secret".into());
    config.search.brave.api_key = Some("brave-secret".into());
    config.search.querit.api_key = Some("querit-secret".into());
    config.search.exa.api_key = Some("exa-secret".into());
    config.search.tavily.api_key = Some("tavily-secret".into());
    config.channels_config.telegram = Some(TelegramConfig {
        bot_token: "telegram-secret".into(),
        chat_id: None,
        allowed_users: vec!["alice".into()],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "discord-secret".into(),
        guild_id: Some("guild".into()),
        channel_id: Some("channel".into()),
        allowed_users: vec![],
        listen_to_bots: false,
        mention_only: false,
    });
    config.channels_config.slack = Some(SlackConfig {
        bot_token: "slack-bot-secret".into(),
        app_token: Some("slack-app-secret".into()),
        channel_id: Some("C123".into()),
        allowed_users: vec![],
    });
    config.channels_config.matrix = Some(MatrixConfig {
        homeserver: "https://matrix.example.test".into(),
        access_token: "matrix-secret".into(),
        user_id: Some("@worker:example.test".into()),
        device_id: None,
        room_id: "!room:example.test".into(),
        allowed_users: vec![],
    });
    config.channels_config.whatsapp = Some(WhatsAppConfig {
        access_token: Some("whatsapp-access-secret".into()),
        phone_number_id: Some("phone".into()),
        verify_token: Some("whatsapp-verify-secret".into()),
        app_secret: Some("whatsapp-app-secret".into()),
        session_path: None,
        pair_phone: None,
        pair_code: None,
        allowed_numbers: vec![],
    });
    config.channels_config.webhook = Some(WebhookConfig {
        port: 0,
        secret: Some("webhook-secret".into()),
    });
    config.channels_config.irc = Some(IrcConfig {
        server: "irc.example.test".into(),
        port: 6697,
        nickname: "worker".into(),
        username: Some("worker".into()),
        channels: vec!["#openhuman".into()],
        allowed_users: vec![],
        server_password: Some("irc-server-secret".into()),
        nickserv_password: Some("irc-nickserv-secret".into()),
        sasl_password: Some("irc-sasl-secret".into()),
        verify_tls: Some(true),
    });
    config.channels_config.lark = Some(LarkConfig {
        app_id: "lark-app".into(),
        app_secret: "lark-app-secret".into(),
        encrypt_key: Some("lark-encrypt-secret".into()),
        verification_token: Some("lark-verify-secret".into()),
        allowed_users: vec![],
        use_feishu: false,
        receive_mode: Default::default(),
        port: None,
    });
    config.channels_config.dingtalk = Some(DingTalkConfig {
        client_id: "dingtalk-client".into(),
        client_secret: "dingtalk-secret".into(),
        allowed_users: vec![],
    });
    config.channels_config.qq = Some(QQConfig {
        app_id: "qq-app".into(),
        app_secret: "qq-secret".into(),
        allowed_users: vec![],
    });

    config.save().await.expect("save encrypted config");
    let raw = std::fs::read_to_string(&config_path).expect("read saved encrypted config");
    for secret in [
        "api-secret",
        "exa-secret",
        "tavily-secret",
        "telegram-secret",
        "discord-secret",
        "slack-bot-secret",
        "slack-app-secret",
        "matrix-secret",
        "whatsapp-access-secret",
        "webhook-secret",
        "irc-server-secret",
        "lark-app-secret",
        "dingtalk-secret",
        "qq-secret",
    ] {
        assert!(
            !raw.contains(secret),
            "saved encrypted config should not contain plaintext {secret}: {raw}"
        );
    }

    let loaded = Config::load_or_init()
        .await
        .expect("load encrypted config from default path");
    assert_eq!(loaded.config_path, config_path);
    assert_eq!(loaded.api_key.as_deref(), Some("api-secret"));
    assert_eq!(loaded.search.exa.api_key.as_deref(), Some("exa-secret"));
    assert_eq!(
        loaded.search.tavily.api_key.as_deref(),
        Some("tavily-secret")
    );
    assert_eq!(
        loaded
            .channels_config
            .telegram
            .as_ref()
            .map(|telegram| telegram.bot_token.as_str()),
        Some("telegram-secret")
    );
    assert_eq!(
        loaded
            .channels_config
            .slack
            .as_ref()
            .and_then(|slack| slack.app_token.as_deref()),
        Some("slack-app-secret")
    );
    assert_eq!(
        loaded
            .channels_config
            .lark
            .as_ref()
            .map(|lark| lark.app_secret.as_str()),
        Some("lark-app-secret")
    );
    assert_eq!(
        loaded
            .channels_config
            .qq
            .as_ref()
            .map(|qq| qq.app_secret.as_str()),
        Some("qq-secret")
    );
}

#[test]
fn auth_service_direct_paths_cover_profile_selection_and_validation() {
    let tmp = tempdir().expect("tempdir");
    let auth = AuthService::new(tmp.path(), false);
    let store = AuthProfilesStore::new(tmp.path(), false);

    assert_eq!(normalize_provider("  GitHub  ").unwrap(), "github");
    assert!(normalize_provider("   ").is_err());
    assert!(auth.set_active_profile("github", "missing").is_err());
    assert_eq!(
        auth.get_provider_bearer_token("github", None)
            .expect("missing profile lookup"),
        None
    );

    let stored = auth
        .store_provider_token(
            "github",
            "personal",
            " ghp-token ",
            [("scope".to_string(), "repo".to_string())]
                .into_iter()
                .collect(),
            false,
        )
        .expect("store token profile");
    assert_eq!(stored.provider, "github");
    assert_eq!(
        auth.get_provider_bearer_token("GitHub", Some("personal"))
            .expect("profile override lookup"),
        Some(" ghp-token ".to_string())
    );
    assert_eq!(
        auth.get_provider_bearer_token("GitHub", None)
            .expect("no active/default lookup"),
        Some(" ghp-token ".to_string())
    );

    let active_id = auth
        .set_active_profile("GitHub", "personal")
        .expect("set active by profile name");
    assert_eq!(active_id, stored.id);
    assert!(auth
        .remove_profile("GitHub", "personal")
        .expect("remove stored profile"));
    assert!(!auth
        .remove_profile("GitHub", "personal")
        .expect("remove missing profile"));

    let oauth_profile = AuthProfile::new_oauth(
        "gitlab",
        "main",
        TokenSet {
            access_token: "gitlab-access-token".to_string(),
            refresh_token: Some("gitlab-refresh-token".to_string()),
            id_token: None,
            expires_at: None,
            token_type: Some("Bearer".to_string()),
            scope: Some("read_user".to_string()),
        },
    );
    let oauth_id = oauth_profile.id.clone();
    store
        .upsert_profile(oauth_profile, true)
        .expect("store oauth profile through shared store");
    assert_eq!(
        auth.get_provider_bearer_token("gitlab", None)
            .expect("oauth bearer lookup"),
        Some("gitlab-access-token".to_string())
    );
    assert!(auth
        .get_profile("gitlab", Some("missing"))
        .expect("missing override lookup")
        .is_none());
    let wrong_provider_err = auth
        .set_active_profile("github", &oauth_id)
        .expect_err("full profile id from another provider should fail")
        .to_string();
    assert!(
        wrong_provider_err.contains("belongs to provider gitlab"),
        "full profile ids must still match the requested provider: {wrong_provider_err}"
    );
}

#[tokio::test]
async fn auth_provider_prefix_listing_sorts_filters_and_excludes_app_session() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };
    config.secrets.encrypt = false;
    std::fs::create_dir_all(config.config_path.parent().expect("config parent"))
        .expect("create config parent");

    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        "channel:slack:bot",
        "default",
        "slack-token",
        [("team".to_string(), "T1".to_string())]
            .into_iter()
            .collect(),
        true,
    )
    .expect("store slack channel token");
    auth.store_provider_token(
        "channel:telegram:managed_dm",
        "default",
        "telegram-token",
        [("chat_id".to_string(), "42".to_string())]
            .into_iter()
            .collect(),
        false,
    )
    .expect("store telegram channel token");
    auth.store_provider_token(
        "github",
        "default",
        "github-token",
        Default::default(),
        true,
    )
    .expect("store non-channel token");
    auth.store_provider_token(
        APP_SESSION_PROVIDER,
        "default",
        "session-token",
        Default::default(),
        true,
    )
    .expect("store app session token");

    let channels = list_provider_credentials_by_prefix(&config, "channel:")
        .await
        .expect("list channel credentials by prefix");
    let providers = channels
        .iter()
        .map(|profile| profile.provider.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        providers,
        vec!["channel:slack:bot", "channel:telegram:managed_dm"]
    );
    assert!(channels.iter().all(|profile| profile.has_token));
    assert!(channels.iter().all(|profile| !profile.has_token_set));

    let missing = list_provider_credentials_by_prefix(&config, "calendar:")
        .await
        .expect("missing prefix listing should succeed");
    assert!(missing.is_empty());
}

#[tokio::test]
async fn composio_direct_credentials_helpers_trim_store_and_clear_key() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };
    config.secrets.encrypt = false;
    std::fs::create_dir_all(config.config_path.parent().expect("config parent"))
        .expect("create config parent");

    assert_eq!(
        get_composio_api_key(&config).expect("empty composio key store"),
        None
    );
    assert!(
        store_composio_api_key(&config, "   ").await.is_err(),
        "blank composio keys should be rejected"
    );

    let stored = store_composio_api_key(&config, "  cmp_worker_a_secret  ")
        .await
        .expect("store direct composio key");
    assert_eq!(
        stored.value.get("provider").and_then(Value::as_str),
        Some(COMPOSIO_DIRECT_PROVIDER)
    );
    assert_eq!(
        get_composio_api_key(&config).expect("stored composio key"),
        Some("cmp_worker_a_secret".to_string())
    );

    let stored_via_rpc = rpc_store_composio_api_key(&config, "cmp_worker_a_second")
        .await
        .expect("store direct composio key via rpc helper");
    assert_eq!(
        stored_via_rpc.value.get("stored").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        get_composio_api_key(&config).expect("updated composio key"),
        Some("cmp_worker_a_second".to_string())
    );

    let cleared = clear_composio_api_key(&config)
        .await
        .expect("clear direct composio key");
    assert_eq!(
        cleared.value.get("removed").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        get_composio_api_key(&config).expect("cleared composio key"),
        None
    );
    let cleared_again = clear_composio_api_key(&config)
        .await
        .expect("clear missing direct composio key");
    assert_eq!(
        cleared_again.value.get("removed").and_then(Value::as_bool),
        Some(false)
    );
}

#[tokio::test]
async fn credentials_public_ops_cover_service_and_missing_session_error_paths() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };
    config.secrets.encrypt = false;
    config.local_ai.runtime_enabled = false;
    config.voice_server.auto_start = false;
    std::fs::create_dir_all(config.config_path.parent().expect("config parent"))
        .expect("create config parent");

    openhuman_core::security::credentials::start_login_gated_services(&config).await;
    openhuman_core::security::credentials::stop_login_gated_services(&config).await;

    // The account-bound link-token and OAuth ops live in `openhuman-tinyhumans`
    // (`hosted::{channel_link, oauth}`); validation still runs first, and a
    // missing credential fails before any request.
    use openhuman_tinyhumans::hosted::{channel_link, oauth};
    assert!(channel_link::auth_create_channel_link_token(&config, "   ")
        .await
        .expect_err("blank channel should fail")
        .contains("channel is required"));
    assert!(
        channel_link::auth_create_channel_link_token(&config, "matrix")
            .await
            .expect_err("unsupported channel should fail")
            .contains("unsupported channel")
    );
    assert!(
        channel_link::auth_create_channel_link_token(&config, "telegram")
            .await
            .expect_err("missing session should fail")
            .contains("no backend session token")
    );
    assert!(oauth::oauth_connect(
        &config,
        "github",
        Some("skill"),
        Some("code"),
        Some("handoff"),
    )
    .await
    .expect_err("oauth connect without session should fail")
    .contains("no backend session token"));
    assert!(oauth::oauth_list_integrations(&config)
        .await
        .expect_err("oauth list without session should fail")
        .contains("no backend session token"));
    assert!(oauth::oauth_fetch_integration_tokens(
        &config,
        "0123456789abcdef01234567",
        "0123456789abcdef0123456789abcdef",
    )
    .await
    .expect_err("oauth token fetch without session should fail")
    .contains("no backend session token"));
    assert!(
        oauth::oauth_fetch_client_key(&config, "0123456789abcdef01234567")
            .await
            .expect_err("client key fetch without session should fail")
            .contains("no backend session token")
    );
    assert!(
        oauth::oauth_revoke_integration(&config, "0123456789abcdef01234567")
            .await
            .expect_err("oauth revoke without session should fail")
            .contains("no backend session token")
    );
}

#[tokio::test]
async fn credentials_secret_helpers_round_trip_with_file_keyring_backend() {
    let _lock = env_lock_async().await;
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let tmp = tempdir().expect("tempdir");
    let config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };
    std::fs::create_dir_all(config.config_path.parent().expect("config parent"))
        .expect("create config parent");

    let encrypted = encrypt_secret(&config, "worker-a-sensitive-value")
        .await
        .expect("encrypt secret")
        .value;
    assert_ne!(encrypted, "worker-a-sensitive-value");
    assert!(
        encrypted.starts_with("enc"),
        "encrypted secret should carry an encrypted payload marker: {encrypted}"
    );

    let decrypted = decrypt_secret(&config, &encrypted)
        .await
        .expect("decrypt secret")
        .value;
    assert_eq!(decrypted, "worker-a-sensitive-value");
}

#[tokio::test]
async fn worker_a_controller_schemas_are_fully_exposed() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let schema = schema(&harness.rpc_base).await;

    for (namespace, expected) in [
        (
            "config",
            vec![
                "openhuman.config_agent_server_status",
                "openhuman.config_get",
                "openhuman.config_get_agent_paths",
                "openhuman.config_get_agent_settings",
                "openhuman.config_get_analytics_settings",
                "openhuman.config_get_autonomy_settings",
                "openhuman.config_get_client_config",
                "openhuman.config_get_composio_trigger_settings",
                "openhuman.config_get_dashboard_settings",
                "openhuman.config_get_data_paths",
                "openhuman.config_get_dictation_settings",
                "openhuman.config_get_onboarding_completed",
                "openhuman.config_get_privacy_mode",
                "openhuman.config_get_runtime_flags",
                "openhuman.config_get_sandbox_settings",
                "openhuman.config_get_search_settings",
                "openhuman.config_get_voice_server_settings",
                "openhuman.config_reset_local_data",
                "openhuman.config_resolve_api_url",
                "openhuman.config_set_browser_allow_all",
                "openhuman.config_set_onboarding_completed",
                "openhuman.config_set_privacy_mode",
                "openhuman.config_update_agent_paths",
                "openhuman.config_update_agent_settings",
                "openhuman.config_update_analytics_settings",
                "openhuman.config_update_autonomy_settings",
                "openhuman.config_update_browser_settings",
                "openhuman.config_update_composio_trigger_settings",
                "openhuman.config_update_computer_settings",
                "openhuman.config_update_dictation_settings",
                "openhuman.config_update_local_ai_settings",
                "openhuman.config_update_memory_settings",
                "openhuman.config_update_model_settings",
                "openhuman.config_update_runtime_settings",
                "openhuman.config_update_sandbox_settings",
                "openhuman.config_update_search_settings",
                "openhuman.config_update_voice_server_settings",
                "openhuman.config_workspace_onboarding_flag_exists",
                "openhuman.config_workspace_onboarding_flag_set",
            ],
        ),
        (
            "auth",
            vec![
                "openhuman.auth_clear_credential",
                "openhuman.auth_create_channel_link_token",
                "openhuman.auth_get_session_token",
                "openhuman.auth_get_state",
                "openhuman.auth_list_provider_credentials",
                "openhuman.auth_oauth_connect",
                "openhuman.auth_oauth_fetch_client_key",
                "openhuman.auth_oauth_fetch_integration_tokens",
                "openhuman.auth_oauth_list_integrations",
                "openhuman.auth_oauth_revoke_integration",
                "openhuman.auth_remove_provider_credentials",
                "openhuman.auth_set_credential",
                "openhuman.auth_store_provider_credentials",
            ],
        ),
        (
            "app_state",
            vec![
                "openhuman.app_state_snapshot",
                "openhuman.app_state_update_local_state",
            ],
        ),
        ("connectivity", vec!["openhuman.connectivity_diag"]),
        (
            "memory",
            vec![
                "openhuman.memory_context_get",
                "openhuman.memory_context_refresh",
                "openhuman.memory_context_set",
                "openhuman.memory_conversations_get",
                "openhuman.memory_conversations_set",
                "openhuman.memory_engine_get",
                "openhuman.memory_engine_set",
                "openhuman.memory_engines_list",
                "openhuman.memory_fetch",
                "openhuman.memory_forget",
                "openhuman.memory_import_scan",
                "openhuman.memory_import_start",
                "openhuman.memory_import_status",
                "openhuman.memory_items_list",
                "openhuman.memory_learn",
                "openhuman.memory_recall",
                "openhuman.memory_sources_add",
                "openhuman.memory_sources_list",
                "openhuman.memory_sources_remove",
                "openhuman.memory_sources_sync",
            ],
        ),
    ] {
        assert_eq!(
            schema_method_names(&schema, namespace),
            expected,
            "schema catalog mismatch for namespace {namespace}"
        );
    }

    // Inference/agent/tools/approval methods must also be advertised.
    let advertised: Vec<String> = schema
        .get("methods")
        .and_then(Value::as_array)
        .expect("schema methods array")
        .iter()
        .filter_map(|method| method.get("method").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    for expected in [
        "openhuman.inference_status",
        "openhuman.inference_get_client_config",
        "openhuman.inference_update_model_settings",
        "openhuman.inference_update_local_settings",
        "openhuman.inference_list_models",
        "openhuman.inference_diagnostics",
        "openhuman.inference_openai_oauth_start",
        "openhuman.inference_openai_oauth_complete",
        "openhuman.inference_openai_oauth_status",
        "openhuman.inference_openai_oauth_disconnect",
        "openhuman.inference_summarize",
        "openhuman.inference_prompt",
        "openhuman.inference_vision_prompt",
        "openhuman.inference_test_provider_model",
        "openhuman.inference_analyze_sentiment",
        "openhuman.agent_chat",
        "openhuman.agent_chat_simple",
        "openhuman.agent_server_status",
        "openhuman.agent_list_definitions",
        "openhuman.agent_get_definition",
        "openhuman.agent_reload_definitions",
        "openhuman.agent_triage_evaluate",
        "openhuman.tools_composio_execute",
        "openhuman.tools_web_search",
        "openhuman.tools_web_answer",
        "openhuman.tools_web_contents",
        "openhuman.tools_searxng_search",
        "openhuman.tools_apify_linkedin_scrape",
        "openhuman.tool_registry_list",
        "openhuman.tool_registry_get",
        "openhuman.tool_registry_diagnostics",
        "openhuman.approval_list_pending",
        "openhuman.approval_list_recent_decisions",
        "openhuman.approval_decide",
    ] {
        assert!(
            advertised.iter().any(|method| method == expected),
            "schema catalog must expose {expected}"
        );
    }

    let unknown_app_state = app_state_schemas("missing");
    assert_eq!(unknown_app_state.namespace, "app_state");
    assert_eq!(unknown_app_state.function, "unknown");
    assert_eq!(unknown_app_state.outputs[0].name, "error");
    assert!(unknown_app_state.description.contains("Unknown app_state"));

    harness.join.abort();
}

#[tokio::test]
async fn config_controller_mutations_round_trip_over_json_rpc() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let initial = rpc(&harness.rpc_base, 10_001, "openhuman.config_get", json!({})).await;
    assert!(
        payload(&initial, "config_get")
            .get("workspace_dir")
            .and_then(Value::as_str)
            .is_some(),
        "config_get should expose resolved paths: {initial}"
    );

    let model = rpc(
        &harness.rpc_base,
        10_002,
        "openhuman.config_update_model_settings",
        json!({
            "api_url": "http://127.0.0.1:9",
            "inference_url": "http://127.0.0.1:19999/v1",
            "api_key": "worker-a-secret",
            "default_model": "worker-a-updated",
            "default_temperature": 0.4,
            "model_routes": [{ "hint": "reasoning", "model": "route-model" }],
            "cloud_providers": [{
                "id": "provider-a",
                "slug": "worker-a-cloud",
                "label": "Worker A Cloud",
                "endpoint": "http://127.0.0.1:19999/v1",
                "auth_style": "bearer"
            }],
            "primary_cloud": "provider-a",
            "chat_provider": "worker-a-cloud:chat",
            "reasoning_provider": "worker-a-cloud:reason",
            "agentic_provider": "worker-a-cloud:agent",
            "coding_provider": "worker-a-cloud:code",
            "memory_provider": "worker-a-cloud:memory",
            "embeddings_provider": "worker-a-cloud:embeddings"
        }),
    )
    .await;
    ok(&model, "update_model_settings");

    let client = rpc(
        &harness.rpc_base,
        10_003,
        "openhuman.config_get_client_config",
        json!({}),
    )
    .await;
    let client_payload = payload(&client, "get_client_config");
    assert_eq!(
        client_payload.get("default_model").and_then(Value::as_str),
        Some("worker-a-updated")
    );
    assert_eq!(
        client_payload.get("api_key_set").and_then(Value::as_bool),
        Some(true),
        "client config should expose only API key presence: {client_payload}"
    );
    assert!(
        !client_payload.to_string().contains("worker-a-secret"),
        "client config must not echo local API keys: {client_payload}"
    );

    // The model registry is seeded + price/context-window-enriched from the
    // static pricing catalog on load (in-memory), so a fresh workspace surfaces
    // real numbers over RPC. Validates the full path: catalog → seed-on-load →
    // client_config_json → JSON-RPC. (The earlier update_model_settings call did
    // not include `model_registry`, so the seeded registry is left intact.)
    let registry = client_payload
        .get("model_registry")
        .and_then(Value::as_array)
        .expect("client config should expose model_registry");
    assert!(
        !registry.is_empty(),
        "model_registry should be auto-seeded from the pricing catalog: {client_payload}"
    );
    let opus = registry
        .iter()
        .find(|m| m.get("id").and_then(Value::as_str) == Some("claude-opus-4-8"))
        .unwrap_or_else(|| panic!("seeded registry should contain claude-opus-4-8: {registry:?}"));
    assert_eq!(
        opus.get("provider").and_then(Value::as_str),
        Some("anthropic")
    );
    assert_eq!(
        opus.get("cost_per_1m_input").and_then(Value::as_f64),
        Some(5.0),
        "input price should be pre-filled from the catalog: {opus:?}"
    );
    assert_eq!(
        opus.get("cost_per_1m_output").and_then(Value::as_f64),
        Some(25.0),
        "output price should be pre-filled from the catalog: {opus:?}"
    );
    assert_eq!(
        opus.get("context_window").and_then(Value::as_u64),
        Some(1_000_000),
        "context window should be pre-filled from the catalog: {opus:?}"
    );
    // Every seeded entry carries non-zero pricing + context window — the whole
    // point of the catalog pre-fill.
    for entry in registry {
        let id = entry.get("id").and_then(Value::as_str).unwrap_or("<none>");
        assert!(
            entry
                .get("cost_per_1m_output")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                > 0.0,
            "{id} missing output price: {entry:?}"
        );
        assert!(
            entry
                .get("context_window")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                > 0,
            "{id} missing context window: {entry:?}"
        );
    }

    let memory = rpc(
        &harness.rpc_base,
        10_004,
        "openhuman.config_update_memory_settings",
        json!({
            "embedding_provider": "none",
            "embedding_model": "none",
            "embedding_dimensions": 0
        }),
    )
    .await;
    ok(&memory, "update_memory_settings");

    for (id, method, params) in [
        (
            10_005,
            "openhuman.config_update_runtime_settings",
            json!({ "kind": "local", "reasoning_enabled": true }),
        ),
        (
            10_006,
            "openhuman.config_update_browser_settings",
            json!({ "enabled": true }),
        ),
        (
            10_008,
            "openhuman.config_update_local_ai_settings",
            json!({
                "runtime_enabled": false,
                "opt_in_confirmed": false,
                "provider": "ollama",
                "base_url": "http://127.0.0.1:11434",
                "model_id": "llama3",
                "chat_model_id": "llama3",
                "usage_embeddings": false
            }),
        ),
        (
            10_009,
            "openhuman.config_update_voice_server_settings",
            json!({
                "auto_start": false,
                "hotkey": "Fn",
                "activation_mode": "push",
                "skip_cleanup": true,
                "min_duration_secs": 0.25,
                "silence_threshold": 0.01,
                "custom_dictionary": ["OpenHuman", "WorkerA"]
            }),
        ),
        (
            10_010,
            "openhuman.config_update_composio_trigger_settings",
            json!({
                "triage_disabled": true,
                "triage_disabled_toolkits": ["gmail", "slack"]
            }),
        ),
        (
            10_011,
            "openhuman.config_update_autonomy_settings",
            json!({
                "level": "supervised",
                "workspace_only": true,
                "allowed_commands": ["git", "cargo"],
                "forbidden_paths": ["/tmp/forbidden-worker-a"],
                "trusted_roots": [{
                    "path": harness.home.display().to_string(),
                    "access": "read"
                }],
                "allow_tool_install": false,
                "max_actions_per_hour": 42,
                "auto_approve": ["memory.search"]
            }),
        ),
        (
            10_012,
            "openhuman.config_update_search_settings",
            json!({
                "engine": "managed",
                "max_results": 5,
                "timeout_secs": 12,
                "providers": {
                    "brave": {"enabled": true, "api_key": "brave-secret"},
                    "querit": {"enabled": true, "api_key": "querit-secret"}
                },
                "allowed_domains": ["example.com"],
                "allow_all": false
            }),
        ),
    ] {
        let response = rpc(&harness.rpc_base, id, method, params).await;
        ok(&response, method);
    }

    for (id, method) in [
        (10_101, "openhuman.config_resolve_api_url"),
        (10_102, "openhuman.config_get_runtime_flags"),
        (10_103, "openhuman.config_get_dashboard_settings"),
        (10_104, "openhuman.config_agent_server_status"),
        (10_105, "openhuman.config_get_data_paths"),
        (10_106, "openhuman.config_get_voice_server_settings"),
        (10_107, "openhuman.config_get_composio_trigger_settings"),
        (10_108, "openhuman.config_get_autonomy_settings"),
        (10_109, "openhuman.config_get_search_settings"),
    ] {
        let response = rpc(&harness.rpc_base, id, method, json!({})).await;
        ok(&response, method);
    }

    let allow_all = rpc(
        &harness.rpc_base,
        10_201,
        "openhuman.config_set_browser_allow_all",
        json!({ "enabled": false }),
    )
    .await;
    ok(&allow_all, "set_browser_allow_all false");

    let exists_before = rpc(
        &harness.rpc_base,
        10_202,
        "openhuman.config_workspace_onboarding_flag_exists",
        json!({ "flag_name": ".worker-a-onboarding" }),
    )
    .await;
    assert_eq!(
        payload(&exists_before, "workspace_onboarding_flag_exists before").as_bool(),
        Some(false)
    );

    let set_flag = rpc(
        &harness.rpc_base,
        10_203,
        "openhuman.config_workspace_onboarding_flag_set",
        json!({ "flag_name": ".worker-a-onboarding", "value": true }),
    )
    .await;
    assert_eq!(
        payload(&set_flag, "workspace_onboarding_flag_set true").as_bool(),
        Some(true)
    );

    let clear_flag = rpc(
        &harness.rpc_base,
        10_204,
        "openhuman.config_workspace_onboarding_flag_set",
        json!({ "flag_name": ".worker-a-onboarding", "value": false }),
    )
    .await;
    assert_eq!(
        payload(&clear_flag, "workspace_onboarding_flag_set false").as_bool(),
        Some(false)
    );

    let reset = rpc(
        &harness.rpc_base,
        10_301,
        "openhuman.config_reset_local_data",
        json!({}),
    )
    .await;
    assert!(
        payload(&reset, "reset_local_data").is_object(),
        "reset should return a result payload: {reset}"
    );

    harness.join.abort();
}

#[tokio::test]
async fn config_runtime_flags_settings_readbacks_and_validation_paths_are_exercised() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let refused = rpc(
        &harness.rpc_base,
        11_001,
        "openhuman.config_set_browser_allow_all",
        json!({ "enabled": true }),
    )
    .await;
    assert_error_contains(
        &refused,
        "set_browser_allow_all true without operator opt-in",
        "Refusing to enable OPENHUMAN_BROWSER_ALLOW_ALL",
    );

    std::env::set_var("OPENHUMAN_BROWSER_ALLOW_ALL_RPC_ENABLE", "1");
    let enabled = rpc(
        &harness.rpc_base,
        11_002,
        "openhuman.config_set_browser_allow_all",
        json!({ "enabled": true }),
    )
    .await;
    assert_eq!(
        payload(&enabled, "set_browser_allow_all true")
            .get("browser_allow_all")
            .and_then(Value::as_bool),
        Some(true)
    );

    let disabled = rpc(
        &harness.rpc_base,
        11_003,
        "openhuman.config_set_browser_allow_all",
        json!({ "enabled": false }),
    )
    .await;
    assert_eq!(
        payload(&disabled, "set_browser_allow_all false")
            .get("browser_allow_all")
            .and_then(Value::as_bool),
        Some(false)
    );

    ok(
        &rpc(
            &harness.rpc_base,
            11_004,
            "openhuman.config_update_analytics_settings",
            json!({ "enabled": false }),
        )
        .await,
        "update_analytics_settings false",
    );
    let analytics = rpc(
        &harness.rpc_base,
        11_005,
        "openhuman.config_get_analytics_settings",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&analytics, "get_analytics_settings")
            .get("enabled")
            .and_then(Value::as_bool),
        Some(false)
    );

    let onboarding_before = rpc(
        &harness.rpc_base,
        11_008,
        "openhuman.config_get_onboarding_completed",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&onboarding_before, "get_onboarding_completed before").as_bool(),
        Some(false)
    );
    for (id, value) in [(11_009, true), (11_010, false)] {
        let updated = rpc(
            &harness.rpc_base,
            id,
            "openhuman.config_set_onboarding_completed",
            json!({ "value": value }),
        )
        .await;
        assert_eq!(
            payload(&updated, "set_onboarding_completed").as_bool(),
            Some(value)
        );
    }

    ok(
        &rpc(
            &harness.rpc_base,
            11_011,
            "openhuman.config_update_dictation_settings",
            json!({
                "enabled": true,
                "hotkey": "Ctrl+Space",
                "activation_mode": "toggle",
                "llm_refinement": false,
                "streaming": true,
                "streaming_interval_ms": 750
            }),
        )
        .await,
        "update_dictation_settings valid",
    );
    let dictation = rpc(
        &harness.rpc_base,
        11_012,
        "openhuman.config_get_dictation_settings",
        json!({}),
    )
    .await;
    let dictation_payload = payload(&dictation, "get_dictation_settings");
    assert_eq!(
        dictation_payload
            .get("activation_mode")
            .and_then(Value::as_str),
        Some("toggle")
    );
    assert_eq!(
        dictation_payload
            .get("streaming_interval_ms")
            .and_then(Value::as_u64),
        Some(750)
    );

    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_013,
            "openhuman.config_update_dictation_settings",
            json!({ "activation_mode": "hold" }),
        )
        .await,
        "update_dictation_settings invalid activation",
        "invalid activation_mode",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_014,
            "openhuman.config_update_voice_server_settings",
            json!({ "activation_mode": "hold" }),
        )
        .await,
        "update_voice_server_settings invalid activation",
        "invalid activation_mode",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_015,
            "openhuman.config_update_search_settings",
            json!({ "engine": "bing" }),
        )
        .await,
        "update_search_settings invalid engine",
        "unknown search engine",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_016,
            "openhuman.config_update_search_settings",
            json!({ "max_results": 0 }),
        )
        .await,
        "update_search_settings invalid max_results",
        "max_results must be between",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_024,
            "openhuman.config_update_search_settings",
            json!({ "timeout_secs": 0 }),
        )
        .await,
        "update_search_settings invalid timeout_secs",
        "timeout_secs must be between",
    );
    let valid_search = rpc(
        &harness.rpc_base,
        11_025,
        "openhuman.config_update_search_settings",
        json!({
            "engine": " brave ",
            "max_results": 12,
            "timeout_secs": 42,
            "providers": {
                "brave": {"api_key": " brave-rpc-key "},
                "querit": {"enabled": true, "api_key": " querit-rpc-key "},
                "exa": {"api_key": " exa-rpc-key "},
                "tavily": {"enabled": true, "api_key": " tavily-rpc-key "}
            },
            "allowed_domains": [" example.com ", "", "example.com", "docs.example.com"],
            "allow_all": false
        }),
    )
    .await;
    let valid_search_payload = payload(&valid_search, "update_search_settings valid");
    assert_eq!(valid_search_payload.get("max_results"), Some(&json!(12)));
    assert_eq!(valid_search_payload.get("timeout_secs"), Some(&json!(42)));
    let search_readback = rpc(
        &harness.rpc_base,
        11_026,
        "openhuman.config_get_search_settings",
        json!({}),
    )
    .await;
    let search_payload = payload(&search_readback, "get_search_settings after valid update");
    let provider = |view: &Value, id: &str| -> Value {
        view.get("providers")
            .and_then(Value::as_array)
            .and_then(|providers| providers.iter().find(|p| p["id"] == id).cloned())
            .unwrap_or_else(|| panic!("provider {id} listed: {view}"))
    };
    for id in ["brave", "querit", "exa", "tavily"] {
        assert_eq!(
            provider(search_payload, id)["key_configured"],
            json!(true),
            "{id} key should be stored"
        );
    }
    assert_eq!(
        search_payload.pointer("/effective_roles/search/0"),
        Some(&json!("brave")),
        "the legacy engine selection leads the search role: {search_payload}"
    );
    assert!(!search_payload.to_string().contains("rpc-key"));
    assert_eq!(
        search_payload.get("allow_all").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        search_payload.get("allowed_domains"),
        Some(&json!(["docs.example.com", "example.com"]))
    );
    let select_tavily = rpc(
        &harness.rpc_base,
        11_126,
        "openhuman.config_update_search_settings",
        json!({ "roles": {"search": ["tavily", "brave"]} }),
    )
    .await;
    let select_tavily_payload = payload(&select_tavily, "put Tavily first for search");
    assert_eq!(
        select_tavily_payload.pointer("/effective_roles/search/0"),
        Some(&json!("tavily"))
    );
    assert_eq!(
        select_tavily_payload.pointer("/effective_roles/contents/0"),
        Some(&json!("tavily")),
        "Tavily also serves contents without a session: {select_tavily_payload}"
    );
    let allow_all_search = rpc(
        &harness.rpc_base,
        11_027,
        "openhuman.config_update_search_settings",
        json!({
            "providers": {
                "brave": {"api_key": " "},
                "querit": {"api_key": " "},
                "exa": {"api_key": " "},
                "tavily": {"api_key": " "}
            },
            "allow_all": true
        }),
    )
    .await;
    let allow_all_payload = payload(&allow_all_search, "update_search_settings allow_all");
    assert_eq!(
        allow_all_payload.get("allowed_domains"),
        Some(&json!(["*"]))
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_017,
            "openhuman.config_update_autonomy_settings",
            json!({ "level": "reckless" }),
        )
        .await,
        "update_autonomy_settings invalid level",
        "invalid autonomy level",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_018,
            "openhuman.config_update_model_settings",
            json!({
                "cloud_providers": [{
                    "slug": "",
                    "endpoint": "http://127.0.0.1:19999/v1",
                    "auth_style": "bearer"
                }]
            }),
        )
        .await,
        "update_model_settings empty cloud provider slug",
        "cloud provider slug must not be empty",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_022,
            "openhuman.config_update_model_settings",
            json!({
                "cloud_providers": [{
                    "slug": "worker-a-invalid-auth-style",
                    "label": "Invalid Auth Style",
                    "endpoint": "http://127.0.0.1:19999/v1",
                    "auth_style": "magic"
                }]
            }),
        )
        .await,
        "update_model_settings invalid cloud provider auth_style",
        "unknown auth_style",
    );
    let filtered_reserved = rpc(
        &harness.rpc_base,
        11_023,
        "openhuman.config_update_model_settings",
        json!({
            "cloud_providers": [
                {
                    "slug": "openhuman",
                    "label": "Reserved OpenHuman",
                    "endpoint": "https://api.openhuman.ai/v1",
                    "auth_style": "openhuman_jwt"
                },
                {
                    "slug": "worker-a-valid-cloud",
                    "label": "Worker A Valid Cloud",
                    "endpoint": "http://127.0.0.1:19999/v1",
                    "auth_style": "none"
                }
            ]
        }),
    )
    .await;
    let filtered_payload = payload(
        &filtered_reserved,
        "update_model_settings filters reserved cloud provider",
    );
    let cloud_providers = filtered_payload
        .pointer("/config/cloud_providers")
        .and_then(Value::as_array)
        .expect("config snapshot should expose cloud_providers");
    assert!(
        cloud_providers
            .iter()
            .any(|provider| provider.get("slug").and_then(Value::as_str)
                == Some("worker-a-valid-cloud")),
        "valid cloud provider should survive reserved filtering: {filtered_payload}"
    );
    assert_eq!(
        cloud_providers
            .iter()
            .filter(|provider| provider.get("slug").and_then(Value::as_str) == Some("openhuman"))
            .count(),
        1,
        "reserved cloud providers already in config should be preserved once, not duplicated by echoed client payloads: {filtered_payload}"
    );

    ok(
        &rpc(
            &harness.rpc_base,
            11_019,
            "openhuman.config_update_voice_server_settings",
            json!({
                "min_duration_secs": -1.0,
                "silence_threshold": -0.5
            }),
        )
        .await,
        "update_voice_server_settings clamps non-negative floats",
    );
    let config = rpc(&harness.rpc_base, 11_020, "openhuman.config_get", json!({})).await;
    let config_payload = payload(&config, "config_get after clamps");
    assert_eq!(
        config_payload.pointer("/config/voice_server/min_duration_secs"),
        Some(&json!(0.0))
    );
    assert_eq!(
        config_payload.pointer("/config/voice_server/silence_threshold"),
        Some(&json!(0.0))
    );

    harness.join.abort();
}

#[tokio::test]
async fn config_auto_approve_public_helper_persists_once_and_is_idempotent() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let _guards = [EnvVarGuard::set_to_path("HOME", &home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset(APP_ENV_VAR),
        EnvVarGuard::unset(VITE_APP_ENV_VAR)];

    openhuman_core::config::add_auto_approve_tool("tool.config.round10")
        .await
        .expect("add auto approve tool");
    openhuman_core::config::add_auto_approve_tool("tool.config.round10")
        .await
        .expect("idempotent auto approve tool");

    let loaded = Config::load_or_init()
        .await
        .expect("load config after auto approve helper");
    assert_eq!(
        loaded
            .autonomy
            .auto_approve
            .iter()
            .filter(|tool| tool.as_str() == "tool.config.round10")
            .count(),
        1
    );
}

#[tokio::test]
async fn auth_credentials_controller_paths_round_trip_and_validate_errors() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let state = rpc(
        &harness.rpc_base,
        20_001,
        "openhuman.auth_get_state",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&state, "auth_get_state")
            .get("isAuthenticated")
            .and_then(Value::as_bool),
        Some(false)
    );

    let token = rpc(
        &harness.rpc_base,
        20_002,
        "openhuman.auth_get_session_token",
        json!({}),
    )
    .await;
    assert!(
        payload(&token, "auth_get_session_token")
            .get("token")
            .is_some(),
        "session token read should return a token field even when empty: {token}"
    );

    // The core no longer talks to the backend's auth endpoints: a session JWT
    // arrives already obtained, and one that is not tied to a user is refused.
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_003,
            "openhuman.auth_set_credential",
            json!({ "token": "opaque-jwt" }),
        )
        .await,
        "auth_set_credential without a user id",
        "userId required",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_004,
            "openhuman.auth_set_credential",
            json!({ "token": "sk-1", "kind": "jwt" }),
        )
        .await,
        "auth_set_credential unknown kind",
        "unknown credential kind",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_005,
            "openhuman.auth_create_channel_link_token",
            json!({ "channel": "mastodon" }),
        )
        .await,
        "auth_create_channel_link_token unsupported",
        "unsupported channel",
    );

    // `auth_oauth_*` are served by
    // `openhuman-tinyhumans` now: validation first, then the core's
    // credential resolution, whose missing-session wording they report.
    for (id, method, params, needle) in [
        (
            20_006,
            "openhuman.auth_oauth_connect",
            json!({ "provider": "github" }),
            "no backend session token",
        ),
        (
            20_007,
            "openhuman.auth_oauth_list_integrations",
            json!({}),
            "no backend session token",
        ),
        (
            20_008,
            "openhuman.auth_oauth_fetch_integration_tokens",
            json!({ "integrationId": "abc", "key": "secret" }),
            "integrationId must be a 24-char hex id",
        ),
        (
            20_009,
            "openhuman.auth_oauth_fetch_client_key",
            json!({ "integrationId": "abc" }),
            "integrationId must be a 24-char hex id",
        ),
        (
            20_010,
            "openhuman.auth_oauth_revoke_integration",
            json!({ "integrationId": "abc" }),
            "no backend session token",
        ),
    ] {
        let response = rpc(&harness.rpc_base, id, method, params).await;
        assert_error_contains(&response, method, needle);
    }

    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_011,
            "openhuman.auth_store_provider_credentials",
            json!({ "provider": "   " }),
        )
        .await,
        "auth_store_provider_credentials empty provider",
        "provider is required",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_012,
            "openhuman.auth_store_provider_credentials",
            json!({ "provider": "worker-a", "fields": "not-an-object" }),
        )
        .await,
        "auth_store_provider_credentials invalid fields",
        "fields must be a JSON object",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_019,
            "openhuman.auth_store_provider_credentials",
            json!({ "provider": "worker-a-empty" }),
        )
        .await,
        "auth_store_provider_credentials missing credential material",
        "provide at least one credential",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_020,
            "openhuman.auth_store_session",
            json!({ "token": "header.payload.local" }),
        )
        .await,
        "auth_store_session local token without user",
        "local session requires a user payload",
    );

    let stored_provider = rpc(
        &harness.rpc_base,
        20_013,
        "openhuman.auth_store_provider_credentials",
        json!({
            "provider": "worker-a",
            "profile": "secondary",
            "token": "provider-secret",
            "fields": { "region": "test" },
            "setActive": false
        }),
    )
    .await;
    assert_eq!(
        payload(&stored_provider, "auth_store_provider_credentials")
            .get("provider")
            .and_then(Value::as_str),
        Some("worker-a")
    );

    let listed = rpc(
        &harness.rpc_base,
        20_014,
        "openhuman.auth_list_provider_credentials",
        json!({ "provider": "worker-a" }),
    )
    .await;
    let listed_payload = payload(&listed, "auth_list_provider_credentials");
    assert!(
        listed_payload
            .as_array()
            .expect("credentials list")
            .iter()
            .any(|profile| profile.get("profileName").and_then(Value::as_str) == Some("secondary")),
        "stored provider profile should be listed: {listed_payload}"
    );

    let listed_without_filter = rpc(
        &harness.rpc_base,
        20_021,
        "openhuman.auth_list_provider_credentials",
        json!({}),
    )
    .await;
    assert!(
        payload(
            &listed_without_filter,
            "auth_list_provider_credentials default params"
        )
        .as_array()
        .expect("unfiltered credentials list")
        .iter()
        .any(|profile| profile.get("provider").and_then(Value::as_str) == Some("worker-a")),
        "unfiltered credentials list should include stored provider: {listed_without_filter}"
    );

    let removed = rpc(
        &harness.rpc_base,
        20_015,
        "openhuman.auth_remove_provider_credentials",
        json!({ "provider": "worker-a", "profile": "secondary" }),
    )
    .await;
    assert_eq!(
        payload(&removed, "auth_remove_provider_credentials")
            .get("removed")
            .and_then(Value::as_bool),
        Some(true)
    );

    let session = rpc(
        &harness.rpc_base,
        20_016,
        "openhuman.auth_store_session",
        json!({
            "token": "header.payload.local",
            "user": {
                "id": "ignored",
                "name": "Worker A",
                "email": "worker-a@example.test"
            }
        }),
    )
    .await;
    assert_eq!(
        payload(&session, "auth_store_session")
            .get("credential")
            .and_then(Value::as_str),
        Some("local")
    );

    let authed = rpc(
        &harness.rpc_base,
        20_017,
        "openhuman.auth_get_state",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&authed, "auth_get_state")
            .get("isAuthenticated")
            .and_then(Value::as_bool),
        Some(true)
    );

    let cleared = rpc(
        &harness.rpc_base,
        20_018,
        "openhuman.auth_clear_session",
        json!({}),
    )
    .await;
    assert!(
        payload(&cleared, "auth_clear_session")
            .get("removed")
            .and_then(Value::as_bool)
            .is_some(),
        "clear session should return removal status: {cleared}"
    );

    harness.join.abort();
}

#[tokio::test]
async fn auth_local_session_normalizes_user_and_app_state_snapshot_uses_stored_identity() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let session = rpc(
        &harness.rpc_base,
        21_001,
        "openhuman.auth_store_session",
        json!({
            "token": "header.payload.local",
            "user": {
                "id": "renderer-supplied-id",
                "name": "Local Worker",
                "email": "local-worker@example.test"
            }
        }),
    )
    .await;
    assert_eq!(
        payload(&session, "auth_store_session local")
            .get("credential")
            .and_then(Value::as_str),
        Some("local")
    );

    let state = rpc(
        &harness.rpc_base,
        21_002,
        "openhuman.auth_get_state",
        json!({}),
    )
    .await;
    let state_payload = payload(&state, "auth_get_state after local session");
    assert_eq!(
        state_payload
            .get("isAuthenticated")
            .and_then(Value::as_bool),
        Some(true)
    );
    let user_id = state_payload
        .get("userId")
        .and_then(Value::as_str)
        .expect("local session should set userId");
    assert!(
        user_id.starts_with("local-"),
        "local session user id should be host-scoped, got {user_id:?}"
    );
    assert_eq!(
        state_payload.pointer("/user/id").and_then(Value::as_str),
        Some(user_id)
    );
    assert_eq!(
        state_payload.pointer("/user/_id").and_then(Value::as_str),
        Some(user_id)
    );

    let token = rpc(
        &harness.rpc_base,
        21_003,
        "openhuman.auth_get_session_token",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&token, "auth_get_session_token after local session")
            .get("token")
            .and_then(Value::as_str),
        Some("header.payload.local")
    );

    let snapshot = rpc(
        &harness.rpc_base,
        21_004,
        "openhuman.app_state_snapshot",
        json!({}),
    )
    .await;
    let snapshot_payload = payload(&snapshot, "app_state_snapshot local session");
    assert_eq!(
        snapshot_payload.get("sessionToken").and_then(Value::as_str),
        Some("header.payload.local")
    );
    assert_eq!(
        snapshot_payload
            .pointer("/currentUser/email")
            .and_then(Value::as_str),
        Some("local-worker@example.test")
    );

    let cleared = rpc(
        &harness.rpc_base,
        21_005,
        "openhuman.auth_clear_session",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&cleared, "auth_clear_session after local session")
            .get("removed")
            .and_then(Value::as_bool),
        Some(true)
    );

    harness.join.abort();
}

#[tokio::test]
async fn auth_remote_backend_bearer_only_paths_round_trip_with_a_handed_over_session() {
    let _lock = env_lock_async().await;
    let (backend_base, backend_state, backend_join) = serve_mock_backend().await;
    let harness = setup().await;
    let _backend_guard = EnvVarGuard::set("BACKEND_URL", &backend_base);

    // The host hands over an already-obtained session with the user it
    // resolved; the core stores it without consulting the backend.
    let session = rpc(
        &harness.rpc_base,
        22_001,
        "openhuman.auth_set_credential",
        json!({
            "token": "remote-jwt",
            "userId": "remote-user-1",
            "user": {
                "id": "remote-user-1",
                "name": "Remote Worker",
                "email": "remote-worker@example.test"
            }
        }),
    )
    .await;
    let state = payload(&session, "auth_set_credential remote");
    assert_eq!(
        state.get("credential").and_then(Value::as_str),
        Some("session")
    );
    assert_eq!(
        state.get("userId").and_then(Value::as_str),
        Some("remote-user-1")
    );
    assert_eq!(
        backend_state.auth_me_hits.load(Ordering::SeqCst),
        0,
        "installing a credential must not touch GET /auth/me"
    );

    // Every remaining auth.* backend call is bearer-only and keeps working
    // against the stored session.
    let link = rpc(
        &harness.rpc_base,
        22_004,
        "openhuman.auth_create_channel_link_token",
        json!({ "channel": " Telegram " }),
    )
    .await;
    assert_eq!(
        payload(&link, "auth_create_channel_link_token remote")
            .get("linkToken")
            .and_then(Value::as_str),
        Some("link-token-123")
    );

    let integrations = rpc(
        &harness.rpc_base,
        22_005,
        "openhuman.auth_oauth_list_integrations",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&integrations, "auth_oauth_list_integrations remote")
            .pointer("/0/provider")
            .and_then(Value::as_str),
        Some("github")
    );

    let oauth_connect = rpc(
        &harness.rpc_base,
        22_011,
        "openhuman.auth_oauth_connect",
        json!({
            "provider": "github",
            "skillId": "worker-a-skill",
            "responseType": "code",
            "encryptionMode": "handoff"
        }),
    )
    .await;
    assert_eq!(
        payload(&oauth_connect, "auth_oauth_connect remote")
            .get("oauthUrl")
            .and_then(Value::as_str),
        Some("https://github.example.test/oauth?state=worker-a-state")
    );

    let integration_tokens = rpc(
        &harness.rpc_base,
        22_012,
        "openhuman.auth_oauth_fetch_integration_tokens",
        json!({
            "integrationId": "0123456789abcdef01234567",
            "key": "0123456789abcdef0123456789abcdef"
        }),
    )
    .await;
    assert_eq!(
        payload(
            &integration_tokens,
            "auth_oauth_fetch_integration_tokens remote"
        )
        .get("accessToken")
        .and_then(Value::as_str),
        Some("gh-access-token")
    );

    let client_key = rpc(
        &harness.rpc_base,
        22_006,
        "openhuman.auth_oauth_fetch_client_key",
        json!({ "integrationId": "0123456789abcdef01234567" }),
    )
    .await;
    assert_eq!(
        payload(&client_key, "auth_oauth_fetch_client_key remote")
            .get("clientKey")
            .and_then(Value::as_str),
        Some("client-key-share")
    );

    let revoked = rpc(
        &harness.rpc_base,
        22_007,
        "openhuman.auth_oauth_revoke_integration",
        json!({ "integrationId": "0123456789abcdef01234567" }),
    )
    .await;
    assert_eq!(
        payload(&revoked, "auth_oauth_revoke_integration remote")
            .get("revoked")
            .and_then(Value::as_bool),
        Some(true)
    );

    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            22_008,
            "openhuman.auth_oauth_fetch_integration_tokens",
            json!({ "integrationId": "short", "key": "secret" }),
        )
        .await,
        "auth_oauth_fetch_integration_tokens invalid id with session",
        "integrationId must be a 24-char hex id",
    );

    // The snapshot reports the handed-over user and never refreshes it.
    let snapshot = rpc(
        &harness.rpc_base,
        22_009,
        "openhuman.app_state_snapshot",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&snapshot, "app_state_snapshot remote")
            .pointer("/currentUser/email")
            .and_then(Value::as_str),
        Some("remote-worker@example.test")
    );
    assert_eq!(
        backend_state.auth_me_hits.load(Ordering::SeqCst),
        0,
        "the snapshot must not call GET /auth/me; the host owns that"
    );

    // Re-handing the same token for the same user is a cheap refresh that
    // replaces the stored payload.
    let refreshed = rpc(
        &harness.rpc_base,
        22_013,
        "openhuman.auth_set_credential",
        json!({
            "token": "remote-jwt",
            "userId": "remote-user-1",
            "user": { "id": "remote-user-1", "name": "Renamed Worker" }
        }),
    )
    .await;
    assert_eq!(
        payload(&refreshed, "auth_set_credential refresh")
            .pointer("/user/name")
            .and_then(Value::as_str),
        Some("Renamed Worker")
    );
    let identity = openhuman_core::security::credentials::identity::peek_credential_user_identity()
        .expect("set_credential should seed the identity slot");
    assert_eq!(identity.id.as_deref(), Some("remote-user-1"));
    assert_eq!(identity.name.as_deref(), Some("Renamed Worker"));

    let cleared = rpc(
        &harness.rpc_base,
        22_014,
        "openhuman.auth_clear_credential",
        json!({ "kind": "session" }),
    )
    .await;
    assert_eq!(
        payload(&cleared, "auth_clear_credential")
            .get("removedSession")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(
        openhuman_core::security::credentials::identity::peek_credential_user_identity().is_none()
    );

    harness.join.abort();
    backend_join.abort();
}

#[tokio::test]
async fn app_state_update_persists_and_snapshot_reads_local_state() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let updated = rpc(
        &harness.rpc_base,
        30_001,
        "openhuman.app_state_update_local_state",
        json!({
            "encryptionKey": "worker-a-key",
            "onboardingTasks": {
                "accessibilityPermissionGranted": true,
                "localModelConsentGiven": true,
                "localModelDownloadStarted": false,
                "enabledTools": ["memory.search", "tools.web_search"],
                "connectedSources": ["gmail"],
                "updatedAtMs": 123456
            }
        }),
    )
    .await;
    assert_eq!(
        payload(&updated, "app_state_update_local_state")
            .get("encryptionKey")
            .and_then(Value::as_str),
        Some("worker-a-key")
    );

    let snapshot = rpc(
        &harness.rpc_base,
        30_002,
        "openhuman.app_state_snapshot",
        json!({}),
    )
    .await;
    let local_state = payload(&snapshot, "app_state_snapshot")
        .get("localState")
        .unwrap_or_else(|| panic!("snapshot should include localState: {snapshot}"));
    assert_eq!(
        local_state.get("encryptionKey").and_then(Value::as_str),
        Some("worker-a-key")
    );
    assert_eq!(
        local_state.pointer("/onboardingTasks/enabledTools/0"),
        Some(&json!("memory.search"))
    );

    let cleared = rpc(
        &harness.rpc_base,
        30_003,
        "openhuman.app_state_update_local_state",
        json!({
            "encryptionKey": null,
            "onboardingTasks": null
        }),
    )
    .await;
    assert!(
        payload(&cleared, "app_state_update_local_state")
            .get("encryptionKey")
            .is_none(),
        "null patch should clear optional encryption key: {cleared}"
    );

    let blank_cleared = rpc(
        &harness.rpc_base,
        30_004,
        "openhuman.app_state_update_local_state",
        json!({ "encryptionKey": "   " }),
    )
    .await;
    assert!(
        payload(&blank_cleared, "app_state_update_local_state blank key")
            .get("encryptionKey")
            .is_none(),
        "blank encryption key should also clear the optional value: {blank_cleared}"
    );

    let invalid_patch = rpc(
        &harness.rpc_base,
        30_005,
        "openhuman.app_state_update_local_state",
        json!({ "onboardingTasks": "not-an-object" }),
    )
    .await;
    assert_error_contains(
        &invalid_patch,
        "app_state_update_local_state invalid onboardingTasks",
        "invalid params",
    );

    let unchanged = rpc(
        &harness.rpc_base,
        30_006,
        "openhuman.app_state_update_local_state",
        json!({}),
    )
    .await;
    assert!(
        payload(&unchanged, "app_state_update_local_state empty patch")
            .get("encryptionKey")
            .is_none(),
        "empty patch should preserve the already-cleared encryption key: {unchanged}"
    );

    harness.join.abort();
}

#[tokio::test]
async fn app_state_snapshot_degrades_runtime_service_status_failures() {
    let _lock = env_lock_async().await;
    let harness = setup().await;
    let service_state_path = harness.home.join("service-status-failure.json");
    std::fs::write(
        &service_state_path,
        serde_json::to_vec_pretty(&json!({
            "installed": true,
            "running": true,
            "agent_running": true,
            "failures": {
                "status": "forced status failure from app_state test"
            }
        }))
        .expect("serialize service mock state"),
    )
    .expect("write service mock state");
    let _service_mock = EnvVarGuard::set("OPENHUMAN_SERVICE_MOCK", "1");
    let _service_state =
        EnvVarGuard::set_to_path("OPENHUMAN_SERVICE_MOCK_STATE_FILE", &service_state_path);

    // The runtime snapshot cache is keyed by config identity (workspace_dir), so
    // this harness's unique workspace guarantees a cache miss regardless of prior
    // app_state_snapshot tests — the call exercises the service-status fallback
    // against our injected mock rather than returning a foreign cached runtime.
    let snapshot = rpc(
        &harness.rpc_base,
        30_051,
        "openhuman.app_state_snapshot",
        json!({}),
    )
    .await;
    let service = payload(&snapshot, "app_state_snapshot with service status failure")
        .pointer("/runtime/service")
        .expect("snapshot should include runtime service status");
    assert!(
        service
            .pointer("/state/Unknown")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("forced status failure")),
        "service status failures should degrade to Unknown state: {service}"
    );
    assert_eq!(
        service.get("label").and_then(Value::as_str),
        Some("OpenHuman")
    );

    harness.join.abort();
}

#[tokio::test]
async fn app_state_snapshot_and_update_surface_state_dir_creation_errors() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let config = rpc(&harness.rpc_base, 30_101, "openhuman.config_get", json!({})).await;
    let workspace_dir = payload(&config, "config_get for app_state state-dir error")
        .get("workspace_dir")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .expect("config_get should expose workspace_dir");
    let state_path = workspace_dir.join("state");
    std::fs::create_dir_all(&workspace_dir).expect("create workspace dir");
    std::fs::write(&state_path, "not a directory").expect("write state path as file");

    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            30_102,
            "openhuman.app_state_snapshot",
            json!({}),
        )
        .await,
        "app_state_snapshot with file at state path",
        "failed to create workspace state dir",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            30_103,
            "openhuman.app_state_update_local_state",
            json!({ "encryptionKey": "cannot-save" }),
        )
        .await,
        "app_state_update_local_state with file at state path",
        "failed to create workspace state dir",
    );

    harness.join.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn app_state_snapshot_keeps_unquarantinable_local_state_path_but_uses_defaults() {
    use std::os::unix::fs::PermissionsExt;

    let _lock = env_lock_async().await;
    let harness = setup().await;

    let config = rpc(&harness.rpc_base, 31_151, "openhuman.config_get", json!({})).await;
    let workspace_dir = payload(&config, "config_get for unquarantinable app_state path")
        .get("workspace_dir")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .expect("config_get should expose workspace_dir");
    let state_dir = workspace_dir.join("state");
    std::fs::create_dir_all(&state_dir).expect("create state dir");
    let app_state_path = state_dir.join("app-state.json");
    std::fs::create_dir(&app_state_path).expect("create app-state directory");

    let original_permissions = std::fs::metadata(&state_dir)
        .expect("state dir metadata")
        .permissions();
    let mut read_only_permissions = original_permissions.clone();
    read_only_permissions.set_mode(0o500);
    std::fs::set_permissions(&state_dir, read_only_permissions).expect("make state dir unwritable");

    // Unix permission bits only block writes for non-root users. CI containers
    // frequently run as root, where 0o500 does NOT prevent rename/removal — the
    // quarantine would succeed and this test's precondition (an *unquarantinable*
    // path) can't be established. Probe whether the mode is actually enforced;
    // if writes still succeed, skip rather than assert a guarantee the OS isn't
    // providing.
    let probe = state_dir.join(".perm-probe");
    if std::fs::write(&probe, b"x").is_ok() {
        let _ = std::fs::remove_file(&probe);
        std::fs::set_permissions(&state_dir, original_permissions)
            .expect("restore state dir permissions");
        eprintln!(
            "[skip] app_state_snapshot_keeps_unquarantinable_local_state_path_but_uses_defaults: \
             filesystem permissions not enforced (running as root?); cannot make state dir unwritable"
        );
        harness.join.abort();
        return;
    }

    let snapshot = rpc(
        &harness.rpc_base,
        31_152,
        "openhuman.app_state_snapshot",
        json!({}),
    )
    .await;

    std::fs::set_permissions(&state_dir, original_permissions)
        .expect("restore state dir permissions");

    let local_state = payload(
        &snapshot,
        "app_state_snapshot after failed quarantine of unreadable path",
    )
    .get("localState")
    .expect("snapshot should include localState");
    assert!(
        local_state.as_object().is_some_and(|map| map.is_empty()),
        "unquarantinable app state should still fall back to defaults: {local_state}"
    );
    assert!(
        app_state_path.exists(),
        "unwritable state dir should prevent quarantine rename/removal of the live path"
    );

    harness.join.abort();
}

#[tokio::test]
async fn app_state_snapshot_quarantines_unreadable_local_state_path() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let config = rpc(&harness.rpc_base, 31_101, "openhuman.config_get", json!({})).await;
    let workspace_dir = payload(&config, "config_get for unreadable app_state path")
        .get("workspace_dir")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .expect("config_get should expose workspace_dir");
    let state_dir = workspace_dir.join("state");
    std::fs::create_dir_all(&state_dir).expect("create state dir");
    let app_state_path = state_dir.join("app-state.json");
    std::fs::create_dir(&app_state_path).expect("create unreadable app-state directory");

    let snapshot = rpc(
        &harness.rpc_base,
        31_102,
        "openhuman.app_state_snapshot",
        json!({}),
    )
    .await;
    let local_state = payload(&snapshot, "app_state_snapshot after unreadable state path")
        .get("localState")
        .expect("snapshot should include localState");
    assert!(
        local_state.as_object().is_some_and(|map| map.is_empty()),
        "unreadable app state should fall back to defaults: {local_state}"
    );
    assert!(
        !app_state_path.exists(),
        "unreadable app state path should be moved out of the live path"
    );
    let quarantined = std::fs::read_dir(&state_dir)
        .expect("read state dir")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("app-state.json.corrupted.")
                && entry.path().is_dir()
        });
    assert!(
        quarantined,
        "unreadable app state directory should be quarantined under {state_dir:?}"
    );

    harness.join.abort();
}

#[test]
fn credentials_profile_store_public_api_persists_updates_and_recovers_bad_files() {
    let _lock = env_lock();
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let tmp = tempdir().expect("tempdir");
    let state_dir = tmp.path().join("profiles");
    let store = AuthProfilesStore::new(&state_dir, true);

    assert_eq!(store.path(), state_dir.join("auth-profiles.json"));
    let empty = store.load().expect("empty store load");
    assert!(empty.profiles.is_empty());

    assert!(TokenSet {
        access_token: "soon".to_string(),
        refresh_token: None,
        id_token: None,
        expires_at: Some(chrono::Utc::now() + chrono::Duration::seconds(1)),
        token_type: None,
        scope: None,
    }
    .is_expiring_within(Duration::from_secs(5)));
    assert!(!TokenSet {
        access_token: "no-expiry".to_string(),
        refresh_token: None,
        id_token: None,
        expires_at: None,
        token_type: None,
        scope: None,
    }
    .is_expiring_within(Duration::from_secs(5)));

    let mut token_profile = AuthProfile::new_token("openai", "work", "sk-worker-a".to_string());
    token_profile
        .metadata
        .insert("region".to_string(), "test".to_string());
    let token_id = token_profile.id.clone();
    store
        .upsert_profile(token_profile, true)
        .expect("upsert token profile");

    let mut oauth_profile = AuthProfile::new_oauth(
        "github",
        "default",
        TokenSet {
            access_token: "gh-access".to_string(),
            refresh_token: Some("gh-refresh".to_string()),
            id_token: Some("gh-id".to_string()),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            token_type: Some("Bearer".to_string()),
            scope: Some("repo user".to_string()),
        },
    );
    oauth_profile.account_id = Some("acct-1".to_string());
    oauth_profile.workspace_id = Some("workspace-1".to_string());
    let oauth_id = oauth_profile.id.clone();
    store
        .upsert_profile(oauth_profile, false)
        .expect("upsert oauth profile");

    let loaded = store.load().expect("load stored profiles");
    assert_eq!(
        loaded.active_profiles.get("openai").map(String::as_str),
        Some(token_id.as_str())
    );
    assert_eq!(
        loaded
            .profiles
            .get(&token_id)
            .and_then(|profile| profile.token.as_deref()),
        Some("sk-worker-a")
    );
    assert_eq!(
        loaded
            .profiles
            .get(&oauth_id)
            .and_then(|profile| profile.token_set.as_ref())
            .map(|tokens| tokens.access_token.as_str()),
        Some("gh-access")
    );

    store
        .set_active_profile("github", &oauth_id)
        .expect("set active oauth profile");
    store
        .clear_active_profile("openai")
        .expect("clear active token profile");
    let updated = store
        .update_profile(&oauth_id, |profile| {
            profile
                .metadata
                .insert("updated".to_string(), "true".to_string());
            Ok(())
        })
        .expect("update oauth metadata");
    assert_eq!(
        updated.metadata.get("updated").map(String::as_str),
        Some("true")
    );

    assert!(
        store
            .remove_profile(&token_id)
            .expect("remove existing token profile"),
        "existing token profile should be removed"
    );
    assert!(
        !store
            .remove_profile(&token_id)
            .expect("remove missing token profile"),
        "missing token profile removal should be idempotent"
    );

    let after_remove = store.load().expect("load after remove");
    assert!(!after_remove.profiles.contains_key(&token_id));
    assert_eq!(
        after_remove
            .active_profiles
            .get("github")
            .map(String::as_str),
        Some(oauth_id.as_str())
    );

    let corrupt_dir = tmp.path().join("corrupt");
    std::fs::create_dir_all(&corrupt_dir).expect("create corrupt profile dir");
    let corrupt_path = corrupt_dir.join("auth-profiles.json");
    std::fs::write(&corrupt_path, "{ not valid json").expect("write corrupt profile store");
    let corrupt_store = AuthProfilesStore::new(&corrupt_dir, true);
    let recovered = corrupt_store
        .load()
        .expect("corrupt profile store recovers");
    assert!(recovered.profiles.is_empty());
    assert!(
        !corrupt_path.exists(),
        "corrupted auth profile store should be moved away"
    );
    assert!(
        std::fs::read_dir(&corrupt_dir)
            .expect("read corrupt dir")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-")),
        "corrupt auth profile store should leave a quarantine file"
    );

    let legacy_dir = tmp.path().join("legacy");
    std::fs::create_dir_all(&legacy_dir).expect("create legacy profile dir");
    std::fs::write(
        legacy_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 0,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {},
            "profiles": {}
        })
        .to_string(),
    )
    .expect("write legacy profile store");
    let legacy = AuthProfilesStore::new(&legacy_dir, true)
        .load()
        .expect("legacy schema 0 should normalize");
    assert_eq!(legacy.schema_version, 1);

    let future_dir = tmp.path().join("future");
    std::fs::create_dir_all(&future_dir).expect("create future profile dir");
    std::fs::write(
        future_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 99,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {},
            "profiles": {}
        })
        .to_string(),
    )
    .expect("write future profile store");
    let future_err = AuthProfilesStore::new(&future_dir, true)
        .load()
        .expect_err("future schema should fail");
    assert!(
        future_err
            .to_string()
            .contains("Unsupported auth profile schema version"),
        "unexpected future schema error: {future_err:#}"
    );
}

#[test]
fn credentials_auth_service_selects_active_default_and_requested_profiles() {
    let _lock = env_lock();
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "disabled");
    let tmp = tempdir().expect("tempdir");
    let service = AuthService::new(&tmp.path().join("auth-service"), false);

    service
        .store_provider_token(
            "github",
            "default",
            "default-token",
            Default::default(),
            false,
        )
        .expect("store default github profile");
    assert_eq!(
        service
            .get_provider_bearer_token("github", None)
            .expect("get default provider token")
            .as_deref(),
        Some("default-token"),
        "without an active profile, provider token lookup should fall back to default"
    );

    let oauth_profile = AuthProfile::new_oauth(
        "github",
        "work",
        TokenSet {
            access_token: "oauth-access".to_string(),
            refresh_token: None,
            id_token: None,
            expires_at: None,
            token_type: Some("Bearer".to_string()),
            scope: Some("repo".to_string()),
        },
    );
    let oauth_id = oauth_profile.id.clone();
    service.load_profiles().expect("load before oauth upsert");
    AuthProfilesStore::new(&tmp.path().join("auth-service"), false)
        .upsert_profile(oauth_profile, false)
        .expect("upsert oauth work profile");
    assert_eq!(
        service
            .set_active_profile(" GITHUB ", "github:work")
            .expect("set active by full profile id"),
        oauth_id
    );
    assert_eq!(
        service
            .get_provider_bearer_token("github", None)
            .expect("get active oauth bearer")
            .as_deref(),
        Some("oauth-access"),
        "active OAuth profiles should expose their access token as bearer material"
    );
    assert_eq!(
        service
            .get_provider_bearer_token("github", Some("default"))
            .expect("explicit default token")
            .as_deref(),
        Some("default-token"),
        "explicit profile overrides should win over active profiles"
    );
    assert_eq!(
        service
            .get_provider_bearer_token("github", Some("missing"))
            .expect("missing explicit profile")
            .as_deref(),
        None
    );

    let slack_profile = AuthProfile::new_token("slack", "main", "slack-token".to_string());
    let slack_id = slack_profile.id.clone();
    AuthProfilesStore::new(&tmp.path().join("auth-service"), false)
        .upsert_profile(slack_profile, false)
        .expect("upsert slack profile");
    let mismatch = service
        .set_active_profile("github", &slack_id)
        .expect_err("provider/profile mismatch should fail");
    assert!(
        mismatch.to_string().contains("belongs to provider slack"),
        "unexpected provider mismatch error: {mismatch:#}"
    );

    assert!(service
        .remove_profile("github", "github:work")
        .expect("remove active oauth profile"));
}

#[test]
fn credentials_profile_store_recovers_dropped_entries_empty_files_and_datetime_errors() {
    let _lock = env_lock();
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let tmp = tempdir().expect("tempdir");

    let default_profiles =
        openhuman_core::security::credentials::profiles::AuthProfilesData::default();
    assert_eq!(default_profiles.schema_version, 1);
    assert!(default_profiles.profiles.is_empty());

    let empty_path_store = AuthProfilesStore::new(Path::new(""), false);
    assert_eq!(empty_path_store.path(), Path::new("auth-profiles.json"));

    let empty_dir = tmp.path().join("empty-file");
    std::fs::create_dir_all(&empty_dir).expect("create empty profile dir");
    std::fs::write(empty_dir.join("auth-profiles.json"), "").expect("write empty profile file");
    let empty = AuthProfilesStore::new(&empty_dir, false)
        .load()
        .expect("empty persisted profile file should load as default");
    assert!(empty.profiles.is_empty());
    assert!(empty.active_profiles.is_empty());

    let mixed_dir = tmp.path().join("mixed");
    std::fs::create_dir_all(&mixed_dir).expect("create mixed profile dir");
    std::fs::write(
        mixed_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 1,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {
                "gitlab": "gitlab:main",
                "legacy": "legacy:bad-kind"
            },
            "profiles": {
                "gitlab:main": {
                    "provider": "gitlab",
                    "profile_name": "main",
                    "kind": "token",
                    "token": "plain-gitlab-token",
                    "created_at": "not-a-date",
                    "updated_at": "also-not-a-date",
                    "metadata": {
                        "origin": "fixture"
                    }
                },
                "legacy:bad-kind": {
                    "provider": "legacy",
                    "profile_name": "bad-kind",
                    "kind": "api_key",
                    "token": "drop-me",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string(),
    )
    .expect("write mixed profile fixture");
    let mixed_store = AuthProfilesStore::new(&mixed_dir, false);
    let mixed = mixed_store
        .load()
        .expect("mixed profile store should drop only bad entries");
    assert_eq!(
        mixed
            .profiles
            .get("gitlab:main")
            .and_then(|profile| profile.token.as_deref()),
        Some("plain-gitlab-token")
    );
    assert!(!mixed.profiles.contains_key("legacy:bad-kind"));
    assert!(!mixed.active_profiles.contains_key("legacy"));
    assert_eq!(
        mixed.active_profiles.get("gitlab").map(String::as_str),
        Some("gitlab:main")
    );
    let rewritten: Value = serde_json::from_str(
        &std::fs::read_to_string(mixed_dir.join("auth-profiles.json"))
            .expect("read rewritten mixed profile store"),
    )
    .expect("rewritten mixed store should be json");
    assert!(
        rewritten.pointer("/profiles/legacy:bad-kind").is_none(),
        "dropped profile should be purged from persisted store: {rewritten}"
    );

    let invalid_datetime_dir = tmp.path().join("invalid-datetime");
    std::fs::create_dir_all(&invalid_datetime_dir).expect("create invalid datetime dir");
    std::fs::write(
        invalid_datetime_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 1,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {
                "github": "github:oauth"
            },
            "profiles": {
                "github:oauth": {
                    "provider": "github",
                    "profile_name": "oauth",
                    "kind": "oauth",
                    "access_token": "plain-access",
                    "expires_at": "not-rfc3339",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string(),
    )
    .expect("write invalid datetime profile fixture");
    let invalid_datetime_err = AuthProfilesStore::new(&invalid_datetime_dir, false)
        .load()
        .expect_err("invalid oauth expiry should fail profile load");
    assert!(
        invalid_datetime_err
            .to_string()
            .contains("Invalid RFC3339 timestamp"),
        "unexpected invalid datetime error: {invalid_datetime_err:#}"
    );

    let missing_oauth_secret_dir = tmp.path().join("missing-oauth-secret");
    std::fs::create_dir_all(&missing_oauth_secret_dir).expect("create missing oauth secret dir");
    std::fs::write(
        missing_oauth_secret_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 1,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {
                "github": "github:missing-access"
            },
            "profiles": {
                "github:missing-access": {
                    "provider": "github",
                    "profile_name": "missing-access",
                    "kind": "oauth",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string(),
    )
    .expect("write missing oauth secret fixture");
    let missing_secret = AuthProfilesStore::new(&missing_oauth_secret_dir, false)
        .load()
        .expect("oauth profile missing access token should be dropped");
    assert!(missing_secret.profiles.is_empty());
    assert!(missing_secret.active_profiles.is_empty());
    let rewritten_missing_secret: Value = serde_json::from_str(
        &std::fs::read_to_string(missing_oauth_secret_dir.join("auth-profiles.json"))
            .expect("read rewritten missing oauth secret profile store"),
    )
    .expect("rewritten missing oauth secret store should be json");
    assert!(
        rewritten_missing_secret
            .pointer("/profiles/github:missing-access")
            .is_none(),
        "missing oauth secret profile should be purged from persisted store: {rewritten_missing_secret}"
    );
    assert!(
        rewritten_missing_secret
            .pointer("/active_profiles/github")
            .is_none(),
        "active pointer to missing oauth secret profile should be purged: {rewritten_missing_secret}"
    );

    let public_api_dir = tmp.path().join("public-api-errors");
    let public_store = AuthProfilesStore::new(&public_api_dir, false);
    assert!(public_store
        .set_active_profile("github", "github:missing")
        .expect_err("missing active profile should fail")
        .to_string()
        .contains("Auth profile not found"));
    assert!(public_store
        .update_profile("github:missing", |_| Ok(()))
        .expect_err("missing update profile should fail")
        .to_string()
        .contains("Auth profile not found"));
}

#[test]
fn credentials_profile_store_round_trips_oauth_secret_fields_after_backend_selection() {
    let _lock = env_lock();
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "disabled");
    let tmp = tempdir().expect("tempdir");
    let state_dir = tmp.path().join("json-fallback");
    let store = AuthProfilesStore::new(&state_dir, false);
    let profile = AuthProfile::new_oauth(
        "github",
        "json-fallback",
        TokenSet {
            access_token: "json-access".to_string(),
            refresh_token: Some("json-refresh".to_string()),
            id_token: Some("json-id".to_string()),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(2)),
            token_type: Some("Bearer".to_string()),
            scope: Some("repo user".to_string()),
        },
    );
    let profile_id = profile.id.clone();

    store
        .upsert_profile(profile, true)
        .expect("upsert oauth profile after backend selection");
    let raw: Value = serde_json::from_str(
        &std::fs::read_to_string(state_dir.join("auth-profiles.json"))
            .expect("read persisted auth profiles"),
    )
    .expect("persisted auth profiles json");
    let persisted_access_token = raw.pointer(&format!("/profiles/{profile_id}/access_token"));
    assert!(
        persisted_access_token.is_some_and(|value| {
            value.is_null()
                || value
                    .as_str()
                    .is_some_and(|secret| secret.starts_with("enc2:"))
        }),
        "persisted profile should either keychain-strip or encrypt access token: {raw}"
    );

    let loaded = store.load().expect("reload json fallback profile");
    let tokens = loaded
        .profiles
        .get(&profile_id)
        .and_then(|profile| profile.token_set.as_ref())
        .expect("oauth token set should round-trip");
    assert_eq!(tokens.access_token, "json-access");
    assert_eq!(tokens.refresh_token.as_deref(), Some("json-refresh"));
    assert_eq!(tokens.id_token.as_deref(), Some("json-id"));

    let root_path_store = AuthProfilesStore::new(Path::new("/"), false);
    assert_eq!(
        root_path_store.path(),
        Path::new("/").join("auth-profiles.json")
    );
}

#[test]
fn credentials_profile_store_keychain_migration_and_fallback_paths_are_deterministic() {
    let _lock = env_lock();
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let tmp = tempdir().expect("tempdir");

    let hit_dir = tmp.path().join("keychain-hit");
    std::fs::create_dir_all(&hit_dir).expect("create keychain hit dir");
    let hit_profile_id = "github:main";
    openhuman_core::security::keyring::set(
        "keychain-hit",
        &format!("auth:{hit_profile_id}"),
        &json!({
            "access_token": "kc-access",
            "refresh_token": "kc-refresh",
            "id_token": "kc-id",
            "token": null
        })
        .to_string(),
    )
    .expect("seed file-backed keyring secret");
    std::fs::write(
        hit_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 1,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {
                "github": hit_profile_id
            },
            "profiles": {
                "github:main": {
                    "provider": "github",
                    "profile_name": "main",
                    "kind": "oauth",
                    "access_token": "legacy-access",
                    "refresh_token": "legacy-refresh",
                    "id_token": "legacy-id",
                    "expires_at": "2026-01-01T00:00:00Z",
                    "token_type": "Bearer",
                    "scope": "repo",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string(),
    )
    .expect("write keychain hit fixture");
    let hit_store = AuthProfilesStore::new(&hit_dir, true);
    let hit_loaded = hit_store.load().expect("keychain hit should load");
    let hit_tokens = hit_loaded
        .profiles
        .get(hit_profile_id)
        .and_then(|profile| profile.token_set.as_ref())
        .expect("keychain hit token set");
    assert_eq!(hit_tokens.access_token, "kc-access");
    assert_eq!(hit_tokens.refresh_token.as_deref(), Some("kc-refresh"));
    let hit_rewritten: Value = serde_json::from_str(
        &std::fs::read_to_string(hit_dir.join("auth-profiles.json"))
            .expect("read rewritten keychain hit profile store"),
    )
    .expect("rewritten keychain hit json");
    assert!(
        hit_rewritten
            .get("profiles")
            .and_then(Value::as_object)
            .and_then(|profiles| profiles.get(hit_profile_id))
            .and_then(|profile| profile.get("access_token"))
            .is_none_or(Value::is_null),
        "legacy JSON secret fields should be cleared after keychain hit: {hit_rewritten}"
    );

    let migrate_dir = tmp.path().join("keychain-migrate");
    std::fs::create_dir_all(&migrate_dir).expect("create keychain migrate dir");
    let migrate_profile_id = "gitlab:work";
    std::fs::write(
        migrate_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 1,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {
                "gitlab": migrate_profile_id
            },
            "profiles": {
                "gitlab:work": {
                    "provider": "gitlab",
                    "profile_name": "work",
                    "kind": "token",
                    "token": "plain-token-for-migration",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string(),
    )
    .expect("write keychain migration fixture");
    let migrate_store = AuthProfilesStore::new(&migrate_dir, true);
    let migrated = migrate_store
        .load()
        .expect("plaintext JSON token should migrate to keychain");
    assert_eq!(
        migrated
            .profiles
            .get(migrate_profile_id)
            .and_then(|profile| profile.token.as_deref()),
        Some("plain-token-for-migration")
    );
    let migrated_keychain = openhuman_core::security::keyring::get(
        "keychain-migrate",
        &format!("auth:{migrate_profile_id}"),
    )
    .expect("read migrated keychain token")
    .expect("migrated keychain token should exist");
    assert!(
        migrated_keychain.contains("plain-token-for-migration"),
        "migrated keychain payload should contain redacted fixture token"
    );

    let fallback_dir = tmp.path().join("keychain-fallback");
    std::fs::create_dir_all(&fallback_dir).expect("create keychain fallback dir");
    let fallback_profile_id = "slack:bot";
    openhuman_core::security::keyring::set(
        "keychain-fallback",
        &format!("auth:{fallback_profile_id}"),
        "not-json",
    )
    .expect("seed malformed keychain payload");
    std::fs::write(
        fallback_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 1,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {
                "slack": fallback_profile_id
            },
            "profiles": {
                "slack:bot": {
                    "provider": "slack",
                    "profile_name": "bot",
                    "kind": "token",
                    "token": "json-fallback-token",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string(),
    )
    .expect("write keychain fallback fixture");
    let fallback = AuthProfilesStore::new(&fallback_dir, true)
        .load()
        .expect("malformed keychain payload should fall back to JSON");
    assert_eq!(
        fallback
            .profiles
            .get(fallback_profile_id)
            .and_then(|profile| profile.token.as_deref()),
        Some("json-fallback-token")
    );

    assert!(
        AuthProfilesStore::new(&migrate_dir, true)
            .remove_profile(migrate_profile_id)
            .expect("remove migrated profile"),
        "migrated profile should be removable"
    );
    assert!(
        openhuman_core::security::keyring::get(
            "keychain-migrate",
            &format!("auth:{migrate_profile_id}"),
        )
        .expect("read deleted migrated keychain token")
        .is_none(),
        "removing a profile should delete its keychain payload"
    );
}
