//! Round 20 near-90 raw integration coverage closures.
//!
//! All fixtures are local and deterministic: temp workspaces, loopback HTTP,
//! and a fake `gh` binary. Run with `--test-threads=1`; several covered
//! surfaces resolve config/workspace through process environment.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration as StdDuration;

use chrono::Utc;
use openhuman_core::openhuman::app_state::{
    snapshot, update_local_state, StoredAppStatePatch, StoredOnboardingTasks,
};
use openhuman_core::openhuman::config::rpc as config_rpc;
use openhuman_core::openhuman::credentials::profiles::{
    AuthProfile, AuthProfileKind, AuthProfilesStore,
};
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::memory::{
    ai_list_memory_files, ai_read_memory_file, ai_write_memory_file, clear_namespace,
    context_query, context_recall, doc_delete, doc_list, doc_put, memory_delete_document,
    memory_init, memory_list_documents, memory_list_namespaces, memory_query_namespace,
    memory_recall_context, memory_recall_memories, namespace_list, ClearNamespaceParams,
    DeleteDocParams, EmptyRequest, ListDocumentsRequest, ListMemoryFilesRequest, MemoryInitRequest,
    PutDocParams, QueryNamespaceParams, QueryNamespaceRequest, ReadMemoryFileRequest,
    RecallContextRequest, RecallMemoriesRequest, RecallNamespaceParams, WriteMemoryFileRequest,
};
use openhuman_core::openhuman::memory_sources::readers::SourceReader;
use openhuman_core::openhuman::memory_sources::sync::sync_source;
use openhuman_core::openhuman::memory_sources::{ContentType, MemorySourceEntry, SourceKind};
use openhuman_core::openhuman::threads::ops as thread_ops;
use openhuman_core::openhuman::threads::welcome_migration::migrate_welcome_agent_artifacts;
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

static ROUND20_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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

    fn set_path(key: &'static str, path: &Path) -> Self {
        Self::set(key, path.to_string_lossy().into_owned())
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

struct Harness {
    _tmp: TempDir,
    root: PathBuf,
    _guards: Vec<EnvGuard>,
}

impl Harness {
    async fn config(&self) -> openhuman_core::openhuman::config::Config {
        config_rpc::load_config_with_timeout()
            .await
            .expect("isolated config should load")
    }

    fn workspace_dir(&self) -> PathBuf {
        self.root.join("workspace")
    }

    fn state_dir(&self) -> PathBuf {
        self.workspace_dir().join("state")
    }

    fn app_state_file(&self) -> PathBuf {
        self.state_dir().join("app-state.json")
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ROUND20_ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("create target");
    Builder::new()
        .prefix("near90-closure-round20-")
        .tempdir_in("target")
        .expect("round20 tempdir")
}

fn write_min_config(root: &Path, api_url: &str) {
    std::fs::create_dir_all(root).expect("create config root");
    let cfg = format!(
        r#"api_url = "{api_url}"
default_model = "round20-coverage-model"
default_temperature = 0.2
onboarding_completed = true
chat_onboarding_completed = false

[observability]
analytics_enabled = false

[secrets]
encrypt = false

[meet]
auto_orchestrator_handoff = false

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
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(&cfg).expect("round20 config must match schema");
}

fn setup(api_url: &str) -> Harness {
    let tmp = tempdir();
    let root = tmp.path().join("openhuman");
    write_min_config(&root, api_url);
    let guards = vec![
        EnvGuard::set_path("OPENHUMAN_WORKSPACE", &root),
        EnvGuard::set_path("HOME", tmp.path()),
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
        _guards: guards,
    }
}

fn source_entry(id: &str, kind: SourceKind) -> MemorySourceEntry {
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
        selector: None,
    }
}

async fn auth_me_server(
    status: &'static str,
    body: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind auth fixture");
    let url = format!("http://{}", listener.local_addr().expect("listener addr"));
    let task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut req = [0_u8; 2048];
            let _ = stream.read(&mut req).await;
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    (url, task)
}

#[tokio::test]
async fn round20_app_state_quarantines_directory_state_and_uses_stored_user_on_http_error() {
    let _lock = env_lock();
    let (api_url, server) = auth_me_server("503 Service Unavailable", r#"{"error":"down"}"#).await;
    let harness = setup(&api_url);
    let config = harness.config().await;

    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "round20.remote.token",
            HashMap::from([
                ("user_id".to_string(), "stored-round20".to_string()),
                (
                    "user_json".to_string(),
                    json!({"id":"stored-round20","fullName":"Stored Round20"}).to_string(),
                ),
            ]),
            true,
        )
        .expect("seed app session");

    let first = update_local_state(StoredAppStatePatch {
        encryption_key: None,
        onboarding_tasks: Some(Some(StoredOnboardingTasks {
            accessibility_permission_granted: false,
            local_model_consent_given: true,
            local_model_download_started: false,
            enabled_tools: vec!["github".to_string()],
            connected_sources: vec!["rss".to_string()],
            updated_at_ms: Some(20),
        })),
    })
    .await
    .expect("write tasks")
    .value;
    assert!(first.encryption_key.is_none());
    assert_eq!(
        first.onboarding_tasks.expect("tasks").enabled_tools,
        vec!["github"]
    );

    let snap = snapshot().await.expect("snapshot").value;
    assert_eq!(
        snap.current_user.as_ref().and_then(|v| v.get("id")),
        Some(&json!("stored-round20"))
    );
    assert_eq!(snap.session_token.as_deref(), Some("round20.remote.token"));
    assert!(!snap.analytics_enabled);
    assert!(!snap.meet_auto_orchestrator_handoff);

    std::fs::remove_file(harness.app_state_file()).expect("remove app-state file");
    std::fs::create_dir_all(harness.app_state_file()).expect("directory at app-state path");
    let recovered = snapshot()
        .await
        .expect("snapshot should quarantine unreadable app-state path")
        .value;
    assert!(recovered.local_state.onboarding_tasks.is_none());
    assert!(!harness.app_state_file().exists());
    assert!(
        std::fs::read_dir(harness.state_dir())
            .expect("state dir")
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains("json.corrupted")),
        "directory app-state should be moved aside"
    );

    server.abort();
}

#[test]
fn round20_credentials_profiles_cover_legacy_plaintext_errors_and_active_edges() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let state_dir = harness.root.join("profile-store");
    let store = AuthProfilesStore::new(&state_dir, false);

    let token = AuthProfile::new_token("linear", "team", "lin-round20".to_string());
    store
        .upsert_profile(token.clone(), false)
        .expect("insert token");
    let inactive = store.load().expect("load inactive token");
    assert!(!inactive.active_profiles.contains_key("linear"));
    store
        .set_active_profile("linear", &token.id)
        .expect("activate token");
    assert_eq!(
        store
            .load()
            .expect("load active")
            .active_profiles
            .get("linear"),
        Some(&token.id)
    );

    let path = store.path().to_path_buf();
    let now = Utc::now().to_rfc3339();
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "updated_at": now,
            "active_profiles": { "github": "legacy-oauth", "token": "legacy-token" },
            "profiles": {
                "legacy-token": {
                    "provider": "token",
                    "profile_name": "plain",
                    "kind": "token",
                    "token": "plain-secret",
                    "metadata": { "k": "v" },
                    "created_at": "bad-created",
                    "updated_at": "bad-updated"
                },
                "legacy-oauth": {
                    "provider": "github",
                    "profile_name": "oauth",
                    "kind": "oauth",
                    "access_token": "plain-access",
                    "refresh_token": "plain-refresh",
                    "id_token": "",
                    "expires_at": null,
                    "token_type": "Bearer",
                    "scope": "repo",
                    "metadata": {},
                    "created_at": "2026-05-29T00:00:00Z",
                    "updated_at": "2026-05-29T00:00:00Z"
                }
            }
        }))
        .expect("profile json"),
    )
    .expect("write legacy plaintext store");

    let loaded = store.load().expect("load plaintext legacy profiles");
    assert_eq!(
        loaded
            .profiles
            .get("legacy-token")
            .and_then(|profile| profile.token.as_deref()),
        Some("plain-secret")
    );
    let oauth = loaded.profiles.get("legacy-oauth").expect("oauth loaded");
    assert_eq!(oauth.kind, AuthProfileKind::OAuth);
    assert_eq!(
        oauth.token_set.as_ref().map(|tokens| (
            tokens.access_token.as_str(),
            tokens.refresh_token.as_deref(),
            tokens.expires_at
        )),
        Some(("plain-access", Some("plain-refresh"), None))
    );

    let set_active_err = store
        .set_active_profile("missing", "not-there")
        .expect_err("missing active profile")
        .to_string();
    assert!(set_active_err.contains("Auth profile not found"));
}

#[tokio::test]
async fn round20_memory_sources_readers_and_sync_cover_error_edges_without_network() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let config = harness.config().await;

    let rss = openhuman_core::openhuman::memory_sources::readers::rss::RssReader;
    let mut missing_url = source_entry("rss-missing-url", SourceKind::RssFeed);
    assert_eq!(
        rss.list_items(&missing_url, &config)
            .await
            .expect_err("rss url required"),
        "rss source requires a url"
    );

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind feed fixture");
    let feed_url = format!("http://{}", listener.local_addr().expect("addr"));
    let server = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut req = [0_u8; 1024];
            let _ = stream.read(&mut req).await;
            let response =
                "HTTP/1.1 200 OK\r\ncontent-type: text/xml\r\ncontent-length: 10\r\n\r\nnot-a-feed";
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    missing_url.url = Some(feed_url);
    let feed_err = rss
        .list_items(&missing_url, &config)
        .await
        .expect_err("unrecognized feed rejected");
    assert!(feed_err.contains("unrecognized feed format"));
    let _ = server.await;

    let tmp = tempdir();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("bin dir");
    let script = bin.join("gh");
    write_fake_gh_round20(&script);
    let old_path = std::env::var("PATH").unwrap_or_default();
    let _path = EnvGuard::set("PATH", format!("{}:{old_path}", bin.display()));

    let github = openhuman_core::openhuman::memory_sources::readers::github::GithubReader;
    let mut entry = source_entry("github-round20", SourceKind::GithubRepo);
    entry.url = Some("git@github.com:tinyhumansai/openhuman.git".to_string());
    let items = github
        .list_items(&entry, &config)
        .await
        .expect("github list via fake gh");
    assert!(items.iter().any(|item| item.id == "commit:def456"));
    assert!(items.iter().any(|item| item.id == "issue:20"));

    let pr = github
        .read_item(&entry, "pr:21", &config)
        .await
        .expect("read merged pr");
    assert_eq!(pr.content_type, ContentType::Markdown);
    assert!(pr.body.contains("merged at 2026-05-29T01:00:00Z"));
    assert_eq!(
        pr.metadata.get("merged").and_then(Value::as_bool),
        Some(true)
    );

    let bad_issue = github
        .read_item(&entry, "issue:not-a-number", &config)
        .await
        .expect_err("bad issue number");
    assert!(bad_issue.contains("invalid issue number"));

    let mut disabled = source_entry("disabled-twitter", SourceKind::TwitterQuery);
    disabled.enabled = false;
    let disabled_err = sync_source(disabled, config.clone())
        .await
        .expect_err("disabled sync rejected");
    assert!(disabled_err.contains("is disabled"));

    let twitter = source_entry("twitter-round20", SourceKind::TwitterQuery);
    sync_source(twitter, config)
        .await
        .expect("twitter placeholder is reported by background task");
    tokio::time::sleep(StdDuration::from_millis(25)).await;
}

#[tokio::test]
async fn round20_memory_documents_files_and_envelopes_cover_success_and_failure_paths() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");

    let init = memory_init(MemoryInitRequest {
        jwt_token: Some("ignored-round20".to_string()),
    })
    .await
    .expect("memory init")
    .value
    .data
    .expect("init data");
    assert!(init.initialized);
    assert!(init.memory_dir.ends_with("/memory"));

    ai_write_memory_file(WriteMemoryFileRequest {
        relative_path: "notes/round20.md".to_string(),
        content: "Round20 local file memory".to_string(),
    })
    .await
    .expect("write memory file");
    let read = ai_read_memory_file(ReadMemoryFileRequest {
        relative_path: "notes/round20.md".to_string(),
    })
    .await
    .expect("read memory file")
    .value
    .data
    .expect("read data");
    assert!(read.content.contains("local file memory"));
    let traversal = ai_write_memory_file(WriteMemoryFileRequest {
        relative_path: "../escape.md".to_string(),
        content: "no".to_string(),
    })
    .await
    .expect_err("traversal rejected");
    assert!(traversal.contains("path traversal"));
    let listed = ai_list_memory_files(ListMemoryFilesRequest {
        relative_dir: "notes".to_string(),
    })
    .await
    .expect("list notes")
    .value
    .data
    .expect("list data");
    assert_eq!(listed.files, vec!["round20.md"]);

    let namespace = format!(
        "round20-{}",
        &uuid::Uuid::new_v4().as_simple().to_string()[..12]
    );
    let put = doc_put(PutDocParams {
        namespace: namespace.clone(),
        key: "launch-note".to_string(),
        title: "Launch Note".to_string(),
        content: "The Calypso launch depends on QA, design, and release owners.".to_string(),
        source_type: "note".to_string(),
        priority: "high".to_string(),
        tags: vec!["round20".to_string()],
        metadata: json!({"round": 20}),
        category: "core".to_string(),
        session_id: Some("session-round20".to_string()),
        document_id: Some("doc-round20".to_string()),
    })
    .await
    .expect("put doc")
    .value;
    assert_eq!(put.document_id, "doc-round20");

    let docs = doc_list(None)
        .await
        .expect("doc list all")
        .value
        .get("documents")
        .and_then(Value::as_array)
        .cloned()
        .expect("documents");
    assert!(docs.iter().any(|doc| doc["documentId"] == "doc-round20"));
    let namespaces = namespace_list().await.expect("namespace list").value;
    assert!(namespaces.contains(&namespace));

    let query = context_query(QueryNamespaceParams {
        namespace: namespace.clone(),
        query: "Calypso QA".to_string(),
        limit: Some(3),
    })
    .await
    .expect("context query")
    .value;
    assert!(query.contains("Calypso") || query.contains("launch"));
    let recall = context_recall(RecallNamespaceParams {
        namespace: namespace.clone(),
        limit: Some(3),
    })
    .await
    .expect("context recall")
    .value;
    assert!(recall.as_deref().unwrap_or_default().contains("Calypso"));

    let envelope_docs = memory_list_documents(ListDocumentsRequest {
        namespace: Some(namespace.clone()),
    })
    .await
    .expect("memory list documents")
    .value
    .data
    .expect("list documents data");
    assert_eq!(envelope_docs.count, 1);
    let envelope_namespaces = memory_list_namespaces(EmptyRequest {})
        .await
        .expect("memory namespaces")
        .value
        .data
        .expect("namespaces data");
    assert!(envelope_namespaces.namespaces.contains(&namespace));

    let hidden_context = memory_query_namespace(QueryNamespaceRequest {
        namespace: namespace.clone(),
        query: "Calypso".to_string(),
        include_references: Some(false),
        document_ids: Some(vec!["doc-round20".to_string()]),
        limit: Some(1),
        max_chunks: Some(2),
    })
    .await
    .expect("query namespace envelope")
    .value
    .data
    .expect("query namespace data");
    assert!(hidden_context.context.is_none());
    assert!(hidden_context.llm_context_message.is_some());

    let recall_context = memory_recall_context(RecallContextRequest {
        namespace: namespace.clone(),
        include_references: Some(true),
        limit: None,
        max_chunks: Some(1),
    })
    .await
    .expect("recall context envelope")
    .value
    .data
    .expect("recall context data");
    assert!(recall_context.llm_context_message.is_some());

    let memories = memory_recall_memories(RecallMemoriesRequest {
        namespace: namespace.clone(),
        min_retention: Some(0.0),
        as_of: Some(0.0),
        limit: Some(1),
        max_chunks: None,
        top_k: Some(1),
    })
    .await
    .expect("recall memories")
    .value
    .data
    .expect("memories data");
    assert!(!memories.memories.is_empty());

    let deleted =
        memory_delete_document(openhuman_core::openhuman::memory::DeleteDocumentRequest {
            namespace: namespace.clone(),
            document_id: "doc-round20".to_string(),
        })
        .await
        .expect("delete document envelope")
        .value
        .data
        .expect("delete data");
    assert!(deleted.deleted);
    let second_delete = doc_delete(DeleteDocParams {
        namespace: namespace.clone(),
        document_id: "doc-round20".to_string(),
    })
    .await
    .expect("direct delete missing")
    .value;
    assert_eq!(second_delete["deleted"], false);
    let cleared = clear_namespace(ClearNamespaceParams { namespace })
        .await
        .expect("clear namespace")
        .value;
    assert!(cleared.cleared);

    let _ = harness;
}

#[tokio::test]
async fn round20_threads_fallback_title_delete_missing_and_welcome_noop_paths() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");

    let created = thread_ops::thread_create_new(
        openhuman_core::openhuman::memory::CreateConversationThreadRequest {
            labels: None,
            personality_id: None,
        },
    )
    .await
    .expect("create thread")
    .value
    .data
    .expect("created data");

    let user_message = openhuman_core::openhuman::memory::ConversationMessageRecord {
        id: "round20-user-msg".to_string(),
        content: "Please map the onboarding telemetry rollout across product analytics and QA."
            .to_string(),
        message_type: "text".to_string(),
        extra_metadata: Value::Null,
        sender: "user".to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    thread_ops::message_append(
        openhuman_core::openhuman::memory::AppendConversationMessageRequest {
            thread_id: created.id.clone(),
            message: user_message,
        },
    )
    .await
    .expect("append user");

    let fallback = thread_ops::thread_generate_title(
        openhuman_core::openhuman::memory::GenerateConversationThreadTitleRequest {
            thread_id: created.id.clone(),
            assistant_message: None,
        },
    )
    .await
    .expect("fallback title")
    .value
    .data
    .expect("fallback data");
    assert!(!fallback.title.trim().is_empty());
    assert_ne!(fallback.title, created.title);

    let missing_update = thread_ops::message_update(
        openhuman_core::openhuman::memory::UpdateConversationMessageRequest {
            thread_id: created.id.clone(),
            message_id: "missing-message".to_string(),
            extra_metadata: Some(json!({"x": true})),
        },
    )
    .await
    .expect_err("missing message update fails");
    assert!(missing_update.contains("message") || missing_update.contains("not found"));

    let missing_delete = thread_ops::thread_delete(
        openhuman_core::openhuman::memory::DeleteConversationThreadRequest {
            thread_id: "missing-thread-round20".to_string(),
            deleted_at: Utc::now().to_rfc3339(),
        },
    )
    .await
    .expect("missing thread delete is idempotent")
    .value
    .data
    .expect("delete data");
    assert!(!missing_delete.deleted);

    std::fs::create_dir_all(harness.workspace_dir().join("session_raw")).expect("raw dir");
    std::fs::write(
        harness.workspace_dir().join("session_raw/ignore.jsonl"),
        "{\"_meta\":{\"agent\":\"orchestrator\",\"thread_id\":\"t\"}}\n",
    )
    .expect("write non-welcome transcript");
    let migration =
        migrate_welcome_agent_artifacts(&harness.workspace_dir()).expect("noop migration");
    assert_eq!(migration.threads_updated, 0);
    assert_eq!(migration.transcripts_updated, 0);
}

fn write_fake_gh_round20(path: &PathBuf) {
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
  repos/tinyhumansai/openhuman/commits?per_page=30)
    cat <<'JSON'
[{"sha":"def456","commit":{"message":"Round20 commit fixture","author":{"name":"Ada","email":"ada@example.test","date":"2026-05-29T00:00:00Z"},"committer":{"name":"Ada","email":"ada@example.test","date":"2026-05-29T00:00:00Z"}}}]
JSON
    ;;
  repos/tinyhumansai/openhuman/issues?per_page=30\&state=all)
    cat <<'JSON'
[{"number":20,"title":"Round20 issue","body":null,"state":"closed","user":null,"labels":[],"created_at":null,"updated_at":"2026-05-29T00:30:00Z","pull_request":null}]
JSON
    ;;
  repos/tinyhumansai/openhuman/pulls?per_page=30\&state=all)
    cat <<'JSON'
[{"number":21,"title":"Round20 merged PR","body":null,"state":"closed","user":null,"labels":[],"created_at":null,"updated_at":"2026-05-29T01:00:00Z","merged_at":"2026-05-29T01:00:00Z","comments":0}]
JSON
    ;;
  repos/tinyhumansai/openhuman/pulls/21)
    cat <<'JSON'
{"number":21,"title":"Round20 merged PR","body":null,"state":"closed","user":null,"labels":[],"created_at":null,"updated_at":"2026-05-29T01:00:00Z","merged_at":"2026-05-29T01:00:00Z","comments":0}
JSON
    ;;
  repos/tinyhumansai/openhuman/issues/21/comments?per_page=50)
    cat <<'JSON'
[]
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
