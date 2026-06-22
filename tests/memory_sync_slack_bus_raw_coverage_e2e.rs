//! Round19 raw coverage for Slack memory sync, Composio bus subscribers,
//! and Gmail post-processing.
//!
//! Everything stays local: temp workspaces plus a loopback backend that
//! returns Composio execute envelopes. Run single-threaded because HOME,
//! OPENHUMAN_WORKSPACE, and config loading are process globals.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
use openhuman_core::openhuman::memory_sync::composio::providers::gmail::GmailProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::slack::{
    run_backfill_via_search, SlackProvider,
};
use openhuman_core::openhuman::memory_sync::composio::providers::{
    ComposioProvider, ProviderContext, SyncReason,
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
        action_dir: tmp.path().join("workspace"),
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
            "round19-session-token",
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
            "user_id": "U19A",
            "user": "round19",
            "team": "Round19 Workspace",
            "team_id": "T19",
            "url": "https://round19.slack.com"
        })),
        "SLACK_RETRIEVE_DETAILED_USER_INFORMATION" => execute_envelope(json!({
            "user": {
                "real_name": "Round Nineteen",
                "profile": {
                    "email": "round19@example.test",
                    "image_192": "https://example.test/r19.png"
                }
            }
        })),
        "SLACK_FETCH_TEAM_INFO" => execute_envelope(json!({
            "team": {
                "email_domain": "example.test",
                "icon": { "image_132": "https://example.test/team19.png" }
            }
        })),
        "SLACK_LIST_ALL_USERS" => {
            let has_cursor = args.get("cursor").is_some();
            execute_envelope(json!({
                "members": [
                    {
                        "id": if has_cursor { "U19B" } else { "U19A" },
                        "profile": {
                            "display_name": if has_cursor { "" } else { "Ava Round19" },
                            "real_name": if has_cursor { "Ben Round19" } else { "" }
                        },
                        "name": if has_cursor { "ben19" } else { "ava19" }
                    },
                    { "id": "", "name": "dropped" }
                ],
                "response_metadata": {
                    "next_cursor": if has_cursor { "" } else { "users-page-2" }
                }
            }))
        }
        "SLACK_LIST_CONVERSATIONS" => {
            let has_cursor = args.get("cursor").is_some();
            execute_envelope(json!({
                "channels": if has_cursor {
                    json!([
                        { "id": "G19", "name": "private-coverage", "is_private": true }
                    ])
                } else {
                    json!([
                        { "id": "C19", "name": "coverage", "is_private": false },
                        { "id": "", "name": "dropped" }
                    ])
                },
                "response_metadata": {
                    "next_cursor": if has_cursor { "" } else { "channels-page-2" }
                }
            }))
        }
        "SLACK_FETCH_CONVERSATION_HISTORY" => {
            let channel = args.get("channel").and_then(Value::as_str).unwrap_or("");
            execute_envelope(json!({
                "messages": [
                    {
                        "ts": if channel == "G19" { "1714004200.000300" } else { "1714003200.000100" },
                        "user": "U19A",
                        "text": if channel == "G19" {
                            "private sync note for <@U19B>"
                        } else {
                            "shipping Slack sync coverage with <@U19B>"
                        },
                        "thread_ts": "1714003200.000100",
                        "permalink": "https://round19.slack.com/archives/C19/p1714003200000100"
                    },
                    {
                        "ts": "1714003300.000200",
                        "bot_id": "B19",
                        "text": "bot authored update"
                    },
                    { "ts": "1714003400.000300", "user": "U19B", "text": "   " }
                ],
                "response_metadata": { "next_cursor": "" }
            }))
        }
        "SLACK_SEARCH_MESSAGES" => execute_envelope(json!({
            "messages": {
                "matches": [
                    {
                        "ts": "1714005200.000400",
                        "user": "U19B",
                        "text": "search backfill hit for <@U19A>",
                        "channel": { "id": "C19" },
                        "permalink": "https://round19.slack.com/archives/C19/p1714005200000400"
                    },
                    {
                        "ts": "1714005300.000500",
                        "user": "U19B",
                        "text": "orphan match should be dropped",
                        "channel": { "name": "missing-id" }
                    }
                ],
                "paging": { "pages": 1 }
            }
        })),
        _ => execute_envelope(json!({ "unknown_tool": tool, "arguments": args })),
    }
}

async fn configured_loopback_context(
    tmp: &TempDir,
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
        toolkit: "slack".to_string(),
        connection_id: Some("conn-slack-round19".to_string()),
        usage: Default::default(),
        max_items: None,
        sync_depth_days: None,
    };
    (config, ctx, server)
}

#[tokio::test]
async fn slack_full_sync_search_backfill_and_bus_use_loopback_composio() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let dump_dir = tmp.path().join("slack-dumps");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    let _triage_off = EnvGuard::set("OPENHUMAN_TRIGGER_TRIAGE_DISABLED", "1");
    let _pacing = EnvGuard::set("OPENHUMAN_SLACK_INTER_CALL_PACING_MS", "0");
    let _backfill = EnvGuard::set("OPENHUMAN_SLACK_BACKFILL_DAYS", "1");
    let _dump = EnvGuard::set_path("OPENHUMAN_SLACK_DUMP_DIR", &dump_dir);

    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (config, ctx, server) = configured_loopback_context(&tmp, Arc::clone(&requests)).await;

    let provider = SlackProvider::new();
    let profile = provider
        .fetch_user_profile(&ctx)
        .await
        .expect("slack profile");
    assert_eq!(profile.username.as_deref(), Some("U19A"));
    assert_eq!(profile.display_name.as_deref(), Some("Round Nineteen"));
    assert_eq!(profile.email.as_deref(), Some("round19@example.test"));
    assert_eq!(profile.extras["team_name"], "Round19 Workspace");

    let outcome = provider
        .sync(&ctx, SyncReason::Manual)
        .await
        .expect("slack full sync");
    assert_eq!(outcome.toolkit, "slack");
    assert_eq!(outcome.connection_id.as_deref(), Some("conn-slack-round19"));
    assert_eq!(outcome.items_ingested, 4);
    // Slack now rides the generic orchestrator: two channels synced cleanly,
    // none errored. (`channels_processed` → orchestrator's `scopes_synced`.)
    assert_eq!(outcome.details["scopes_synced"], 2);
    assert_eq!(outcome.details["scopes_errored"], 0);

    let search = run_backfill_via_search(&ctx, 2)
        .await
        .expect("slack search backfill");
    assert_eq!(search.items_ingested, 1);
    assert_eq!(search.details["channels_flushed"], 1);
    assert_eq!(search.details["channels_failed"], 0);

    let raw_root = config.memory_tree_content_root().join("raw");
    let raw_files = walk_files(&raw_root);
    let raw_bodies = raw_files
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(raw_bodies.contains("shipping Slack sync coverage with @Ben Round19"));
    assert!(raw_bodies.contains("private sync note for @Ben Round19"));
    assert!(raw_bodies.contains("search backfill hit for @Ava Round19"));
    assert!(raw_bodies.contains("**Channel:** #coverage"));
    assert!(raw_bodies.contains("**Channel:** private:private-coverage"));

    let dumped = walk_files(&dump_dir);
    assert!(
        dumped.iter().any(|p| p.to_string_lossy().contains("users")),
        "user directory response should be dumped"
    );
    assert!(
        dumped
            .iter()
            .any(|p| p.to_string_lossy().contains("history")),
        "history responses should be dumped"
    );

    let trigger_sub = ComposioTriggerSubscriber::new();
    assert_eq!(trigger_sub.name(), "composio::trigger");
    assert_eq!(trigger_sub.domains().unwrap(), &["composio"]);
    trigger_sub
        .handle(&DomainEvent::ComposioTriggerReceived {
            toolkit: "slack".to_string(),
            trigger: "SLACK_MESSAGE_POSTED".to_string(),
            metadata_id: "id-round19".to_string(),
            metadata_uuid: "uuid-round19".to_string(),
            payload: json!({ "text": "bus coverage" }),
        })
        .await;

    let connection_sub = ComposioConnectionCreatedSubscriber::new();
    assert_eq!(connection_sub.name(), "composio::connection_created");
    assert_eq!(connection_sub.domains().unwrap(), &["composio"]);
    connection_sub
        .handle(&DomainEvent::ComposioConfigChanged {
            mode: "backend".to_string(),
            api_key_set: false,
        })
        .await;

    let config_sub = ComposioConfigChangedSubscriber::new();
    assert_eq!(config_sub.name(), "composio::config_changed");
    assert_eq!(config_sub.domains().unwrap(), &["composio"]);
    config_sub
        .handle(&DomainEvent::ComposioConfigChanged {
            mode: "direct".to_string(),
            api_key_set: true,
        })
        .await;

    let calls = requests.lock().unwrap().clone();
    let called_tools: Vec<String> = calls
        .iter()
        .filter_map(|b| b.get("tool").and_then(Value::as_str).map(str::to_string))
        .collect();
    assert!(called_tools.contains(&"SLACK_LIST_ALL_USERS".to_string()));
    assert!(called_tools.contains(&"SLACK_LIST_CONVERSATIONS".to_string()));
    assert!(called_tools.contains(&"SLACK_FETCH_CONVERSATION_HISTORY".to_string()));
    assert!(called_tools.contains(&"SLACK_SEARCH_MESSAGES".to_string()));

    let history_args: Vec<Value> = calls
        .iter()
        .filter(|b| {
            b.get("tool").and_then(Value::as_str) == Some("SLACK_FETCH_CONVERSATION_HISTORY")
        })
        .filter_map(|b| b.get("arguments").cloned())
        .collect();
    assert!(history_args
        .iter()
        .any(|args| args.get("channel").and_then(Value::as_str) == Some("C19")));
    assert!(history_args
        .iter()
        .any(|args| args.get("channel").and_then(Value::as_str) == Some("G19")));
    assert!(history_args.iter().all(|args| {
        args.get("inclusive").and_then(Value::as_bool) == Some(false)
            && args.get("oldest").and_then(Value::as_str).is_some()
    }));

    server.abort();
}

#[tokio::test]
async fn gmail_post_process_reshapes_nested_messages_and_honors_raw_html_flag() {
    let _guard = env_lock();
    let provider = GmailProvider::new();

    let mut data = json!({
        "data": {
            "messages": [
                {
                    "messageId": "gmail-round19-a",
                    "threadId": "thread-a",
                    "subject": "Round19 A",
                    "sender": "Ava <ava@example.test>",
                    "to": ["Ben <ben@example.test>"],
                    "messageText": "Plain fallback body",
                    "labelIds": ["INBOX", "UNREAD"],
                    "attachmentList": [
                        { "filename": "notes.pdf", "mimeType": "application/pdf" },
                        { "filename": "", "mimeType": "text/plain" }
                    ],
                    "payload": {
                        "headers": [
                            { "name": "Date", "value": "Fri, 29 May 2026 10:00:00 GMT" },
                            { "name": "List-Unsubscribe", "value": "<mailto:unsubscribe@example.test>" }
                        ]
                    }
                }
            ],
            "nextPageToken": "next-round19",
            "resultSizeEstimate": 7
        }
    });
    provider.post_process_action_result("GMAIL_FETCH_EMAILS", None, &mut data);
    let msg = &data["data"]["messages"][0];
    assert_eq!(msg["id"], "gmail-round19-a");
    assert_eq!(msg["threadId"], "thread-a");
    assert_eq!(msg["markdown"], "Plain fallback body");
    assert_eq!(msg["labels"][0], "INBOX");
    assert_eq!(msg["attachments"][0]["filename"], "notes.pdf");
    assert_eq!(msg["list_unsubscribe"], "<mailto:unsubscribe@example.test>");
    assert_eq!(data["data"]["nextPageToken"], "next-round19");
    assert_eq!(data["data"]["resultSizeEstimate"], 7);

    let mut raw_passthrough = json!({
        "messages": [
            { "messageId": "raw-round19", "messageText": "<b>keep raw</b>" }
        ]
    });
    provider.post_process_action_result(
        "GMAIL_FETCH_EMAILS",
        Some(&json!({ "rawHtml": true })),
        &mut raw_passthrough,
    );
    assert_eq!(raw_passthrough["messages"][0]["messageId"], "raw-round19");
    assert!(raw_passthrough["messages"][0].get("markdown").is_none());
}

fn walk_files(root: &Path) -> Vec<PathBuf> {
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
