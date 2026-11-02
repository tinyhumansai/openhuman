use super::*;

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

    restore_api_key(&config, None, "test");
    assert!(!api_key::has_api_key(&config), "nothing to put back");

    restore_api_key(&config, Some("sk-restored"), "test");
    assert_eq!(
        api_key::get_api_key(&config).unwrap().as_deref(),
        Some("sk-restored")
    );

    // A blank key cannot be stored: the failure is logged and the stored key stays.
    restore_api_key(&config, Some("   "), "test");
    assert_eq!(
        api_key::get_api_key(&config).unwrap().as_deref(),
        Some("sk-restored")
    );
}

/// Signs `user-42` in on file-backed profiles under a temp HOME and stores
/// `sk-keep` beside the user-scoped config it activates; returns that config.
async fn signed_in_with_key() -> Config {
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
        api_key::get_api_key(&user_scoped).unwrap().as_deref(),
        Some("sk-keep"),
        "control"
    );
    user_scoped
}

// The carry into the signed-out workspace failing returns the error with the
// API key back at its source. With file-backed profiles the source stays
// reachable after the teardown, so it is put back.
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
    let user_scoped = signed_in_with_key().await;

    let error = clear_credential_with(
        &DispatchCtx::process_default(),
        &user_scoped,
        Some(CredentialKind::Session),
        |_, _| Err(anyhow::anyhow!("injected carry failure")),
    )
    .await
    .unwrap_err();
    assert!(error.contains("injected carry failure"), "{error}");
    assert!(!error.contains("must be entered again"), "{error}");
    assert_eq!(
        api_key::get_api_key(&user_scoped).unwrap().as_deref(),
        Some("sk-keep"),
        "a failed carry must put the API key back where it was"
    );
}
