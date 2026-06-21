use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use openhuman_core::openhuman::app_state::{
    peek_cached_current_user_identity, snapshot, update_local_state, StoredAppStatePatch,
    StoredOnboardingTasks,
};
use openhuman_core::openhuman::config::rpc as config_rpc;
use openhuman_core::openhuman::credentials::ops::store_session;
use openhuman_core::openhuman::credentials::profiles::{
    AuthProfile, AuthProfileKind, AuthProfilesStore, TokenSet,
};
use openhuman_core::openhuman::credentials::{
    list_provider_credentials_by_prefix, AuthService, APP_SESSION_PROVIDER,
    DEFAULT_AUTH_PROFILE_NAME,
};
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

static ROUND14_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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

    fn state_file(&self) -> PathBuf {
        self.root.join("workspace/state/app-state.json")
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ROUND14_ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("create target");
    Builder::new()
        .prefix("app-state-credentials-round14-")
        .tempdir_in("target")
        .expect("round14 tempdir")
}

fn write_min_config(root: &Path, api_url: &str) {
    std::fs::create_dir_all(root).expect("create config root");
    let cfg = format!(
        r#"api_url = "{api_url}"
default_model = "round14-coverage-model"
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
        toml::from_str(&cfg).expect("round14 config must match schema");
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

async fn auth_me_server(
    body: &'static str,
) -> (
    String,
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind auth/me listener");
    let url = format!("http://{}", listener.local_addr().expect("listener addr"));
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((mut stream, _)) = accepted else {
                        break;
                    };
                    let mut req = [0_u8; 2048];
                    let _ = stream.read(&mut req).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                }
            }
        }
    });
    (url, task, shutdown_tx)
}

/// Like `auth_me_server` but always replies HTTP 500, so `store_session`'s
/// `GET /auth/me` validation gate fails — exercising the WARN + `Err` path that
/// leaves the session unpersisted (the "OAuth succeeded but app is back on the
/// signin page" bug).
async fn auth_me_failing_server() -> (
    String,
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind failing auth/me listener");
    let url = format!("http://{}", listener.local_addr().expect("listener addr"));
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((mut stream, _)) = accepted else { break; };
                    let mut req = [0_u8; 2048];
                    let _ = stream.read(&mut req).await;
                    let body = "{\"error\":\"mock /auth/me 500\"}";
                    let response = format!(
                        "HTTP/1.1 500 Internal Server Error\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                }
            }
        }
    });
    (url, task, shutdown_tx)
}

/// Store-time `/auth/me` failure must surface as `Err` (and NOT persist a
/// profile), which is what bounces the user back to signin after a "successful"
/// OAuth. Covers the WARN/`Err` gate added in `credentials::ops::store_session`.
#[tokio::test]
async fn store_session_auth_me_failure_returns_err_and_does_not_persist() {
    let _lock = env_lock();
    let (api_url, server_task, shutdown_tx) = auth_me_failing_server().await;
    let harness = setup(&api_url);
    let config = harness.config().await;

    // Non-local token (3 dot-parts, not ".local") forces the backend
    // `GET /auth/me` validation path inside `store_session`.
    let result = store_session(&config, "header.payload.signature", None, None).await;

    let _ = shutdown_tx.send(());
    let _ = server_task.await;

    let err = result.expect_err("store_session must fail when GET /auth/me returns 500");
    // Lock the cross-layer error-string contract: this exact prefix is what the
    // frontend `classifyAuthStoreFailure` matches on. Assert `starts_with` (not
    // just `contains`) so a reword in `store_session`/`rest.rs` fails CI here
    // instead of silently degrading the FE classifier to 'other'.
    assert!(
        err.starts_with("Session validation failed (GET /auth/me):"),
        "store_session error must keep the contract prefix; got: {err}"
    );

    // Explicitly verify the failure path persisted NOTHING — the gate returns
    // before the persist step, so the snapshot must read back as unauthenticated
    // with no session token (a partial regression that wrote a profile would
    // otherwise still pass on the Err check alone). This is what leaves the user
    // on the signin page.
    let snap = snapshot()
        .await
        .expect("snapshot after failed store_session")
        .value;
    assert!(
        !snap.auth.is_authenticated,
        "auth must remain unauthenticated after a failed store_session"
    );
    assert!(
        snap.session_token.is_none(),
        "session token must not be persisted after a failed store_session"
    );
}

#[tokio::test]
async fn round14_snapshot_preserves_rich_local_state_with_backend_or_stored_user() {
    let _lock = env_lock();
    let (api_url, server_task, shutdown_tx) = auth_me_server(
        r#"{"data":{"id":"fresh-user","name":"Fresh User","email":"fresh@example.test","ignored":true}}"#,
    )
    .await;
    let harness = setup(&api_url);
    let config = harness.config().await;

    let mut metadata = HashMap::new();
    metadata.insert("user_id".to_string(), "stored-user".to_string());
    metadata.insert(
        "user_json".to_string(),
        json!({
            "id": "stored-user",
            "name": "Stored User",
            "email": "stored@example.test"
        })
        .to_string(),
    );
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "round14.header.payload",
            metadata,
            true,
        )
        .expect("seed app session profile");

    let updated = update_local_state(StoredAppStatePatch {
        keyring_consent: None,
        encryption_key: Some(Some("  round14-key  ".to_string())),
        onboarding_tasks: Some(Some(StoredOnboardingTasks {
            accessibility_permission_granted: true,
            local_model_consent_given: true,
            local_model_download_started: true,
            enabled_tools: vec!["gmail".to_string(), "calendar".to_string()],
            connected_sources: vec!["slack".to_string()],
            updated_at_ms: Some(123_456),
        })),
    })
    .await
    .expect("write local state")
    .value;
    assert_eq!(updated.encryption_key.as_deref(), Some("round14-key"));
    assert_eq!(
        updated
            .onboarding_tasks
            .as_ref()
            .expect("tasks")
            .enabled_tools,
        vec!["gmail", "calendar"]
    );

    let snap = snapshot().await.expect("snapshot").value;
    assert!(snap.auth.is_authenticated);
    assert_eq!(
        snap.session_token.as_deref(),
        Some("round14.header.payload")
    );
    assert_eq!(snap.auth.user_id.as_deref(), Some("stored-user"));
    let current_user_id = snap
        .current_user
        .as_ref()
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str);
    assert!(
        matches!(current_user_id, Some("fresh-user" | "stored-user")),
        "unexpected current user id: {current_user_id:?}"
    );
    assert!(snap.onboarding_completed);
    assert!(snap.analytics_enabled);
    assert!(snap.meet_auto_orchestrator_handoff);
    assert_eq!(
        snap.local_state.encryption_key.as_deref(),
        Some("round14-key")
    );

    if current_user_id == Some("fresh-user") {
        let identity = peek_cached_current_user_identity().expect("cached identity");
        assert_eq!(identity.id.as_deref(), Some("fresh-user"));
        assert_eq!(identity.name.as_deref(), Some("Fresh User"));
        assert_eq!(identity.email.as_deref(), Some("fresh@example.test"));
    }

    let raw = std::fs::read_to_string(harness.state_file()).expect("state file");
    let persisted: Value = serde_json::from_str(&raw).expect("valid state json");
    assert_eq!(persisted["encryptionKey"], "round14-key");
    assert_eq!(
        persisted["onboardingTasks"]["connectedSources"],
        json!(["slack"])
    );

    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}

#[tokio::test]
async fn round14_snapshot_uses_stored_user_when_backend_user_is_empty_or_unreachable() {
    let _lock = env_lock();
    let (api_url, server_task, shutdown_tx) = auth_me_server(r#"{"data":{}}"#).await;
    let harness = setup(&api_url);
    let config = harness.config().await;

    let mut metadata = HashMap::new();
    metadata.insert("user_id".to_string(), "fallback-user".to_string());
    metadata.insert(
        "user_json".to_string(),
        json!({
            "id": "fallback-user",
            "displayName": "Fallback User",
            "email": "fallback@example.test"
        })
        .to_string(),
    );
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "round14.empty.backend",
            metadata,
            true,
        )
        .expect("seed app session");

    let snap = snapshot()
        .await
        .expect("snapshot with empty backend user")
        .value;
    assert_eq!(
        snap.current_user.as_ref().and_then(|v| v.get("id")),
        Some(&json!("fallback-user"))
    );
    assert_eq!(
        snap.auth.user.as_ref().and_then(|v| v.get("displayName")),
        Some(&json!("Fallback User"))
    );

    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}

#[test]
fn round14_profiles_cover_oauth_token_selection_schema_and_quarantine_edges() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let state_dir = harness.root.join("profile-store");
    let store = AuthProfilesStore::new(&state_dir, false);

    let token_profile = AuthProfile::new_token("channel:slack:bot", "default", "xoxb-token".into());
    store
        .upsert_profile(token_profile.clone(), true)
        .expect("insert token profile");

    let mut oauth = AuthProfile::new_oauth(
        "github",
        "work",
        TokenSet {
            access_token: "gh-access".into(),
            refresh_token: Some("gh-refresh".into()),
            id_token: Some("gh-id".into()),
            expires_at: Some(Utc::now() + chrono::Duration::minutes(30)),
            token_type: Some("Bearer".into()),
            scope: Some("repo user".into()),
        },
    );
    oauth.account_id = Some("acct-gh".into());
    oauth.workspace_id = Some("workspace-gh".into());
    oauth.metadata = BTreeMap::from([("team".to_string(), "core".to_string())]);
    store
        .upsert_profile(oauth.clone(), false)
        .expect("insert oauth");
    store
        .set_active_profile("github", &oauth.id)
        .expect("activate oauth");

    let data = store.load().expect("load profiles");
    let loaded_oauth = data.profiles.get(&oauth.id).expect("loaded oauth");
    assert_eq!(loaded_oauth.kind, AuthProfileKind::OAuth);
    assert_eq!(
        loaded_oauth
            .token_set
            .as_ref()
            .map(|tokens| tokens.access_token.as_str()),
        Some("gh-access")
    );
    assert_eq!(loaded_oauth.workspace_id.as_deref(), Some("workspace-gh"));
    assert_eq!(data.active_profiles.get("github"), Some(&oauth.id));

    let service = AuthService::new(&state_dir, false);
    assert_eq!(
        service
            .get_provider_bearer_token("github", None)
            .expect("active github token")
            .as_deref(),
        Some("gh-access")
    );
    assert_eq!(
        service
            .get_provider_bearer_token("channel:slack:bot", None)
            .expect("active channel token")
            .as_deref(),
        Some("xoxb-token")
    );
    let err = service
        .set_active_profile("github", &token_profile.id)
        .expect_err("wrong-provider activation should fail")
        .to_string();
    assert!(err.contains("belongs to provider"));

    let path = store.path().to_path_buf();
    let mut raw: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("profile json"))
            .expect("valid profile json");
    raw["schema_version"] = json!(0);
    raw["profiles"]["legacy-empty"] = json!({
        "provider": "legacy",
        "profile_name": "empty",
        "kind": "token",
        "token": "",
        "created_at": "not-a-date",
        "updated_at": "also-not-a-date",
        "metadata": { "note": "kept" }
    });
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&raw).expect("serialize"),
    )
    .expect("write schema-zero profile json");

    let migrated = store.load().expect("schema 0 should migrate in memory");
    assert_eq!(migrated.schema_version, 1);
    assert!(migrated.profiles.contains_key("legacy-empty"));
    assert!(migrated
        .profiles
        .get("legacy-empty")
        .expect("legacy")
        .token
        .as_deref()
        .is_none_or(str::is_empty));

    raw["schema_version"] = json!(999);
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&raw).expect("serialize"),
    )
    .expect("write future schema");
    let err = store
        .load()
        .expect_err("future schema should fail")
        .to_string();
    assert!(err.contains("Unsupported auth profile schema version 999"));

    std::fs::write(&path, "{not-json").expect("write corrupt profile store");
    let empty = store.load().expect("corrupt profile store quarantined");
    assert!(empty.profiles.is_empty());
    assert!(
        !path.exists(),
        "corrupt auth-profiles.json should be renamed"
    );
    let quarantined = std::fs::read_dir(path.parent().expect("profile parent"))
        .expect("profile dir")
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"));
    assert!(quarantined, "corrupt profile store should leave artifact");
}

#[tokio::test]
async fn round14_credentials_prefix_listing_and_composio_direct_edges() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let config = harness.config().await;

    let empty =
        openhuman_core::openhuman::credentials::store_composio_api_key(&config, "   ").await;
    assert_eq!(
        empty.expect_err("empty composio key rejected"),
        "composio api_key must not be empty"
    );

    openhuman_core::openhuman::credentials::store_composio_api_key(
        &config,
        "  composio-round14-key  ",
    )
    .await
    .expect("store composio key");
    assert_eq!(
        openhuman_core::openhuman::credentials::get_composio_api_key(&config)
            .expect("get composio key")
            .as_deref(),
        Some("composio-round14-key")
    );

    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        "channel:telegram:managed_dm",
        "primary",
        "telegram-token",
        HashMap::from([("chat_id".to_string(), "123".to_string())]),
        true,
    )
    .expect("store telegram channel token");
    auth.store_provider_token(
        "channel:discord:bot",
        "primary",
        "discord-token",
        HashMap::new(),
        true,
    )
    .expect("store discord channel token");
    auth.store_provider_token("other", "primary", "other-token", HashMap::new(), true)
        .expect("store other token");

    let channels = list_provider_credentials_by_prefix(&config, "channel:")
        .await
        .expect("prefix list");
    let providers = channels
        .iter()
        .map(|profile| profile.provider.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        providers,
        vec!["channel:discord:bot", "channel:telegram:managed_dm"]
    );
    assert!(channels
        .iter()
        .any(|profile| profile.metadata_keys == vec!["chat_id"]));

    let cleared = openhuman_core::openhuman::credentials::clear_composio_api_key(&config)
        .await
        .expect("clear composio key");
    assert_eq!(cleared.value["removed"], true);
    assert_eq!(
        openhuman_core::openhuman::credentials::get_composio_api_key(&config)
            .expect("get cleared composio key"),
        None
    );
    let cleared_again = openhuman_core::openhuman::credentials::clear_composio_api_key(&config)
        .await
        .expect("clear composio key idempotent");
    assert_eq!(cleared_again.value["removed"], false);
}
