//! Round24 focused raw coverage for app_state, credentials profiles, and
//! threads public operations.
//!
//! Uses temp workspaces and local filesystem state only. No real backend,
//! keychain service, or non-loopback network access is required.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{Duration as ChronoDuration, Utc};
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};

use openhuman_core::openhuman::app_state::{
    snapshot, update_local_state, StoredAppStatePatch, StoredOnboardingTasks,
};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::profiles::{AuthProfile, AuthProfilesStore, TokenSet};
use openhuman_core::openhuman::memory::{
    AppendConversationMessageRequest, ConversationMessageRecord, ConversationMessagesRequest,
    CreateConversationThreadRequest, DeleteConversationThreadRequest, EmptyRequest,
    UpdateConversationMessageRequest, UpdateConversationThreadLabelsRequest,
    UpdateConversationThreadTitleRequest,
};
use openhuman_core::openhuman::threads::ops::{
    message_append, message_update, messages_list, thread_create_new, thread_delete,
    thread_update_labels, thread_update_title, threads_list, threads_purge,
};

static ROUND24_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ROUND24_ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("create target");
    Builder::new()
        .prefix("app-credentials-threads-round24-")
        .tempdir_in("target")
        .expect("round24 tempdir")
}

async fn setup() -> Harness {
    let tmp = tempdir();
    let root = tmp.path().join("openhuman");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");

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

    let mut config = Config {
        workspace_dir: workspace.clone(),
        config_path: root.join("config.toml"),
        api_url: Some("http://127.0.0.1:9".to_string()),
        onboarding_completed: true,
        chat_onboarding_completed: false,
        ..Config::default()
    };
    config.observability.analytics_enabled = false;
    config.secrets.encrypt = false;
    config.save().await.expect("save config");

    Harness {
        _tmp: tmp,
        root,
        _guards: guards,
    }
}

fn profile_store(harness: &Harness) -> AuthProfilesStore {
    AuthProfilesStore::new(&harness.root, false)
}

#[tokio::test]
async fn round24_app_state_update_and_snapshot_preserve_local_state() {
    let _lock = env_lock();
    let _harness = setup().await;

    let tasks = StoredOnboardingTasks {
        accessibility_permission_granted: true,
        local_model_consent_given: true,
        local_model_download_started: true,
        enabled_tools: vec!["search".into(), "memory".into()],
        connected_sources: vec!["slack".into()],
        updated_at_ms: Some(4242),
    };

    let updated = update_local_state(StoredAppStatePatch {
        encryption_key: Some(Some("  round24-key  ".into())),
        onboarding_tasks: Some(Some(tasks.clone())),
    })
    .await
    .expect("update local state")
    .value;
    assert_eq!(updated.encryption_key.as_deref(), Some("round24-key"));
    assert_eq!(
        updated.onboarding_tasks.as_ref().unwrap().connected_sources,
        tasks.connected_sources
    );

    let snapshot = snapshot().await.expect("snapshot").value;
    assert!(snapshot.onboarding_completed);
    assert!(!snapshot.chat_onboarding_completed);
    assert!(!snapshot.analytics_enabled);
    assert_eq!(
        snapshot.local_state.encryption_key.as_deref(),
        Some("round24-key")
    );
    assert!(snapshot.current_user.is_none());
    assert!(snapshot.session_token.is_none());

    let cleared = update_local_state(StoredAppStatePatch {
        encryption_key: Some(Some("   ".into())),
        onboarding_tasks: Some(None),
    })
    .await
    .expect("clear local state")
    .value;
    assert!(cleared.encryption_key.is_none());
    assert!(cleared.onboarding_tasks.is_none());
}

#[test]
fn round24_credentials_profiles_cover_schema_and_mutation_edges() {
    let _lock = env_lock();
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let harness = runtime.block_on(setup());
    let store = profile_store(&harness);

    std::fs::create_dir_all(&harness.root).expect("state dir");
    std::fs::write(store.path(), "").expect("empty profile store");
    let empty = store.load().expect("empty store loads default");
    assert!(empty.profiles.is_empty());

    let oauth = AuthProfile::new_oauth(
        "gmail",
        "work",
        TokenSet {
            access_token: "access-round24".into(),
            refresh_token: Some("refresh-round24".into()),
            id_token: Some("id-round24".into()),
            expires_at: Some(Utc::now() + ChronoDuration::minutes(30)),
            token_type: Some("Bearer".into()),
            scope: Some("email profile".into()),
        },
    );
    let oauth_id = oauth.id.clone();
    store.upsert_profile(oauth, true).expect("upsert oauth");

    let token = AuthProfile::new_token("anthropic", "prod", "sk-round24".into());
    let token_id = token.id.clone();
    store.upsert_profile(token, false).expect("upsert token");
    store
        .set_active_profile("anthropic", &token_id)
        .expect("set active token profile");

    let updated = store
        .update_profile(&token_id, |profile| {
            profile.workspace_id = Some("workspace-round24".into());
            profile.metadata.insert("tier".into(), "prod".into());
            Ok(())
        })
        .expect("update profile");
    assert_eq!(updated.workspace_id.as_deref(), Some("workspace-round24"));

    let loaded = store.load().expect("load profiles");
    assert_eq!(loaded.active_profiles.get("gmail"), Some(&oauth_id));
    assert_eq!(loaded.active_profiles.get("anthropic"), Some(&token_id));
    assert_eq!(
        loaded
            .profiles
            .get(&token_id)
            .and_then(|profile| profile.metadata.get("tier"))
            .map(String::as_str),
        Some("prod")
    );
    assert!(loaded
        .profiles
        .get(&oauth_id)
        .and_then(|profile| profile.token_set.as_ref())
        .is_some_and(|tokens| tokens.id_token.as_deref() == Some("id-round24")));

    store
        .clear_active_profile("gmail")
        .expect("clear active oauth");
    assert!(store
        .remove_profile(&oauth_id)
        .expect("remove oauth profile"));
    assert!(!store
        .remove_profile(&oauth_id)
        .expect("second remove is false"));

    let mut raw: Value =
        serde_json::from_str(&std::fs::read_to_string(store.path()).expect("raw store"))
            .expect("profile store json");
    raw["schema_version"] = json!(999);
    std::fs::write(store.path(), serde_json::to_vec_pretty(&raw).expect("json"))
        .expect("write future schema");
    let err = store.load().expect_err("future schema must fail");
    assert!(
        err.to_string()
            .contains("Unsupported auth profile schema version"),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn round24_threads_public_ops_cover_crud_and_error_branches() {
    let _lock = env_lock();
    let _harness = setup().await;

    let created = thread_create_new(CreateConversationThreadRequest {
        labels: Some(vec!["personal".into(), "round24".into()]),
        personality_id: Some("default-personality".into()),
    })
    .await
    .expect("create thread")
    .value
    .data
    .expect("created data");
    assert!(created.id.starts_with("thread-"));
    assert_eq!(created.labels, vec!["personal", "round24"]);
    assert_eq!(
        created.personality_id.as_deref(),
        Some("default-personality")
    );

    let msg = ConversationMessageRecord {
        id: "msg-round24".into(),
        content: "hello round24".into(),
        message_type: "text".into(),
        extra_metadata: Value::Null,
        sender: "user".into(),
        created_at: "2026-05-30T00:00:00Z".into(),
    };
    let appended = message_append(AppendConversationMessageRequest {
        thread_id: created.id.clone(),
        message: msg,
    })
    .await
    .expect("append message")
    .value
    .data
    .expect("appended data");
    assert_eq!(appended.id, "msg-round24");

    let patched = message_update(UpdateConversationMessageRequest {
        thread_id: created.id.clone(),
        message_id: "msg-round24".into(),
        extra_metadata: Some(json!({ "source": "round24" })),
    })
    .await
    .expect("update message")
    .value
    .data
    .expect("patched data");
    assert_eq!(patched.extra_metadata["source"], "round24");

    let messages = messages_list(ConversationMessagesRequest {
        thread_id: created.id.clone(),
    })
    .await
    .expect("list messages")
    .value
    .data
    .expect("messages data");
    assert_eq!(messages.count, 1);

    let relabeled = thread_update_labels(UpdateConversationThreadLabelsRequest {
        thread_id: created.id.clone(),
        labels: vec!["work".into()],
    })
    .await
    .expect("update labels")
    .value
    .data
    .expect("labels data");
    assert_eq!(relabeled.labels, vec!["work"]);

    let empty_title = thread_update_title(UpdateConversationThreadTitleRequest {
        thread_id: created.id.clone(),
        title: "   ".into(),
    })
    .await
    .expect_err("empty titles are rejected");
    assert!(empty_title.contains("title must not be empty"));

    let renamed = thread_update_title(UpdateConversationThreadTitleRequest {
        thread_id: created.id.clone(),
        title: "  Round24 covered thread  ".into(),
    })
    .await
    .expect("update title")
    .value
    .data
    .expect("title data");
    assert_eq!(renamed.title, "Round24 covered thread");

    let listed = threads_list(EmptyRequest {})
        .await
        .expect("list threads")
        .value
        .data
        .expect("threads data");
    assert_eq!(listed.count, 1);

    let deleted = thread_delete(DeleteConversationThreadRequest {
        thread_id: created.id,
        deleted_at: "2026-05-30T00:01:00Z".into(),
    })
    .await
    .expect("delete thread")
    .value
    .data
    .expect("delete data");
    assert!(deleted.deleted);

    let purged = threads_purge(EmptyRequest {})
        .await
        .expect("purge threads")
        .value
        .data
        .expect("purge data");
    assert_eq!(purged.agent_threads_deleted, 0);
}
