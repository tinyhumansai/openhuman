//! Round17 raw/E2E coverage for Composio tools, ops, trigger history, and
//! nearby local state/profile paths.
//!
//! This binary uses loopback Composio routes plus temp workspace/keyring state.
//! It intentionally drives public Rust surfaces so coverage lands on the same
//! paths used by JSON-RPC controllers and agent-callable tools.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};

use openhuman_core::openhuman::app_state::{
    snapshot, update_local_state, StoredAppStatePatch, StoredOnboardingTasks,
};
use openhuman_core::openhuman::composio::ops::{
    composio_execute, composio_list_tools, composio_list_trigger_history,
};
use openhuman_core::openhuman::composio::trigger_history::ComposioTriggerHistoryStore;
use openhuman_core::openhuman::composio::{
    init_composio_trigger_history, invalidate_connected_integrations_cache,
};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::profiles::{AuthProfile, AuthProfilesStore, TokenSet};
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::tools::{
    ComposioAuthorizeTool, ComposioExecuteTool, ComposioListConnectionsTool, ComposioListToolsTool,
    Tool, ToolCallOptions,
};

static ROUND17_ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    scenario: Arc<Mutex<Scenario>>,
}

#[derive(Clone, Debug, Default)]
enum Scenario {
    #[default]
    Normal,
    ConnectionsFail,
    ToolsFail,
    DropboxOnly,
}

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    path: String,
    query: String,
    body: Value,
}

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

struct Harness {
    _tmp: TempDir,
    workspace: PathBuf,
    config: Config,
    _guards: Vec<EnvGuard>,
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ROUND17_ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("create target");
    Builder::new()
        .prefix("composio-tools-ops-state-round17-")
        .tempdir_in("target")
        .expect("round17 tempdir")
}

async fn setup(api_url: &str) -> Harness {
    let tmp = tempdir();
    let root = tmp.path().join("openhuman");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");

    let guards = vec![
        EnvGuard::set_to_path("OPENHUMAN_WORKSPACE", &root),
        EnvGuard::set_to_path("HOME", tmp.path()),
        EnvGuard::unset("BACKEND_URL"),
        EnvGuard::unset("VITE_BACKEND_URL"),
        EnvGuard::unset("OPENHUMAN_API_URL"),
        EnvGuard::unset("OPENHUMAN_CORE_RPC_URL"),
        EnvGuard::unset("OPENHUMAN_CORE_PORT"),
        EnvGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
    ];

    let mut config = Config {
        workspace_dir: workspace.clone(),
        config_path: root.join("config.toml"),
        api_url: Some(api_url.to_string()),
        ..Config::default()
    };
    config.observability.analytics_enabled = false;
    config.secrets.encrypt = false;
    config.save().await.expect("save config");

    Harness {
        _tmp: tmp,
        workspace,
        config,
        _guards: guards,
    }
}

#[tokio::test]
async fn round17_agent_tools_cover_authorize_filters_scopes_and_error_branches() {
    let _lock = env_lock();
    let state = MockState::default();
    let base = start_loopback_backend(
        Router::new()
            .fallback(any(composio_backend_handler))
            .with_state(state.clone()),
    )
    .await;
    let harness = setup(&base).await;
    store_app_session_token(&harness.config, "round17-session-token");
    invalidate_connected_integrations_cache();
    let config = Arc::new(harness.config.clone());

    let authorize = ComposioAuthorizeTool::new(config.clone());
    let auth = authorize
        .execute(json!({ "toolkit": " Gmail " }))
        .await
        .expect("authorize tool");
    assert!(!auth.is_error);
    assert!(auth.text().contains("https://connect.example/round17"));
    assert!(auth.text().contains("conn-authorized-round17"));

    let connections = ComposioListConnectionsTool::new(config.clone())
        .execute(json!({}))
        .await
        .expect("connections tool");
    assert!(!connections.is_error);
    assert!(connections.text().contains("conn-gmail"));
    assert!(!connections.text().contains("conn-github-expired"));

    let list_tools = ComposioListToolsTool::new(config.clone());
    let connected_markdown = list_tools
        .execute_with_options(
            json!({ "toolkits": ["gmail", "github"], "tags": ["repos"], "include_unconnected": false }),
            ToolCallOptions {
                prefer_markdown: true,
                ..ToolCallOptions::default()
            },
        )
        .await
        .expect("connected list tools");
    assert!(!connected_markdown.is_error);
    assert!(connected_markdown.text().contains("GMAIL_FETCH_EMAILS"));
    assert!(!connected_markdown
        .text()
        .contains("GITHUB_GET_A_REPOSITORY"));
    assert!(connected_markdown
        .markdown_formatted
        .as_deref()
        .unwrap_or_default()
        .contains("query"));

    let include_unconnected = list_tools
        .execute(json!({
            "toolkits": ["github"],
            "tags": ["repos", "stars"],
            "include_unconnected": true
        }))
        .await
        .expect("include unconnected list tools");
    assert!(!include_unconnected.is_error);
    assert!(include_unconnected
        .text()
        .contains("GITHUB_GET_A_REPOSITORY"));

    *state.scenario.lock().expect("scenario") = Scenario::ConnectionsFail;
    let filter_error = list_tools
        .execute(json!({ "toolkits": ["gmail"], "include_unconnected": false }))
        .await
        .expect("connection filter error is tool result");
    assert!(filter_error.is_error);
    assert!(filter_error
        .text()
        .contains("include_unconnected=true to skip this check"));

    *state.scenario.lock().expect("scenario") = Scenario::DropboxOnly;
    let unsupported = list_tools
        .execute(json!({ "toolkits": ["zendesk"], "include_unconnected": false }))
        .await
        .expect("uncurated toolkit error");
    assert!(unsupported.is_error);
    assert!(unsupported
        .text()
        .contains("no agent-ready actions are available"));

    let execute = ComposioExecuteTool::new(config.clone());
    let admin_blocked = execute
        .execute(json!({
            "tool": "GMAIL_DELETE_MESSAGE",
            "arguments": { "message_id": "m1" }
        }))
        .await
        .expect("admin scope block");
    assert!(admin_blocked.is_error);
    assert!(admin_blocked.text().contains("classified `admin`"));
    assert!(admin_blocked.text().contains("Connections"));

    let not_curated = execute
        .execute(json!({
            "tool": "GMAIL_UNKNOWN_EXPERIMENT",
            "arguments": {}
        }))
        .await
        .expect("not curated block");
    assert!(not_curated.is_error);
    assert!(not_curated.text().contains("not in the curated whitelist"));

    *state.scenario.lock().expect("scenario") = Scenario::Normal;
    let success = execute
        .execute(json!({
            "tool": "GMAIL_FETCH_EMAILS",
            "connection_id": "conn-gmail",
            "arguments": { "query": "label:INBOX" }
        }))
        .await
        .expect("execute success");
    assert!(!success.is_error);
    assert_eq!(success.text(), "Fetched round17 inbox");

    *state.scenario.lock().expect("scenario") = Scenario::ToolsFail;
    let ops_error = composio_list_tools(&harness.config, Some(vec!["gmail".into()]), None)
        .await
        .expect_err("ops list_tools backend failure");
    assert!(ops_error.contains("[composio] list_tools failed"));

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().any(|req| {
        req.method == "GET"
            && req.path == "/agent-integrations/composio/tools"
            && req.query.contains("toolkits=github")
            && req.query.contains("tags=")
    }));
    assert!(requests.iter().any(|req| {
        req.method == "POST"
            && req.path == "/agent-integrations/composio/authorize"
            && req.body["toolkit"] == "Gmail"
    }));
}

#[tokio::test]
async fn round17_ops_trigger_history_app_state_and_profiles_cover_local_edges() {
    let _lock = env_lock();
    let state = MockState::default();
    let base = start_loopback_backend(
        Router::new()
            .fallback(any(composio_backend_handler))
            .with_state(state),
    )
    .await;
    let harness = setup(&base).await;
    store_app_session_token(&harness.config, "round17-session-token");

    let missing_history = composio_list_trigger_history(&harness.config, Some(5))
        .await
        .expect_err("history not initialized");
    assert!(missing_history.contains("archive store is not initialized"));

    let store = ComposioTriggerHistoryStore::new(&harness.workspace).expect("history store");
    for idx in 0..3 {
        store
            .record_trigger(
                "gmail",
                "GMAIL_NEW_GMAIL_MESSAGE",
                &format!("metadata-{idx}"),
                &format!("uuid-{idx}"),
                &json!({ "idx": idx }),
            )
            .expect("record trigger");
    }
    init_composio_trigger_history(harness.workspace.clone()).expect("init history");
    let clamped = composio_list_trigger_history(&harness.config, Some(9999))
        .await
        .expect("list initialized history")
        .value;
    assert_eq!(clamped.entries.len(), 3);
    assert!(clamped.archive_dir.ends_with("/state/triggers"));
    assert!(clamped.current_day_file.ends_with(".jsonl"));

    let state_dir = harness.workspace.join("state");
    std::fs::create_dir_all(&state_dir).expect("state dir");
    std::fs::write(state_dir.join("app-state.json"), "{not-json").expect("corrupt state");
    let updated = update_local_state(StoredAppStatePatch {
        keyring_consent: None,
        encryption_key: Some(Some(" round17-key ".into())),
        onboarding_tasks: Some(Some(StoredOnboardingTasks {
            accessibility_permission_granted: false,
            local_model_consent_given: true,
            local_model_download_started: false,
            enabled_tools: vec!["gmail".into(), "github".into()],
            connected_sources: vec!["gmail".into()],
            updated_at_ms: Some(17),
        })),
    })
    .await
    .expect("update state after corrupt file")
    .value;
    assert_eq!(updated.encryption_key.as_deref(), Some("round17-key"));
    let snap = snapshot().await.expect("snapshot").value;
    assert!(snap.auth.is_authenticated);
    assert_eq!(snap.session_token.as_deref(), Some("round17-session-token"));

    let profile_store = AuthProfilesStore::new(&harness.workspace.join("profiles"), false);
    let oauth = AuthProfile::new_oauth(
        "gmail",
        "round17",
        TokenSet {
            access_token: "access-round17".into(),
            refresh_token: Some("refresh-round17".into()),
            id_token: Some("id-round17".into()),
            expires_at: Some(Utc::now() + ChronoDuration::minutes(2)),
            token_type: Some("Bearer".into()),
            scope: Some("email profile".into()),
        },
    );
    profile_store
        .upsert_profile(oauth.clone(), true)
        .expect("insert oauth");
    let updated_profile = profile_store
        .update_profile(&oauth.id, |profile| {
            profile.metadata = BTreeMap::from([("round".into(), "17".into())]);
            profile.account_id = Some("acct-round17".into());
            Ok(())
        })
        .expect("update profile");
    assert_eq!(
        updated_profile.metadata.get("round"),
        Some(&"17".to_string())
    );
    assert!(updated_profile
        .token_set
        .as_ref()
        .expect("token set")
        .is_expiring_within(std::time::Duration::from_secs(180)));

    let direct_ops_success = composio_execute(
        &harness.config,
        "GMAIL_FETCH_EMAILS",
        Some(json!({ "query": "from:round17" })),
        None,
    )
    .await
    .expect("ops execute success")
    .value;
    assert!(direct_ops_success.successful);
    assert_eq!(
        direct_ops_success.markdown_formatted.as_deref(),
        Some("Fetched round17 inbox")
    );
}

async fn composio_backend_handler(State(state): State<MockState>, request: Request) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or_default().to_string();
    let body_bytes = to_bytes(request.into_body(), usize::MAX)
        .await
        .expect("mock request body");
    let body: Value = if body_bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&body_bytes).expect("json body")
    };
    state
        .requests
        .lock()
        .expect("requests")
        .push(RecordedRequest {
            method: method.as_str().to_string(),
            path: path.clone(),
            query: query.clone(),
            body: body.clone(),
        });
    let scenario = state.scenario.lock().expect("scenario").clone();

    match (method, path.as_str(), scenario) {
        (Method::GET, "/agent-integrations/composio/toolkits", _) => ok(json!({
            "toolkits": ["gmail", "github", "dropbox"]
        })),
        (Method::GET, "/agent-integrations/composio/connections", Scenario::ConnectionsFail) => {
            fail(StatusCode::BAD_GATEWAY, "connections unavailable")
        }
        (Method::GET, "/agent-integrations/composio/connections", Scenario::DropboxOnly) => {
            ok(json!({
                "connections": [{
                    "id": "conn-zendesk",
                    "toolkit": "zendesk",
                    "status": "ACTIVE",
                    "createdAt": "2026-05-29T12:00:00Z"
                }]
            }))
        }
        (Method::GET, "/agent-integrations/composio/connections", _) => ok(json!({
            "connections": [
                {
                    "id": "conn-gmail",
                    "toolkit": "gmail",
                    "status": "ACTIVE",
                    "createdAt": "2026-05-29T12:00:00Z"
                },
                {
                    "id": "conn-github-expired",
                    "toolkit": "github",
                    "status": "EXPIRED",
                    "createdAt": "2026-05-28T12:00:00Z"
                }
            ]
        })),
        (Method::POST, "/agent-integrations/composio/authorize", _) => ok(json!({
            "connectUrl": "https://connect.example/round17",
            "connectionId": "conn-authorized-round17"
        })),
        (Method::GET, "/agent-integrations/composio/tools", Scenario::ToolsFail) => {
            fail(StatusCode::SERVICE_UNAVAILABLE, "tools unavailable")
        }
        (Method::GET, "/agent-integrations/composio/tools", Scenario::DropboxOnly) => {
            ok(json!({ "tools": [] }))
        }
        (Method::GET, "/agent-integrations/composio/tools", _) => ok(json!({
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "GMAIL_FETCH_EMAILS",
                        "description": "Fetch Gmail messages over multiple lines\nfor markdown collapsing.",
                        "parameters": {
                            "type": "object",
                            "required": ["query"],
                            "properties": {
                                "query": { "type": "string" },
                                "max_results": { "type": "number" }
                            }
                        }
                    }
                },
                {
                    "type": "function",
                    "function": {
                        "name": "GMAIL_DELETE_EMAIL",
                        "description": "Delete Gmail message",
                        "parameters": { "type": "object" }
                    }
                },
                {
                    "type": "function",
                    "function": {
                        "name": "GITHUB_GET_A_REPOSITORY",
                        "description": "Star repository",
                        "parameters": { "type": "object" }
                    }
                }
            ]
        })),
        (Method::POST, "/agent-integrations/composio/execute", _) => {
            match body.get("tool").and_then(Value::as_str) {
                Some("GMAIL_FETCH_EMAILS") => ok(json!({
                    "data": { "messages": [{ "id": "msg-round17" }] },
                    "successful": true,
                    "error": null,
                    "costUsd": 0.04,
                    "markdownFormatted": "Fetched round17 inbox"
                })),
                other => fail(
                    StatusCode::BAD_REQUEST,
                    &format!("unexpected execute tool: {other:?}"),
                ),
            }
        }
        _ => fail(StatusCode::NOT_FOUND, &format!("unhandled {path}")),
    }
}

async fn start_loopback_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock backend");
    let addr = listener.local_addr().expect("mock backend addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn store_app_session_token(config: &Config, token: &str) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            token,
            HashMap::new(),
            true,
        )
        .expect("store app session token");
}

fn ok(data: Value) -> Response {
    Json(json!({ "success": true, "data": data })).into_response()
}

fn fail(status: StatusCode, error: &str) -> Response {
    (
        status,
        Json(json!({ "success": false, "error": error.to_string() })),
    )
        .into_response()
}
