use super::*;
use crate::core::runtime::DispatchCtx;
use crate::credentials::api_key;
use crate::credentials::session_state;

// ── set_credential (local session) ─────────────────────────────

/// A local session token requires a non-empty user payload — the backend
/// fetch path is bypassed entirely, so there is no fallback to derive the
/// user from an API response.
#[tokio::test]
async fn store_session_local_token_rejects_missing_user_payload() {
    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let _home = EnvVarGuard::set("HOME", tmp.path());
    let config = test_config(&tmp);
    let local_token = "header.payload.local";
    let err = store_session(
        &DispatchCtx::process_default(),
        &config,
        local_token,
        None,
        None,
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("local session requires a user payload"),
        "expected 'local session requires a user payload', got: {err}"
    );
}

/// A local session token with a user payload must be accepted without any
/// network call, must force a deterministic `local-<device>` user id
/// regardless of what the caller passes, and must return a stored profile
/// summary.
#[tokio::test]
async fn store_session_local_token_succeeds_without_network_and_forces_local_user_id() {
    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let _home = EnvVarGuard::set("HOME", tmp.path());
    let config = test_config(&tmp);
    let local_token = "header.payload.local";
    let user = serde_json::json!({
        "id": "local",
        "name": "Local User",
        "email": "local@openhuman.local"
    });
    // Pass a different user_id to verify it is overridden.
    let result = store_session(
        &DispatchCtx::process_default(),
        &config,
        local_token,
        Some("should-be-overridden".to_string()),
        Some(user),
    )
    .await
    .unwrap();
    // The credential is installed (no network call was required).
    assert!(result.value.is_authenticated);
    assert_eq!(result.value.credential.as_deref(), Some("local"));
    // Logs must mention that backend validation was skipped.
    let log_text = result.logs.join(" ");
    assert!(
        log_text.contains("local session accepted without backend validation"),
        "expected log confirming no backend call, got: {log_text}"
    );
    let expected_local_user_id = local_session_user_id();
    assert!(
        log_text.contains(&format!(
            "user directory activated for {expected_local_user_id}"
        )),
        "expected user-directory activation log for deterministic local uid, got: {log_text}"
    );
    assert!(
        log_text.contains("onboarding left incomplete for local session setup"),
        "expected local session to remain in onboarding, got: {log_text}"
    );
    // The forced local user id wins over the caller's hint, and the user
    // payload's id fields are rewritten to match.
    assert_eq!(
        result.value.user_id.as_deref(),
        Some(expected_local_user_id.as_str())
    );
    let user = result.value.user.unwrap();
    assert_eq!(user["id"], expected_local_user_id);
    assert_eq!(user["_id"], expected_local_user_id);
    assert_eq!(user["name"], "Local User");
}

/// Installs a SQLite backend (as the desktop boot does) for one test and
/// removes it on drop. Hold `lock_storage_slot()` for as long as it lives.
#[cfg(feature = "storage-sqlite")]
struct StorageBackendForTest(crate::storage::test_support::SqliteBackend);

#[cfg(feature = "storage-sqlite")]
impl StorageBackendForTest {
    fn install() -> Self {
        let fixture = crate::storage::test_support::sqlite_backend().unwrap();
        assert!(
            crate::storage::install(std::sync::Arc::clone(&fixture.backend)).is_none(),
            "no storage backend may be left installed by another test"
        );
        assert!(
            matches!(crate::storage::secrets::current(), Ok(Some(_))),
            "the storage backend should remain installed for the backend credential test"
        );
        Self(fixture)
    }
}

#[cfg(feature = "storage-sqlite")]
impl Drop for StorageBackendForTest {
    fn drop(&mut self) {
        crate::storage::clear();
    }
}

/// Signs `user-42` in and stores `key` against the user-scoped config it
/// activates; returns that config.
#[cfg(feature = "storage-sqlite")]
async fn signed_in_with_api_key(
    config: &crate::config::Config,
    key: &str,
) -> crate::config::Config {
    let exp = chrono::Utc::now() + chrono::Duration::hours(1);
    let token = jwt_with_payload(json!({ "sub": "user-42", "exp": exp.timestamp() }));
    store_session(
        &DispatchCtx::process_default(),
        config,
        &token,
        None,
        Some(json!({ "id": "user-42" })),
    )
    .await
    .unwrap();
    let user_scoped = crate::config::load_config_with_timeout().await.unwrap();
    set_credential(
        &DispatchCtx::process_default(),
        &user_scoped,
        SetCredentialRequest {
            token: key.into(),
            kind: Some("api-key".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(api_key::has_api_key(&user_scoped).await);
    user_scoped
}

// #6318 on the desktop's storage: the host boot installs a SQLite backend
// (`session_store::install_for_host`). A session-only sign-out must still
// leave the API key usable from the signed-out config, while profile metadata
// remains in its independent profile-local encrypted file.
#[cfg(feature = "storage-sqlite")]
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::await_holding_lock)] // Serializes the process-wide storage slot.
async fn clearing_the_session_keeps_the_api_key_on_a_storage_backend() {
    let _module = crate::integrations::composio::module_client::module_lock_guard().await;
    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    let _slot = crate::storage::test_support::lock_storage_slot();
    let _backend = StorageBackendForTest::install();
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let _home = EnvVarGuard::set("HOME", tmp.path());
    let config = test_config(&tmp);
    let user_scoped = signed_in_with_api_key(&config, "sk-live").await;

    let cleared = clear_credential(
        &DispatchCtx::process_default(),
        &user_scoped,
        Some(session_state::CredentialKind::Session),
    )
    .await
    .unwrap()
    .value;
    assert_eq!(cleared["removedSession"], true);
    assert_eq!(cleared["removedApiKey"], false);

    let signed_out = crate::config::load_config_with_timeout().await.unwrap();
    assert_ne!(signed_out.config_path, user_scoped.config_path);
    assert_eq!(
        api_key::get_api_key(&signed_out).await.unwrap().as_deref(),
        Some("sk-live"),
        "the API key must survive a session sign-out on a storage backend"
    );
    let state = auth_get_state(&signed_out).await.unwrap().value;
    assert!(state.is_authenticated);
    assert_eq!(state.credential.as_deref(), Some("api-key"));
}

// The other side of the same rule: signing out of everything (no kind) on a
// storage backend still removes the API key, so keeping it on a session-only
// sign-out cannot leave a key behind after a full sign-out.
#[cfg(feature = "storage-sqlite")]
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::await_holding_lock)] // Serializes the process-wide storage slot.
async fn clearing_everything_removes_the_api_key_on_a_storage_backend() {
    let _module = crate::integrations::composio::module_client::module_lock_guard().await;
    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    let _slot = crate::storage::test_support::lock_storage_slot();
    let _backend = StorageBackendForTest::install();
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let _home = EnvVarGuard::set("HOME", tmp.path());
    let config = test_config(&tmp);
    let user_scoped = signed_in_with_api_key(&config, "sk-live").await;

    let cleared = clear_credential(&DispatchCtx::process_default(), &user_scoped, None)
        .await
        .unwrap()
        .value;
    assert_eq!(cleared["removedSession"], true);
    assert_eq!(cleared["removedApiKey"], true);

    let signed_out = crate::config::load_config_with_timeout().await.unwrap();
    assert_eq!(api_key::get_api_key(&signed_out).await.unwrap(), None);
    assert_eq!(api_key::get_api_key(&user_scoped).await.unwrap(), None);
    let state = auth_get_state(&signed_out).await.unwrap().value;
    assert!(!state.is_authenticated);
}
