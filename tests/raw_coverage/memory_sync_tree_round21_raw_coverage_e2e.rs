//! Round 21 focused raw coverage for memory_sync + memory_tree gaps.
//!
//! Hermetic: temp workspaces, loopback Composio backend, and no real network.
//! Run with `--test-threads=1` because config/HOME/workspace env vars and the
//! global memory client are process-global.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use axum::routing::{any, get};
use axum::{Json, Router};
use chrono::{TimeZone, Utc};
use serde_json::{json, Value};
use tempfile::TempDir;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::memory::global as memory_global;
use openhuman_core::openhuman::memory_store::chunks::store::with_connection;
use openhuman_core::openhuman::memory_store::content::atomic::stage_summary;
use openhuman_core::openhuman::memory_store::content::{SummaryComposeInput, SummaryTreeKind};
use openhuman_core::openhuman::memory_store::trees::types::{SummaryNode, Tree, TreeKind};
use openhuman_core::openhuman::memory_sync::composio::periodic::record_sync_success;
use openhuman_core::openhuman::memory_sync::composio::providers::gmail::GmailProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::linear::LinearProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::slack::rpc::{
    sync_status_rpc, SyncStatusRequest,
};
use openhuman_core::openhuman::memory_sync::composio::providers::sync_state::SyncState;
use openhuman_core::openhuman::memory_sync::composio::providers::{
    ComposioProvider, ProviderContext, SyncReason, TaskFetchFilter,
};
use openhuman_core::openhuman::memory_tree::retrieval::source::query_source;
use tinycortex::memory::score::embed::{pack_embedding, EMBEDDING_DIM};
use openhuman_core::openhuman::memory_tree::tree::store as tree_store;
use openhuman_core::openhuman::memory_tree::tree::TreeStatus;

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

struct EnvGuard {
    key: &'static str,
    old: Option<OsString>,
}

impl EnvGuard {
    fn set_path(key: &'static str, value: impl AsRef<Path>) -> Self {
        let old = std::env::var_os(key);
        unsafe { std::env::set_var(key, value.as_ref()) };
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var_os(key);
        unsafe { std::env::remove_var(key) };
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.old {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
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
            "round21-session-token",
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

fn linear_execute_response(body: &Value) -> Value {
    let tool = body.get("tool").and_then(Value::as_str).unwrap_or("");
    let args = body.get("arguments").cloned().unwrap_or_else(|| json!({}));
    match tool {
        "LINEAR_LIST_LINEAR_USERS" => execute_envelope(json!({
            "data": {
                "nodes": [{
                    "id": "usr-round21",
                    "name": "Round Twenty One",
                    "email": "round21@example.test",
                    "avatarUrl": "https://example.test/linear.png",
                    "url": "https://linear.app/openhuman/profiles/round21"
                }]
            }
        })),
        "LINEAR_LIST_LINEAR_ISSUES" => {
            let after = args.get("after").and_then(Value::as_str);
            let state = args.get("state").and_then(Value::as_str).unwrap_or("open");
            let nodes = if after == Some("cursor-page-2") {
                vec![
                    json!({
                        "id": "lin-round21-older",
                        "identifier": "OH-20",
                        "title": "Older issue past cursor",
                        "description": "This page proves cursor pagination is followed.",
                        "updatedAt": "2026-05-29T08:00:00.000Z",
                        "url": "https://linear.app/openhuman/issue/OH-20",
                        "state": { "name": state },
                        "assignee": { "name": "Round Twenty One" },
                        "labels": { "nodes": [{ "name": "coverage" }] },
                        "priorityLabel": "Medium"
                    }),
                    json!({
                        "identifier": "OH-MISSING-ID",
                        "title": "Missing id is skipped",
                        "updatedAt": "2026-05-29T07:00:00.000Z"
                    }),
                ]
            } else {
                vec![
                    json!({
                        "id": "lin-round21-new",
                        "identifier": "OH-21",
                        "title": "Cover Linear provider branches",
                        "description": "Exercise profile, task normalization, sync persistence.",
                        "updatedAt": "2026-05-30T10:00:00.000Z",
                        "url": "https://linear.app/openhuman/issue/OH-21",
                        "state": { "name": state },
                        "assignee": { "name": "Round Twenty One" },
                        "dueDate": "2026-06-01",
                        "labels": { "nodes": [{ "name": "coverage" }, { "name": "round21" }] },
                        "priorityLabel": "High"
                    }),
                    json!({
                        "data": {
                            "id": "lin-round21-wrapped",
                            "title": "Wrapped Linear issue",
                            "description": "Wrapped shape exercises data.* fallbacks.",
                            "updated_at": "2026-05-30T09:00:00.000Z",
                            "state": { "name": state },
                            "assignee": { "name": "Round Twenty One" },
                            "priorityLabel": "Low"
                        }
                    }),
                ]
            };
            execute_envelope(json!({
                "data": {
                    "nodes": nodes,
                    "pageInfo": {
                        "hasNextPage": after.is_none(),
                        "endCursor": if after.is_none() { "cursor-page-2" } else { "" }
                    }
                }
            }))
        }
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
                Json(linear_execute_response(&body))
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

#[test]
fn gmail_post_process_slims_wrapped_messages_and_honours_raw_flag() {
    let provider = GmailProvider::new();
    let mut wrapped = json!({
        "data": {
            "messages": [
                {
                    "messageId": "gmail-round21-a",
                    "threadId": "thread-a",
                    "subject": "Round 21 A",
                    "sender": "Ava <ava@example.test>",
                    "to": "Ben <ben@example.test>",
                    "labelIds": ["INBOX", "IMPORTANT"],
                    "markdownFormatted": "## Round 21 A\nUseful body from backend markdown.",
                    "messageText": "fallback body",
                    "payload": {
                        "headers": [
                            { "name": "Date", "value": "Sat, 30 May 2026 10:00:00 +0000" },
                            { "name": "List-Unsubscribe", "value": "<mailto:unsubscribe@example.test>" }
                        ]
                    },
                    "attachmentList": [
                        { "filename": "brief.pdf", "mimeType": "application/pdf" },
                        { "filename": "", "mimeType": "text/plain" }
                    ]
                },
                "non-object-passthrough"
            ],
            "nextPageToken": "next-round21",
            "resultSizeEstimate": 2,
            "verboseNoise": { "dropped": true }
        }
    });

    provider.post_process_action_result("GMAIL_FETCH_EMAILS", None, &mut wrapped);
    let messages = wrapped["data"]["messages"]
        .as_array()
        .expect("slim messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["id"], "gmail-round21-a");
    assert_eq!(messages[0]["date"], "Sat, 30 May 2026 10:00:00 +0000");
    assert_eq!(
        messages[0]["list_unsubscribe"],
        "<mailto:unsubscribe@example.test>"
    );
    assert_eq!(messages[0]["attachments"][0]["filename"], "brief.pdf");
    assert_eq!(
        messages[0]["markdown"],
        "## Round 21 A\nUseful body from backend markdown."
    );
    assert_eq!(wrapped["data"]["nextPageToken"], "next-round21");
    assert!(wrapped["data"].get("verboseNoise").is_none());

    let mut raw = json!({
        "messages": [{ "messageId": "gmail-raw", "messageText": "<b>raw html stays</b>" }]
    });
    provider.post_process_action_result(
        "GMAIL_FETCH_EMAILS",
        Some(&json!({ "rawHtml": true })),
        &mut raw,
    );
    assert_eq!(raw["messages"][0]["messageId"], "gmail-raw");

    let mut unknown = json!({ "messages": [{ "messageId": "gmail-unknown" }] });
    provider.post_process_action_result("GMAIL_SEND_EMAIL", None, &mut unknown);
    assert_eq!(unknown["messages"][0]["messageId"], "gmail-unknown");
}

#[tokio::test]
async fn linear_provider_profile_tasks_sync_and_periodic_bookkeeping_use_loopback() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (_config, ctx, server) =
        configured_loopback_context(&tmp, "linear", "conn-linear-round21", Arc::clone(&requests))
            .await;

    let provider = LinearProvider::new();
    let profile = provider.fetch_user_profile(&ctx).await.expect("profile");
    assert_eq!(profile.username.as_deref(), Some("usr-round21"));
    assert_eq!(profile.display_name.as_deref(), Some("Round Twenty One"));
    assert_eq!(profile.email.as_deref(), Some("round21@example.test"));

    let tasks = provider
        .fetch_tasks(
            &ctx,
            &TaskFetchFilter {
                assignee_is_me: true,
                state: Some("open".to_string()),
                max: 5,
                extra: json!({ "includeArchived": false }),
                ..TaskFetchFilter::default()
            },
        )
        .await
        .expect("tasks");
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].external_id, "lin-round21-new");
    assert_eq!(
        tasks[0].labels,
        vec!["coverage".to_string(), "round21".to_string()]
    );
    assert_eq!(tasks[0].priority.as_deref(), Some("High"));
    assert_eq!(tasks[1].external_id, "lin-round21-wrapped");

    let sync = provider
        .sync(&ctx, SyncReason::ConnectionCreated)
        .await
        .expect("linear sync");
    assert_eq!(sync.items_ingested, 4);
    assert_eq!(sync.details["more_pending"], false);
    assert_eq!(sync.details["actions_called"], 3);

    let second = provider
        .sync(&ctx, SyncReason::Manual)
        .await
        .expect("second sync");
    assert_eq!(second.items_ingested, 0);
    assert_eq!(second.details["more_pending"], false);
    assert_eq!(second.details["actions_called"], 3);

    record_sync_success("linear", "conn-linear-round21");
    record_sync_success("linear", "conn-linear-round21");

    let called_tools: Vec<String> = requests
        .lock()
        .unwrap()
        .iter()
        .filter_map(|b| b.get("tool").and_then(Value::as_str).map(str::to_string))
        .collect();
    assert!(called_tools.contains(&"LINEAR_LIST_LINEAR_USERS".to_string()));
    assert!(called_tools.contains(&"LINEAR_LIST_LINEAR_ISSUES".to_string()));
    assert!(requests.lock().unwrap().iter().any(|body| {
        body.get("arguments")
            .and_then(|args| args.get("after"))
            .and_then(Value::as_str)
            == Some("cursor-page-2")
    }));

    server.abort();
}

#[tokio::test]
async fn slack_sync_status_rpc_reads_mock_connections_and_persisted_state() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    let mut config = config_in(&tmp);
    let router = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            Json(json!({
                "success": true,
                "data": {
                    "connections": [
                        { "id": "conn-slack-round21", "toolkit": "slack", "status": "ACTIVE" },
                        { "id": "conn-slack-pending", "toolkit": "slack", "status": "PENDING" },
                        { "id": "conn-gmail-round21", "toolkit": "gmail", "status": "ACTIVE" }
                    ]
                }
            }))
        }),
    );
    let (base, server) = loopback_router(router).await;
    config.api_url = Some(base);
    persist_config(&config).await;
    store_session(&config);
    let memory = memory_global::init(config.workspace_dir.clone()).expect("memory global");
    let mut state = SyncState::new("slack", "conn-slack-round21");
    state.advance_cursor(r#"{"C21":"1714003200.000100"}"#);
    state.mark_synced("C21:1714003200.000100");
    state.record_requests(7);
    let state_adapter =
        openhuman_core::openhuman::tinycortex::HostSyncAdapter::new(memory.clone());
    state
        .save(&state_adapter)
        .await
        .expect("save slack sync state");

    let outcome = sync_status_rpc(&config, SyncStatusRequest::default())
        .await
        .expect("status rpc");
    assert_eq!(outcome.value.connections.len(), 1);
    let row = &outcome.value.connections[0];
    assert_eq!(row.connection_id, "conn-slack-round21");
    assert_eq!(row.synced_ids_count, 1);
    assert_eq!(row.requests_used_today, 7);
    assert!(row.per_channel_cursors.contains("C21"));
    assert!(outcome
        .logs
        .iter()
        .any(|line| line.contains("connections=1")));

    server.abort();
}

#[tokio::test]
async fn memory_tree_source_query_filters_reranks_and_hydrates_manual_summaries() {
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in(&tmp);
    std::fs::create_dir_all(config.memory_tree_content_root()).expect("content root");
    seed_source_summary(
        &config,
        "slack:#round21",
        "summary-round21-chat",
        "Full chat summary body from disk.",
        1_780_313_600_000,
        Some(one_hot(0)),
    );
    seed_source_summary(
        &config,
        "gmail:round21@example.test",
        "summary-round21-email",
        "Full email summary body from disk.",
        1_780_227_200_000,
        None,
    );

    let all = query_source(&config, None, None, None, None, 0)
        .await
        .expect("all source query");
    assert_eq!(all.total, 2);
    assert_eq!(all.hits.len(), 2);

    let chat = query_source(
        &config,
        None,
        Some(openhuman_core::openhuman::memory_store::chunks::types::SourceKind::Chat),
        None,
        Some("semantic query keeps embedded rows first"),
        10,
    )
    .await
    .expect("chat query");
    assert_eq!(chat.hits.len(), 1);
    assert_eq!(chat.hits[0].tree_scope, "slack:#round21");
    assert_eq!(chat.hits[0].content, "Full chat summary body from disk.");

    let missing = query_source(&config, Some("slack:#missing"), None, None, None, 10)
        .await
        .expect("missing source");
    assert!(missing.hits.is_empty());
}

fn one_hot(index: usize) -> Vec<f32> {
    let mut values = vec![0.0; EMBEDDING_DIM];
    values[index] = 1.0;
    values
}

fn seed_source_summary(
    config: &Config,
    scope: &str,
    summary_id: &str,
    body: &str,
    timestamp_ms: i64,
    embedding: Option<Vec<f32>>,
) {
    let ts = Utc.timestamp_millis_opt(timestamp_ms).unwrap();
    let tree = Tree {
        id: format!("tree:{summary_id}"),
        kind: TreeKind::Source,
        scope: scope.to_string(),
        root_id: Some(summary_id.to_string()),
        max_level: 1,
        status: TreeStatus::Active,
        created_at: ts,
        last_sealed_at: Some(ts),
    };
    tree_store::insert_tree(config, &tree).expect("insert source tree");

    let node = SummaryNode {
        id: summary_id.to_string(),
        tree_id: tree.id.clone(),
        tree_kind: TreeKind::Source,
        level: 1,
        parent_id: None,
        child_ids: vec!["leaf-a".to_string(), "leaf-b".to_string()],
        content: "preview only".to_string(),
        token_count: 64,
        entities: vec!["round21".to_string()],
        topics: vec!["coverage".to_string()],
        time_range_start: ts,
        time_range_end: ts,
        score: 0.75,
        sealed_at: ts,
        deleted: false,
        embedding: embedding.clone(),
        doc_id: None,
        version_ms: None,
    };
    let staged = stage_summary(
        &config.memory_tree_content_root(),
        &SummaryComposeInput {
            summary_id: &node.id,
            tree_kind: SummaryTreeKind::Source,
            tree_id: &node.tree_id,
            tree_scope: &tree.scope,
            level: node.level,
            child_ids: &node.child_ids,
            child_basenames: None,
            child_count: node.child_ids.len(),
            time_range_start: node.time_range_start,
            time_range_end: node.time_range_end,
            sealed_at: node.sealed_at,
            body,
        },
        scope,
    )
    .expect("stage summary body");
    let embedding_blob = embedding.as_ref().map(|values| pack_embedding(values));

    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO mem_tree_summaries (
                id, tree_id, tree_kind, level, parent_id,
                child_ids_json, content, token_count,
                entities_json, topics_json,
                time_range_start_ms, time_range_end_ms,
                score, sealed_at_ms, deleted, embedding,
                content_path, content_sha256
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            rusqlite::params![
                node.id,
                node.tree_id,
                node.tree_kind.as_str(),
                node.level,
                node.parent_id,
                serde_json::to_string(&node.child_ids).unwrap(),
                node.content,
                node.token_count,
                serde_json::to_string(&node.entities).unwrap(),
                serde_json::to_string(&node.topics).unwrap(),
                node.time_range_start.timestamp_millis(),
                node.time_range_end.timestamp_millis(),
                node.score,
                node.sealed_at.timestamp_millis(),
                node.deleted as i64,
                embedding_blob,
                staged.content_path,
                staged.content_sha256,
            ],
        )?;
        Ok(())
    })
    .expect("insert summary row");
}
