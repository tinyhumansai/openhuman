//! Round26 closure coverage for near-threshold app, credentials, threads,
//! and memory_sources paths.
//!
//! Uses only temp workspaces, local files, and intentionally unreachable
//! loopback endpoints.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use filetime::FileTime;
use serde_json::Value;
use tempfile::{Builder, TempDir};

use openhuman_core::openhuman::app_state::{snapshot, update_local_state, StoredAppStatePatch};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::profiles::{AuthProfile, AuthProfilesStore, TokenSet};
use openhuman_core::openhuman::memory::{
    AppendConversationMessageRequest, ConversationMessageRecord, CreateConversationThreadRequest,
    GenerateConversationThreadTitleRequest, UpsertConversationThreadRequest,
};
use openhuman_core::openhuman::memory_sources::reconcile::ensure_composio_sources;
use openhuman_core::openhuman::threads::ops::{
    message_append, thread_create_new, thread_generate_title, thread_upsert,
};
use openhuman_core::openhuman::threads::welcome_migration::migrate_welcome_agent_artifacts;

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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

    fn set_path(key: &'static str, path: &Path) -> Self {
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

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn setup(prefix: &str) -> Harness {
    std::fs::create_dir_all("target").expect("target dir");
    let tmp = Builder::new()
        .prefix(prefix)
        .tempdir_in("target")
        .expect("tempdir");
    let root = tmp.path().join("openhuman");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let guards = vec![
        EnvGuard::set_path("OPENHUMAN_WORKSPACE", &root),
        EnvGuard::set_path("HOME", tmp.path()),
        EnvGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvGuard::unset("BACKEND_URL"),
        EnvGuard::unset("VITE_BACKEND_URL"),
        EnvGuard::unset("OPENHUMAN_API_URL"),
        EnvGuard::unset("OPENHUMAN_CORE_RPC_URL"),
        EnvGuard::unset("OPENHUMAN_CORE_PORT"),
    ];

    let mut config = Config {
        workspace_dir: workspace.clone(),
        config_path: root.join("config.toml"),
        api_url: Some("http://127.0.0.1:9".to_string()),
        onboarding_completed: false,
        chat_onboarding_completed: true,
        ..Config::default()
    };
    config.secrets.encrypt = false;
    config.composio.mode = "backend".to_string();
    config.save().await.expect("save config");

    Harness {
        _tmp: tmp,
        root,
        workspace,
        _guards: guards,
    }
}

fn write_transcript(path: &Path, agent: &str) {
    let body = format!(
        "{{\"_meta\":{{\"agent\":\"{agent}\",\"thread_id\":\"round26-thread\",\"dispatcher\":\"native\"}}}}\n{{\"role\":\"user\",\"content\":\"hello\"}}\n"
    );
    std::fs::create_dir_all(path.parent().unwrap()).expect("transcript dir");
    std::fs::write(path, body).expect("write transcript");
}

#[tokio::test]
async fn round26_app_state_quarantines_corrupt_local_file_and_preserves_patch_noops() {
    let _lock = env_lock();
    let harness = setup("round26-app-state-").await;
    let state_dir = harness.workspace.join("state");
    std::fs::create_dir_all(&state_dir).expect("state dir");
    let app_state_path = state_dir.join("app-state.json");
    std::fs::write(&app_state_path, b"{ not valid json").expect("corrupt app state");

    let snapshot = snapshot().await.expect("snapshot with corrupt state").value;
    assert!(!snapshot.onboarding_completed);
    assert!(snapshot.chat_onboarding_completed);
    assert!(snapshot.local_state.encryption_key.is_none());
    assert!(snapshot.local_state.onboarding_tasks.is_none());
    assert!(
        !app_state_path.exists(),
        "corrupt file should be quarantined"
    );
    assert!(
        std::fs::read_dir(&state_dir)
            .expect("state listing")
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains("json.corrupted")),
        "quarantine file should remain for diagnostics"
    );

    let unchanged = update_local_state(StoredAppStatePatch {
        encryption_key: None,
        onboarding_tasks: None,
    })
    .await
    .expect("noop patch")
    .value;
    assert!(unchanged.encryption_key.is_none());
    assert!(unchanged.onboarding_tasks.is_none());
}

#[test]
fn round26_credentials_profiles_reclaim_stale_locks_and_reject_bad_active_profile() {
    let _lock = env_lock();
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let harness = runtime.block_on(setup("round26-credentials-"));
    let store = AuthProfilesStore::new(&harness.root, false);
    let lock_path = harness.root.join("auth-profiles.lock");

    std::fs::write(&lock_path, "pid=4294967295\n").expect("stale pid lock");
    let empty = store.load().expect("stale pid lock should be reclaimed");
    assert!(empty.profiles.is_empty());
    assert!(!lock_path.exists());

    std::fs::write(&lock_path, "not-a-pid\n").expect("malformed lock");
    let old = FileTime::from_system_time(SystemTime::now() - Duration::from_secs(3));
    filetime::set_file_mtime(&lock_path, old).expect("old lock mtime");
    store
        .upsert_profile(
            AuthProfile::new_token("anthropic", "default", "sk-round26".to_string()),
            false,
        )
        .expect("malformed old lock should be reclaimed");
    assert!(!lock_path.exists());

    let missing_active = store
        .set_active_profile("anthropic", "anthropic:missing")
        .expect_err("missing active profile");
    assert!(missing_active
        .to_string()
        .contains("Auth profile not found"));

    let oauth = AuthProfile::new_oauth(
        "gmail",
        "round26",
        TokenSet {
            access_token: "access-round26".to_string(),
            refresh_token: None,
            id_token: None,
            expires_at: None,
            token_type: None,
            scope: None,
        },
    );
    let oauth_id = oauth.id.clone();
    store.upsert_profile(oauth, true).expect("upsert oauth");
    let loaded = store.load().expect("load oauth");
    assert_eq!(loaded.active_profiles.get("gmail"), Some(&oauth_id));
    assert_eq!(
        loaded
            .profiles
            .get(&oauth_id)
            .and_then(|profile| profile.token_set.as_ref())
            .map(|tokens| tokens.access_token.as_str()),
        Some("access-round26")
    );
}

#[tokio::test]
async fn round26_threads_generate_fallback_titles_and_migrate_in_place_transcripts() {
    let _lock = env_lock();
    let harness = setup("round26-threads-").await;

    let created = thread_create_new(CreateConversationThreadRequest {
        labels: Some(vec!["round26".to_string()]),
        personality_id: None,
    })
    .await
    .expect("create thread")
    .value
    .data
    .expect("created thread");
    message_append(AppendConversationMessageRequest {
        thread_id: created.id.clone(),
        message: ConversationMessageRecord {
            id: "msg-round26-user".to_string(),
            content: "/please summarize this long launch checklist for me".to_string(),
            message_type: "text".to_string(),
            extra_metadata: Value::Null,
            sender: "user".to_string(),
            created_at: "2026-05-30T00:00:00Z".to_string(),
        },
    })
    .await
    .expect("append user message");

    let titled = thread_generate_title(GenerateConversationThreadTitleRequest {
        thread_id: created.id.clone(),
        assistant_message: None,
    })
    .await
    .expect("fallback title")
    .value
    .data
    .expect("titled thread");
    assert_eq!(
        titled.title,
        "please summarize this long launch checklist for me"
    );

    let custom = thread_upsert(UpsertConversationThreadRequest {
        id: "round26-custom-title".to_string(),
        title: "Already Named Thread".to_string(),
        created_at: "2026-05-30T00:00:01Z".to_string(),
        parent_thread_id: None,
        labels: Some(vec!["work".to_string()]),
        personality_id: Some("p1".to_string()),
    })
    .await
    .expect("upsert custom")
    .value
    .data
    .expect("custom thread");
    let skipped = thread_generate_title(GenerateConversationThreadTitleRequest {
        thread_id: custom.id,
        assistant_message: Some("assistant text".to_string()),
    })
    .await
    .expect("skip custom title")
    .value
    .data
    .expect("skipped title");
    assert_eq!(skipped.title, "Already Named Thread");

    let transcript = harness.workspace.join("session_raw/round26-chat.jsonl");
    write_transcript(&transcript, "welcome");
    let result = migrate_welcome_agent_artifacts(&harness.workspace).expect("migration");
    assert_eq!(result.transcripts_updated, 1);
    assert_eq!(result.transcript_files_renamed, 0);
    assert!(transcript.exists());
    let rewritten = std::fs::read_to_string(&transcript).expect("rewritten transcript");
    assert!(rewritten.contains("\"agent\":\"orchestrator\""));
}

#[tokio::test]
async fn round26_memory_sources_reconcile_handles_unavailable_composio_without_registry_changes() {
    let _lock = env_lock();
    let harness = setup("round26-memory-sources-").await;

    ensure_composio_sources().await;

    let config = Config::load_or_init().await.expect("reload config");
    assert_eq!(config.config_path, harness.root.join("config.toml"));
    assert!(config.memory_sources.is_empty());
}
