use super::*;
use crate::credentials::api_key as current_api_key;
use crate::security::credentials::api_key;
use crate::security::credentials::ops::restore_api_key;
use serde_json::json;

#[test]
fn process_wide_credentials_are_single_user_only() {
    assert!(process_credential_refusal(false, "set_credential").is_ok());
    let err = process_credential_refusal(true, "set_credential").unwrap_err();
    assert!(err.contains("profiles.set_credential"), "{err}");
}

// A failed sign-out puts the API key it took out of its source back; a key
// that cannot be stored again is logged, never a panic or an error of its own.
#[tokio::test]
#[allow(clippy::await_holding_lock)] // Serializes the process-wide storage slot.
async fn restore_api_key_puts_the_key_back_and_tolerates_a_failed_store() {
    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    let _slot = crate::storage::test_support::lock_storage_slot();
    assert!(
        crate::storage::installed().is_none(),
        "file-backed profiles in this test"
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let _home = crate::config::test_env::EnvVarGuard::set("HOME", tmp.path());
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };

    restore_api_key(&config, None, "test").await;
    assert!(!api_key::has_api_key(&config).await, "nothing to put back");

    restore_api_key(&config, Some("sk-restored"), "test").await;
    assert_eq!(
        api_key::get_api_key(&config).await.unwrap().as_deref(),
        Some("sk-restored")
    );

    // A blank key cannot be stored: the failure is logged and the stored key stays.
    restore_api_key(&config, Some("   "), "test").await;
    assert_eq!(
        api_key::get_api_key(&config).await.unwrap().as_deref(),
        Some("sk-restored")
    );
}

/// Signs `user-42` in on file-backed profiles under a temp HOME and stores
/// `sk-keep` beside the user-scoped config it activates; returns the pre-login
/// and the user-scoped config.
///
/// Everything here goes through this tree's operations and `crate::credentials::api_key`,
/// the store production reads. The legacy `security::credentials` copy keeps
/// its keys in another document, so mixing the two read back `None`.
async fn signed_in_with_key() -> (Config, Config) {
    let pre_login = crate::config::load_config_with_timeout().await.unwrap();
    store_session(
        &DispatchCtx::process_default(),
        &pre_login,
        "synthetic-session-token",
        Some("user-42".into()),
        Some(json!({ "id": "user-42" })),
    )
    .await
    .unwrap();
    let user_scoped = crate::config::load_config_with_timeout().await.unwrap();
    set_credential(
        &DispatchCtx::process_default(),
        &user_scoped,
        SetCredentialRequest {
            token: "sk-keep".into(),
            kind: Some("api-key".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        current_api_key::get_api_key(&user_scoped)
            .await
            .unwrap()
            .as_deref(),
        Some("sk-keep"),
        "control"
    );
    (pre_login, user_scoped)
}

// The carry into the signed-out workspace failing returns the error with the
// API key still at its source: the source copy is cleared only after the carry
// is stored. With file-backed profiles the source stays reachable after the
// teardown.
#[tokio::test]
#[allow(clippy::await_holding_lock)] // Serializes the process-wide storage slot.
async fn a_failed_carry_during_sign_out_puts_the_api_key_back() {
    let _module = crate::integrations::composio::module_client::module_lock_guard().await;
    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    let _slot = crate::storage::test_support::lock_storage_slot();
    assert!(
        crate::storage::installed().is_none(),
        "file-backed profiles in this test"
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let _home = crate::config::test_env::EnvVarGuard::set("HOME", tmp.path());
    let (pre_login, user_scoped) = signed_in_with_key().await;

    // The signed-out workspace's credential document cannot be written.
    let destination = crate::credentials::profiles::profiles_file(
        &crate::credentials::state_dir_from_config(&pre_login),
    );
    let _ = std::fs::remove_file(&destination);
    std::fs::create_dir_all(&destination).unwrap();
    assert!(
        current_api_key::store_api_key(&pre_login, "cannot-write")
            .await
            .is_err(),
        "positive control: the destination credential document cannot be written"
    );

    let error = clear_credential(
        &DispatchCtx::process_default(),
        &user_scoped,
        Some(crate::credentials::session_state::CredentialKind::Session),
    )
    .await
    .unwrap_err();
    assert!(!error.contains("must be entered again"), "{error}");
    assert_eq!(
        current_api_key::get_api_key(&user_scoped)
            .await
            .unwrap()
            .as_deref(),
        Some("sk-keep"),
        "a failed carry must put the API key back where it was"
    );
}
