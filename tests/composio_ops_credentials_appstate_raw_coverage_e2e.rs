//! Round18 raw/E2E coverage for Composio ops/tools, credentials profiles,
//! and app-state local snapshot branches.
//!
//! Uses temp stores plus loopback mocks only. No real keychain, network, or
//! Composio tenant calls are required.

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
    peek_cached_current_user_identity, snapshot, update_local_state, StoredAppStatePatch,
    StoredOnboardingTasks,
};
use openhuman_core::openhuman::composio::ops::{
    composio_authorize, composio_execute, composio_list_connections, composio_list_toolkits,
    composio_list_tools,
};
use openhuman_core::openhuman::composio::{
    all_composio_agent_tools, invalidate_connected_integrations_cache,
};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::profiles::{AuthProfile, AuthProfilesStore, TokenSet};
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::tools::{
    ComposioAuthorizeTool, ComposioExecuteTool, ComposioListConnectionsTool,
    ComposioListToolkitsTool, ComposioListToolsTool, Tool, ToolCallOptions,
};

static ROUND18_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    scenario: Arc<Mutex<Scenario>>,
}

#[derive(Clone, Debug, Default)]
enum Scenario {
    #[default]
    Normal,
    ToolkitsFail,
    AuthorizeFail,
    ConnectionsFail,
    ToolsFail,
    ExecuteFail,
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
    root: PathBuf,
    config: Config,
    _guards: Vec<EnvGuard>,
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ROUND18_ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("create target");
    Builder::new()
        .prefix("composio-ops-credentials-appstate-round18-")
        .tempdir_in("target")
        .expect("round18 tempdir")
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
        EnvGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];

    let mut config = Config {
        workspace_dir: workspace,
        config_path: root.join("config.toml"),
        api_url: Some(api_url.to_string()),
        onboarding_completed: true,
        chat_onboarding_completed: false,
        ..Config::default()
    };
    config.observability.analytics_enabled = false;
    config.secrets.encrypt = false;
    config.save().await.expect("save config");

    Harness {
        _tmp: tmp,
        root,
        config,
        _guards: guards,
    }
}

#[tokio::test]
async fn round18_composio_ops_and_agent_tools_cover_backend_errors_and_metadata() {
    let _lock = env_lock();
    let state = MockState::default();
    let base = start_loopback_backend(
        Router::new()
            .fallback(any(composio_backend_handler))
            .with_state(state.clone()),
    )
    .await;
    let harness = setup(&base).await;
    store_app_session_token(&harness.config, "round18.header.payload");
    invalidate_connected_integrations_cache();

    let toolkits = composio_list_toolkits(&harness.config)
        .await
        .expect("list toolkits")
        .value;
    assert_eq!(toolkits.toolkits, vec!["gmail", "github"]);

    let connections = composio_list_connections(&harness.config)
        .await
        .expect("list connections")
        .value;
    assert_eq!(connections.connections.len(), 2);

    let listed = composio_list_tools(
        &harness.config,
        Some(vec![" Gmail ".into(), "github".into()]),
        Some(vec![" repos ".into(), " ".into()]),
    )
    .await
    .expect("list tools")
    .value;
    assert!(listed
        .tools
        .iter()
        .any(|tool| tool.function.name == "GMAIL_FETCH_EMAILS"));

    let no_tags_forwarded = composio_list_tools(
        &harness.config,
        Some(vec!["gmail".into()]),
        Some(vec!["should-not-forward".into()]),
    )
    .await
    .expect("gmail tags suppressed")
    .value;
    assert_eq!(no_tags_forwarded.tools.len(), 3);

    let authorize = composio_authorize(&harness.config, " gmail ", None)
        .await
        .expect("authorize")
        .value;
    assert_eq!(authorize.connection_id, "conn-round18");

    let executed = composio_execute(
        &harness.config,
        "GMAIL_FETCH_EMAILS",
        Some(json!({ "query": "newer_than:1d" })),
        None,
    )
    .await
    .expect("execute")
    .value;
    assert!(executed.successful);
    assert_eq!(
        executed.markdown_formatted.as_deref(),
        Some("Round18 inbox markdown")
    );

    let config = Arc::new(harness.config.clone());
    let list_toolkits_tool = ComposioListToolkitsTool::new(config.clone());
    assert_eq!(list_toolkits_tool.name(), "composio_list_toolkits");
    assert!(list_toolkits_tool.description().contains("toolkits"));
    assert_eq!(
        list_toolkits_tool.permission_level().to_string(),
        "ReadOnly"
    );
    assert_eq!(list_toolkits_tool.category().to_string(), "skill");
    assert!(list_toolkits_tool
        .parameters_schema()
        .as_object()
        .is_some_and(|obj| obj.contains_key("properties")));
    let list_toolkits_result = list_toolkits_tool
        .execute(json!({}))
        .await
        .expect("list toolkits tool");
    assert!(!list_toolkits_result.is_error);

    let list_connections_tool = ComposioListConnectionsTool::new(config.clone());
    assert_eq!(list_connections_tool.name(), "composio_list_connections");
    assert!(list_connections_tool
        .description()
        .contains("currently-connected"));
    assert!(list_connections_tool
        .parameters_schema()
        .pointer("/additionalProperties")
        .is_some());

    let authorize_tool = ComposioAuthorizeTool::new(config.clone());
    assert_eq!(authorize_tool.name(), "composio_authorize");
    assert!(authorize_tool.description().contains("OAuth"));
    assert!(authorize_tool
        .parameters_schema()
        .pointer("/required/0")
        .is_some());
    let missing_toolkit = authorize_tool
        .execute(json!({}))
        .await
        .expect("authorize validation");
    assert!(missing_toolkit.is_error);

    let list_tools_tool = ComposioListToolsTool::new(config.clone());
    assert!(list_tools_tool.supports_markdown());
    assert_eq!(list_tools_tool.name(), "composio_list_tools");
    let markdown_empty = list_tools_tool
        .execute_with_options(
            json!({ "toolkits": ["zendesk"], "include_unconnected": true }),
            ToolCallOptions {
                prefer_markdown: true,
                ..ToolCallOptions::default()
            },
        )
        .await
        .expect("empty markdown branch");
    assert!(markdown_empty.is_error);
    assert!(markdown_empty.text().contains("no agent-ready actions"));

    let execute_tool = ComposioExecuteTool::new(config.clone());
    assert_eq!(execute_tool.name(), "composio_execute");
    assert_eq!(execute_tool.permission_level().to_string(), "Write");
    let missing_action = execute_tool
        .execute(json!({}))
        .await
        .expect("execute validation");
    assert!(missing_action.is_error);
    let blocked_action = execute_tool
        .execute(json!({ "tool": "GMAIL_UNKNOWN_WRITE", "arguments": {} }))
        .await
        .expect("execute not curated");
    assert!(blocked_action.is_error);
    assert!(blocked_action
        .text()
        .contains("not in the curated whitelist"));

    let registered_tools = all_composio_agent_tools(&harness.config);
    // 6 generic composio agent tools incl. the inline-connect card tool (#3993).
    assert_eq!(registered_tools.len(), 6);

    *state.scenario.lock().expect("scenario") = Scenario::ToolkitsFail;
    let toolkits_error = composio_list_toolkits(&harness.config)
        .await
        .expect_err("toolkits backend error");
    assert!(toolkits_error.contains("list_toolkits failed"));

    *state.scenario.lock().expect("scenario") = Scenario::ConnectionsFail;
    let connections_error = list_connections_tool
        .execute(json!({}))
        .await
        .expect_err("connections tool propagates backend error")
        .to_string();
    assert!(connections_error.contains("composio_list_connections"));

    *state.scenario.lock().expect("scenario") = Scenario::ToolsFail;
    let tools_error = list_tools_tool
        .execute(json!({ "include_unconnected": true }))
        .await
        .expect("tools backend error is tool result");
    assert!(tools_error.is_error);

    *state.scenario.lock().expect("scenario") = Scenario::AuthorizeFail;
    let authorize_error = authorize_tool
        .execute(json!({ "toolkit": "gmail" }))
        .await
        .expect("authorize backend error is tool result");
    assert!(authorize_error.is_error);

    *state.scenario.lock().expect("scenario") = Scenario::ExecuteFail;
    let execute_error = execute_tool
        .execute(json!({ "tool": "GMAIL_FETCH_EMAILS", "arguments": {} }))
        .await
        .expect("execute backend error is tool result");
    assert!(execute_error.is_error);

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().any(|req| {
        req.method == "GET"
            && req.path == "/agent-integrations/composio/tools"
            && req.query.contains("toolkits=Gmail,github")
            && req.query.contains("tags=repos")
    }));
    assert!(requests.iter().any(|req| {
        req.method == "GET"
            && req.path == "/agent-integrations/composio/tools"
            && req.query.contains("toolkits=gmail")
            && !req.query.contains("tags=should-not-forward")
    }));
    assert!(requests.iter().any(|req| {
        req.method == "POST"
            && req.path == "/agent-integrations/composio/execute"
            && req.body.pointer("/tool") == Some(&json!("GMAIL_FETCH_EMAILS"))
    }));
}

#[tokio::test]
async fn round18_credentials_profiles_recover_active_and_corrupt_store_edges() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9").await;
    let state_dir = harness.root.join("profile-state");
    let store = AuthProfilesStore::new(&state_dir, false);

    assert!(store.load().expect("fresh load").profiles.is_empty());

    std::fs::create_dir_all(&state_dir).expect("state dir");
    std::fs::write(store.path(), "").expect("empty profile file");
    assert!(store.load().expect("empty load").profiles.is_empty());

    std::fs::write(store.path(), "{not-json").expect("corrupt profile file");
    assert!(store.load().expect("corrupt load").profiles.is_empty());
    assert!(std::fs::read_dir(&state_dir)
        .expect("read state dir")
        .any(|entry| entry
            .expect("dir entry")
            .file_name()
            .to_string_lossy()
            .contains("auth-profiles.corrupt")));

    std::fs::write(
        store.path(),
        json!({
            "schema_version": 999,
            "updated_at": Utc::now().to_rfc3339(),
            "active_profiles": {},
            "profiles": {}
        })
        .to_string(),
    )
    .expect("future schema profile file");
    let future_error = store
        .load()
        .expect_err("future schema should fail")
        .to_string();
    assert!(future_error.contains("Unsupported auth profile schema version"));
    std::fs::remove_file(store.path()).expect("reset future schema store");

    let token_profile = AuthProfile::new_token("github", "work", "ghp_round18".to_string());
    store
        .upsert_profile(token_profile.clone(), true)
        .expect("insert token profile");
    let loaded = store.load().expect("load token profile");
    assert_eq!(
        loaded.active_profiles.get("github"),
        Some(&token_profile.id)
    );
    assert_eq!(
        loaded
            .profiles
            .get(&token_profile.id)
            .and_then(|profile| profile.token.as_deref()),
        Some("ghp_round18")
    );

    let missing_active = store
        .set_active_profile("github", "missing-profile")
        .expect_err("missing active profile")
        .to_string();
    assert!(missing_active.contains("Auth profile not found"));

    let oauth_profile = AuthProfile::new_oauth(
        "gmail",
        "personal",
        TokenSet {
            access_token: "access-round18".to_string(),
            refresh_token: Some("refresh-round18".to_string()),
            id_token: Some("id-round18".to_string()),
            expires_at: Some(Utc::now() + ChronoDuration::minutes(4)),
            token_type: Some("Bearer".to_string()),
            scope: Some("email profile".to_string()),
        },
    );
    store
        .upsert_profile(oauth_profile.clone(), false)
        .expect("insert oauth profile");
    store
        .set_active_profile("gmail", &oauth_profile.id)
        .expect("set active");
    let updated = store
        .update_profile(&oauth_profile.id, |profile| {
            profile.metadata = BTreeMap::from([("round".to_string(), "18".to_string())]);
            profile.account_id = Some("acct-round18".to_string());
            Ok(())
        })
        .expect("update profile");
    assert_eq!(updated.account_id.as_deref(), Some("acct-round18"));
    assert!(updated
        .token_set
        .as_ref()
        .expect("token set")
        .is_expiring_within(std::time::Duration::from_secs(300)));

    store
        .clear_active_profile("gmail")
        .expect("clear active profile");
    assert!(store
        .load()
        .expect("load after clear")
        .active_profiles
        .get("gmail")
        .is_none());
    assert!(!store
        .remove_profile("missing-profile")
        .expect("remove missing"));
    assert!(store
        .remove_profile(&token_profile.id)
        .expect("remove token profile"));
}

#[tokio::test]
async fn round18_app_state_snapshot_uses_local_session_cache_and_patch_edges() {
    let _lock = env_lock();
    let state = MockState::default();
    let base = start_loopback_backend(
        Router::new()
            .fallback(any(composio_backend_handler))
            .with_state(state),
    )
    .await;
    let harness = setup(&base).await;

    let mut metadata = HashMap::new();
    metadata.insert("user_id".to_string(), "stored-round18".to_string());
    metadata.insert(
        "user_json".to_string(),
        json!({
            "id": "stored-round18",
            "name": "Stored Round18",
            "email": "round18@example.test"
        })
        .to_string(),
    );
    AuthService::from_config(&harness.config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "round18.payload.local",
            metadata,
            true,
        )
        .expect("store local app session");

    let first = update_local_state(StoredAppStatePatch {
        keyring_consent: None,
        encryption_key: Some(Some("   ".to_string())),
        onboarding_tasks: Some(Some(StoredOnboardingTasks {
            accessibility_permission_granted: true,
            local_model_consent_given: false,
            local_model_download_started: true,
            enabled_tools: vec!["gmail".to_string()],
            connected_sources: vec!["github".to_string()],
            updated_at_ms: None,
        })),
    })
    .await
    .expect("write local state")
    .value;
    assert!(first.encryption_key.is_none());
    assert!(first.onboarding_tasks.is_some());

    let cleared = update_local_state(StoredAppStatePatch {
        keyring_consent: None,
        encryption_key: Some(None),
        onboarding_tasks: Some(None),
    })
    .await
    .expect("clear local state")
    .value;
    assert!(cleared.encryption_key.is_none());
    assert!(cleared.onboarding_tasks.is_none());

    let snap = snapshot().await.expect("snapshot").value;
    assert!(snap.auth.is_authenticated);
    assert_eq!(snap.session_token.as_deref(), Some("round18.payload.local"));
    assert_eq!(
        snap.current_user
            .as_ref()
            .and_then(|user| user.get("id"))
            .and_then(Value::as_str),
        Some("stored-round18")
    );
    assert!(snap.onboarding_completed);
    assert!(!snap.analytics_enabled);

    assert!(peek_cached_current_user_identity().is_none());
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
        (Method::GET, "/agent-integrations/composio/toolkits", Scenario::ToolkitsFail) => {
            fail(StatusCode::BAD_GATEWAY, "toolkits unavailable")
        }
        (Method::GET, "/agent-integrations/composio/toolkits", _) => ok(json!({
            "toolkits": ["gmail", "github"]
        })),
        (Method::GET, "/agent-integrations/composio/connections", Scenario::ConnectionsFail) => {
            fail(StatusCode::BAD_GATEWAY, "connections unavailable")
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
                    "id": "conn-github",
                    "toolkit": "github",
                    "status": "CONNECTED",
                    "createdAt": "2026-05-29T12:00:00Z"
                }
            ]
        })),
        (Method::POST, "/agent-integrations/composio/authorize", Scenario::AuthorizeFail) => {
            fail(StatusCode::BAD_GATEWAY, "authorize unavailable")
        }
        (Method::POST, "/agent-integrations/composio/authorize", _) => ok(json!({
            "connectUrl": "https://connect.example/round18",
            "connectionId": "conn-round18"
        })),
        (Method::GET, "/agent-integrations/composio/tools", Scenario::ToolsFail) => {
            fail(StatusCode::SERVICE_UNAVAILABLE, "tools unavailable")
        }
        (Method::GET, "/agent-integrations/composio/tools", _) => {
            if query_contains_toolkit(&query, "zendesk") {
                return ok(json!({ "tools": [] }));
            }
            ok(json!({
                "tools": [
                    {
                        "type": "function",
                        "function": {
                            "name": "GMAIL_FETCH_EMAILS",
                            "description": "Fetch Gmail messages for round18 coverage",
                            "parameters": {
                                "type": "object",
                                "required": ["query"],
                                "properties": {
                                    "query": { "type": "string" },
                                    "max_results": { "type": "integer" }
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
                            "description": "Get repository",
                            "parameters": { "type": "object" }
                        }
                    }
                ]
            }))
        }
        (Method::POST, "/agent-integrations/composio/execute", Scenario::ExecuteFail) => {
            fail(StatusCode::BAD_GATEWAY, "execute unavailable")
        }
        (Method::POST, "/agent-integrations/composio/execute", _) => ok(json!({
            "data": { "messages": [{ "id": "msg-round18" }] },
            "successful": true,
            "error": null,
            "costUsd": 0.01,
            "markdownFormatted": "Round18 inbox markdown"
        })),
        (Method::GET, "/auth/me", _) => ok(json!({
            "id": "fresh-round18",
            "name": "Fresh Round18",
            "email": "fresh-round18@example.test"
        })),
        _ => fail(StatusCode::NOT_FOUND, &format!("unhandled {path}")),
    }
}

fn query_contains_toolkit(query: &str, toolkit: &str) -> bool {
    query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .filter(|(key, _)| *key == "toolkits")
        .flat_map(|(_, value)| value.split(','))
        .any(|value| value.eq_ignore_ascii_case(toolkit))
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
