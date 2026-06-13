use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use openhuman_core::openhuman::config::rpc as config_rpc;
use openhuman_core::openhuman::credentials::profiles::{
    AuthProfile, AuthProfileKind, AuthProfilesStore, TokenSet,
};
use openhuman_core::openhuman::memory::{
    AppendConversationMessageRequest, ConversationMessageRecord, ConversationMessagesRequest,
    DeleteConversationThreadRequest, EmptyRequest, UpdateConversationMessageRequest,
    UpdateConversationThreadLabelsRequest, UpdateConversationThreadTitleRequest,
    UpsertConversationThreadRequest,
};
use openhuman_core::openhuman::threads::ops as thread_ops;
use serde_json::json;
use tempfile::{Builder, TempDir};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, path.as_os_str()) };
        Self { key, old }
    }

    fn set(key: &'static str, value: impl Into<String>) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value.into()) };
        Self { key, old }
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
    fn workspace_dir(&self) -> PathBuf {
        self.root.join("workspace")
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("target dir");
    Builder::new()
        .prefix("credentials-threads-round22-")
        .tempdir_in("target")
        .expect("tempdir")
}

fn setup() -> Harness {
    let tmp = tempdir();
    let root = tmp.path().join("openhuman");
    std::fs::create_dir_all(&root).expect("root dir");
    std::fs::write(
        root.join("config.toml"),
        r#"api_url = "http://127.0.0.1:9"
default_model = "round22-coverage-model"
default_temperature = 0.2
onboarding_completed = true
chat_onboarding_completed = false

[observability]
analytics_enabled = false

[secrets]
encrypt = false

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
"#,
    )
    .expect("config");
    let guards = vec![
        EnvGuard::set_path("OPENHUMAN_WORKSPACE", &root),
        EnvGuard::set_path("HOME", tmp.path()),
        EnvGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
        EnvGuard::unset("OPENHUMAN_API_URL"),
        EnvGuard::unset("BACKEND_URL"),
        EnvGuard::unset("VITE_BACKEND_URL"),
    ];
    Harness {
        _tmp: tmp,
        root,
        _guards: guards,
    }
}

#[test]
fn round22_credentials_profiles_cover_schema_quarantine_and_oauth_save_paths() {
    let _lock = env_lock();
    let harness = setup();
    let state_dir = harness.root.join("profile-state");
    let store = AuthProfilesStore::new(&state_dir, false);

    let oauth = AuthProfile::new_oauth(
        "github",
        "work",
        TokenSet {
            access_token: "access-round22".to_string(),
            refresh_token: Some("refresh-round22".to_string()),
            id_token: Some("id-round22".to_string()),
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            token_type: Some("Bearer".to_string()),
            scope: Some("repo read:user".to_string()),
        },
    );
    assert_eq!(oauth.kind, AuthProfileKind::OAuth);
    store
        .upsert_profile(oauth.clone(), true)
        .expect("save oauth profile");
    let loaded = store.load().expect("load oauth");
    assert_eq!(
        loaded
            .profiles
            .get(&oauth.id)
            .and_then(|profile| profile.token_set.as_ref())
            .map(|tokens| tokens.access_token.as_str()),
        Some("access-round22")
    );
    let persisted = std::fs::read_to_string(store.path()).expect("persisted oauth json");
    assert!(persisted.contains("access_token"));
    assert!(persisted.contains("refresh_token"));

    std::fs::write(store.path(), b"{not-json").expect("corrupt store");
    let recovered = store.load().expect("corrupt store is quarantined");
    assert!(recovered.profiles.is_empty());
    assert!(
        std::fs::read_dir(&state_dir)
            .expect("state dir")
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains("auth-profiles.corrupt")),
        "corrupt store should be quarantined"
    );

    std::fs::write(
        store.path(),
        serde_json::to_vec_pretty(&json!({
            "schema_version": 99,
            "updated_at": Utc::now().to_rfc3339(),
            "active_profiles": {},
            "profiles": {}
        }))
        .expect("future schema json"),
    )
    .expect("future schema");
    let schema_err = store
        .load()
        .expect_err("future schema rejected")
        .to_string();
    assert!(schema_err.contains("Unsupported auth profile schema version 99"));
}

#[tokio::test]
async fn round22_threads_cover_update_delete_and_error_edges() {
    let _lock = env_lock();
    let harness = setup();
    let _ = config_rpc::load_config_with_timeout()
        .await
        .expect("config loads");

    let created_at = Utc::now().to_rfc3339();
    let thread = thread_ops::thread_upsert(UpsertConversationThreadRequest {
        id: "round22-thread".to_string(),
        title: "Round22 Original".to_string(),
        created_at: created_at.clone(),
        parent_thread_id: Some("parent-round22".to_string()),
        labels: Some(vec!["alpha".to_string()]),
        personality_id: Some("persona-round22".to_string()),
    })
    .await
    .expect("upsert thread")
    .value
    .data
    .expect("thread data");
    assert_eq!(thread.parent_thread_id.as_deref(), Some("parent-round22"));

    let message = ConversationMessageRecord {
        id: "msg-round22".to_string(),
        content: "Round22 message".to_string(),
        message_type: "text".to_string(),
        extra_metadata: json!({"old": true}),
        sender: "user".to_string(),
        created_at,
    };
    thread_ops::message_append(AppendConversationMessageRequest {
        thread_id: "round22-thread".to_string(),
        message,
    })
    .await
    .expect("append message");

    let updated_message = thread_ops::message_update(UpdateConversationMessageRequest {
        thread_id: "round22-thread".to_string(),
        message_id: "msg-round22".to_string(),
        extra_metadata: Some(json!({"round": 22})),
    })
    .await
    .expect("update message")
    .value
    .data
    .expect("message data");
    assert_eq!(updated_message.extra_metadata["round"], 22);

    let listed = thread_ops::messages_list(ConversationMessagesRequest {
        thread_id: "round22-thread".to_string(),
    })
    .await
    .expect("list messages")
    .value
    .data
    .expect("messages data");
    assert_eq!(listed.count, 1);

    let labels = thread_ops::thread_update_labels(UpdateConversationThreadLabelsRequest {
        thread_id: "round22-thread".to_string(),
        labels: Vec::new(),
    })
    .await
    .expect("clear labels")
    .value
    .data
    .expect("labels data");
    assert!(labels.labels.is_empty());

    let empty_title = thread_ops::thread_update_title(UpdateConversationThreadTitleRequest {
        thread_id: "round22-thread".to_string(),
        title: "   ".to_string(),
    })
    .await
    .expect_err("empty title rejected");
    assert!(empty_title.contains("title must not be empty"));

    let renamed = thread_ops::thread_update_title(UpdateConversationThreadTitleRequest {
        thread_id: "round22-thread".to_string(),
        title: "  Round22 Renamed  ".to_string(),
    })
    .await
    .expect("rename thread")
    .value
    .data
    .expect("rename data");
    assert_eq!(renamed.title, "Round22 Renamed");

    let deleted = thread_ops::thread_delete(DeleteConversationThreadRequest {
        thread_id: "round22-thread".to_string(),
        deleted_at: Utc::now().to_rfc3339(),
    })
    .await
    .expect("delete thread")
    .value
    .data
    .expect("delete data");
    assert!(deleted.deleted);

    let after_delete = thread_ops::threads_list(EmptyRequest {})
        .await
        .expect("list after delete")
        .value
        .data
        .expect("list data");
    assert_eq!(after_delete.count, 0);

    let deleted_again = thread_ops::thread_delete(DeleteConversationThreadRequest {
        thread_id: "round22-thread".to_string(),
        deleted_at: Utc::now().to_rfc3339(),
    })
    .await
    .expect("delete missing thread is idempotent")
    .value
    .data
    .expect("delete missing data");
    assert!(!deleted_again.deleted);

    let workspace = harness.workspace_dir();
    assert!(workspace.exists());
}
