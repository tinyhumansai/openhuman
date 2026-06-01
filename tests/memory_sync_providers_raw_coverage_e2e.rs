//! Focused raw coverage for Composio memory-sync providers.
//!
//! These tests stay local: temp workspaces plus a loopback backend that
//! returns Composio execute envelopes. Run with `--test-threads=1` because
//! config, HOME, and OPENHUMAN_WORKSPACE are process globals.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::TempDir;

use openhuman_core::core::event_bus::{DomainEvent, EventHandler};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::memory::global as memory_global;
use openhuman_core::openhuman::memory_sync::composio::bus::{
    ComposioConfigChangedSubscriber, ComposioConnectionCreatedSubscriber, ComposioTriggerSubscriber,
};
use openhuman_core::openhuman::memory_sync::composio::providers::clickup::ClickUpProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::github::GitHubProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::gmail::ingest as gmail_ingest;
use openhuman_core::openhuman::memory_sync::composio::providers::slack::ingest as slack_ingest;
use openhuman_core::openhuman::memory_sync::composio::providers::slack::{
    SlackMessage, SlackProvider,
};
use openhuman_core::openhuman::memory_sync::composio::providers::{
    ComposioProvider, ProviderContext, SyncReason, TaskFetchFilter,
};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl Into<String>) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value.into()) };
        Self { key, old }
    }

    fn set_path(key: &'static str, value: &Path) -> Self {
        Self::set(key, value.to_string_lossy().into_owned())
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::remove_var(key) };
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

fn config_in(tmp: &TempDir) -> Config {
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    config
}

async fn persist_config(config: &Config) {
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
    config.save().await.expect("save config");
}

fn store_session(config: &Config) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "round17-session-token",
            HashMap::new(),
            true,
        )
        .expect("store app session token");
}

async fn loopback_router(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("loopback addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve loopback");
    });
    (format!("http://{addr}"), handle)
}

fn execute_envelope(data: Value) -> Value {
    json!({
        "success": true,
        "data": {
            "data": data,
            "successful": true,
            "error": null,
            "costUsd": 0.0
        }
    })
}

fn execute_response_for(body: &Value) -> Value {
    let tool = body.get("tool").and_then(Value::as_str).unwrap_or("");
    let args = body.get("arguments").cloned().unwrap_or_else(|| json!({}));
    match tool {
        "SLACK_TEST_AUTH" => execute_envelope(json!({
            "user_id": "U17A",
            "user": "round17",
            "team": "Coverage Workspace",
            "team_id": "T17",
            "url": "https://coverage.slack.com"
        })),
        "SLACK_RETRIEVE_DETAILED_USER_INFORMATION" => execute_envelope(json!({
            "user": {
                "real_name": "Round Seventeen",
                "profile": {
                    "email": "round17@example.test",
                    "image_192": "https://example.test/avatar.png"
                }
            }
        })),
        "SLACK_FETCH_TEAM_INFO" => execute_envelope(json!({
            "team": {
                "email_domain": "example.test",
                "icon": { "image_132": "https://example.test/team.png" }
            }
        })),
        "SLACK_LIST_ALL_USERS" => {
            let has_cursor = args.get("cursor").is_some();
            execute_envelope(json!({
                "members": [
                    {
                        "id": if has_cursor { "U17B" } else { "U17A" },
                        "profile": {
                            "display_name": if has_cursor { "" } else { "Ava" },
                            "real_name": if has_cursor { "Ben" } else { "" }
                        },
                        "name": if has_cursor { "ben" } else { "ava" }
                    },
                    { "id": "", "name": "dropped" }
                ],
                "response_metadata": {
                    "next_cursor": if has_cursor { "" } else { "page-2" }
                }
            }))
        }
        "SLACK_LIST_CONVERSATIONS" => execute_envelope(json!({
            "channels": [
                { "id": "C17", "name": "coverage", "is_private": false },
                { "id": "G17", "name": "private-coverage", "is_private": true },
                { "id": "", "name": "dropped" }
            ],
            "response_metadata": { "next_cursor": "" }
        })),
        "SLACK_FETCH_CONVERSATION_HISTORY" => {
            let channel = args.get("channel").and_then(Value::as_str).unwrap_or("");
            execute_envelope(json!({
                "messages": [
                    {
                        "ts": if channel == "G17" { "1714004200.000300" } else { "1714003200.000100" },
                        "user": "U17A",
                        "text": if channel == "G17" { "private note for <@U17B>" } else { "shipping coverage with <@U17B>" },
                        "thread_ts": "1714003200.000100",
                        "permalink": "https://coverage.slack.com/archives/C17/p1714003200000100"
                    },
                    { "ts": "1714003300.000200", "user": "U17B", "text": "   " }
                ],
                "response_metadata": { "next_cursor": "" }
            }))
        }
        "SLACK_SEARCH_MESSAGES" => execute_envelope(json!({
            "messages": {
                "matches": [
                    {
                        "ts": "1714005200.000400",
                        "user": "U17B",
                        "text": "search backfill hit for <@U17A>",
                        "channel": { "id": "C17" },
                        "permalink": "https://coverage.slack.com/archives/C17/p1714005200000400"
                    },
                    {
                        "ts": "1714005300.000500",
                        "user": "U17B",
                        "text": "orphan match stays out",
                        "channel": { "name": "missing-id" }
                    }
                ],
                "paging": { "pages": 1 }
            }
        })),
        "GITHUB_GET_THE_AUTHENTICATED_USER" => execute_envelope(json!({
            "login": "octo-round17",
            "name": "Octo Coverage",
            "email": "octo@example.test",
            "avatar_url": "https://example.test/octo.png",
            "html_url": "https://github.com/octo-round17"
        })),
        "GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS" => execute_envelope(json!({
            "items": [
                {
                    "id": 1701,
                    "title": "Cover GitHub provider",
                    "body": "Raw provider coverage",
                    "state": "open",
                    "labels": [{ "name": "coverage" }],
                    "assignee": { "login": "octo-round17" },
                    "updated_at": "2026-05-29T10:00:00Z",
                    "html_url": "https://github.com/tinyhumansai/openhuman/issues/1701"
                },
                {
                    "title": "Missing id is skipped",
                    "updated_at": "2026-05-29T09:00:00Z"
                }
            ],
            "total_count": 2
        })),
        "CLICKUP_GET_AUTHORIZED_USER" => execute_envelope(json!({
            "user": {
                "id": 9917,
                "username": "click round17",
                "email": "click17@example.test",
                "profilePicture": "https://example.test/click.png"
            }
        })),
        "CLICKUP_GET_AUTHORIZED_TEAMS_WORKSPACES" => execute_envelope(json!({
            "teams": [
                { "id": "team_17", "name": "Coverage Team" },
                { "name": "missing id" }
            ]
        })),
        "CLICKUP_GET_FILTERED_TEAM_TASKS" => execute_envelope(json!({
            "tasks": [
                {
                    "id": "task_17",
                    "name": "Cover ClickUp provider",
                    "text_content": "Exercise task persistence",
                    "status": { "status": "to do" },
                    "assignees": [{ "username": "click round17" }],
                    "priority": { "priority": "high" },
                    "date_updated": "1798545600000",
                    "url": "https://app.clickup.com/t/task_17"
                },
                { "name": "missing id skips", "date_updated": "1798545500000" }
            ]
        })),
        _ => execute_envelope(json!({ "unknown_tool": tool, "arguments": args })),
    }
}

async fn configured_loopback_context(
    tmp: &TempDir,
    toolkit: &str,
    connection_id: &str,
    requests: Arc<Mutex<Vec<Value>>>,
) -> (Config, ProviderContext, tokio::task::JoinHandle<()>) {
    let mut config = config_in(tmp);
    let router = Router::new().route(
        "/agent-integrations/composio/execute",
        any(move |Json(body): Json<Value>| {
            let requests = Arc::clone(&requests);
            async move {
                requests.lock().unwrap().push(body.clone());
                Json(execute_response_for(&body))
            }
        }),
    );
    let (base, server) = loopback_router(router).await;
    config.api_url = Some(base);
    persist_config(&config).await;
    store_session(&config);
    memory_global::init(config.workspace_dir.clone()).expect("init global memory client");
    let ctx = ProviderContext {
        config: Arc::new(config.clone()),
        toolkit: toolkit.to_string(),
        connection_id: Some(connection_id.to_string()),
    };
    (config, ctx, server)
}

#[tokio::test]
async fn gmail_ingest_archives_account_messages_and_legacy_participant_buckets() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in(&tmp);
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    persist_config(&config).await;

    let page = vec![
        json!({
            "id": "gmail-round17-a",
            "from": "Ava <ava@example.test>",
            "to": ["Ben <ben@example.test>", "Casey <casey@example.test>"],
            "cc": "ignored@example.test",
            "subject": "Re: Coverage thread",
            "date": "2026-05-29T10:00:00Z",
            "markdown": "First useful message body."
        }),
        json!({
            "id": "gmail-round17-b",
            "from": "ben@example.test",
            "to": "ava@example.test, casey@example.test",
            "subject": "Fwd: Coverage thread",
            "internalDate": "1780052400000",
            "markdown": "Second useful message body."
        }),
        json!({
            "id": "gmail-round17-empty",
            "from": "nobody@example.test",
            "to": "ava@example.test",
            "subject": "No archive body",
            "date": "2026-05-29T12:00:00Z",
            "markdown": "   "
        }),
        json!({
            "from": "missing-id@example.test",
            "to": "ava@example.test",
            "subject": "No id",
            "date": "2026-05-29T13:00:00Z",
            "markdown": "missing id skips per-account ingest"
        }),
    ];

    let chunks = gmail_ingest::ingest_page_into_memory_tree(
        &config,
        "owner-round17",
        Some("round17@example.test"),
        &page,
    )
    .await
    .expect("per-account gmail ingest");
    assert!(chunks >= 2, "expected useful account messages to chunk");

    let raw_root = config.memory_tree_content_root().join("raw");
    let archived: Vec<_> = walk_files(&raw_root)
        .into_iter()
        .filter(|p| p.to_string_lossy().contains("gmail-round17-a"))
        .collect();
    assert_eq!(archived.len(), 1, "raw archive should include message a");
    let archived_body = std::fs::read_to_string(&archived[0]).expect("archived body");
    assert!(archived_body.contains("**From:** Ava"));
    assert!(archived_body.contains("First useful message body."));

    let legacy = gmail_ingest::ingest_page_into_memory_tree(
        &config,
        "owner-round17",
        None,
        &[
            json!({
                "id": "legacy-orphan",
                "from": "not an address",
                "to": [],
                "subject": "Fw: ",
                "date": "2026-05-29T14:00:00Z",
                "markdown": "orphan fallback body"
            }),
            json!({
                "from": "",
                "to": [],
                "subject": "Skipped",
                "date": "2026-05-29T15:00:00Z",
                "markdown": "no id and no participants"
            }),
        ],
    )
    .await
    .expect("legacy gmail ingest");
    assert!(legacy >= 1, "orphan fallback bucket should ingest");
}

#[tokio::test]
async fn slack_provider_profile_postprocess_trigger_and_ingest_use_loopback_composio() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (config, ctx, server) =
        configured_loopback_context(&tmp, "slack", "conn-slack-round17", Arc::clone(&requests))
            .await;

    let provider = SlackProvider::new();
    let profile = provider
        .fetch_user_profile(&ctx)
        .await
        .expect("slack profile");
    assert_eq!(profile.username.as_deref(), Some("U17A"));
    assert_eq!(profile.email.as_deref(), Some("round17@example.test"));

    let mut channels = json!({
        "data": {
            "channels": [
                { "id": "C17", "name": "coverage", "is_private": false },
                { "id": "", "name": "dropped" }
            ]
        }
    });
    provider.post_process_action_result("SLACK_LIST_CONVERSATIONS", None, &mut channels);
    assert_eq!(channels["channels"].as_array().unwrap().len(), 1);

    let mut history = json!({
        "data": {
            "messages": [
                {
                    "ts": "1714003200.000100",
                    "user": "U17A",
                    "text": "shipping coverage with <@U17B>",
                    "permalink": "https://coverage.slack.com/archives/C17/p1714003200000100"
                },
                { "ts": "1714003300.000200", "user": "U17B", "text": "   " }
            ]
        }
    });
    provider.post_process_action_result("SLACK_FETCH_CONVERSATION_HISTORY", None, &mut history);
    assert_eq!(history["messages"].as_array().unwrap().len(), 1);

    provider
        .on_trigger(
            &ctx,
            "SLACK_CHANNEL_ARCHIVE",
            &json!({ "event": "channel" }),
        )
        .await
        .expect("slack non-message trigger");

    let messages = vec![
        SlackMessage {
            channel_id: "C17".to_string(),
            channel_name: "coverage".to_string(),
            is_private: false,
            author: "Ava".to_string(),
            author_id: "U17A".to_string(),
            text: "Slack raw archive body".to_string(),
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-05-29T10:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            ts_raw: "1714003200.000100".to_string(),
            thread_ts: Some("1714003200.000100".to_string()),
            permalink: Some(
                "https://coverage.slack.com/archives/C17/p1714003200000100".to_string(),
            ),
        },
        SlackMessage {
            channel_id: "G17".to_string(),
            channel_name: "private-coverage".to_string(),
            is_private: true,
            author: String::new(),
            author_id: String::new(),
            text: "  ".to_string(),
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-05-29T10:01:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            ts_raw: "1714003260.000200".to_string(),
            thread_ts: None,
            permalink: None,
        },
    ];
    let chunks = slack_ingest::ingest_page_into_memory_tree(
        &config,
        "owner-round17",
        "conn-slack-round17",
        &messages,
    )
    .await
    .expect("slack ingest");
    assert!(chunks >= 1);

    let called_tools: Vec<String> = requests
        .lock()
        .unwrap()
        .iter()
        .filter_map(|b| b.get("tool").and_then(Value::as_str).map(str::to_string))
        .collect();
    assert!(called_tools.contains(&"SLACK_TEST_AUTH".to_string()));
    assert!(called_tools.contains(&"SLACK_RETRIEVE_DETAILED_USER_INFORMATION".to_string()));

    server.abort();
}

#[tokio::test]
async fn github_clickup_and_composio_bus_cover_provider_branches() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (_config, github_ctx, server) =
        configured_loopback_context(&tmp, "github", "conn-github-round17", Arc::clone(&requests))
            .await;

    let github = GitHubProvider::new();
    let github_profile = github
        .fetch_user_profile(&github_ctx)
        .await
        .expect("github profile");
    assert_eq!(github_profile.username.as_deref(), Some("octo-round17"));

    let github_tasks = github
        .fetch_tasks(
            &github_ctx,
            &TaskFetchFilter {
                repo: Some("tinyhumansai/openhuman".to_string()),
                labels: vec!["coverage".to_string()],
                state: Some("open".to_string()),
                max: 5,
                ..TaskFetchFilter::default()
            },
        )
        .await
        .expect("github tasks");
    assert_eq!(github_tasks.len(), 1);
    assert_eq!(github_tasks[0].external_id, "1701");

    let github_sync = github
        .sync(&github_ctx, SyncReason::ConnectionCreated)
        .await
        .expect("github sync");
    assert_eq!(github_sync.items_ingested, 1);

    let click_ctx = ProviderContext {
        config: github_ctx.config.clone(),
        toolkit: "clickup".to_string(),
        connection_id: Some("conn-clickup-round17".to_string()),
    };
    let clickup = ClickUpProvider::new();
    let click_profile = clickup
        .fetch_user_profile(&click_ctx)
        .await
        .expect("clickup profile");
    assert_eq!(click_profile.username.as_deref(), Some("9917"));

    let click_tasks = clickup
        .fetch_tasks(
            &click_ctx,
            &TaskFetchFilter {
                team_id: Some("team_17".to_string()),
                list_id: Some("list_17".to_string()),
                max: 5,
                ..TaskFetchFilter::default()
            },
        )
        .await
        .expect("clickup tasks");
    assert_eq!(click_tasks.len(), 1);
    assert_eq!(click_tasks[0].external_id, "task_17");

    let click_sync = clickup
        .sync(&click_ctx, SyncReason::Manual)
        .await
        .expect("clickup sync");
    assert_eq!(click_sync.items_ingested, 1);

    let trigger_sub = ComposioTriggerSubscriber::new();
    assert_eq!(trigger_sub.name(), "composio::trigger");
    assert_eq!(trigger_sub.domains().unwrap(), &["composio"]);
    trigger_sub
        .handle(&DomainEvent::ComposioTriggerReceived {
            toolkit: "slack".to_string(),
            trigger: "SLACK_MESSAGE_POSTED".to_string(),
            metadata_id: "id-round17".to_string(),
            metadata_uuid: "uuid-round17".to_string(),
            payload: json!({ "text": "hello" }),
        })
        .await;

    let connection_sub = ComposioConnectionCreatedSubscriber::new();
    assert_eq!(connection_sub.name(), "composio::connection_created");
    connection_sub
        .handle(&DomainEvent::ComposioConfigChanged {
            mode: "backend".to_string(),
            api_key_set: false,
        })
        .await;

    let config_sub = ComposioConfigChangedSubscriber::new();
    assert_eq!(config_sub.name(), "composio::config_changed");
    config_sub
        .handle(&DomainEvent::ComposioConfigChanged {
            mode: "direct".to_string(),
            api_key_set: true,
        })
        .await;

    let called_tools: Vec<String> = requests
        .lock()
        .unwrap()
        .iter()
        .filter_map(|b| b.get("tool").and_then(Value::as_str).map(str::to_string))
        .collect();
    assert!(called_tools.contains(&"GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS".to_string()));
    assert!(called_tools.contains(&"CLICKUP_GET_FILTERED_TEAM_TASKS".to_string()));

    server.abort();
}

fn walk_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if !root.exists() {
        return out;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let entries = match std::fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let child = entry.path();
            if child.is_dir() {
                stack.push(child);
            } else {
                out.push(child);
            }
        }
    }
    out
}
