use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use openhuman_core::openhuman::app_state::{
    snapshot, update_local_state, StoredAppStatePatch, StoredOnboardingTasks,
};
use openhuman_core::openhuman::config::rpc as config_rpc;
use openhuman_core::openhuman::credentials::profiles::{
    profile_id, AuthProfile, AuthProfilesStore, TokenSet,
};
use openhuman_core::openhuman::credentials::{
    list_provider_credentials_by_prefix, AuthService, APP_SESSION_PROVIDER,
    DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::memory::{
    AppendConversationMessageRequest, ConversationMessageRecord, ConversationMessagesRequest,
    CreateConversationThreadRequest, DeleteConversationThreadRequest, EmptyRequest,
    GenerateConversationThreadTitleRequest, UpdateConversationMessageRequest,
    UpdateConversationThreadLabelsRequest, UpdateConversationThreadTitleRequest,
};
use openhuman_core::openhuman::memory_sources::readers::SourceReader;
use openhuman_core::openhuman::memory_sources::{
    self, MemorySourceEntry, MemorySourcePatch, SourceKind,
};
use openhuman_core::openhuman::threads::{migrate_welcome_agent_artifacts, ops as thread_ops};
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

static ROUND19_ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

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
    ROUND19_ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("create target");
    Builder::new()
        .prefix("app-credentials-threads-memory-sources-round19-")
        .tempdir_in("target")
        .expect("round19 tempdir")
}

fn write_min_config(root: &Path, api_url: &str) {
    std::fs::create_dir_all(root).expect("create config root");
    let cfg = format!(
        r#"api_url = "{api_url}"
default_model = "round19-coverage-model"
default_temperature = 0.2
onboarding_completed = true
chat_onboarding_completed = false

[observability]
analytics_enabled = true

[secrets]
encrypt = false

[meet]
auto_orchestrator_handoff = true

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
        toml::from_str(&cfg).expect("round19 config must match schema");
}

fn setup(api_url: &str) -> Harness {
    let tmp = tempdir();
    let root = tmp.path().join("openhuman");
    write_min_config(&root, api_url);
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
        _guards: guards,
    }
}

fn source_entry(id: &str, kind: SourceKind, label: &str) -> MemorySourceEntry {
    MemorySourceEntry {
        id: id.to_string(),
        kind,
        label: label.to_string(),
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

async fn one_response_server(
    body: &'static str,
    content_type: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind fixture listener");
    let url = format!("http://{}", listener.local_addr().expect("listener addr"));
    let task = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut req = [0_u8; 2048];
            let _ = stream.read(&mut req).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
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
async fn round19_app_state_local_state_snapshot_and_corruption_edges() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let config = harness.config().await;

    let mut metadata = HashMap::new();
    metadata.insert("user_id".to_string(), "round19-user".to_string());
    metadata.insert(
        "user_json".to_string(),
        json!({
            "id": "round19-user",
            "fullName": "Round Nineteen",
            "email": "round19@example.test"
        })
        .to_string(),
    );
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "local-dev-token-round19",
            metadata,
            true,
        )
        .expect("seed local app session");

    let updated = update_local_state(StoredAppStatePatch {
        keyring_consent: None,
        encryption_key: Some(Some("  round19-key  ".to_string())),
        onboarding_tasks: Some(Some(StoredOnboardingTasks {
            accessibility_permission_granted: true,
            local_model_consent_given: false,
            local_model_download_started: true,
            enabled_tools: vec!["rss".to_string()],
            connected_sources: vec!["folder".to_string(), "github".to_string()],
            updated_at_ms: Some(19),
        })),
    })
    .await
    .expect("write local app state")
    .value;
    assert_eq!(updated.encryption_key.as_deref(), Some("round19-key"));
    assert_eq!(
        updated
            .onboarding_tasks
            .as_ref()
            .expect("tasks")
            .connected_sources,
        vec!["folder", "github"]
    );

    let snap = snapshot().await.expect("snapshot").value;
    assert!(snap.auth.is_authenticated);
    assert_eq!(
        snap.session_token.as_deref(),
        Some("local-dev-token-round19")
    );
    assert_eq!(snap.auth.user_id.as_deref(), Some("round19-user"));
    assert_eq!(
        snap.current_user.as_ref().and_then(|v| v.get("fullName")),
        Some(&json!("Round Nineteen"))
    );
    assert!(snap.onboarding_completed);
    assert!(snap.analytics_enabled);
    assert!(snap.meet_auto_orchestrator_handoff);

    let cleared = update_local_state(StoredAppStatePatch {
        keyring_consent: None,
        encryption_key: Some(Some("   ".to_string())),
        onboarding_tasks: Some(None),
    })
    .await
    .expect("clear local app state")
    .value;
    assert!(cleared.encryption_key.is_none());
    assert!(cleared.onboarding_tasks.is_none());

    std::fs::create_dir_all(harness.state_dir()).expect("state dir");
    std::fs::write(harness.app_state_file(), b"{not-json").expect("corrupt app state");
    let recovered = snapshot()
        .await
        .expect("snapshot quarantines corrupt state")
        .value;
    assert!(recovered.local_state.encryption_key.is_none());
    assert!(
        !harness.app_state_file().exists(),
        "corrupt app-state.json should be moved aside"
    );
    let has_quarantine = std::fs::read_dir(harness.state_dir())
        .expect("state entries")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains("json.corrupted")
        });
    assert!(has_quarantine, "corrupt app state should leave artifact");
}

#[test]
fn round19_credentials_profile_mutation_errors_and_secret_fallbacks() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let state_dir = harness.root.join("profile-store");
    let store = AuthProfilesStore::new(&state_dir, false);

    assert_eq!(profile_id(" github ", " work "), "github:work");
    let expiring = TokenSet {
        access_token: "access".into(),
        refresh_token: Some("refresh".into()),
        id_token: Some("id".into()),
        expires_at: Some(Utc::now() + chrono::Duration::seconds(5)),
        token_type: Some("Bearer".into()),
        scope: Some("repo".into()),
    };
    assert!(expiring.is_expiring_within(std::time::Duration::from_secs(10)));

    let mut oauth = AuthProfile::new_oauth("github", "work", expiring);
    oauth.metadata = BTreeMap::from([("team".to_string(), "core".to_string())]);
    store
        .upsert_profile(oauth.clone(), true)
        .expect("insert oauth");
    let updated = store
        .update_profile(&oauth.id, |profile| {
            profile.workspace_id = Some("workspace-round19".to_string());
            profile.metadata.insert("updated".into(), "yes".into());
            Ok(())
        })
        .expect("update profile");
    assert_eq!(updated.workspace_id.as_deref(), Some("workspace-round19"));

    let updater_err = store
        .update_profile(&oauth.id, |_profile| {
            anyhow::bail!("round19 updater failed")
        })
        .expect_err("updater error should propagate")
        .to_string();
    assert!(updater_err.contains("round19 updater failed"));
    let missing_err = store
        .update_profile("missing-profile", |_profile| Ok(()))
        .expect_err("missing update should fail")
        .to_string();
    assert!(missing_err.contains("Auth profile not found"));

    store
        .clear_active_profile("github")
        .expect("clear active profile");
    assert!(store
        .load()
        .expect("load after clear")
        .active_profiles
        .get("github")
        .is_none());
    store
        .set_active_profile("github", &oauth.id)
        .expect("reactivate profile");
    assert!(!store
        .remove_profile("missing-profile")
        .expect("remove missing profile"));
    assert!(store.remove_profile(&oauth.id).expect("remove oauth"));
    assert!(store.load().expect("load after remove").profiles.is_empty());
}

#[tokio::test]
async fn round19_credentials_service_prefix_and_corrupt_store_recovery() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let config = harness.config().await;
    let auth = AuthService::from_config(&config);

    auth.store_provider_token(
        "channel:slack:bot",
        "primary",
        "xoxb-round19",
        HashMap::from([("team_id".to_string(), "T19".to_string())]),
        true,
    )
    .expect("store slack token");
    auth.store_provider_token(
        "channel:telegram:managed_dm",
        "primary",
        "telegram-round19",
        HashMap::new(),
        true,
    )
    .expect("store telegram token");
    auth.store_provider_token("github", "work", "ghp-round19", HashMap::new(), true)
        .expect("store github token");

    let channels = list_provider_credentials_by_prefix(&config, "channel:")
        .await
        .expect("list channel credentials");
    assert_eq!(
        channels
            .iter()
            .map(|profile| profile.provider.as_str())
            .collect::<Vec<_>>(),
        vec!["channel:slack:bot", "channel:telegram:managed_dm"]
    );
    assert!(channels
        .iter()
        .any(|profile| profile.metadata_keys == vec!["team_id"]));

    let store = AuthProfilesStore::new(&harness.state_dir().join("credentials"), false);
    let path = store.path().to_path_buf();
    std::fs::create_dir_all(path.parent().expect("profile parent")).expect("profile dir");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "updated_at": Utc::now().to_rfc3339(),
            "active_profiles": { "bad": "legacy-bad-kind" },
            "profiles": {
                "legacy-bad-kind": {
                    "provider": "bad",
                    "profile_name": "legacy",
                    "kind": "api_key",
                    "token": "plain-token",
                    "created_at": Utc::now().to_rfc3339(),
                    "updated_at": Utc::now().to_rfc3339()
                }
            }
        }))
        .expect("profile json"),
    )
    .expect("write bad profile");
    let recovered = store.load().expect("bad kind should be dropped");
    assert!(recovered.profiles.is_empty());
    assert!(recovered.active_profiles.is_empty());

    std::fs::write(&path, "{broken").expect("write corrupt profile store");
    let empty = store.load().expect("corrupt store quarantined");
    assert!(empty.profiles.is_empty());
    assert!(!path.exists());
}

#[tokio::test]
async fn round19_threads_ops_cover_title_message_delete_and_purge_edges() {
    let _lock = env_lock();
    let _harness = setup("http://127.0.0.1:9");

    let created = thread_ops::thread_create_new(CreateConversationThreadRequest {
        labels: Some(vec!["personal".to_string(), "onboarding".to_string()]),
        personality_id: Some("coach".to_string()),
    })
    .await
    .expect("create thread")
    .value
    .data
    .expect("created thread");
    assert!(created.title.starts_with("Chat "));
    assert_eq!(created.personality_id.as_deref(), Some("coach"));

    let empty_title = thread_ops::thread_update_title(UpdateConversationThreadTitleRequest {
        thread_id: created.id.clone(),
        title: "   ".to_string(),
    })
    .await
    .expect_err("empty title rejected");
    assert!(empty_title.contains("title must not be empty"));

    let renamed = thread_ops::thread_update_title(UpdateConversationThreadTitleRequest {
        thread_id: created.id.clone(),
        title: "  Durable user title  ".to_string(),
    })
    .await
    .expect("rename thread")
    .value
    .data
    .expect("renamed thread");
    assert_eq!(renamed.title, "Durable user title");

    let labels = thread_ops::thread_update_labels(UpdateConversationThreadLabelsRequest {
        thread_id: created.id.clone(),
        labels: Vec::new(),
    })
    .await
    .expect("clear labels")
    .value
    .data
    .expect("labels response");
    assert!(labels.labels.is_empty());

    let user_message = ConversationMessageRecord {
        id: "msg-user".to_string(),
        content: "Plan the June launch checklist with design, QA, and release owners.".to_string(),
        message_type: "text".to_string(),
        extra_metadata: json!({"source":"round19"}),
        sender: "user".to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    thread_ops::message_append(AppendConversationMessageRequest {
        thread_id: created.id.clone(),
        message: user_message.clone(),
    })
    .await
    .expect("append user message");
    let updated_message = thread_ops::message_update(UpdateConversationMessageRequest {
        thread_id: created.id.clone(),
        message_id: user_message.id.clone(),
        extra_metadata: Some(json!({"edited": true})),
    })
    .await
    .expect("update message")
    .value
    .data
    .expect("message data");
    assert_eq!(updated_message.extra_metadata["edited"], true);

    let messages = thread_ops::messages_list(ConversationMessagesRequest {
        thread_id: created.id.clone(),
    })
    .await
    .expect("list messages")
    .value
    .data
    .expect("messages");
    assert_eq!(messages.count, 1);

    let non_placeholder =
        thread_ops::thread_generate_title(GenerateConversationThreadTitleRequest {
            thread_id: created.id.clone(),
            assistant_message: Some("Here is a concise plan.".to_string()),
        })
        .await
        .expect("non-placeholder skips generation")
        .value
        .data
        .expect("title generation response");
    assert_eq!(non_placeholder.title, "Durable user title");

    let listed = thread_ops::threads_list(EmptyRequest {})
        .await
        .expect("list threads")
        .value
        .data
        .expect("thread list");
    assert_eq!(listed.count, 1);

    let missing_append = thread_ops::message_append(AppendConversationMessageRequest {
        thread_id: "missing-thread".to_string(),
        message: ConversationMessageRecord {
            id: "missing-msg".to_string(),
            content: "hello".to_string(),
            message_type: "text".to_string(),
            extra_metadata: Value::Null,
            sender: "user".to_string(),
            created_at: Utc::now().to_rfc3339(),
        },
    })
    .await
    .expect_err("missing thread should map to ThreadsError");
    assert_eq!(
        missing_append.to_string(),
        "thread missing-thread not found"
    );

    let deleted = thread_ops::thread_delete(DeleteConversationThreadRequest {
        thread_id: created.id.clone(),
        deleted_at: Utc::now().to_rfc3339(),
    })
    .await
    .expect("delete thread")
    .value
    .data
    .expect("delete response");
    assert!(deleted.deleted);

    let purged = thread_ops::threads_purge(EmptyRequest {})
        .await
        .expect("purge empty")
        .value
        .data
        .expect("purge response");
    assert_eq!(purged.agent_threads_deleted, 0);
}

#[test]
fn round19_welcome_migration_handles_renames_collisions_and_marker() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let workspace = harness.workspace_dir();
    std::fs::create_dir_all(workspace.join("session_raw")).expect("raw dir");

    let blocked = workspace.join("session_raw/1715000000_welcome_thread-abc.jsonl");
    write_transcript(&blocked, "welcome_thread-abc", "thread-abc");
    let collision = workspace.join("session_raw/1715000000_orchestrator_thread-abc.jsonl");
    write_transcript(&collision, "orchestrator_thread-abc", "thread-abc");
    let err = migrate_welcome_agent_artifacts(&workspace)
        .expect_err("destination collision should fail migration");
    assert!(err.contains("partial migration"));
    assert!(std::fs::read_to_string(&blocked)
        .expect("blocked transcript")
        .contains("\"agent\":\"welcome_thread-abc\""));

    std::fs::remove_file(collision).expect("remove collision");
    let result = migrate_welcome_agent_artifacts(&workspace).expect("retry migration");
    assert_eq!(result.transcripts_updated, 1);
    assert_eq!(result.transcript_files_renamed, 1);
    assert!(workspace
        .join("session_raw/1715000000_orchestrator_thread-abc.jsonl")
        .exists());

    let again = migrate_welcome_agent_artifacts(&workspace).expect("marker skip");
    assert!(again.already_done);
}

fn write_transcript(path: &Path, agent: &str, thread_id: &str) {
    let body = format!(
        "{{\"_meta\":{{\"agent\":\"{agent}\",\"dispatcher\":\"native\",\"created\":\"2026-05-01T00:00:00Z\",\"updated\":\"2026-05-01T00:00:00Z\",\"turn_count\":1,\"input_tokens\":0,\"output_tokens\":0,\"cached_input_tokens\":0,\"charged_amount_usd\":0.0,\"thread_id\":\"{thread_id}\"}}}}\n{{\"role\":\"user\",\"content\":\"hi\"}}\n"
    );
    std::fs::create_dir_all(path.parent().expect("transcript parent")).expect("transcript dir");
    std::fs::write(path, body).expect("write transcript");
}

#[tokio::test]
async fn round19_memory_sources_registry_readers_sync_and_reconcile_edges() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let config = harness.config().await;

    let invalid = memory_sources::add_source(source_entry("", SourceKind::Folder, "No id"))
        .await
        .expect_err("id required");
    assert!(invalid.contains("id is required"));

    let folder_dir = harness.root.join("notes");
    std::fs::create_dir_all(&folder_dir).expect("notes dir");
    std::fs::write(folder_dir.join("note.md"), "# Round 19\nbody").expect("note");
    std::fs::write(folder_dir.join("skip.txt"), "ignored").expect("skip");
    let mut folder = source_entry("src-folder", SourceKind::Folder, "Notes");
    folder.path = Some(folder_dir.to_string_lossy().to_string());
    folder.glob = Some("**/*".to_string());
    let added = memory_sources::add_source(folder.clone())
        .await
        .expect("add folder source");
    assert_eq!(added.id, "src-folder");
    let duplicate = memory_sources::add_source(folder.clone())
        .await
        .expect_err("duplicate source rejected");
    assert!(duplicate.contains("already exists"));

    let updated = memory_sources::update_source(
        "src-folder",
        MemorySourcePatch {
            label: Some("Renamed notes".to_string()),
            enabled: Some(false),
            ..MemorySourcePatch::default()
        },
    )
    .await
    .expect("update source");
    assert_eq!(updated.label, "Renamed notes");
    assert!(!updated.enabled);
    assert_eq!(
        memory_sources::list_enabled_by_kind(SourceKind::Folder)
            .await
            .expect("list enabled folders")
            .len(),
        0
    );
    let disabled_sync = memory_sources::sync::sync_source(updated.clone(), config.clone())
        .await
        .expect_err("disabled source rejected");
    assert!(disabled_sync.contains("disabled"));

    let reader = openhuman_core::openhuman::memory_sources::readers::folder::FolderReader;
    let listed = reader
        .list_items(&folder, &config)
        .await
        .expect("folder list items");
    assert_eq!(listed.len(), 2);
    let md = reader
        .read_item(&folder, "note.md", &config)
        .await
        .expect("read note");
    assert_eq!(md.title, "note.md");
    let traversal = reader
        .read_item(&folder, "../outside.md", &config)
        .await
        .expect_err("path traversal denied");
    assert!(traversal.contains("path traversal") || traversal.contains("file not found"));

    let twitter = source_entry("src-twitter", SourceKind::TwitterQuery, "Tweets");
    let twitter_sync = memory_sources::sync::sync_source(
        MemorySourceEntry {
            query: Some("openhuman".to_string()),
            ..twitter
        },
        config.clone(),
    )
    .await;
    assert!(
        twitter_sync.is_ok(),
        "twitter placeholder is reported async"
    );

    let upserted =
        memory_sources::upsert_composio_source("gmail", "conn-round19-abcdefghi", "Gmail first")
            .await
            .expect("insert composio source");
    let updated_composio =
        memory_sources::upsert_composio_source("gmail", "conn-round19-abcdefghi", "Gmail updated")
            .await
            .expect("update composio source");
    assert_eq!(updated_composio.id, upserted.id);
    assert_eq!(updated_composio.label, "Gmail updated");

    let github_reader = openhuman_core::openhuman::memory_sources::readers::github::GithubReader;
    let github_err = github_reader
        .list_items(
            &MemorySourceEntry {
                url: Some("https://example.com/not/github".to_string()),
                ..source_entry("src-gh", SourceKind::GithubRepo, "Bad repo")
            },
            &config,
        )
        .await
        .expect_err("invalid github url");
    assert!(github_err.contains("not a GitHub URL"));
    let item_err = github_reader
        .read_item(
            &MemorySourceEntry {
                url: Some("https://github.com/tinyhumansai/openhuman".to_string()),
                ..source_entry("src-gh-good", SourceKind::GithubRepo, "Repo")
            },
            "bad:123",
            &config,
        )
        .await
        .expect_err("invalid github item id");
    assert!(item_err.contains("invalid item id"));

    let feed_body = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Round19</title>
<item><title>First &amp; only</title><guid>guid-1</guid><description><![CDATA[<p>Hello</p>]]></description><pubDate>Fri, 29 May 2026 00:00:00 GMT</pubDate></item>
</channel></rss>"#;
    let (feed_url, server_task) = one_response_server(feed_body, "application/rss+xml").await;
    let rss = MemorySourceEntry {
        url: Some(feed_url),
        max_items: Some(1),
        ..source_entry("src-rss", SourceKind::RssFeed, "Feed")
    };
    let rss_reader = openhuman_core::openhuman::memory_sources::readers::rss::RssReader;
    let feed_items = rss_reader
        .list_items(&rss, &config)
        .await
        .expect("rss list items");
    assert_eq!(feed_items[0].id, "guid-1");
    let _ = server_task.await;

    memory_sources::reconcile::ensure_composio_sources().await;
    assert!(
        memory_sources::remove_source("missing-source")
            .await
            .expect("remove missing is idempotent")
            == false
    );
    assert!(memory_sources::remove_source("src-folder")
        .await
        .expect("remove folder"));
}
