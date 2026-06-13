//! Round 23 raw coverage focused on memory_sync gaps.
//!
//! Local-only: temp workspaces, loopback Composio execute responses, and no
//! real provider network. Run single-threaded because HOME,
//! OPENHUMAN_WORKSPACE, and config loading are process globals.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::TempDir;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::memory::global as memory_global;
use openhuman_core::openhuman::memory_sync::composio::providers::gmail::GmailProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::notion::NotionProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::profile::{
    delete_connected_identity_facets, is_self_identity, is_self_identity_any_toolkit,
    load_connected_identities, persist_provider_profile, render_connected_identities_section,
    IdentityKind,
};
use openhuman_core::openhuman::memory_sync::composio::providers::slack::SlackProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::{
    ComposioProvider, ProviderContext, ProviderUserProfile, SyncReason,
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
    config.memory_tree.embedding_endpoint = None;
    config.memory_tree.embedding_model = None;
    config.memory_tree.embedding_strict = false;
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
            "round23-session-token",
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

fn execute_error(error: &str) -> Value {
    json!({
        "success": true,
        "data": {
            "data": {},
            "successful": false,
            "error": error,
            "costUsd": 0.0
        }
    })
}

async fn configured_context(
    tmp: &TempDir,
    toolkit: &str,
    connection_id: &str,
    requests: Arc<Mutex<Vec<Value>>>,
    response_for: fn(&Value) -> Value,
) -> (Config, ProviderContext, tokio::task::JoinHandle<()>) {
    let mut config = config_in(tmp);
    let router = Router::new().route(
        "/agent-integrations/composio/execute",
        any(move |Json(body): Json<Value>| {
            let requests = Arc::clone(&requests);
            async move {
                requests.lock().unwrap().push(body.clone());
                Json(response_for(&body))
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
        usage: Default::default(),
        max_items: None,
        sync_depth_days: None,
    };
    (config, ctx, server)
}

fn slack_profile_response(body: &Value) -> Value {
    match body.get("tool").and_then(Value::as_str).unwrap_or("") {
        "SLACK_TEST_AUTH" => execute_envelope(json!({
            "user_id": "U23SELF",
            "user": "Round23Handle",
            "team": "Round 23 Workspace",
            "team_id": "T23",
            "url": "https://round23.slack.com"
        })),
        "SLACK_RETRIEVE_DETAILED_USER_INFORMATION" => {
            execute_error("missing_scope: users:read.email")
        }
        "SLACK_FETCH_TEAM_INFO" => execute_envelope(json!({
            "team": {
                "email_domain": "round23.example",
                "icon": { "image_132": "https://example.test/team23.png" }
            }
        })),
        other => execute_envelope(json!({ "unexpected": other })),
    }
}

fn notion_response(body: &Value) -> Value {
    let tool = body.get("tool").and_then(Value::as_str).unwrap_or("");
    let args = body.get("arguments").cloned().unwrap_or_else(|| json!({}));
    match tool {
        "NOTION_GET_ABOUT_ME" => execute_envelope(json!({
            "name": "Integration Bot",
            "id": "bot-id",
            "bot": {
                "owner": {
                    "user": {
                        "id": "notion-user-23",
                        "name": "Round Twenty Three",
                        "person": { "email": "round23@notion.test" },
                        "avatar_url": "https://example.test/notion23.png"
                    }
                }
            },
            "url": "https://notion.so/profile/round23"
        })),
        "NOTION_FETCH_DATA" => {
            if args.get("start_cursor").and_then(Value::as_str) == Some("page-2") {
                execute_envelope(json!({
                    "results": [
                        {
                            "id": "notion-page-23-b",
                            "object": "page",
                            "last_edited_time": "2026-05-29T08:00:00.000Z",
                            "properties": {
                                "Name": {
                                    "type": "title",
                                    "title": [{ "plain_text": "Second page" }]
                                }
                            },
                            "body_excerpt": "Second page proves cursor pagination."
                        },
                        {
                            "object": "page",
                            "last_edited_time": "2026-05-29T07:00:00.000Z",
                            "body_excerpt": "Missing ids are skipped."
                        }
                    ],
                    "next_cursor": null
                }))
            } else {
                execute_envelope(json!({
                    "results": [
                        {
                            "id": "notion-page-23-a",
                            "object": "page",
                            "last_edited_time": "2026-05-30T10:00:00.000Z",
                            "properties": {
                                "Name": {
                                    "type": "title",
                                    "title": [{ "plain_text": "Round 23 launch notes" }]
                                }
                            },
                            "url": "https://notion.so/notionpage23a",
                            "body_excerpt": "Alice owns launch notes. Bob handles rollback."
                        }
                    ],
                    "next_cursor": "page-2"
                }))
            }
        }
        other => execute_envelope(json!({ "unexpected": other, "args": args })),
    }
}

#[tokio::test]
async fn slack_profile_falls_back_to_auth_and_team_info_without_email_scope() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (_config, ctx, server) = configured_context(
        &tmp,
        "slack",
        "conn-slack-23",
        Arc::clone(&requests),
        slack_profile_response,
    )
    .await;

    let profile = SlackProvider::new()
        .fetch_user_profile(&ctx)
        .await
        .expect("slack profile");

    assert_eq!(profile.username.as_deref(), Some("U23SELF"));
    assert_eq!(profile.display_name.as_deref(), Some("Round23Handle"));
    assert_eq!(profile.email, None);
    assert_eq!(
        profile.avatar_url.as_deref(),
        Some("https://example.test/team23.png")
    );
    assert_eq!(
        profile.profile_url.as_deref(),
        Some("https://round23.slack.com")
    );
    assert_eq!(profile.extras["handle"], "Round23Handle");
    assert_eq!(profile.extras["team_email_domain"], "round23.example");

    let seen_tools: Vec<String> = requests
        .lock()
        .unwrap()
        .iter()
        .filter_map(|v| v.get("tool").and_then(Value::as_str).map(str::to_string))
        .collect();
    assert_eq!(
        seen_tools,
        vec![
            "SLACK_TEST_AUTH",
            "SLACK_RETRIEVE_DETAILED_USER_INFORMATION",
            "SLACK_FETCH_TEAM_INFO"
        ]
    );
    server.abort();
}

#[tokio::test]
async fn notion_profile_prefers_bot_owner_and_sync_paginates_into_memory_tree() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (_config, ctx, server) = configured_context(
        &tmp,
        "notion",
        "conn-notion-23",
        Arc::clone(&requests),
        notion_response,
    )
    .await;
    let provider = NotionProvider::new();

    let profile = provider
        .fetch_user_profile(&ctx)
        .await
        .expect("notion profile");
    assert_eq!(profile.display_name.as_deref(), Some("Round Twenty Three"));
    assert_eq!(profile.email.as_deref(), Some("round23@notion.test"));
    assert_eq!(profile.username.as_deref(), Some("notion-user-23"));
    assert_eq!(
        profile.avatar_url.as_deref(),
        Some("https://example.test/notion23.png")
    );

    let outcome = provider
        .sync(&ctx, SyncReason::ConnectionCreated)
        .await
        .expect("notion sync");
    assert_eq!(outcome.items_ingested, 2);
    assert!(outcome.summary.contains("fetched 3, persisted 2"));
    assert_eq!(outcome.details["results_fetched"], 3);
    assert_eq!(outcome.details["results_persisted"], 2);

    let calls = requests.lock().unwrap().clone();
    let fetch_calls: Vec<Value> = calls
        .iter()
        .filter(|v| v.get("tool").and_then(Value::as_str) == Some("NOTION_FETCH_DATA"))
        .cloned()
        .collect();
    assert_eq!(fetch_calls.len(), 2);
    assert_eq!(fetch_calls[0]["arguments"]["page_size"], 50);
    assert_eq!(fetch_calls[1]["arguments"]["start_cursor"], "page-2");
    server.abort();
}

#[test]
fn gmail_post_process_handles_nested_payloads_and_raw_html_opt_out() {
    let provider = GmailProvider::new();
    let mut nested = json!({
        "data": {
            "messages": [
                {
                    "messageId": "gmail-round23-a",
                    "threadId": "thread-a",
                    "subject": "Round 23 subject",
                    "sender": "Ava <ava@example.test>",
                    "to": "Ben <ben@example.test>",
                    "labelIds": ["INBOX", "UNREAD"],
                    "messageText": "fallback text should not win",
                    "markdown_formatted": "Backend markdown body",
                    "payload": {
                        "headers": [
                            { "name": "date", "value": "Sat, 30 May 2026 10:00:00 +0000" },
                            { "name": "List-Unsubscribe", "value": "<mailto:unsubscribe@example.test>" }
                        ]
                    },
                    "attachmentList": [
                        { "filename": "notes.pdf", "mimeType": "application/pdf" },
                        { "filename": "", "mimeType": "text/plain" }
                    ]
                }
            ],
            "nextPageToken": "next-23",
            "resultSizeEstimate": 1,
            "ignored": "removed"
        }
    });
    provider.post_process_action_result("GMAIL_FETCH_EMAILS", None, &mut nested);
    let slim = &nested["data"]["messages"][0];
    assert_eq!(slim["id"], "gmail-round23-a");
    assert_eq!(slim["date"], "Sat, 30 May 2026 10:00:00 +0000");
    assert_eq!(
        slim["list_unsubscribe"],
        "<mailto:unsubscribe@example.test>"
    );
    assert_eq!(slim["markdown"], "Backend markdown body");
    assert_eq!(slim["attachments"][0]["filename"], "notes.pdf");
    assert_eq!(nested["data"]["nextPageToken"], "next-23");
    assert!(nested["data"].get("ignored").is_none());

    let mut raw = json!({ "messages": [{ "messageId": "raw-23", "payload": { "parts": [] } }] });
    provider.post_process_action_result(
        "GMAIL_FETCH_EMAILS",
        Some(&json!({ "rawHtml": true })),
        &mut raw,
    );
    assert_eq!(raw["messages"][0]["messageId"], "raw-23");

    let mut untouched = json!({ "messages": [{ "messageId": "other-23" }] });
    provider.post_process_action_result("GMAIL_SEND_EMAIL", None, &mut untouched);
    assert_eq!(untouched["messages"][0]["messageId"], "other-23");
}

#[tokio::test]
async fn profile_persistence_loads_matches_renders_and_deletes_connected_identities() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let config = config_in(&tmp);
    persist_config(&config).await;
    memory_global::init(config.workspace_dir.clone()).expect("init global memory client");

    let slack = ProviderUserProfile {
        toolkit: "Slack!".to_string(),
        connection_id: Some("Conn:23".to_string()),
        display_name: Some("  Round\tTwenty\nThree  ".to_string()),
        email: Some("ROUND23@Example.TEST".to_string()),
        username: Some("U23SELF".to_string()),
        avatar_url: Some("https://example.test/avatar.png".to_string()),
        profile_url: Some("https://example.test/profile|unsafe".to_string()),
        extras: json!({ "handle": "@Round23" }),
    };
    let notion = ProviderUserProfile {
        toolkit: "notion".to_string(),
        connection_id: Some("notion-conn-23".to_string()),
        display_name: Some("Notion Owner".to_string()),
        email: Some("owner@notion.test".to_string()),
        username: Some("notion-user-23".to_string()),
        avatar_url: None,
        profile_url: None,
        extras: Value::Null,
    };

    assert_eq!(persist_provider_profile(&slack), 6);
    assert_eq!(persist_provider_profile(&notion), 3);

    assert!(is_self_identity("slack_", IdentityKind::UserId, "U23SELF"));
    assert!(is_self_identity("slack_", IdentityKind::Handle, "@round23"));
    assert!(is_self_identity_any_toolkit(
        IdentityKind::Email,
        "round23@example.test"
    ));
    assert!(!is_self_identity(
        "slack_",
        IdentityKind::AvatarUrl,
        "https://example.test/avatar.png"
    ));

    let identities = load_connected_identities();
    let slack_identity = identities
        .iter()
        .find(|id| id.source == "slack" && id.identifier == "conn_23")
        .expect("slack identity loaded");
    assert_eq!(
        slack_identity.email.as_deref(),
        Some("round23@example.test")
    );
    assert_eq!(slack_identity.handle.as_deref(), Some("round23"));
    assert_eq!(slack_identity.user_id.as_deref(), Some("U23SELF"));

    let rendered = render_connected_identities_section(&identities);
    assert!(rendered.contains("Round Twenty Three"));
    assert!(rendered.contains("@round23"));
    assert!(rendered.contains("https://example.test/profile/unsafe"));

    let deleted = delete_connected_identity_facets("Slack!", "Conn:23");
    assert_eq!(deleted, 6);
    assert!(!is_self_identity("slack_", IdentityKind::UserId, "U23SELF"));
    assert!(is_self_identity(
        "notion",
        IdentityKind::UserId,
        "notion-user-23"
    ));
}
