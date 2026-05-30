#[path = "config_auth_app_state_connectivity_e2e.rs"]
mod base_coverage;

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::openhuman::app_state::{snapshot, update_local_state, StoredAppStatePatch};
use openhuman_core::openhuman::config::rpc as config_rpc;
use openhuman_core::openhuman::credentials::{
    auth_get_session_token_json, clear_session, list_provider_credentials,
    remove_provider_credentials, store_provider_credentials, store_session, AuthService,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Round13EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl Round13EnvVarGuard {
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

impl Drop for Round13EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

struct Round13Harness {
    _tmp: TempDir,
    workspace_override: PathBuf,
    _guards: Vec<Round13EnvVarGuard>,
}

impl Round13Harness {
    async fn config(&self) -> openhuman_core::openhuman::config::Config {
        config_rpc::load_config_with_timeout()
            .await
            .expect("isolated config should load")
    }

    fn app_state_file(&self) -> PathBuf {
        self.workspace_override
            .join("workspace/state/app-state.json")
    }
}

fn round13_env_lock() -> std::sync::MutexGuard<'static, ()> {
    // Delegate to the lock owned by the included `base_coverage` (config_auth)
    // module so round13 env mutations serialize against every other
    // OPENHUMAN_WORKSPACE/BACKEND_URL-mutating test in this combined binary.
    // Two separate mutexes let the two groups race and quarantine the wrong
    // workspace (flaky `!state_file.exists()` / spurious JSON-RPC errors).
    base_coverage::env_lock()
}

fn write_round13_min_config(openhuman_dir: &Path) {
    std::fs::create_dir_all(openhuman_dir).expect("create openhuman config dir");
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "round13-raw-coverage-model"
default_temperature = 0.2
onboarding_completed = false
chat_onboarding_completed = false

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
"#;
    std::fs::write(openhuman_dir.join("config.toml"), cfg).expect("write config.toml");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(cfg).expect("test config must match schema");
}

fn round13_setup() -> Round13Harness {
    let tmp = tempdir().expect("tempdir");
    let workspace_override = tmp.path().join("openhuman");
    write_round13_min_config(&workspace_override);

    let guards = vec![
        Round13EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace_override),
        Round13EnvVarGuard::set_to_path("HOME", tmp.path()),
        Round13EnvVarGuard::unset("BACKEND_URL"),
        Round13EnvVarGuard::unset("VITE_BACKEND_URL"),
        Round13EnvVarGuard::unset("OPENHUMAN_API_URL"),
        Round13EnvVarGuard::unset("OPENHUMAN_CORE_RPC_URL"),
        Round13EnvVarGuard::unset("OPENHUMAN_CORE_PORT"),
        Round13EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        Round13EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        Round13EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        Round13EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
        Round13EnvVarGuard::set("OPENHUMAN_BROWSER_ALLOW_ALL_RPC_ENABLE", ""),
    ];

    Round13Harness {
        _tmp: tmp,
        workspace_override,
        _guards: guards,
    }
}

#[tokio::test]
async fn raw_round13_provider_field_fallbacks_default_profile_and_non_active_listing() {
    let _lock = round13_env_lock();
    let harness = round13_setup();
    let config = harness.config().await;

    let empty = store_provider_credentials(&config, "  ", None, None, None, None).await;
    assert_eq!(empty.unwrap_err(), "provider is required");

    let invalid_fields = store_provider_credentials(
        &config,
        "raw-provider",
        None,
        None,
        Some(json!("bad")),
        None,
    )
    .await;
    assert!(invalid_fields
        .unwrap_err()
        .contains("fields must be a JSON object"));

    let missing =
        store_provider_credentials(&config, "raw-provider", None, None, Some(json!({})), None)
            .await;
    assert!(missing
        .unwrap_err()
        .contains("provide at least one credential"));

    let stored_from_token_field = store_provider_credentials(
        &config,
        "raw-provider",
        None,
        None,
        Some(json!({
            "token": "field-token",
            "region": "us-test-1"
        })),
        Some(true),
    )
    .await
    .expect("store token field credential");
    assert_eq!(stored_from_token_field.value.provider, "raw-provider");
    assert_eq!(stored_from_token_field.value.profile_name, "default");
    assert!(stored_from_token_field.value.has_token);

    let auth = AuthService::from_config(&config);
    assert_eq!(
        auth.get_provider_bearer_token("raw-provider", None)
            .expect("read active bearer")
            .as_deref(),
        Some("field-token")
    );

    let stored_from_api_key = store_provider_credentials(
        &config,
        "raw-provider",
        Some("secondary"),
        None,
        Some(json!({
            "api_key": "api-key-token",
            "label": "kept as metadata"
        })),
        Some(false),
    )
    .await
    .expect("store api_key field credential");
    assert_eq!(stored_from_api_key.value.profile_name, "secondary");

    let listed = list_provider_credentials(&config, Some("raw-provider".to_string()))
        .await
        .expect("list raw provider credentials")
        .value;
    let names = listed
        .iter()
        .map(|profile| profile.profile_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["default", "secondary"]);

    let removed_default = remove_provider_credentials(&config, "raw-provider", None)
        .await
        .expect("remove default profile");
    assert_eq!(removed_default.value["removed"], true);
    assert_eq!(removed_default.value["profile"], "default");

    let removed_missing = remove_provider_credentials(&config, "raw-provider", Some("missing"))
        .await
        .expect("remove missing profile is non-fatal");
    assert_eq!(removed_missing.value["removed"], false);
}

#[tokio::test]
async fn raw_round13_local_session_string_payload_and_double_clear_are_offline() {
    let _lock = round13_env_lock();
    let harness = round13_setup();
    let config = harness.config().await;

    let empty_token = store_session(&config, "   ", None, None).await;
    assert_eq!(empty_token.unwrap_err(), "token is required");

    let local_without_user = store_session(&config, "header.payload.local", None, None).await;
    assert_eq!(
        local_without_user.unwrap_err(),
        "local session requires a user payload"
    );

    let stored = store_session(
        &config,
        "header.payload.local",
        Some("ignored-user-hint".to_string()),
        Some(json!("string user payload is preserved")),
    )
    .await
    .expect("store local session with non-object user payload");
    assert_eq!(stored.value.provider, "app-session");
    assert!(stored
        .logs
        .iter()
        .any(|log| log == "local session accepted without backend validation"));

    let effective_config = config_rpc::load_config_with_timeout()
        .await
        .expect("reload active local user config");
    let token = auth_get_session_token_json(&effective_config)
        .await
        .expect("session token")
        .value;
    assert_eq!(token["token"], "header.payload.local");

    let first_clear = clear_session(&effective_config)
        .await
        .expect("clear stored session");
    assert_eq!(first_clear.value["removed"], true);

    let signed_out_config = config_rpc::load_config_with_timeout()
        .await
        .expect("reload signed-out config");
    let second_clear = clear_session(&signed_out_config)
        .await
        .expect("clear missing session is idempotent");
    assert_eq!(second_clear.value["removed"], false);
}

#[tokio::test]
async fn raw_round13_app_state_update_trims_clears_and_preserves_optional_local_state() {
    let _lock = round13_env_lock();
    let harness = round13_setup();

    let updated = update_local_state(StoredAppStatePatch {
        encryption_key: Some(Some("  raw-key  ".to_string())),
        onboarding_tasks: Some(Some(Default::default())),
    })
    .await
    .expect("update local app state")
    .value;
    assert_eq!(updated.encryption_key.as_deref(), Some("raw-key"));
    assert!(updated.onboarding_tasks.is_some());

    let cleared_key = update_local_state(StoredAppStatePatch {
        encryption_key: Some(Some("   ".to_string())),
        onboarding_tasks: None,
    })
    .await
    .expect("blank key clears encryption key")
    .value;
    assert!(cleared_key.encryption_key.is_none());
    assert!(cleared_key.onboarding_tasks.is_some());

    let cleared_tasks = update_local_state(StoredAppStatePatch {
        encryption_key: None,
        onboarding_tasks: Some(None),
    })
    .await
    .expect("null tasks clear onboarding tasks")
    .value;
    assert!(cleared_tasks.encryption_key.is_none());
    assert!(cleared_tasks.onboarding_tasks.is_none());

    let unchanged = update_local_state(StoredAppStatePatch {
        encryption_key: None,
        onboarding_tasks: None,
    })
    .await
    .expect("empty patch preserves cleared state")
    .value;
    assert!(unchanged.encryption_key.is_none());
    assert!(unchanged.onboarding_tasks.is_none());

    let raw = std::fs::read_to_string(harness.app_state_file()).expect("app state persisted");
    assert_eq!(
        serde_json::from_str::<Value>(&raw).expect("valid app state json"),
        json!({})
    );
}

#[tokio::test]
async fn raw_round13_app_state_snapshot_quarantines_null_and_malformed_local_state_files() {
    let _lock = round13_env_lock();
    let harness = round13_setup();
    let state_file = harness.app_state_file();
    let state_dir = state_file.parent().expect("state dir");
    std::fs::create_dir_all(state_dir).expect("create state dir");

    std::fs::write(&state_file, "null").expect("write semantically invalid app state");
    let null_snapshot = snapshot().await.expect("snapshot with null state").value;
    assert!(null_snapshot.local_state.encryption_key.is_none());
    assert!(null_snapshot.local_state.onboarding_tasks.is_none());
    assert!(
        !state_file.exists(),
        "invalid app-state.json should be quarantined or removed"
    );
    assert!(
        std::fs::read_dir(state_dir)
            .expect("state dir entries")
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("app-state.json.corrupted.")),
        "null app-state file should leave a quarantine artifact"
    );

    std::fs::write(&state_file, "{not-json").expect("write malformed app state");
    let malformed_snapshot = snapshot()
        .await
        .expect("snapshot with malformed state")
        .value;
    assert!(malformed_snapshot.local_state.encryption_key.is_none());
    assert!(malformed_snapshot.local_state.onboarding_tasks.is_none());
    assert!(!state_file.exists());
}

async fn spawn_probe_listener(
    host: &str,
    status: &str,
    body: &'static str,
) -> Option<(
    u16,
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Sender<()>,
)> {
    let listener = match tokio::net::TcpListener::bind((host, 0)).await {
        Ok(listener) => listener,
        Err(_) => return None,
    };
    let port = listener.local_addr().expect("probe listener addr").port();
    let status = status.to_string();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((mut stream, _addr)) = accepted else {
                        break;
                    };
                    let mut req_buf = [0_u8; 1024];
                    let _ = stream.read(&mut req_buf).await;
                    let response = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                }
            }
        }
    });
    Some((port, task, shutdown_tx))
}

#[tokio::test]
async fn raw_round13_connectivity_picker_identifies_openhuman_probe_listener() {
    let Some((preferred, task, shutdown_tx)) =
        spawn_probe_listener("127.0.0.1", "200 OK", r#"{"name":"openhuman","ok":true}"#).await
    else {
        return;
    };

    let result = openhuman_core::openhuman::connectivity::rpc::pick_listen_port_for_host(
        "127.0.0.1",
        preferred,
    )
    .await;
    let err = result.expect_err("openhuman probe listener should request takeover");
    assert!(
        matches!(
            err,
            openhuman_core::openhuman::connectivity::rpc::PickListenPortError::WouldTakeOver {
                preferred: p,
                ref fingerprint
            } if p == preferred && fingerprint == "openhuman-core"
        ),
        "unexpected picker error: {err:?}"
    );

    let _ = shutdown_tx.send(());
    let _ = task.await;
}

#[tokio::test]
async fn raw_round13_connectivity_picker_falls_back_for_non_success_probe_status() {
    let Some((preferred, task, shutdown_tx)) = spawn_probe_listener(
        "127.0.0.1",
        "503 Service Unavailable",
        r#"{"name":"openhuman"}"#,
    )
    .await
    else {
        return;
    };

    let picked = openhuman_core::openhuman::connectivity::rpc::pick_listen_port_for_host(
        "127.0.0.1",
        preferred,
    )
    .await
    .expect("non-openhuman status should fall back");
    assert_ne!(picked.port, preferred);
    assert_eq!(picked.fallback_from, Some(preferred));
    drop(picked.listener);

    let _ = shutdown_tx.send(());
    let _ = task.await;
}

#[tokio::test]
async fn raw_round13_connectivity_picker_falls_back_for_non_identifying_probe_body() {
    let Some((preferred, task, shutdown_tx)) =
        spawn_probe_listener("127.0.0.1", "200 OK", r#"{"name":"someone-else"}"#).await
    else {
        return;
    };

    let picked = openhuman_core::openhuman::connectivity::rpc::pick_listen_port_for_host(
        "127.0.0.1",
        preferred,
    )
    .await
    .expect("non-identifying body should fall back");
    assert_ne!(picked.port, preferred);
    assert_eq!(picked.fallback_from, Some(preferred));
    drop(picked.listener);

    let _ = shutdown_tx.send(());
    let _ = task.await;
}
