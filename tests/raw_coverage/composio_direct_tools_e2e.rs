//! Round25 focused raw coverage for large Composio direct-mode misses.
//!
//! The direct-mode factory normally pins Composio's production HTTPS API.
//! This test uses debug-only loopback base overrides and temp config stores so
//! no real network, keychain, or backend session is required.

use crate::env_guard::EnvVarGuard;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};

use openhuman_core::config::{Config, PrivacyMode};
use openhuman_core::security::{live_policy, SecurityPolicy};
use openhuman_core::integrations::composio::ops::{
    cached_active_integrations, composio_authorize, composio_execute, composio_list_connections,
    composio_list_toolkits, composio_list_tools, fetch_connected_integrations_status,
};
use openhuman_core::integrations::composio::tools::ComposioListToolsTool;
use openhuman_core::integrations::composio::{
    invalidate_connected_integrations_cache, FetchConnectedIntegrationsStatus,
};
use tinytools::{Tool, ToolCallOptions};

static ENV_LOCK: &OnceLock<tokio::sync::Mutex<()>> = &crate::SHARED_ENV_LOCK;

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: Method,
    path: String,
    query: String,
    body: Value,
    api_key: Option<String>,
}

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

struct Harness {
    _tmp: TempDir,
    config: Config,
    _guards: Vec<EnvVarGuard>,
}

fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .blocking_lock()
}

async fn env_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

async fn setup_direct_config(base: &str) -> Harness {
    crate::tinyhumans_boot::boot();
    std::fs::create_dir_all("target").expect("target dir");
    let tmp = Builder::new()
        .prefix("tools-composio-large-round25-")
        .tempdir_in("target")
        .expect("round25 tempdir");
    let root = tmp.path().join("openhuman");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let guards = vec![
        EnvVarGuard::set_path("OPENHUMAN_WORKSPACE", &root),
        EnvVarGuard::set_path("HOME", tmp.path()),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set(
            "OPENHUMAN_COMPOSIO_DIRECT_BASE_V2",
            format!("{base}/api/v2"),
        ),
        EnvVarGuard::set(
            "OPENHUMAN_COMPOSIO_DIRECT_BASE_V3",
            format!("{base}/api/v3"),
        ),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::unset("OPENHUMAN_CORE_RPC_URL"),
        EnvVarGuard::unset("OPENHUMAN_CORE_PORT"),
    ];

    let mut config = Config {
        workspace_dir: workspace,
        config_path: root.join("config.toml"),
        ..Config::default()
    };
    config.composio.mode = "direct".to_string();
    config.composio.api_key = Some(" ck_round25_direct ".to_string());
    config.composio.entity_id = " entity-round25 ".to_string();
    config.secrets.encrypt = false;
    config.observability.analytics_enabled = false;
    config.save().await.expect("save config");
    crate::tinyhumans_boot::boot_default_context(config.clone()).await;

    Harness {
        _tmp: tmp,
        config,
        _guards: guards,
    }
}

#[tokio::test]
async fn round25_direct_mode_ops_use_loopback_factory_for_tools_connections_and_execute() {
    let _lock = env_lock_async().await;
    let _storage = crate::HostStorage::install(&_lock);
    invalidate_connected_integrations_cache();

    let state = MockState::default();
    let base = start_loopback(
        Router::new()
            .fallback(any(composio_direct_handler))
            .with_state(state.clone()),
    )
    .await;
    let harness = setup_direct_config(&base).await;

    let toolkits = composio_list_toolkits(&harness.config)
        .await
        .expect("direct list toolkits")
        .value;
    assert!(toolkits.toolkits.is_empty());

    let connections = composio_list_connections(
        &openhuman_core::core::runtime::DispatchCtx::ambient(),
        &harness.config,
    )
    .await
    .expect("direct list connections")
    .value;
    assert_eq!(connections.connections.len(), 3);
    assert!(connections
        .connections
        .iter()
        .any(|conn| conn.id == "acct-gmail" && conn.normalized_toolkit() == "gmail"));

    let listed = composio_list_tools(
        &openhuman_core::core::runtime::DispatchCtx::ambient(),
        &harness.config,
        None,
        Some(vec![" readOnlyHint ".into(), " ".into()]),
    )
    .await
    .expect("direct list tools")
    .value;
    assert!(listed
        .tools
        .iter()
        .any(|tool| tool.function.name == "GMAIL_FETCH_EMAILS"));

    let markdown = ComposioListToolsTool::new(Arc::new(harness.config.clone()))
        .execute_with_options(
            json!({ "toolkits": ["gmail"], "tags": ["readOnlyHint"], "include_unconnected": true }),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("direct list tools tool markdown");
    assert!(!markdown.is_error, "{}", markdown.output());
    assert_eq!(markdown.output(), "{\"tools\":[]}");

    let authorize = composio_authorize(
        &openhuman_core::core::runtime::DispatchCtx::host_default(),
        &harness.config,
        " gmail ",
        Some(json!({ "ignored_in_direct": true })),
    )
    .await
    .expect("direct authorize")
    .value;
    assert_eq!(authorize.connection_id, "");
    assert_eq!(
        authorize.connect_url,
        "https://connect.example.test/round25"
    );

    let executed = composio_execute(
        &harness.config,
        "GMAIL_FETCH_EMAILS",
        Some(json!({ "query": "label:INBOX" })),
        None,
    )
    .await
    .expect("direct execute")
    .value;
    assert!(executed.successful, "{executed:?}");
    assert_eq!(
        executed.data.pointer("/messages/0/id"),
        Some(&json!("msg-round25"))
    );
    assert_eq!(executed.cost_usd, 0.0);
    // TS-E2E-009 (RD-12): the provider echoes the caller's key in every form;
    // the RPC op's result carries none of them, as the agent tools' don't.
    let returned = serde_json::to_string(&executed).expect("serialize result");
    for form in round25_key_forms() {
        assert!(!returned.contains(&form), "the result carried the key as {form}");
    }

    let failed_execute = composio_execute(
        &harness.config,
        "GMAIL_SEND_EMAIL",
        Some(json!({ "to": "person@example.test" })),
        None,
    )
    .await
    .expect("direct execute provider failure")
    .value;
    assert!(!failed_execute.successful);
    assert!(failed_execute
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("provider rejected send"));

    match fetch_connected_integrations_status(&harness.config).await {
        FetchConnectedIntegrationsStatus::Authoritative(integrations) => {
            assert!(integrations
                .iter()
                .any(|item| item.toolkit == "gmail" && item.connected));
            assert!(integrations
                .iter()
                .any(|item| item.toolkit == "slack" && item.connected));
            assert!(!integrations
                .iter()
                .any(|item| item.toolkit == "github" && item.connected));
        }
        FetchConnectedIntegrationsStatus::Unavailable => {
            panic!("direct loopback integrations should be authoritative")
        }
    }
    assert!(cached_active_integrations(&harness.config).await.is_some());

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().all(|request| {
        request.api_key.as_deref() == Some("ck_round25_direct") || request.path == "/health"
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::GET
            && request.path.ends_with("/connected_accounts")
            && request.query.contains("limit=200")
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::GET
            && request.path.ends_with("/tools")
            && request.query.contains("tags=readOnlyHint")
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::POST
            && request.path.ends_with("/connected_accounts/link")
            && request.body["toolkit"] == "gmail"
            && request.body["entity_id"] == " entity-round25 "
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::POST
            && request.path.ends_with("/tools/execute/GMAIL_FETCH_EMAILS")
            && request.body["arguments"]["query"] == "label:INBOX"
            && request.body["entity_id"] == " entity-round25 "
    }));

    // An Execute whose result cannot be redacted is withheld, but still
    // recorded as executed: the action ran and may have cost something. A key
    // that is part of the result's own JSON leaves the redacted result
    // undecodable.
    openhuman_core::core::bus::init().await.expect("bus init");
    let mut events = openhuman_core::core::bus::BUS
        .get()
        .expect("bus initialized")
        .receiver();
    let mut unredactable = harness.config.clone();
    unredactable.composio.api_key = Some("\"successful\":true".to_string());
    let withheld = composio_execute(&unredactable, "GMAIL_FETCH_EMAILS", None, None).await;
    assert_eq!(
        withheld.err().as_deref(),
        Some("Composio result withheld by security redactor")
    );
    let recorded = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            match events.recv().await {
                Some(openhuman_core::core::events::DomainEvent::ComposioActionExecuted {
                    tool,
                    success,
                    error,
                    ..
                }) if tool == "GMAIL_FETCH_EMAILS" => return (success, error),
                Some(_) => continue,
                None => panic!("the bus closed before the execute was recorded"),
            }
        }
    })
    .await
    .expect("the withheld execute was not recorded");
    assert_eq!(recorded, (true, None));

    // TS-E2E-008 (EG-16): the UI's `composio.execute` RPC op is held to the
    // local-only privacy mode like the agent tool, and refused before the
    // connector module dials Composio. Checked in this test because the
    // security module serves the first workspace that calls it.
    let _privacy = PrivacyScope::set(PrivacyMode::LocalOnly);
    state.requests.lock().expect("requests").clear();
    let refused = composio_execute(
        &harness.config,
        "GMAIL_FETCH_EMAILS",
        Some(json!({ "query": "label:INBOX" })),
        None,
    )
    .await;
    let error = refused.expect_err("composio_execute ran under local-only privacy");
    assert!(
        error.to_lowercase().contains("privacy mode is active"),
        "unexpected refusal: {error}"
    );
    let requests = state.requests.lock().expect("requests").clone();
    assert!(
        !requests
            .iter()
            .any(|request| request.path.contains("/tools/execute/")),
        "Composio received an execute under local-only privacy: {requests:?}"
    );
}

/// Holds the live privacy mode at `mode` for one test and puts the previous
/// mode back on drop: the aggregated raw-coverage binary shares one process.
struct PrivacyScope(PrivacyMode);

impl PrivacyScope {
    fn set(mode: PrivacyMode) -> Self {
        let previous = live_policy::current_privacy_mode();
        if live_policy::reload_privacy(mode).is_err() {
            let workspace = std::env::temp_dir();
            live_policy::install(
                Arc::new(SecurityPolicy::default().with_privacy_mode(mode)),
                workspace.clone(),
                workspace,
            );
        }
        Self(previous)
    }
}

impl Drop for PrivacyScope {
    fn drop(&mut self) {
        let _ = live_policy::reload_privacy(self.0);
    }
}

/// The direct-mode key `setup_direct_config` stores, and the encoded forms a
/// provider might echo it in. The mock's Execute echoes all four.
fn round25_key_forms() -> [String; 4] {
    let key = "ck_round25_direct";
    [
        key.to_owned(),
        urlencoding::encode(key).into_owned(),
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key),
        hex::encode(key),
    ]
}

async fn start_loopback(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("loopback server");
    });
    format!("http://127.0.0.1:{}", addr.port())
}

async fn composio_direct_handler(State(state): State<MockState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let method = parts.method;
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().unwrap_or_default().to_string();
    let api_key = parts
        .headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let body_bytes = to_bytes(body, 1024 * 1024).await.expect("body bytes");
    let body_json = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body_bytes).unwrap_or(Value::Null)
    };

    state
        .requests
        .lock()
        .expect("requests")
        .push(RecordedRequest {
            method: method.clone(),
            path: path.clone(),
            query: query.clone(),
            body: body_json,
            api_key,
        });

    // TinyConnectors v0.3 serves direct-mode routes from the configured base
    // without the host's historical `/api/v3` prefix. Keep this fixture
    // tolerant of both shapes while retaining the recorded raw request above.
    let route = path
        .strip_prefix("/api/v3")
        .map_or_else(|| path.clone(), ToString::to_string);

    match (method, format!("/api/v3{route}").as_str()) {
        (Method::GET, "/api/v3/connected_accounts") => Json(json!({
            "items": [
                {
                    "id": "acct-gmail",
                    "status": "ACTIVE",
                    "toolkit": { "slug": "gmail" },
                    "created_at": "2026-05-30T00:00:00Z"
                },
                {
                    "id": "acct-github",
                    "status": "INITIATED",
                    "toolkit": "github",
                    "createdAt": "2026-05-30T00:00:01Z"
                },
                {
                    "id": "acct-slack",
                    "status": "CONNECTED",
                    "appName": "slack",
                    "created_at": "2026-05-30T00:00:02Z"
                },
                {
                    "id": "   ",
                    "status": "ACTIVE",
                    "toolkit": "dropme"
                }
            ]
        }))
        .into_response(),
        (Method::GET, "/api/v3/tools") => Json(json!({
            "items": [
                {
                    "slug": "GMAIL_FETCH_EMAILS",
                    "description": "Fetch Gmail messages",
                    "toolkit": { "slug": "gmail" },
                    "input_parameters": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" }
                        }
                    }
                },
                {
                    "slug": "",
                    "description": "Malformed row should be dropped",
                    "toolkit": { "slug": "gmail" }
                },
                {
                    "name": "SLACK_FETCH_MESSAGES",
                    "description": "Fetch Slack messages",
                    "appName": "slack",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "channel": { "type": "string" }
                        }
                    }
                }
            ]
        }))
        .into_response(),
        (Method::GET, "/api/v3/auth_configs") => Json(json!({
            "items": [
                { "id": "auth-disabled", "status": "disabled", "enabled": false },
                { "id": "auth-round25", "status": "enabled", "enabled": true }
            ]
        }))
        .into_response(),
        (Method::POST, "/api/v3/connected_accounts/link") => Json(json!({
            "redirectUrl": "https://connect.example.test/round25"
        }))
        .into_response(),
        (Method::POST, "/api/v3/tools/execute/GMAIL_FETCH_EMAILS") => Json(json!({
            "successful": true,
            "data": {
                "messages": [
                    { "id": "msg-round25", "subject": "Coverage" }
                ],
                "echo": round25_key_forms(),
            }
        }))
        .into_response(),
        (Method::POST, "/api/v3/tools/execute/GMAIL_SEND_EMAIL") => Json(json!({
            "successful": false,
            "error": "provider rejected send",
            "data": {
                "status": "blocked"
            }
        }))
        .into_response(),
        _ => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response(),
    }
}
