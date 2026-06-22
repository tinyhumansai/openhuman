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
use openhuman_core::openhuman::memory::jobs::drain_until_idle;
use openhuman_core::openhuman::memory::tree_source::get_or_create_source_tree;
use openhuman_core::openhuman::memory_store::chunks::store::{
    count_chunks_by_lifecycle_status, get_chunk_raw_refs, list_chunks, ListChunksQuery,
    CHUNK_STATUS_BUFFERED,
};
use openhuman_core::openhuman::memory_store::chunks::types::SourceKind;
use openhuman_core::openhuman::memory_store::content::read::read_chunk_body;
use openhuman_core::openhuman::memory_store::trees::store as tree_store;
use openhuman_core::openhuman::memory_sync::composio::bus::{
    ComposioConfigChangedSubscriber, ComposioConnectionCreatedSubscriber, ComposioTriggerSubscriber,
};
use openhuman_core::openhuman::memory_sync::composio::providers::clickup::ClickUpProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::github::GitHubProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::gmail::ingest as gmail_ingest;
use openhuman_core::openhuman::memory_sync::composio::providers::gmail::GmailProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::linear::LinearProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::notion::NotionProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::slack::ingest as slack_ingest;
use openhuman_core::openhuman::memory_sync::composio::providers::slack::{
    SlackMessage, SlackProvider,
};
use openhuman_core::openhuman::memory_sync::composio::providers::{
    ComposioProvider, ProviderContext, SyncReason, TaskFetchFilter,
};
use openhuman_core::openhuman::memory_tree::tree::bucket_seal::LabelStrategy;

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
        usage: Default::default(),
        max_items: None,
        sync_depth_days: None,
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
        .filter(|p| {
            p.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains("gmail-round17-a"))
        })
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
async fn gmail_raw_backed_messages_drain_into_source_tree_summary() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    let config = config_in(&tmp);
    persist_config(&config).await;

    let page = vec![
        json!({
            "id": "gmail-tree-a",
            "from": "Ava <ava@example.test>",
            "to": ["Ben <ben@example.test>"],
            "subject": "Phoenix launch plan",
            "date": "2026-05-29T10:00:00Z",
            "markdown": "Phoenix migration launches Friday. Ava owns rollout validation and Ben owns customer notices."
        }),
        json!({
            "id": "gmail-tree-b",
            "from": "Ben <ben@example.test>",
            "to": ["Ava <ava@example.test>"],
            "subject": "Re: Phoenix launch plan",
            "date": "2026-05-29T10:05:00Z",
            "markdown": "Confirmed. Customer notices go out after staging checks and the rollback doc is reviewed."
        }),
    ];

    let outcome = gmail_ingest::ingest_page_into_memory_tree_with_outcome(
        &config,
        "owner-gmail-tree",
        Some("flow@example.test"),
        &page,
    )
    .await
    .expect("gmail ingest outcome");

    assert_eq!(
        outcome.item_ids_ingested,
        vec!["gmail-tree-a".to_string(), "gmail-tree-b".to_string()]
    );
    assert!(
        outcome.chunks_written >= 2,
        "expected one or more chunks per message"
    );

    let source_id = "gmail:flow-at-example-dot-test";
    let chunks = list_chunks(
        &config,
        &ListChunksQuery {
            source_kind: Some(SourceKind::Email),
            source_id: Some(source_id.to_string()),
            limit: Some(10),
            ..Default::default()
        },
    )
    .expect("list gmail chunks");
    assert_eq!(chunks.len(), outcome.chunks_written);

    for chunk in &chunks {
        let refs = get_chunk_raw_refs(&config, &chunk.id)
            .expect("raw refs lookup")
            .expect("raw refs must be set before extract can run");
        assert_eq!(refs.len(), 1);
        assert!(
            refs[0].path.contains("gmail-tree-"),
            "raw ref should point at the source message file: {:?}",
            refs[0].path
        );
        let full_body = read_chunk_body(&config, &chunk.id).expect("read raw-backed chunk body");
        assert!(
            full_body.contains("Phoenix") || full_body.contains("Customer notices"),
            "chunk body should hydrate from raw archive, got: {full_body}"
        );
    }

    drain_until_idle(&config)
        .await
        .expect("extract and append jobs should drain");
    let buffered =
        count_chunks_by_lifecycle_status(&config, CHUNK_STATUS_BUFFERED).expect("buffered count");
    assert_eq!(buffered, outcome.chunks_written as u64);

    let tree = get_or_create_source_tree(&config, source_id).expect("source tree");
    let l0 = tree_store::get_buffer(&config, &tree.id, 0).expect("source L0 buffer");
    assert_eq!(
        l0.item_ids.len(),
        outcome.chunks_written,
        "all Gmail chunks should reach the source tree buffer"
    );

    let sealed = openhuman_core::openhuman::memory_tree::tree::flush::flush_stale_buffers(
        &config,
        chrono::Duration::zero(),
        &LabelStrategy::Empty,
    )
    .await
    .expect("force flush gmail source tree");
    assert!(sealed > 0, "low-volume Gmail source should seal on flush");

    let l1 =
        tree_store::list_summaries_at_level(&config, &tree.id, 1).expect("list source summaries");
    assert!(
        !l1.is_empty(),
        "Gmail source tree should have a sealed summary after flush"
    );
    let summary_body = openhuman_core::openhuman::memory_store::content::read::read_summary_body(
        &config, &l1[0].id,
    )
    .expect("read gmail summary body");
    assert!(
        summary_body.contains("Phoenix") || summary_body.contains("Customer"),
        "summary should preserve Gmail content, got: {summary_body}"
    );
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
        usage: Default::default(),
        max_items: None,
        sync_depth_days: None,
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

// ─────────────────────────────────────────────────────────────────────────
// Slack cap enforcement
//
// Proves that max_items=N caps Slack ingestion to exactly N messages even
// when a single channel/page returns more than N messages. The mock returns
// 5 messages in one channel; with max_items=2 only 2 must be persisted.
// ─────────────────────────────────────────────────────────────────────────

/// Build a loopback router for the Slack cap test. Returns 5 messages for
/// the single channel when `SLACK_FETCH_CONVERSATION_HISTORY` is called;
/// all other Slack bootstrap calls (auth, users, channel listing) return
/// minimal but valid responses.
fn slack_cap_router(requests: Arc<Mutex<Vec<Value>>>) -> Router {
    Router::new().route(
        "/agent-integrations/composio/execute",
        any(move |Json(body): Json<Value>| {
            let requests = Arc::clone(&requests);
            async move {
                requests.lock().unwrap().push(body.clone());
                let tool = body.get("tool").and_then(Value::as_str).unwrap_or("");
                let resp = match tool {
                    "SLACK_TEST_AUTH" => execute_envelope(json!({
                        "user_id": "UCAP",
                        "user": "capuser",
                        "team": "Cap Workspace",
                        "team_id": "TCAP",
                        "url": "https://cap.slack.com"
                    })),
                    "SLACK_RETRIEVE_DETAILED_USER_INFORMATION" => execute_envelope(json!({
                        "user": {
                            "real_name": "Cap User",
                            "profile": { "email": "cap@example.test" }
                        }
                    })),
                    "SLACK_FETCH_TEAM_INFO" => execute_envelope(json!({
                        "team": { "email_domain": "example.test" }
                    })),
                    // User directory — one member, no next page.
                    "SLACK_LIST_ALL_USERS" => execute_envelope(json!({
                        "members": [
                            { "id": "UCAP", "name": "capuser", "profile": { "real_name": "Cap User" } }
                        ],
                        "response_metadata": { "next_cursor": "" }
                    })),
                    // One channel, no next page.
                    "SLACK_LIST_CONVERSATIONS" => execute_envelope(json!({
                        "channels": [
                            { "id": "CCAP", "name": "cap-channel", "is_private": false }
                        ],
                        "response_metadata": { "next_cursor": "" }
                    })),
                    // Return 5 distinct messages for the single channel;
                    // the cap is 2 so only 2 must be persisted.
                    "SLACK_FETCH_CONVERSATION_HISTORY" => execute_envelope(json!({
                        "messages": [
                            { "ts": "1800000001.000001", "user": "UCAP", "text": "cap message 1",
                              "permalink": "https://cap.slack.com/archives/CCAP/p1800000001000001" },
                            { "ts": "1800000002.000002", "user": "UCAP", "text": "cap message 2",
                              "permalink": "https://cap.slack.com/archives/CCAP/p1800000002000002" },
                            { "ts": "1800000003.000003", "user": "UCAP", "text": "cap message 3",
                              "permalink": "https://cap.slack.com/archives/CCAP/p1800000003000003" },
                            { "ts": "1800000004.000004", "user": "UCAP", "text": "cap message 4",
                              "permalink": "https://cap.slack.com/archives/CCAP/p1800000004000004" },
                            { "ts": "1800000005.000005", "user": "UCAP", "text": "cap message 5",
                              "permalink": "https://cap.slack.com/archives/CCAP/p1800000005000005" }
                        ],
                        "response_metadata": { "next_cursor": "" }
                    })),
                    _ => execute_envelope(json!({})),
                };
                Json(resp)
            }
        }),
    )
}

#[tokio::test]
async fn slack_sync_max_items_caps_ingest_to_exact_count() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    // Disable inter-call pacing so the test runs quickly.
    let _pacing = EnvGuard::set("OPENHUMAN_SLACK_INTER_CALL_PACING_MS", "0");

    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (base, server) = loopback_router(slack_cap_router(Arc::clone(&requests))).await;

    let mut config = config_in(&tmp);
    config.api_url = Some(base);
    persist_config(&config).await;
    store_session(&config);
    memory_global::init(config.workspace_dir.clone()).expect("init global memory");

    let ctx = ProviderContext {
        config: Arc::new(config),
        toolkit: "slack".to_string(),
        connection_id: Some("conn-slack-cap".to_string()),
        usage: Default::default(),
        // The mock returns 5 messages; cap is 2 — only 2 must be ingested.
        max_items: Some(2),
        sync_depth_days: None,
    };

    let outcome = SlackProvider::new()
        .sync(&ctx, SyncReason::ConnectionCreated)
        .await
        .expect("slack cap sync");

    assert_eq!(
        outcome.items_ingested, 2,
        "max_items=2 must cap Slack ingest to exactly 2 even though the channel/page held 5"
    );

    server.abort();
}

// ─────────────────────────────────────────────────────────────────────────
// Gmail cap enforcement
//
// Proves that max_items=N caps Gmail ingestion to exactly N messages even
// when a single GMAIL_FETCH_EMAILS page returns more than N. The mock
// returns 5 messages in one page; with max_items=2 only 2 must be
// persisted.
//
// Gmail messages arrive in Composio's "upstream" shape before post-process
// reshapes them into the slim envelope:
//   - messageId  → id (used by MESSAGE_ID_PATHS dedup)
//   - sender     → from (used by ingest bucketing)
//   - messageText → markdown body (from extract_markdown_body fallback)
//   - messageTimestamp → date
// All 5 messages are unique (different messageId) and valid (non-empty
// messageText), so without the cap every one would be ingested.
// ─────────────────────────────────────────────────────────────────────────

/// Build M distinct, valid Gmail messages in the upstream (pre-post-process)
/// Composio shape. Uses `messageId` / `sender` / `messageText` /
/// `messageTimestamp` so `reshape_message` maps them correctly into the slim
/// envelope that `ingest_page_into_memory_tree` expects.
fn gmail_cap_messages(m: usize) -> Vec<Value> {
    (1..=m)
        .map(|i| {
            json!({
                "messageId": format!("gmail-cap-msg-{i}"),
                "threadId": format!("thread-cap-{i}"),
                "sender": format!("sender{i}@cap.example.test"),
                "to": "recipient@cap.example.test",
                "subject": format!("Cap test message {i}"),
                "messageTimestamp": format!("2026-06-0{}T10:00:00Z", (i % 9) + 1),
                "messageText": format!("Body of cap test message {i}. Sufficient content.")
            })
        })
        .collect()
}

/// Loopback router that answers the two Gmail sync tool calls.
/// GMAIL_GET_PROFILE returns a minimal valid profile; GMAIL_FETCH_EMAILS
/// returns one page containing all `messages` and no next-page token.
fn gmail_cap_router(messages: Vec<Value>, requests: Arc<Mutex<Vec<Value>>>) -> Router {
    Router::new().route(
        "/agent-integrations/composio/execute",
        any(move |Json(body): Json<Value>| {
            let messages = messages.clone();
            let requests = Arc::clone(&requests);
            async move {
                requests.lock().unwrap().push(body.clone());
                let tool = body.get("tool").and_then(Value::as_str).unwrap_or("");
                let resp = match tool {
                    "GMAIL_GET_PROFILE" => execute_envelope(json!({
                        "emailAddress": "cap@example.test",
                        "messagesTotal": messages.len()
                    })),
                    "GMAIL_FETCH_EMAILS" => execute_envelope(json!({
                        "messages": messages,
                        "nextPageToken": ""
                    })),
                    _ => execute_envelope(json!({})),
                };
                Json(resp)
            }
        }),
    )
}

#[tokio::test]
async fn gmail_sync_max_items_caps_ingest_to_exact_count() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");

    // One page returns 5 valid messages; the cap is 2.
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (base, server) = loopback_router(gmail_cap_router(
        gmail_cap_messages(5),
        Arc::clone(&requests),
    ))
    .await;

    let mut config = config_in(&tmp);
    config.api_url = Some(base);
    persist_config(&config).await;
    store_session(&config);
    memory_global::init(config.workspace_dir.clone()).expect("init global memory");

    let ctx = ProviderContext {
        config: Arc::new(config),
        toolkit: "gmail".to_string(),
        connection_id: Some("conn-gmail-cap".to_string()),
        usage: Default::default(),
        // Mock returns 5 messages; cap is 2 — only 2 must be ingested.
        max_items: Some(2),
        sync_depth_days: None,
    };

    let outcome = GmailProvider::new()
        .sync(&ctx, SyncReason::ConnectionCreated)
        .await
        .expect("gmail cap sync");

    assert_eq!(
        outcome.items_ingested, 2,
        "max_items=2 must cap Gmail ingest to exactly 2 even though the page held 5"
    );

    server.abort();
}

// ─────────────────────────────────────────────────────────────────────────
// Notion cap enforcement
//
// Proves that max_items=N caps Notion ingestion to exactly N pages even
// when a single NOTION_FETCH_DATA page returns more than N. The mock
// returns 5 pages; with max_items=2 only 2 must be persisted.
//
// sync_depth_days is None and all items carry a recent last_edited_time
// so no depth-filter skipping happens — the cap is the only thing that
// limits ingestion.
// ─────────────────────────────────────────────────────────────────────────

/// Build M distinct, valid Notion page objects. Each has a unique id and a
/// recent `last_edited_time` so the depth filter (when enabled) would keep
/// them all.
fn notion_cap_pages(m: usize) -> Vec<Value> {
    (1..=m)
        .map(|i| {
            json!({
                "id": format!("notion-cap-page-{i:04}"),
                "object": "page",
                "last_edited_time": format!("2026-06-0{}T10:00:00.000Z", (i % 9) + 1),
                "properties": {
                    "Name": {
                        "type": "title",
                        "title": [{ "plain_text": format!("Cap page {i}") }]
                    }
                },
                "url": format!("https://www.notion.so/cap-page-{i}")
            })
        })
        .collect()
}

/// Loopback router for the Notion cap test.
/// NOTION_GET_ABOUT_ME returns a minimal identity; NOTION_FETCH_DATA returns
/// one page with all `pages` as results and no next_cursor.
fn notion_cap_router(pages: Vec<Value>, requests: Arc<Mutex<Vec<Value>>>) -> Router {
    Router::new().route(
        "/agent-integrations/composio/execute",
        any(move |Json(body): Json<Value>| {
            let pages = pages.clone();
            let requests = Arc::clone(&requests);
            async move {
                requests.lock().unwrap().push(body.clone());
                let tool = body.get("tool").and_then(Value::as_str).unwrap_or("");
                let resp = match tool {
                    "NOTION_GET_ABOUT_ME" => execute_envelope(json!({
                        "id": "notion-cap-user",
                        "name": "Cap User",
                        "type": "bot"
                    })),
                    "NOTION_FETCH_DATA" => execute_envelope(json!({
                        "results": pages,
                        "next_cursor": null,
                        "has_more": false
                    })),
                    _ => execute_envelope(json!({})),
                };
                Json(resp)
            }
        }),
    )
}

#[tokio::test]
async fn notion_sync_max_items_caps_ingest_to_exact_count() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");

    // One page returns 5 valid Notion pages; the cap is 2.
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (base, server) = loopback_router(notion_cap_router(
        notion_cap_pages(5),
        Arc::clone(&requests),
    ))
    .await;

    let mut config = config_in(&tmp);
    config.api_url = Some(base);
    persist_config(&config).await;
    store_session(&config);
    memory_global::init(config.workspace_dir.clone()).expect("init global memory");

    let ctx = ProviderContext {
        config: Arc::new(config),
        toolkit: "notion".to_string(),
        connection_id: Some("conn-notion-cap".to_string()),
        usage: Default::default(),
        // Mock returns 5 pages; cap is 2 — only 2 must be ingested.
        max_items: Some(2),
        sync_depth_days: None,
    };

    let outcome = NotionProvider::new()
        .sync(&ctx, SyncReason::ConnectionCreated)
        .await
        .expect("notion cap sync");

    assert_eq!(
        outcome.items_ingested, 2,
        "max_items=2 must cap Notion ingest to exactly 2 even though the page held 5"
    );

    server.abort();
}

// ─────────────────────────────────────────────────────────────────────────
// Notion sync_depth_days enforcement (via the shared orchestrator)
//
// Proves the generic orchestrator's depth window actually drops items older
// than the floor end-to-end through the real Notion provider. The mock returns
// 2 recent pages (far-future `last_edited_time`, always inside the window) and
// 3 ancient pages (year-2000, always outside it), in the descending order the
// provider requests. With sync_depth_days=7 only the 2 recent pages persist —
// the orchestrator truncates the page at the first item below the floor.
// ─────────────────────────────────────────────────────────────────────────

/// Build `recent` + `old` Notion pages in descending `last_edited_time` order.
/// Recent pages use a far-future timestamp (always within any depth window);
/// old pages use a year-2000 timestamp (always outside it).
fn notion_depth_pages(recent: usize, old: usize) -> Vec<Value> {
    let mut pages = Vec::new();
    for i in 0..recent {
        pages.push(json!({
            "id": format!("notion-recent-{i:04}"),
            "object": "page",
            "last_edited_time": format!("2099-12-{:02}T10:00:00.000Z", 28 - i),
            "properties": { "Name": { "type": "title", "title": [{ "plain_text": format!("Recent {i}") }] } }
        }));
    }
    for i in 0..old {
        pages.push(json!({
            "id": format!("notion-old-{i:04}"),
            "object": "page",
            "last_edited_time": format!("2000-01-{:02}T10:00:00.000Z", 3 - i),
            "properties": { "Name": { "type": "title", "title": [{ "plain_text": format!("Old {i}") }] } }
        }));
    }
    pages
}

#[tokio::test]
async fn notion_sync_depth_days_filters_old_pages() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");

    // One page: 2 recent + 3 ancient. With a 7-day window only the 2 recent
    // pages must be ingested.
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (base, server) = loopback_router(notion_cap_router(
        notion_depth_pages(2, 3),
        Arc::clone(&requests),
    ))
    .await;

    let mut config = config_in(&tmp);
    config.api_url = Some(base);
    persist_config(&config).await;
    store_session(&config);
    memory_global::init(config.workspace_dir.clone()).expect("init global memory");

    let ctx = ProviderContext {
        config: Arc::new(config),
        toolkit: "notion".to_string(),
        connection_id: Some("conn-notion-depth".to_string()),
        usage: Default::default(),
        max_items: None,
        // 7-day window: drops the year-2000 pages, keeps the far-future ones.
        sync_depth_days: Some(7),
    };

    let outcome = NotionProvider::new()
        .sync(&ctx, SyncReason::ConnectionCreated)
        .await
        .expect("notion depth sync");

    assert_eq!(
        outcome.items_ingested, 2,
        "sync_depth_days=7 must drop the 3 year-2000 pages and keep the 2 recent ones"
    );

    server.abort();
}

// ─────────────────────────────────────────────────────────────────────────
// Linear cap enforcement
//
// Proves that max_items=N caps Linear ingestion to exactly N issues even
// when a single LINEAR_LIST_LINEAR_ISSUES page returns more than N.
//
// Tool sequence:
//   1. LINEAR_LIST_LINEAR_USERS { isMe: true }  → viewer id
//   2. LINEAR_LIST_LINEAR_ISSUES (assigneeId=...) → nodes of issues
//
// Issues are in Linear's `{ nodes: [...], pageInfo: {...} }` shape.
// We return hasNextPage=false so pagination stops after one page.
// Each issue has a unique id and a recent updatedAt.
// ─────────────────────────────────────────────────────────────────────────

/// Build M distinct, valid Linear issue objects.
fn linear_cap_issues(m: usize) -> Vec<Value> {
    (1..=m)
        .map(|i| {
            json!({
                "id": format!("linear-cap-issue-{i:04}"),
                "identifier": format!("ENG-{i}"),
                "title": format!("Cap issue {i}"),
                "updatedAt": format!("2026-06-0{}T10:00:00.000Z", (i % 9) + 1),
                "url": format!("https://linear.app/cap/issue/ENG-{i}"),
                "description": format!("Description for cap issue {i}.")
            })
        })
        .collect()
}

/// Loopback router for the Linear cap test.
/// LINEAR_LIST_LINEAR_USERS returns a single viewer node so `resolve_viewer_id`
/// succeeds. LINEAR_LIST_LINEAR_ISSUES returns one page with all issues and no
/// next-page cursor (hasNextPage=false).
fn linear_cap_router(issues: Vec<Value>, requests: Arc<Mutex<Vec<Value>>>) -> Router {
    Router::new().route(
        "/agent-integrations/composio/execute",
        any(move |Json(body): Json<Value>| {
            let issues = issues.clone();
            let requests = Arc::clone(&requests);
            async move {
                requests.lock().unwrap().push(body.clone());
                let tool = body.get("tool").and_then(Value::as_str).unwrap_or("");
                let resp = match tool {
                    "LINEAR_LIST_LINEAR_USERS" => execute_envelope(json!({
                        "nodes": [
                            { "id": "linear-cap-viewer", "name": "Cap Viewer", "email": "cap@linear.test" }
                        ]
                    })),
                    "LINEAR_LIST_LINEAR_ISSUES" => execute_envelope(json!({
                        "nodes": issues,
                        "pageInfo": {
                            "hasNextPage": false,
                            "endCursor": null
                        }
                    })),
                    _ => execute_envelope(json!({})),
                };
                Json(resp)
            }
        }),
    )
}

#[tokio::test]
async fn linear_sync_max_items_caps_ingest_to_exact_count() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");

    // One page returns 5 valid Linear issues; the cap is 2.
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (base, server) = loopback_router(linear_cap_router(
        linear_cap_issues(5),
        Arc::clone(&requests),
    ))
    .await;

    let mut config = config_in(&tmp);
    config.api_url = Some(base);
    persist_config(&config).await;
    store_session(&config);
    memory_global::init(config.workspace_dir.clone()).expect("init global memory");

    let ctx = ProviderContext {
        config: Arc::new(config),
        toolkit: "linear".to_string(),
        connection_id: Some("conn-linear-cap".to_string()),
        usage: Default::default(),
        // Mock returns 5 issues; cap is 2 — only 2 must be ingested.
        max_items: Some(2),
        sync_depth_days: None,
    };

    let outcome = LinearProvider::new()
        .sync(&ctx, SyncReason::ConnectionCreated)
        .await
        .expect("linear cap sync");

    assert_eq!(
        outcome.items_ingested, 2,
        "max_items=2 must cap Linear ingest to exactly 2 even though the page held 5"
    );

    server.abort();
}

// ─────────────────────────────────────────────────────────────────────────
// Linear sync_depth_days enforcement (via the shared orchestrator)
//
// Linear applies the depth window client-side (RFC3339 `updatedAt`). The mock
// returns 2 recent issues (far-future) + 3 ancient (year-2000) in descending
// order; with sync_depth_days=7 only the 2 recent issues persist — the
// orchestrator truncates the page at the first item below the floor.
// ─────────────────────────────────────────────────────────────────────────

/// Build `recent` + `old` Linear issues in descending `updatedAt` order.
fn linear_depth_issues(recent: usize, old: usize) -> Vec<Value> {
    let mut issues = Vec::new();
    for i in 0..recent {
        issues.push(json!({
            "id": format!("linear-recent-{i:04}"),
            "identifier": format!("ENG-R{i}"),
            "title": format!("Recent {i}"),
            "updatedAt": format!("2099-12-{:02}T10:00:00.000Z", 28 - i),
            "url": format!("https://linear.app/cap/issue/ENG-R{i}"),
            "description": "recent",
        }));
    }
    for i in 0..old {
        issues.push(json!({
            "id": format!("linear-old-{i:04}"),
            "identifier": format!("ENG-O{i}"),
            "title": format!("Old {i}"),
            "updatedAt": format!("2000-01-{:02}T10:00:00.000Z", 3 - i),
            "url": format!("https://linear.app/cap/issue/ENG-O{i}"),
            "description": "old",
        }));
    }
    issues
}

#[tokio::test]
async fn linear_sync_depth_days_filters_old_issues() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");

    // One page: 2 recent + 3 ancient. With a 7-day window only the 2 recent
    // issues must be ingested.
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (base, server) = loopback_router(linear_cap_router(
        linear_depth_issues(2, 3),
        Arc::clone(&requests),
    ))
    .await;

    let mut config = config_in(&tmp);
    config.api_url = Some(base);
    persist_config(&config).await;
    store_session(&config);
    memory_global::init(config.workspace_dir.clone()).expect("init global memory");

    let ctx = ProviderContext {
        config: Arc::new(config),
        toolkit: "linear".to_string(),
        connection_id: Some("conn-linear-depth".to_string()),
        usage: Default::default(),
        max_items: None,
        sync_depth_days: Some(7),
    };

    let outcome = LinearProvider::new()
        .sync(&ctx, SyncReason::ConnectionCreated)
        .await
        .expect("linear depth sync");

    assert_eq!(
        outcome.items_ingested, 2,
        "sync_depth_days=7 must drop the 3 year-2000 issues and keep the 2 recent ones"
    );

    server.abort();
}

// ─────────────────────────────────────────────────────────────────────────
// ClickUp cap enforcement
//
// Proves that max_items=N caps ClickUp ingestion to exactly N tasks even
// when a single CLICKUP_GET_FILTERED_TEAM_TASKS page returns more than N.
//
// Tool sequence:
//   1. CLICKUP_GET_AUTHORIZED_USER             → user numeric id
//   2. CLICKUP_GET_AUTHORIZED_TEAMS_WORKSPACES → one workspace
//   3. CLICKUP_GET_FILTERED_TEAM_TASKS         → tasks page
//
// The tasks page returns 5 items; with max_items=2 only 2 must be
// persisted. Because INITIAL_PAGE_SIZE=100 for ConnectionCreated and we
// return only 5 tasks (< 100), the short-page guard stops the loop so no
// second page is requested, matching the cap path cleanly.
// ─────────────────────────────────────────────────────────────────────────

/// Build M distinct, valid ClickUp task objects.
fn clickup_cap_tasks(m: usize) -> Vec<Value> {
    // date_updated must be large enough to sort lexicographically correctly
    // but recent enough not to trip any depth filter. We use ms-since-epoch
    // strings in the vicinity of mid-2026 (≈1780000000000 ms).
    (1..=m)
        .map(|i| {
            json!({
                "id": format!("clickup-cap-task-{i:04}"),
                "name": format!("Cap task {i}"),
                "text_content": format!("Content for cap task {i}."),
                "status": { "status": "open" },
                "date_updated": format!("{}", 1_780_000_000_000_u64 + i as u64 * 1000),
                "url": format!("https://app.clickup.com/t/clickup-cap-task-{i:04}")
            })
        })
        .collect()
}

/// Loopback router for the ClickUp cap test.
fn clickup_cap_router(tasks: Vec<Value>, requests: Arc<Mutex<Vec<Value>>>) -> Router {
    Router::new().route(
        "/agent-integrations/composio/execute",
        any(move |Json(body): Json<Value>| {
            let tasks = tasks.clone();
            let requests = Arc::clone(&requests);
            async move {
                requests.lock().unwrap().push(body.clone());
                let tool = body.get("tool").and_then(Value::as_str).unwrap_or("");
                let resp = match tool {
                    "CLICKUP_GET_AUTHORIZED_USER" => execute_envelope(json!({
                        "user": {
                            "id": 42,
                            "username": "cap-user",
                            "email": "cap@clickup.test",
                            "profilePicture": null
                        }
                    })),
                    "CLICKUP_GET_AUTHORIZED_TEAMS_WORKSPACES" => execute_envelope(json!({
                        "teams": [
                            { "id": "ws-cap-01", "name": "Cap Workspace" }
                        ]
                    })),
                    "CLICKUP_GET_FILTERED_TEAM_TASKS" => execute_envelope(json!({
                        "tasks": tasks
                    })),
                    _ => execute_envelope(json!({})),
                };
                Json(resp)
            }
        }),
    )
}

#[tokio::test]
async fn clickup_sync_max_items_caps_ingest_to_exact_count() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");

    // One workspace, one page returning 5 valid tasks; the cap is 2.
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (base, server) = loopback_router(clickup_cap_router(
        clickup_cap_tasks(5),
        Arc::clone(&requests),
    ))
    .await;

    let mut config = config_in(&tmp);
    config.api_url = Some(base);
    persist_config(&config).await;
    store_session(&config);
    memory_global::init(config.workspace_dir.clone()).expect("init global memory");

    let ctx = ProviderContext {
        config: Arc::new(config),
        toolkit: "clickup".to_string(),
        connection_id: Some("conn-clickup-cap".to_string()),
        usage: Default::default(),
        // Mock returns 5 tasks; cap is 2 — only 2 must be ingested.
        max_items: Some(2),
        sync_depth_days: None,
    };

    let outcome = ClickUpProvider::new()
        .sync(&ctx, SyncReason::ConnectionCreated)
        .await
        .expect("clickup cap sync");

    assert_eq!(
        outcome.items_ingested, 2,
        "max_items=2 must cap ClickUp ingest to exactly 2 even though the page held 5"
    );

    server.abort();
}

// ─────────────────────────────────────────────────────────────────────────
// ClickUp sync_depth_days enforcement (via the shared orchestrator)
//
// ClickUp's `date_updated` is an epoch-millis string, so the orchestrator's
// depth floor must be epoch-ms too (ClickUpSource::depth_floor override). The
// mock returns 2 recent tasks (year-2030 epoch-ms) + 3 ancient (year-2001) in
// descending order; with sync_depth_days=7 only the 2 recent tasks persist.
// ─────────────────────────────────────────────────────────────────────────

/// Build `recent` + `old` ClickUp tasks in descending `date_updated` order,
/// using epoch-millis strings so the epoch-ms depth floor compares correctly.
fn clickup_depth_tasks(recent: usize, old: usize) -> Vec<Value> {
    let mut tasks = Vec::new();
    for i in 0..recent {
        tasks.push(json!({
            "id": format!("clickup-recent-{i:04}"),
            "name": format!("Recent {i}"),
            "text_content": "recent",
            "status": { "status": "open" },
            "date_updated": format!("{}", 1_900_000_000_000_u64 + (recent - i) as u64),
            "url": format!("https://app.clickup.com/t/clickup-recent-{i:04}")
        }));
    }
    for i in 0..old {
        tasks.push(json!({
            "id": format!("clickup-old-{i:04}"),
            "name": format!("Old {i}"),
            "text_content": "old",
            "status": { "status": "open" },
            "date_updated": format!("{}", 1_000_000_000_000_u64 + (old - i) as u64),
            "url": format!("https://app.clickup.com/t/clickup-old-{i:04}")
        }));
    }
    tasks
}

#[tokio::test]
async fn clickup_sync_depth_days_filters_old_tasks() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");

    // One workspace, one page: 2 recent + 3 ancient. With a 7-day window only
    // the 2 recent tasks must be ingested.
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (base, server) = loopback_router(clickup_cap_router(
        clickup_depth_tasks(2, 3),
        Arc::clone(&requests),
    ))
    .await;

    let mut config = config_in(&tmp);
    config.api_url = Some(base);
    persist_config(&config).await;
    store_session(&config);
    memory_global::init(config.workspace_dir.clone()).expect("init global memory");

    let ctx = ProviderContext {
        config: Arc::new(config),
        toolkit: "clickup".to_string(),
        connection_id: Some("conn-clickup-depth".to_string()),
        usage: Default::default(),
        max_items: None,
        sync_depth_days: Some(7),
    };

    let outcome = ClickUpProvider::new()
        .sync(&ctx, SyncReason::ConnectionCreated)
        .await
        .expect("clickup depth sync");

    assert_eq!(
        outcome.items_ingested, 2,
        "sync_depth_days=7 must drop the 3 year-2001 tasks and keep the 2 recent ones"
    );

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

// ─────────────────────────────────────────────────────────────────────────
// Sync-cap enforcement: max_items and sync_depth_days
//
// These verify the user-facing promise of the per-source sync settings:
//   - max_items=N ingests AT MOST N items, even when a single API page
//     returns more (precise mid-page cap, not just a page cap).
//   - sync_depth_days injects an `updated:>{floor}` date filter into the
//     GitHub search query so only recent items are requested.
// The cap logic is shared verbatim across the gmail/notion/linear/clickup
// providers, so GitHub stands in for all of them here.
// ─────────────────────────────────────────────────────────────────────────

/// Build `n` distinct, valid GitHub search items (each with an id + updated_at).
fn github_issue_items(n: usize) -> Vec<Value> {
    (1..=n)
        .map(|i| {
            json!({
                "id": 3000 + i,
                "title": format!("Cap issue {i}"),
                "body": "cap enforcement body",
                "state": "open",
                "updated_at": format!("2026-05-{:02}T10:00:00Z", 10 + i),
                "html_url": format!("https://github.com/tinyhumansai/openhuman/issues/{i}")
            })
        })
        .collect()
}

/// Loopback router that answers the two GitHub sync tools. The search tool
/// always returns `items` (one page), and every request body is captured.
fn github_cap_router(items: Vec<Value>, requests: Arc<Mutex<Vec<Value>>>) -> Router {
    Router::new().route(
        "/agent-integrations/composio/execute",
        any(move |Json(body): Json<Value>| {
            let items = items.clone();
            let requests = Arc::clone(&requests);
            async move {
                requests.lock().unwrap().push(body.clone());
                let tool = body.get("tool").and_then(Value::as_str).unwrap_or("");
                let resp = match tool {
                    "GITHUB_GET_THE_AUTHENTICATED_USER" => execute_envelope(json!({
                        "login": "octo-cap",
                        "html_url": "https://github.com/octo-cap"
                    })),
                    "GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS" => execute_envelope(json!({
                        "items": items,
                        "total_count": items.len()
                    })),
                    _ => execute_envelope(json!({})),
                };
                Json(resp)
            }
        }),
    )
}

async fn github_cap_context(
    tmp: &TempDir,
    items: Vec<Value>,
    requests: Arc<Mutex<Vec<Value>>>,
) -> (Config, tokio::task::JoinHandle<()>) {
    let (base, server) = loopback_router(github_cap_router(items, requests)).await;
    let mut config = config_in(tmp);
    config.api_url = Some(base);
    persist_config(&config).await;
    store_session(&config);
    memory_global::init(config.workspace_dir.clone()).expect("init global memory");
    (config, server)
}

fn github_ctx(
    config: &Config,
    max_items: Option<u32>,
    sync_depth_days: Option<u32>,
) -> ProviderContext {
    ProviderContext {
        config: Arc::new(config.clone()),
        toolkit: "github".to_string(),
        connection_id: Some("conn-cap".to_string()),
        usage: Default::default(),
        max_items,
        sync_depth_days,
    }
}

#[tokio::test]
async fn github_sync_max_items_caps_ingest_to_exact_count() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");

    // One page returns 5 valid items; the cap is 2.
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (config, server) =
        github_cap_context(&tmp, github_issue_items(5), Arc::clone(&requests)).await;

    let outcome = GitHubProvider::new()
        .sync(
            &github_ctx(&config, Some(2), None),
            SyncReason::ConnectionCreated,
        )
        .await
        .expect("github sync");

    assert_eq!(
        outcome.items_ingested, 2,
        "max_items=2 must cap ingest to exactly 2 even though the page held 5"
    );
    server.abort();
}

#[tokio::test]
async fn github_sync_without_max_items_ingests_full_page() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");

    // Control: no cap → every valid item on the page is ingested.
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (config, server) =
        github_cap_context(&tmp, github_issue_items(5), Arc::clone(&requests)).await;

    let outcome = GitHubProvider::new()
        .sync(
            &github_ctx(&config, None, None),
            SyncReason::ConnectionCreated,
        )
        .await
        .expect("github sync");

    assert_eq!(
        outcome.items_ingested, 5,
        "with no max_items cap, all 5 page items must be ingested"
    );
    server.abort();
}

#[tokio::test]
async fn github_sync_depth_days_injects_updated_floor_into_query() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");

    let requests = Arc::new(Mutex::new(Vec::new()));
    let (config, server) =
        github_cap_context(&tmp, github_issue_items(1), Arc::clone(&requests)).await;

    GitHubProvider::new()
        .sync(
            &github_ctx(&config, None, Some(7)),
            SyncReason::ConnectionCreated,
        )
        .await
        .expect("github sync");

    // The search request must carry an `updated:>{date}` floor derived from the
    // 7-day window, proving sync_depth_days actually narrows what is fetched.
    let reqs = requests.lock().unwrap();
    let search = reqs
        .iter()
        .find(|b| {
            b.get("tool").and_then(Value::as_str) == Some("GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS")
        })
        .expect("a search request was issued");
    let q = search
        .get("arguments")
        .and_then(|a| a.get("q"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        q.contains("updated:>"),
        "sync_depth_days=7 must inject an `updated:>` date floor, got query: {q}"
    );
    server.abort();
}
