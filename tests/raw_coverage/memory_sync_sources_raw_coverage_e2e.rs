//! Focused raw integration coverage for memory sync + memory sources.
//!
//! Everything here is local: temp workspaces, loopback HTTP, and a fake `gh`
//! binary. Run with `--test-threads=1` because config and PATH are process
//! globals.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use axum::extract::Request;
use axum::response::IntoResponse;
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::TempDir;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::memory_sources::readers::SourceReader;
use openhuman_core::openhuman::memory_sources::{
    add_source, get_source, list_enabled_by_kind, list_sources,
    remove_composio_source_by_connection_id, remove_source, update_source, upsert_composio_source,
    MemorySourceEntry, MemorySourcePatch, SourceKind,
};
use openhuman_core::openhuman::memory_sync::composio::bus::{
    ComposioConfigChangedSubscriber, ComposioConnectionCreatedSubscriber, ComposioTriggerSubscriber,
};
use openhuman_core::openhuman::memory_sync::composio::providers::clickup::ClickUpProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::github::GitHubProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::gmail::GmailProvider;
use openhuman_core::openhuman::memory_sync::composio::providers::slack::{
    run_backfill_via_search, SlackProvider,
};
use openhuman_core::openhuman::memory_sync::composio::providers::{
    ComposioProvider, ProviderContext, SyncReason, TaskFetchFilter,
};
use openhuman_core::openhuman::memory_sync::composio::{
    all_composio_sync_providers, get_composio_sync_provider, init_default_composio_sync_providers,
};

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

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

fn source(kind: SourceKind, id: &str) -> MemorySourceEntry {
    MemorySourceEntry {
        id: id.to_string(),
        kind,
        label: format!("{id} label"),
        enabled: true,
        toolkit: None,
        connection_id: None,
        path: None,
        glob: None,
        url: None,
        branch: None,
        paths: Vec::new(),
        query: None,
        since_days: None,
        max_items: None,
        max_commits: None,
        max_issues: None,
        max_prs: None,
        selector: None,
        max_tokens_per_sync: None,
        max_cost_per_sync_usd: None,
        sync_depth_days: None,
    }
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

#[tokio::test]
async fn memory_sources_registry_persists_crud_and_composio_upserts() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in(&tmp);
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    persist_config(&config).await;

    let mut folder = source(SourceKind::Folder, "src_folder_round15");
    folder.path = Some(tmp.path().join("notes").to_string_lossy().into_owned());
    folder.glob = Some("**/*.md".to_string());
    let added = add_source(folder.clone()).await.expect("add folder");
    assert_eq!(added.id, folder.id);

    let duplicate = add_source(folder.clone())
        .await
        .expect_err("duplicate id rejected");
    assert!(duplicate.contains("already exists"));

    let enabled_folders = list_enabled_by_kind(SourceKind::Folder)
        .await
        .expect("enabled folders");
    assert_eq!(enabled_folders.len(), 1);

    let updated = update_source(
        &folder.id,
        MemorySourcePatch {
            label: Some("Renamed notes".to_string()),
            enabled: Some(false),
            glob: Some("*.txt".to_string()),
            ..MemorySourcePatch::default()
        },
    )
    .await
    .expect("update folder");
    assert_eq!(updated.label, "Renamed notes");
    assert!(!updated.enabled);
    assert_eq!(updated.glob.as_deref(), Some("*.txt"));

    let none_enabled = list_enabled_by_kind(SourceKind::Folder)
        .await
        .expect("disabled folder filtered");
    assert!(none_enabled.is_empty());

    let first = upsert_composio_source("slack", "conn-round15", "Slack workspace")
        .await
        .expect("insert composio source");
    assert_eq!(first.toolkit.as_deref(), Some("slack"));
    let second = upsert_composio_source("slack", "conn-round15", "Slack renamed")
        .await
        .expect("update composio source");
    assert_eq!(second.id, first.id);
    assert_eq!(second.label, "Slack renamed");

    let fetched = get_source(&first.id)
        .await
        .expect("get source")
        .expect("source exists");
    assert_eq!(fetched.connection_id.as_deref(), Some("conn-round15"));

    assert!(remove_source(&folder.id).await.expect("remove folder"));
    assert!(!remove_source("missing-source")
        .await
        .expect("remove missing"));
    let all = list_sources().await.expect("list sources");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, first.id);
}

#[tokio::test]
async fn remove_composio_source_by_connection_id_prunes_on_disconnect_and_survives_reconnect() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in(&tmp);
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    persist_config(&config).await;

    // Two live composio connections plus an unrelated folder source.
    let gmail_old = upsert_composio_source("gmail", "conn-old", "Gmail · conn-old")
        .await
        .expect("insert gmail");
    upsert_composio_source("slack", "conn-slack", "Slack")
        .await
        .expect("insert slack");
    let mut folder = source(SourceKind::Folder, "src_folder_disc");
    folder.path = Some(tmp.path().join("notes").to_string_lossy().into_owned());
    folder.glob = Some("**/*.md".to_string());
    add_source(folder.clone()).await.expect("add folder");

    // No-match is a no-op (returns 0, removes nothing).
    assert_eq!(
        remove_composio_source_by_connection_id("conn-does-not-exist")
            .await
            .expect("no-match remove"),
        0
    );
    assert_eq!(list_sources().await.expect("list").len(), 3);

    // Disconnect: prune ONLY the matching composio source, by connection_id.
    assert_eq!(
        remove_composio_source_by_connection_id("conn-old")
            .await
            .expect("prune on disconnect"),
        1
    );
    let after_disconnect = list_sources().await.expect("list after disconnect");
    assert_eq!(after_disconnect.len(), 2);
    assert!(
        after_disconnect.iter().all(|s| s.id != gmail_old.id),
        "old gmail entry must be gone"
    );
    assert!(
        after_disconnect
            .iter()
            .any(|s| s.connection_id.as_deref() == Some("conn-slack")),
        "the other composio connection must be untouched"
    );
    assert!(
        after_disconnect.iter().any(|s| s.id == folder.id),
        "non-composio folder source must be untouched"
    );

    // Reconnect: backend mints a NEW connection_id for the same Gmail account.
    // upsert inserts a fresh entry; no stale duplicate is left behind.
    let gmail_new = upsert_composio_source("gmail", "conn-new", "Gmail · conn-new")
        .await
        .expect("reconnect gmail");
    assert_ne!(gmail_new.id, gmail_old.id);
    let final_sources = list_sources().await.expect("final list");
    let gmail_entries: Vec<_> = final_sources
        .iter()
        .filter(|s| s.toolkit.as_deref() == Some("gmail"))
        .collect();
    assert_eq!(
        gmail_entries.len(),
        1,
        "exactly one gmail source after reconnect — no orphan"
    );
    assert_eq!(gmail_entries[0].connection_id.as_deref(), Some("conn-new"));
}

#[tokio::test]
async fn rss_reader_lists_reads_and_reports_feed_errors_from_loopback() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in(&tmp);
    let rss_xml = r#"<?xml version="1.0"?>
    <rss version="2.0"><channel>
      <item>
        <title>First &amp; useful</title>
        <link>https://example.test/first</link>
        <description><![CDATA[<p>HTML body &amp; details</p>]]></description>
        <pubDate>Fri, 29 May 2026 10:00:00 GMT</pubDate>
      </item>
      <item>
        <title>Second</title>
        <guid>guid-second</guid>
        <description>Plain &lt;encoded&gt; body</description>
      </item>
    </channel></rss>"#;
    let atom_xml = r#"<?xml version="1.0"?>
    <feed><entry>
      <title>Atom item</title>
      <id>urn:round15:atom</id>
      <summary>Atom summary</summary>
      <link href="https://example.test/atom" />
      <updated>2026-05-29T12:00:00Z</updated>
    </entry></feed>"#;
    let router = Router::new().route(
        "/{feed}",
        any(move |req: Request| {
            let rss_xml = rss_xml.to_string();
            let atom_xml = atom_xml.to_string();
            async move {
                match req.uri().path() {
                    "/rss" => (
                        [(axum::http::header::CONTENT_TYPE, "application/rss+xml")],
                        rss_xml,
                    )
                        .into_response(),
                    "/atom" => (
                        [(axum::http::header::CONTENT_TYPE, "application/atom+xml")],
                        atom_xml,
                    )
                        .into_response(),
                    "/broken" => (axum::http::StatusCode::BAD_GATEWAY, "bad feed").into_response(),
                    _ => (axum::http::StatusCode::NOT_FOUND, "missing").into_response(),
                }
            }
        }),
    );
    let (base, server) = loopback_router(router).await;

    let reader = openhuman_core::openhuman::memory_sources::readers::rss::RssReader;
    let mut entry = source(SourceKind::RssFeed, "rss-round15");
    entry.url = Some(format!("{base}/rss"));
    entry.max_items = Some(1);

    let items = reader.list_items(&entry, &config).await.expect("list rss");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "First & useful");

    let content = reader
        .read_item(&entry, "https://example.test/first", &config)
        .await
        .expect("read rss item");
    assert_eq!(content.id, "https://example.test/first");
    assert_eq!(
        content.content_type,
        openhuman_core::openhuman::memory_sources::ContentType::Html
    );
    assert!(content.body.contains("HTML body"));

    let mut atom = entry.clone();
    atom.url = Some(format!("{base}/atom"));
    let atom_content = reader
        .read_item(&atom, "urn:round15:atom", &config)
        .await
        .expect("read atom item");
    assert_eq!(atom_content.title, "Atom item");
    assert_eq!(
        atom_content.metadata.get("link").and_then(Value::as_str),
        Some("https://example.test/atom")
    );

    let missing = reader
        .read_item(&atom, "missing", &config)
        .await
        .expect_err("missing atom item");
    assert!(missing.contains("not found"));

    let mut broken = entry;
    broken.url = Some(format!("{base}/broken"));
    let err = reader
        .list_items(&broken, &config)
        .await
        .expect_err("http status error");
    assert!(err.contains("502"));

    server.abort();
}

#[tokio::test]
async fn github_reader_uses_fake_gh_for_list_and_read_paths() {
    let _guard = env_lock();
    if std::process::Command::new("gh")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| !s.success())
        .unwrap_or(true)
    {
        eprintln!("skipping: gh CLI not available");
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in(&tmp);
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("bin dir");
    let script = bin.join("gh");
    write_fake_gh(&script);
    write_fake_git(&bin.join("git"));
    let old_path = std::env::var("PATH").unwrap_or_default();
    let _path = EnvGuard::set("PATH", format!("{}:{old_path}", bin.display()));

    let reader = openhuman_core::openhuman::memory_sources::readers::github::GithubReader;
    let mut entry = source(SourceKind::GithubRepo, "github-round15");
    entry.url = Some("https://github.com/tinyhumansai/openhuman.git".to_string());
    entry.max_commits = Some(30);
    entry.max_issues = Some(30);
    entry.max_prs = Some(30);

    let items = reader
        .list_items(&entry, &config)
        .await
        .expect("list github activity");
    assert!(items.iter().any(|i| i.id == "commit:abc123"));
    assert!(items.iter().any(|i| i.id == "issue:7"));
    assert!(items.iter().any(|i| i.id == "pr:9"));
    assert!(!items.iter().any(|i| i.id == "issue:99"));

    let commit = reader
        .read_item(&entry, "commit:abc123", &config)
        .await
        .expect("read commit");
    assert!(commit.body.contains("Add coverage hooks"));
    assert_eq!(
        commit.metadata.get("sha").and_then(Value::as_str),
        Some("abc123")
    );

    let issue = reader
        .read_item(&entry, "issue:7", &config)
        .await
        .expect("read issue");
    assert!(issue.body.contains("## Description"));
    assert!(issue.body.contains("Needs fixture coverage"));
    assert_eq!(
        issue.metadata.get("state").and_then(Value::as_str),
        Some("open")
    );

    let pr = reader
        .read_item(&entry, "pr:9", &config)
        .await
        .expect("read pr");
    assert!(pr.body.contains("not merged"));
    assert_eq!(
        pr.metadata.get("merged").and_then(Value::as_bool),
        Some(false)
    );

    let invalid = reader
        .read_item(&entry, "unknown:1", &config)
        .await
        .expect_err("invalid id rejected");
    assert!(invalid.contains("invalid item id"));

    let mut bad_url = entry;
    bad_url.url = Some("https://github.com/tinyhumansai/openhuman/tree/main".to_string());
    let bad = reader
        .list_items(&bad_url, &config)
        .await
        .expect_err("deep link rejected");
    assert!(bad.contains("expected https://github.com/<owner>/<repo>"));
}

#[tokio::test]
async fn composio_providers_fetch_profiles_tasks_and_cover_error_branches() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let mut config = config_in(&tmp);
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let router = {
        let requests = Arc::clone(&requests);
        Router::new().route(
            "/agent-integrations/composio/execute",
            any(move |Json(body): Json<Value>| {
                let requests = Arc::clone(&requests);
                async move {
                    requests.lock().unwrap().push(body.clone());
                    Json(json!({
                        "success": true,
                        "data": execute_response_for(&body),
                    }))
                }
            }),
        )
    };
    let (base, server) = loopback_router(router).await;
    config.api_url = Some(base);
    persist_config(&config).await;
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "round15-session-token",
            HashMap::new(),
            true,
        )
        .expect("store session token");

    let ctx = ProviderContext {
        config: Arc::new(config.clone()),
        toolkit: "github".to_string(),
        connection_id: Some("conn-github".to_string()),
        usage: Default::default(),
        max_items: None,
        sync_depth_days: None,
    };
    let github = GitHubProvider::new();
    let github_profile = github
        .fetch_user_profile(&ctx)
        .await
        .expect("github profile");
    assert_eq!(github_profile.username.as_deref(), Some("octo-round15"));
    assert_eq!(
        github_profile.display_name.as_deref(),
        Some("Round Fifteen")
    );

    let tasks = github
        .fetch_tasks(
            &ctx,
            &TaskFetchFilter {
                repo: Some("tinyhumansai/openhuman".to_string()),
                labels: vec!["coverage".to_string()],
                state: Some("open".to_string()),
                max: 2,
                ..TaskFetchFilter::default()
            },
        )
        .await
        .expect("github tasks");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].provider, "github");
    assert_eq!(tasks[0].labels, vec!["coverage"]);

    let clickup_ctx = ProviderContext {
        toolkit: "clickup".to_string(),
        connection_id: Some("conn-clickup".to_string()),
        ..ctx.clone()
    };
    let clickup = ClickUpProvider::new();
    let clickup_profile = clickup
        .fetch_user_profile(&clickup_ctx)
        .await
        .expect("clickup profile");
    assert_eq!(clickup_profile.username.as_deref(), Some("9988"));
    assert_eq!(clickup_profile.email.as_deref(), Some("click@example.test"));

    let clickup_tasks = clickup
        .fetch_tasks(
            &clickup_ctx,
            &TaskFetchFilter {
                team_id: Some("team_1".to_string()),
                list_id: Some("list_1".to_string()),
                max: 3,
                ..TaskFetchFilter::default()
            },
        )
        .await
        .expect("clickup tasks");
    assert_eq!(clickup_tasks.len(), 1);
    assert_eq!(clickup_tasks[0].provider, "clickup");
    assert_eq!(clickup_tasks[0].priority.as_deref(), Some("high"));

    let gmail = GmailProvider::new();
    let gmail_err = gmail
        .fetch_tasks(&ctx, &TaskFetchFilter::default())
        .await
        .expect_err("gmail has no task surface");
    assert!(gmail_err.contains("no task-fetch surface"));

    let slack = SlackProvider::new();
    assert_eq!(slack.toolkit_slug(), "slack");
    assert_eq!(slack.sync_interval_secs(), Some(15 * 60));
    assert!(slack.curated_tools().is_some());
    slack
        .on_trigger(&ctx, "message.created", &json!({"event": "ignored"}))
        .await
        .expect("slack trigger path is defensive");

    let bad_backfill = run_backfill_via_search(&ctx, 0)
        .await
        .expect_err("zero days rejected");
    assert!(!bad_backfill.trim().is_empty());

    assert!(!requests.lock().unwrap().is_empty());
    server.abort();
}

#[test]
fn composio_provider_registry_and_bus_subscribers_expose_stable_metadata() {
    init_default_composio_sync_providers();
    assert!(get_composio_sync_provider("slack").is_some());
    assert!(get_composio_sync_provider("github").is_some());
    assert!(get_composio_sync_provider("clickup").is_some());
    assert!(get_composio_sync_provider("missing").is_none());
    assert!(
        all_composio_sync_providers().len() >= 6,
        "default providers should include gmail/notion/slack/clickup/github/linear"
    );

    let trigger = ComposioTriggerSubscriber::new();
    let connection = ComposioConnectionCreatedSubscriber::new();
    let config_changed = ComposioConfigChangedSubscriber::new();
    assert_eq!(
        openhuman_core::core::event_bus::EventHandler::name(&trigger),
        "composio::trigger"
    );
    assert_eq!(
        openhuman_core::core::event_bus::EventHandler::domains(&trigger),
        Some(&["composio"][..])
    );
    assert_eq!(
        openhuman_core::core::event_bus::EventHandler::name(&connection),
        "composio::connection_created"
    );
    assert_eq!(
        openhuman_core::core::event_bus::EventHandler::name(&config_changed),
        "composio::config_changed"
    );

    for reason in [
        SyncReason::ConnectionCreated,
        SyncReason::Periodic,
        SyncReason::Manual,
    ] {
        assert!(!reason.as_str().is_empty());
    }
}

fn execute_response_for(body: &Value) -> Value {
    let tool = body.get("tool").and_then(Value::as_str).unwrap_or_default();
    let args = body.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let data = match tool {
        "GITHUB_GET_THE_AUTHENTICATED_USER" => json!({
            "login": "octo-round15",
            "name": "Round Fifteen",
            "email": "octo@example.test",
            "avatar_url": "https://example.test/avatar.png",
            "html_url": "https://github.com/octo-round15"
        }),
        "GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS" => json!({
            "items": [{
                "id": 1701,
                "number": 17,
                "title": "Cover provider task normalization",
                "body": "Add deterministic task fixture coverage",
                "html_url": "https://github.com/tinyhumansai/openhuman/issues/17",
                "state": "open",
                "updated_at": "2026-05-29T12:34:56Z",
                "labels": [{"name": "coverage"}],
                "user": {"login": "octo-round15"}
            }],
            "total_count": 1,
            "arguments_echo": args
        }),
        "CLICKUP_GET_AUTHORIZED_USER" => json!({
            "user": {
                "id": 9988,
                "username": "Click Round",
                "email": "click@example.test",
                "profilePicture": "https://example.test/click.png"
            }
        }),
        "CLICKUP_GET_AUTHORIZED_TEAMS_WORKSPACES" => json!({
            "teams": [{ "id": "team_1", "name": "Coverage Team" }]
        }),
        "CLICKUP_GET_FILTERED_TEAM_TASKS" => json!({
            "tasks": [{
                "id": "task_1",
                "name": "Exercise ClickUp normalization",
                "description": "Task body",
                "url": "https://app.clickup.com/t/task_1",
                "status": {"status": "in progress"},
                "assignees": [{"username": "Click Round"}],
                "date_updated": "1780046400000",
                "priority": {"priority": "high"},
                "tags": [{"name": "coverage"}]
            }],
            "arguments_echo": args
        }),
        _ => json!({ "tool": tool, "unhandled": true }),
    };
    json!({
        "data": data,
        "successful": true,
        "error": null,
        "costUsd": 0.0
    })
}

fn write_fake_gh(path: &PathBuf) {
    let script = r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then
  echo "gh version 2.0.0"
  exit 0
fi
if [[ "${1:-}" != "api" ]]; then
  echo "unsupported gh command" >&2
  exit 2
fi
case "${2:-}" in
  repos/tinyhumansai/openhuman/commits\?*)
    cat <<'JSON'
[{"sha":"abc123","commit":{"message":"Add coverage hooks\n\nMore details","author":{"name":"Ada","email":"ada@example.test","date":"2026-05-28T10:00:00Z"},"committer":{"name":"Ada","email":"ada@example.test","date":"2026-05-28T10:00:00Z"}}}]
JSON
    ;;
  repos/tinyhumansai/openhuman/issues\?*)
    cat <<'JSON'
[{"number":7,"title":"Memory source reader gap","body":"Needs fixture coverage","state":"open","user":{"login":"ada"},"labels":[{"name":"coverage"}],"created_at":"2026-05-27T10:00:00Z","updated_at":"2026-05-28T11:00:00Z","pull_request":null},{"number":99,"title":"PR-shaped issue","body":"","state":"open","user":{"login":"bot"},"labels":[],"created_at":"2026-05-27T10:00:00Z","updated_at":"2026-05-28T11:00:00Z","pull_request":{}}]
JSON
    ;;
  repos/tinyhumansai/openhuman/pulls\?*)
    cat <<'JSON'
[{"number":9,"title":"Raw coverage PR","body":"PR body","state":"open","user":{"login":"grace"},"labels":[{"name":"tests"}],"created_at":"2026-05-27T10:00:00Z","updated_at":"2026-05-28T12:00:00Z","merged_at":null,"comments":1}]
JSON
    ;;
  repos/tinyhumansai/openhuman/commits/abc123)
    cat <<'JSON'
{"sha":"abc123","commit":{"message":"Add coverage hooks\n\nMore details","author":{"name":"Ada","email":"ada@example.test","date":"2026-05-28T10:00:00Z"},"committer":{"name":"Ada","email":"ada@example.test","date":"2026-05-28T10:00:00Z"}}}
JSON
    ;;
  repos/tinyhumansai/openhuman/issues/7)
    cat <<'JSON'
{"number":7,"title":"Memory source reader gap","body":"Needs fixture coverage","state":"open","user":{"login":"ada"},"labels":[{"name":"coverage"}],"created_at":"2026-05-27T10:00:00Z","updated_at":"2026-05-28T11:00:00Z","pull_request":null}
JSON
    ;;
  repos/tinyhumansai/openhuman/pulls/9)
    cat <<'JSON'
{"number":9,"title":"Raw coverage PR","body":"PR body","state":"open","user":{"login":"grace"},"labels":[{"name":"tests"}],"created_at":"2026-05-27T10:00:00Z","updated_at":"2026-05-28T12:00:00Z","merged_at":null,"comments":1}
JSON
    ;;
  repos/tinyhumansai/openhuman/issues/7/comments\?*|repos/tinyhumansai/openhuman/issues/9/comments\?*)
    cat <<'JSON'
[{"user":{"login":"reviewer"},"body":"Looks deterministic","created_at":"2026-05-28T13:00:00Z"}]
JSON
    ;;
  *)
    echo "unexpected gh api path: ${2:-}" >&2
    exit 3
    ;;
esac
"#;
    std::fs::write(path, script).expect("write fake gh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .expect("fake gh metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod fake gh");
    }
}

fn write_fake_git(path: &PathBuf) {
    let script = r#"#!/usr/bin/env bash
# Fake git that fails on clone/fetch so the GitHub reader falls through to the
# gh CLI API path (which is intercepted by write_fake_gh).
case "${1:-}" in
  clone|fetch)
    echo "fatal: repository not found" >&2
    exit 128
    ;;
  *)
    exec /usr/bin/git "$@"
    ;;
esac
"#;
    std::fs::write(path, script).expect("write fake git");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .expect("fake git metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod fake git");
    }
}
