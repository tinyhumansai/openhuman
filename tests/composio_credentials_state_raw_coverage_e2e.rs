//! Round15 raw integration coverage for Composio, credentials, app state, and threads.
//!
//! Everything stays on loopback mocks and temp stores. The tests drive public
//! Rust surfaces so coverage lands on the same ops/tool paths used by JSON-RPC
//! and the agent runtime without real Composio, keychain, or backend calls.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};

use openhuman_core::openhuman::app_state::{
    snapshot, update_local_state, StoredAppStatePatch, StoredOnboardingTasks,
};
use openhuman_core::openhuman::composio::ops::{
    cached_active_integrations, composio_authorize, composio_clear_api_key, composio_get_mode,
    composio_list_connections, composio_list_tools, composio_list_trigger_history,
    composio_set_api_key, fetch_connected_integrations_status,
};
use openhuman_core::openhuman::composio::trigger_history::ComposioTriggerHistoryStore;
use openhuman_core::openhuman::composio::{
    init_composio_trigger_history, invalidate_connected_integrations_cache, ComposioActionTool,
    FetchConnectedIntegrationsStatus,
};
use openhuman_core::openhuman::config::rpc as config_rpc;
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::profiles::{AuthProfile, AuthProfilesStore, TokenSet};
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::memory::{
    AppendConversationMessageRequest, ConversationMessageRecord, CreateConversationThreadRequest,
    EmptyRequest, GenerateConversationThreadTitleRequest, UpdateConversationMessageRequest,
    UpdateConversationThreadTitleRequest,
};
use openhuman_core::openhuman::threads::migrate_welcome_agent_artifacts;
use openhuman_core::openhuman::threads::ops::{
    message_append, message_update, messages_list, thread_create_new, thread_generate_title,
    thread_update_title, threads_list,
};
use openhuman_core::openhuman::tools::{
    ComposioExecuteTool, ComposioListConnectionsTool, ComposioListToolkitsTool,
    ComposioListToolsTool, Tool, ToolCallOptions,
};

static ROUND15_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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
    workspace: PathBuf,
    _guards: Vec<EnvGuard>,
}

impl Harness {
    async fn config(&self) -> Config {
        config_rpc::load_config_with_timeout()
            .await
            .expect("isolated config should load")
    }

    fn app_state_file(&self) -> PathBuf {
        self.workspace.join("state/app-state.json")
    }
}

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    path: String,
    query: String,
    body: Value,
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ROUND15_ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("create target");
    Builder::new()
        .prefix("composio-credentials-state-round15-")
        .tempdir_in("target")
        .expect("round15 tempdir")
}

fn write_min_config(root: &Path, api_url: &str) {
    std::fs::create_dir_all(root).expect("create openhuman root");
    let cfg = format!(
        r#"api_url = "{api_url}"
default_model = "round15-coverage-model"
default_temperature = 0.2
onboarding_completed = true
chat_onboarding_completed = false

[observability]
analytics_enabled = false

[secrets]
encrypt = false

[local_ai]
enabled = false
runtime_enabled = false
opt_in_confirmed = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0
auto_save = false

[memory_tree]
embedding_strict = false
"#
    );
    std::fs::write(root.join("config.toml"), &cfg).expect("write config.toml");
    let _: Config = toml::from_str(&cfg).expect("round15 config must match schema");
}

fn setup(api_url: &str) -> Harness {
    let tmp = tempdir();
    let root = tmp.path().join("openhuman");
    write_min_config(&root, api_url);
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
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

    Harness {
        _tmp: tmp,
        root,
        workspace,
        _guards: guards,
    }
}

#[tokio::test]
async fn round15_composio_agent_tools_backend_cache_and_trigger_history_edges() {
    let _lock = env_lock();
    let state = MockState::default();
    let base = start_loopback_backend(
        Router::new()
            .fallback(any(composio_backend_handler))
            .with_state(state.clone()),
    )
    .await;
    let harness = setup(&base);
    let config = harness.config().await;
    store_app_session_token(&config, "round15-session-token");
    invalidate_connected_integrations_cache();

    let arc_config = Arc::new(config.clone());
    let toolkits = ComposioListToolkitsTool::new(arc_config.clone())
        .execute(json!({}))
        .await
        .expect("list toolkits tool");
    assert!(!toolkits.is_error);
    assert!(toolkits.text().contains("\"gmail\""));

    let connections = ComposioListConnectionsTool::new(arc_config.clone())
        .execute(json!({}))
        .await
        .expect("list connections tool");
    assert!(!connections.is_error);
    assert!(connections.text().contains("conn-gmail"));
    assert!(
        !connections.text().contains("conn-github"),
        "agent-facing connection tool should filter non-active rows"
    );

    let list_tools = ComposioListToolsTool::new(arc_config.clone())
        .execute_with_options(
            json!({
                "toolkits": ["gmail", "github"],
                "tags": ["repos", " "],
                "include_unconnected": false
            }),
            ToolCallOptions {
                prefer_markdown: true,
                ..ToolCallOptions::default()
            },
        )
        .await
        .expect("list tools markdown");
    assert!(!list_tools.is_error);
    assert!(list_tools.text().contains("GMAIL_FETCH_EMAILS"));
    assert!(list_tools
        .markdown_formatted
        .as_deref()
        .unwrap_or_default()
        .contains("# Composio tools"));
    assert!(
        !list_tools
            .text()
            .contains("GITHUB_STAR_A_REPOSITORY_FOR_THE_AUTHENTICATED_USER"),
        "github is expired in the mock and should be filtered when include_unconnected=false"
    );

    let integrations = fetch_connected_integrations_status(&config).await;
    let FetchConnectedIntegrationsStatus::Authoritative(items) = integrations else {
        panic!("backend mock should produce authoritative integrations");
    };
    assert!(items.iter().any(|item| item.toolkit == "gmail"
        && item.connected
        && item
            .tools
            .iter()
            .any(|tool| tool.name == "GMAIL_FETCH_EMAILS")));
    assert!(items.iter().any(|item| item.toolkit == "github"
        && !item.connected
        && item.non_active_status.as_deref() == Some("EXPIRED")));
    assert!(cached_active_integrations(&config).is_some());

    let execute_tool = ComposioExecuteTool::new(arc_config.clone());
    let executed = execute_tool
        .execute(json!({
            "tool": "GMAIL_FETCH_EMAILS",
            "arguments": { "query": "label:INBOX", "max_results": 1 }
        }))
        .await
        .expect("execute tool success");
    assert!(!executed.is_error);
    assert_eq!(executed.text(), "Fetched 1 inbox message");

    let failed = execute_tool
        .execute(json!({
            "tool": "GMAIL_SEND_EMAIL",
            "arguments": { "to": "person@example.test" }
        }))
        .await
        .expect("execute tool provider failure");
    assert!(!failed.is_error);
    assert!(failed.text().contains("[composio:error:validation]"));

    let action_tool = ComposioActionTool::new(
        arc_config,
        "GMAIL_FETCH_EMAILS".to_string(),
        "Fetch inbox".to_string(),
        Some(json!({
            "type": "object",
            "properties": { "query": { "type": "string" } }
        })),
    );
    assert_eq!(action_tool.name(), "GMAIL_FETCH_EMAILS");
    assert_eq!(action_tool.category().to_string(), "skill");
    let action_result = action_tool
        .execute(json!({ "query": "from:me" }))
        .await
        .expect("per-action tool execute");
    assert_eq!(action_result.text(), "Fetched 1 inbox message");

    let reserved = composio_authorize(&config, "gmail", Some(json!({ "toolkit": "github" })))
        .await
        .expect_err("reserved extra param rejected before request");
    assert!(reserved.contains("cannot override reserved key"));

    let listed = composio_list_connections(&config)
        .await
        .expect("ops list connections")
        .value;
    assert_eq!(listed.connections.len(), 3);

    let ops_tools = composio_list_tools(
        &config,
        Some(vec!["slack".to_string()]),
        Some(vec!["ignored".to_string()]),
    )
    .await
    .expect("ops list tools drops non-queryable tags")
    .value;
    assert!(ops_tools
        .tools
        .iter()
        .any(|tool| tool.function.name == "SLACK_FETCH_CONVERSATION_HISTORY"));

    let local_store =
        ComposioTriggerHistoryStore::new(&harness.workspace).expect("local history store");
    local_store
        .record_trigger(
            "gmail",
            "GMAIL_NEW_GMAIL_MESSAGE",
            "metadata-a",
            "uuid-a",
            &json!({ "subject": "first" }),
        )
        .expect("record first trigger");
    local_store
        .record_trigger(
            "github",
            "GITHUB_PULL_REQUEST_EVENT",
            "metadata-b",
            "uuid-b",
            &json!({ "repo": "openhuman" }),
        )
        .expect("record second trigger");
    let recent = local_store.list_recent(1).expect("local recent history");
    assert_eq!(recent.entries.len(), 1);
    assert_eq!(recent.entries[0].metadata_id, "metadata-b");

    init_composio_trigger_history(config.workspace_dir.clone())
        .expect("init global trigger history once");
    let rpc_history = composio_list_trigger_history(&config, Some(0))
        .await
        .expect("history rpc clamps low limit")
        .value;
    assert_eq!(rpc_history.entries.len(), 1);

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().any(|req| {
        req.method == "GET"
            && req.path == "/agent-integrations/composio/tools"
            && req.query.contains("toolkits=slack")
            && !req.query.contains("tags=")
    }));
    assert!(requests.iter().any(|req| {
        req.method == "POST"
            && req.path == "/agent-integrations/composio/execute"
            && req.body["tool"] == "GMAIL_FETCH_EMAILS"
    }));
}

#[tokio::test]
async fn round15_composio_direct_key_mode_flips_without_network() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let config = harness.config().await;
    let direct_base = start_loopback_backend(Router::new().route(
        "/connected_accounts",
        get(composio_direct_connected_accounts),
    ))
    .await;
    let _direct_v2_guard = EnvGuard::set("OPENHUMAN_COMPOSIO_DIRECT_BASE_V2", &direct_base);
    let _direct_v3_guard = EnvGuard::set("OPENHUMAN_COMPOSIO_DIRECT_BASE_V3", &direct_base);

    let empty = composio_set_api_key(&config, "   ", false)
        .await
        .expect_err("blank direct key rejected");
    assert!(empty.contains("api_key must not be empty"));

    let stored = composio_set_api_key(&config, "  cmp_round15_key  ", true)
        .await
        .expect("store direct key and activate")
        .value;
    assert_eq!(stored["stored"], true);
    assert_eq!(stored["mode"], "direct");

    let reloaded = harness.config().await;
    assert_eq!(reloaded.composio.mode, "direct");
    let mode = composio_get_mode(&reloaded).await.expect("get mode").value;
    assert_eq!(mode["api_key_set"], true);

    let direct_toolkits =
        openhuman_core::openhuman::composio::ops::composio_list_toolkits(&reloaded)
            .await
            .expect("direct list toolkits is local")
            .value;
    assert!(direct_toolkits.toolkits.is_empty());

    let cleared = composio_clear_api_key(&reloaded)
        .await
        .expect("clear direct key")
        .value;
    assert_eq!(cleared["cleared"], true);
    assert_eq!(cleared["mode"], "backend");
    let backend_again = harness.config().await;
    assert_eq!(backend_again.composio.mode, "backend");
    assert_eq!(
        composio_get_mode(&backend_again)
            .await
            .expect("mode after clear")
            .value["api_key_set"],
        false
    );
}

#[tokio::test]
async fn round15_app_state_corruption_clear_and_snapshot_local_session_paths() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let config = harness.config().await;

    std::fs::create_dir_all(harness.app_state_file().parent().expect("state parent"))
        .expect("state dir");
    std::fs::write(harness.app_state_file(), "{bad-json").expect("write corrupt app state");
    let recovered = update_local_state(StoredAppStatePatch {
        keyring_consent: None,
        encryption_key: Some(Some("  round15-secret  ".to_string())),
        onboarding_tasks: Some(Some(StoredOnboardingTasks {
            accessibility_permission_granted: true,
            local_model_consent_given: false,
            local_model_download_started: true,
            enabled_tools: vec!["gmail".to_string()],
            connected_sources: vec!["slack".to_string(), "github".to_string()],
            updated_at_ms: Some(15),
        })),
    })
    .await
    .expect("update after corrupt state")
    .value;
    assert_eq!(recovered.encryption_key.as_deref(), Some("round15-secret"));
    let quarantined = std::fs::read_dir(harness.app_state_file().parent().expect("state parent"))
        .expect("state entries")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains("app-state.json.corrupted")
        });
    assert!(quarantined, "corrupt app state should be quarantined");

    let cleared = update_local_state(StoredAppStatePatch {
        keyring_consent: None,
        encryption_key: Some(None),
        onboarding_tasks: Some(None),
    })
    .await
    .expect("clear local state fields")
    .value;
    assert!(cleared.encryption_key.is_none());
    assert!(cleared.onboarding_tasks.is_none());

    let mut metadata = HashMap::new();
    metadata.insert("user_id".to_string(), "local-round15".to_string());
    metadata.insert(
        "user_json".to_string(),
        json!({
            "userId": "local-round15",
            "display_name": "Local Round15",
            "email": "round15@example.test"
        })
        .to_string(),
    );
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "header.payload.local",
            metadata,
            true,
        )
        .expect("store local app session");

    let snap = snapshot().await.expect("snapshot with local session").value;
    assert!(snap.auth.is_authenticated);
    assert_eq!(snap.session_token.as_deref(), Some("header.payload.local"));
    assert_eq!(
        snap.current_user
            .as_ref()
            .and_then(|value| value.get("userId")),
        Some(&json!("local-round15"))
    );
}

#[test]
fn round15_auth_profiles_drop_bad_entries_update_remove_and_clear_active() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let state_dir = harness.root.join("profile-store");
    let store = AuthProfilesStore::new(&state_dir, false);

    let token = AuthProfile::new_token("slack", "bot", "xoxb-round15".to_string());
    store
        .upsert_profile(token.clone(), true)
        .expect("insert token profile");
    let oauth = AuthProfile::new_oauth(
        "github",
        "work",
        TokenSet {
            access_token: "gh-round15".to_string(),
            refresh_token: Some("refresh-round15".to_string()),
            id_token: Some("id-round15".to_string()),
            expires_at: Some(Utc::now() + ChronoDuration::minutes(5)),
            token_type: Some("Bearer".to_string()),
            scope: Some("repo".to_string()),
        },
    );
    store
        .upsert_profile(oauth.clone(), true)
        .expect("insert oauth profile");

    let updated = store
        .update_profile(&oauth.id, |profile| {
            profile.account_id = Some("acct-round15".to_string());
            profile.workspace_id = Some("workspace-round15".to_string());
            profile.metadata = BTreeMap::from([("team".to_string(), "core".to_string())]);
            Ok(())
        })
        .expect("update profile");
    assert_eq!(updated.account_id.as_deref(), Some("acct-round15"));
    assert!(updated
        .token_set
        .as_ref()
        .expect("token set")
        .is_expiring_within(std::time::Duration::from_secs(600)));

    let missing_update = store.update_profile("missing-profile", |_| Ok(()));
    assert!(missing_update
        .expect_err("missing update fails")
        .to_string()
        .contains("Auth profile not found"));

    store
        .clear_active_profile("github")
        .expect("clear active github");
    let data = store.load().expect("load after clear active");
    assert!(!data.active_profiles.contains_key("github"));

    let removed = store
        .remove_profile(&token.id)
        .expect("remove token profile");
    assert!(removed);
    let removed_again = store
        .remove_profile(&token.id)
        .expect("remove missing profile is false");
    assert!(!removed_again);

    let path = store.path().to_path_buf();
    let mut raw: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("profile json"))
            .expect("valid profile json");
    raw["profiles"]["legacy-bad-kind"] = json!({
        "provider": "legacy",
        "profile_name": "bad",
        "kind": "api_key",
        "token": "legacy-token",
        "created_at": Utc::now().to_rfc3339(),
        "updated_at": Utc::now().to_rfc3339(),
        "metadata": {}
    });
    raw["active_profiles"]["legacy"] = json!("legacy-bad-kind");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&raw).expect("serialize"),
    )
    .expect("write profile json with bad kind");

    let migrated = store.load().expect("bad kind should be dropped");
    assert!(!migrated.profiles.contains_key("legacy-bad-kind"));
    assert!(!migrated.active_profiles.contains_key("legacy"));
    assert!(migrated.profiles.contains_key(&oauth.id));
}

#[tokio::test]
async fn round15_threads_ops_and_welcome_migration_public_paths() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");

    let created = thread_create_new(CreateConversationThreadRequest {
        labels: Some(vec!["onboarding".to_string(), "personal".to_string()]),
        personality_id: Some("default".to_string()),
    })
    .await
    .expect("create thread")
    .value
    .data
    .expect("created thread data");
    let thread_id = created.id.clone();
    assert!(created.title.starts_with("Chat "));

    let msg = ConversationMessageRecord {
        id: "msg-round15".to_string(),
        content: "Please summarize the team standup action items for tomorrow.".to_string(),
        message_type: "text".to_string(),
        extra_metadata: json!({ "source": "round15" }),
        sender: "user".to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    let appended = message_append(AppendConversationMessageRequest {
        thread_id: thread_id.clone(),
        message: msg,
    })
    .await
    .expect("append message")
    .value
    .data
    .expect("message data");
    assert_eq!(appended.id, "msg-round15");

    let updated_msg = message_update(UpdateConversationMessageRequest {
        thread_id: thread_id.clone(),
        message_id: "msg-round15".to_string(),
        extra_metadata: Some(json!({ "source": "round15", "edited": true })),
    })
    .await
    .expect("update message")
    .value
    .data
    .expect("updated message data");
    assert_eq!(updated_msg.extra_metadata["edited"], true);

    let messages = messages_list(
        openhuman_core::openhuman::memory::ConversationMessagesRequest {
            thread_id: thread_id.clone(),
        },
    )
    .await
    .expect("list messages")
    .value
    .data
    .expect("messages data");
    assert_eq!(messages.count, 1);

    let generated = thread_generate_title(GenerateConversationThreadTitleRequest {
        thread_id: thread_id.clone(),
        assistant_message: None,
    })
    .await
    .expect("fallback title generation")
    .value
    .data
    .expect("generated title data");
    assert_ne!(generated.title, created.title);

    let blank_title = thread_update_title(UpdateConversationThreadTitleRequest {
        thread_id: thread_id.clone(),
        title: "   ".to_string(),
    })
    .await
    .expect_err("blank title rejected");
    assert!(blank_title.contains("title must not be empty"));

    let titled = thread_update_title(UpdateConversationThreadTitleRequest {
        thread_id: thread_id.clone(),
        title: "Round15 Manual Title".to_string(),
    })
    .await
    .expect("manual title")
    .value
    .data
    .expect("manual title data");
    assert_eq!(titled.title, "Round15 Manual Title");

    let raw = harness
        .workspace
        .join("session_raw/1715000015_welcome_thread-round15.jsonl");
    write_transcript(&raw, "welcome_thread-round15", &thread_id);
    let md = harness
        .workspace
        .join("sessions/2026_05_15/1715000015_welcome_thread-round15.md");
    std::fs::create_dir_all(md.parent().expect("md parent")).expect("md dir");
    std::fs::write(&md, "# welcome transcript\n").expect("write md companion");

    let migration = migrate_welcome_agent_artifacts(&harness.workspace).expect("welcome migration");
    assert_eq!(migration.threads_updated, 1);
    assert_eq!(migration.transcripts_updated, 1);
    assert_eq!(migration.transcript_files_renamed, 1);
    assert_eq!(migration.markdown_files_renamed, 1);
    let skipped = migrate_welcome_agent_artifacts(&harness.workspace).expect("migration marker");
    assert!(skipped.already_done);

    let listed = threads_list(EmptyRequest {})
        .await
        .expect("list threads")
        .value
        .data
        .expect("threads data");
    let summary = listed
        .threads
        .iter()
        .find(|thread| thread.id == thread_id)
        .expect("created thread listed");
    assert!(!summary.labels.iter().any(|label| label == "onboarding"));
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
            query,
            body: body.clone(),
        });

    match (method, path.as_str()) {
        (Method::GET, "/agent-integrations/composio/toolkits") => ok(json!({
            "toolkits": ["gmail", "github", "slack"]
        })),
        (Method::GET, "/agent-integrations/composio/connections") => ok(json!({
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
                    "status": "EXPIRED",
                    "createdAt": "2026-05-28T12:00:00Z"
                },
                {
                    "id": "conn-slack",
                    "toolkit": "slack",
                    "status": "CONNECTED",
                    "createdAt": "2026-05-27T12:00:00Z"
                }
            ]
        })),
        (Method::POST, "/agent-integrations/composio/authorize") => ok(json!({
            "connectUrl": "https://connect.example/round15",
            "connectionId": "conn-authorized-round15"
        })),
        (Method::GET, "/agent-integrations/composio/tools") => ok(json!({
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "GMAIL_FETCH_EMAILS",
                        "description": "Fetch Gmail messages",
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
                        "name": "GMAIL_SEND_EMAIL",
                        "description": "Send Gmail messages",
                        "parameters": { "type": "object" }
                    }
                },
                {
                    "type": "function",
                    "function": {
                        "name": "GITHUB_STAR_A_REPOSITORY_FOR_THE_AUTHENTICATED_USER",
                        "description": "Star repository",
                        "parameters": { "type": "object" }
                    }
                },
                {
                    "type": "function",
                    "function": {
                        "name": "SLACK_FETCH_CONVERSATION_HISTORY",
                        "description": "Fetch Slack history",
                        "parameters": { "type": "object" }
                    }
                }
            ]
        })),
        (Method::POST, "/agent-integrations/composio/execute") => {
            match body.get("tool").and_then(Value::as_str) {
                Some("GMAIL_FETCH_EMAILS") => ok(json!({
                    "data": { "messages": [{ "id": "msg-round15" }] },
                    "successful": true,
                    "error": null,
                    "costUsd": 0.03,
                    "markdownFormatted": "Fetched 1 inbox message"
                })),
                Some("GMAIL_SEND_EMAIL") => ok(json!({
                    "data": null,
                    "successful": false,
                    "error": "missing required field `body`",
                    "costUsd": 0.0
                })),
                other => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "success": false,
                        "error": format!("unexpected execute tool: {other:?}")
                    })),
                )
                    .into_response(),
            }
        }
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": format!("unhandled {path}") })),
        )
            .into_response(),
    }
}

async fn composio_direct_connected_accounts() -> Json<Value> {
    Json(json!({ "items": [] }))
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

fn write_transcript(path: &Path, agent: &str, thread_id: &str) {
    let body = format!(
        "{{\"_meta\":{{\"agent\":\"{agent}\",\"dispatcher\":\"native\",\"created\":\"2026-05-15T00:00:00Z\",\"updated\":\"2026-05-15T00:00:00Z\",\"turn_count\":1,\"input_tokens\":0,\"output_tokens\":0,\"cached_input_tokens\":0,\"charged_amount_usd\":0.0,\"thread_id\":\"{thread_id}\"}}}}\n{{\"role\":\"user\",\"content\":\"hi\"}}\n"
    );
    std::fs::create_dir_all(path.parent().expect("transcript parent")).expect("transcript dir");
    std::fs::write(path, body).expect("write transcript");
}
