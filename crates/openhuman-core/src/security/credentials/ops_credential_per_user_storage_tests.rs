use super::*;
use crate::core::runtime::DispatchCtx;
use crate::security::credentials::api_key;

// Credentials on the desktop's DEFAULT storage: the backend belongs to the
// profile (`users/<id>/storage.db` under layout v2), the host installs it from
// its URL, and every credential change rebinds the process to the next
// profile's own database. The other storage tests install one backend with no
// URL, which is never rebound, so they cannot see a record written to the
// wrong database.

/// Installs the default backend of `config`'s workspace from its URL, as the
/// desktop host does (`install_for_url`), and empties the slot on drop. Hold
/// `lock_storage_slot()` for as long as it lives.
struct DefaultStorageForTest;

impl DefaultStorageForTest {
    async fn install(config: &crate::config::Config) -> Self {
        let url = crate::storage::configured_url(config).expect("a default storage URL");
        assert!(url.starts_with("sqlite:"), "the default is SQLite: {url}");
        let backend = crate::storage::open(&url).await.unwrap();
        assert!(
            crate::storage::install_for_url(&url, backend).is_none(),
            "no storage backend may be left installed by another test"
        );
        assert!(
            matches!(crate::storage::secrets::current(), Ok(Some(_))),
            "the auth profiles must be served from storage secrets in this test"
        );
        Self
    }
}

impl Drop for DefaultStorageForTest {
    fn drop(&mut self) {
        crate::storage::clear();
    }
}

fn session_token(user: &str) -> String {
    let exp = chrono::Utc::now() + chrono::Duration::hours(1);
    jwt_with_payload(json!({ "sub": user, "exp": exp.timestamp() }))
}

async fn sign_in(config: &crate::config::Config, user: &str) -> crate::config::Config {
    store_session(
        &DispatchCtx::process_default(),
        config,
        &session_token(user),
        None,
        Some(json!({ "id": user })),
    )
    .await
    .unwrap();
    crate::config::load_config_with_timeout().await.unwrap()
}

async fn set_api_key(config: &crate::config::Config, key: &str) {
    set_credential(
        &DispatchCtx::process_default(),
        config,
        SetCredentialRequest {
            token: key.into(),
            kind: Some("api-key".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

/// The default storage database every profile under `home` holds, by user id.
fn profile_databases(home: &std::path::Path) -> Vec<String> {
    let users = home.join(".openhuman").join("users");
    let mut ids: Vec<String> = std::fs::read_dir(&users)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().join("storage.db").is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

// A session installed on the default storage is stored in the signed-in
// user's own database, so it reads back after the rebind.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::await_holding_lock)] // Serializes the process-wide storage slot.
async fn a_session_is_stored_in_the_users_own_database() {
    let _module = crate::integrations::composio::module_client::module_lock_guard().await;
    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    let _slot = crate::storage::test_support::lock_storage_slot();
    let tmp = TempDir::new().unwrap();
    let _home = EnvVarGuard::set("HOME", tmp.path());
    let _env = EnvVarGuard::unset_many(&["OPENHUMAN_STORAGE_URL", "OPENHUMAN_WORKSPACE"]);
    let pre_login = crate::config::load_config_with_timeout().await.unwrap();
    let _storage = DefaultStorageForTest::install(&pre_login).await;

    let token = session_token("user-42");
    store_session(
        &DispatchCtx::process_default(),
        &pre_login,
        &token,
        None,
        Some(json!({ "id": "user-42" })),
    )
    .await
    .unwrap();
    let user_scoped = crate::config::load_config_with_timeout().await.unwrap();
    assert_ne!(user_scoped.config_path, pre_login.config_path);
    assert!(
        profile_databases(tmp.path()).contains(&"user-42".to_string()),
        "signing in rebinds to the user's own storage.db: {:?}",
        profile_databases(tmp.path())
    );

    assert_eq!(
        session_support::get_session_token(&user_scoped)
            .unwrap()
            .as_deref(),
        Some(token.as_str()),
        "the session must read back from the user's own database"
    );
    let state = auth_get_state(&user_scoped).await.unwrap().value;
    assert!(state.is_authenticated);
    assert_eq!(state.credential.as_deref(), Some("session"));
    assert_eq!(state.user_id.as_deref(), Some("user-42"));
}

// #6318 / #8366 on the default storage: a session-only sign-out moves the
// API key to the signed-out profile, where it stays in effect (also after the
// process reopens the store), and takes it out of the user's database so a
// later login to this account does not resurrect it.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::await_holding_lock)] // Serializes the process-wide storage slot.
async fn a_session_sign_out_keeps_the_api_key_on_per_user_storage() {
    let _module = crate::integrations::composio::module_client::module_lock_guard().await;
    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    let _slot = crate::storage::test_support::lock_storage_slot();
    let tmp = TempDir::new().unwrap();
    let _home = EnvVarGuard::set("HOME", tmp.path());
    let _env = EnvVarGuard::unset_many(&["OPENHUMAN_STORAGE_URL", "OPENHUMAN_WORKSPACE"]);
    let pre_login = crate::config::load_config_with_timeout().await.unwrap();
    let _storage = DefaultStorageForTest::install(&pre_login).await;

    let user_scoped = sign_in(&pre_login, "user-42").await;
    set_api_key(&user_scoped, "sk-live").await;
    let before = auth_get_state(&user_scoped).await.unwrap().value;
    assert_eq!(before.credential.as_deref(), Some("api-key"), "control");

    let cleared = clear_credential(
        &DispatchCtx::process_default(),
        &user_scoped,
        Some(session_support::CredentialKind::Session),
    )
    .await
    .unwrap()
    .value;
    assert_eq!(cleared["removedSession"], true);
    assert_eq!(cleared["removedApiKey"], false);

    let signed_out = crate::config::load_config_with_timeout().await.unwrap();
    assert_eq!(signed_out.config_path, pre_login.config_path);
    assert_eq!(
        api_key::get_api_key(&signed_out).unwrap().as_deref(),
        Some("sk-live"),
        "the API key must survive a session sign-out on per-user storage"
    );
    let state = auth_get_state(&signed_out).await.unwrap().value;
    assert!(state.is_authenticated);
    assert_eq!(state.credential.as_deref(), Some("api-key"));

    // Reopened from its URL, as a restarted host would.
    crate::storage::clear();
    let reopened = DefaultStorageForTest::install(&signed_out).await;
    let state = auth_get_state(&signed_out).await.unwrap().value;
    assert_eq!(
        state.credential.as_deref(),
        Some("api-key"),
        "the key must still be in effect after the store is reopened"
    );
    drop(reopened);
    let _storage = DefaultStorageForTest::install(&signed_out).await;

    // Moved, not copied: signing back in finds no stale key in the user's
    // database (only the session authenticates there).
    let again = sign_in(&signed_out, "user-42").await;
    assert_eq!(api_key::get_api_key(&again).unwrap(), None);
    let state = auth_get_state(&again).await.unwrap().value;
    assert_eq!(state.credential.as_deref(), Some("session"));
}

// The multi-user side: the key carried into the signed-out profile belongs to
// this device's signed-out state, never to another account. A different user
// signing in afterwards does not get it.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::await_holding_lock)] // Serializes the process-wide storage slot.
async fn another_user_signing_in_does_not_get_the_first_users_api_key() {
    let _module = crate::integrations::composio::module_client::module_lock_guard().await;
    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    let _slot = crate::storage::test_support::lock_storage_slot();
    let tmp = TempDir::new().unwrap();
    let _home = EnvVarGuard::set("HOME", tmp.path());
    let _env = EnvVarGuard::unset_many(&["OPENHUMAN_STORAGE_URL", "OPENHUMAN_WORKSPACE"]);
    let pre_login = crate::config::load_config_with_timeout().await.unwrap();
    let _storage = DefaultStorageForTest::install(&pre_login).await;

    let first = sign_in(&pre_login, "user-42").await;
    set_api_key(&first, "sk-first-user").await;
    clear_credential(
        &DispatchCtx::process_default(),
        &first,
        Some(session_support::CredentialKind::Session),
    )
    .await
    .unwrap();
    let signed_out = crate::config::load_config_with_timeout().await.unwrap();
    assert_eq!(
        api_key::get_api_key(&signed_out).unwrap().as_deref(),
        Some("sk-first-user"),
        "control: the key is in effect while signed out"
    );

    let second = sign_in(&signed_out, "user-77").await;
    assert_eq!(
        api_key::get_api_key(&second).unwrap(),
        None,
        "another account must not get the first user's API key"
    );
    let state = auth_get_state(&second).await.unwrap().value;
    assert!(state.is_authenticated);
    assert_eq!(state.credential.as_deref(), Some("session"));
    assert_eq!(state.user_id.as_deref(), Some("user-77"));
}

// A carry that fails after the sign-out rebound storage to the signed-out
// database cannot put the key back: its source is another database, no longer
// installed. The error says so, and nothing is written through the old
// user-scoped config into the signed-out store, where the record would sit
// under this user's secret namespace and read back as nothing.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::await_holding_lock)] // Serializes the process-wide storage slot.
async fn a_failed_carry_on_per_user_storage_says_the_key_must_be_entered_again() {
    let _module = crate::integrations::composio::module_client::module_lock_guard().await;
    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    let _slot = crate::storage::test_support::lock_storage_slot();
    let tmp = TempDir::new().unwrap();
    let _home = EnvVarGuard::set("HOME", tmp.path());
    let _env = EnvVarGuard::unset_many(&["OPENHUMAN_STORAGE_URL", "OPENHUMAN_WORKSPACE"]);
    let pre_login = crate::config::load_config_with_timeout().await.unwrap();
    let _storage = DefaultStorageForTest::install(&pre_login).await;
    let user_scoped = sign_in(&pre_login, "user-42").await;
    set_api_key(&user_scoped, "sk-live").await;

    let error = clear_credential_with(
        &DispatchCtx::process_default(),
        &user_scoped,
        Some(session_support::CredentialKind::Session),
        |_, _| Err(anyhow::anyhow!("injected carry failure")),
    )
    .await
    .unwrap_err();
    assert!(error.contains("injected carry failure"), "{error}");
    assert!(error.contains("must be entered again"), "{error}");

    let signed_out = crate::config::load_config_with_timeout().await.unwrap();
    assert_eq!(signed_out.config_path, pre_login.config_path);
    assert!(
        crate::security::credentials::AuthService::from_config(&signed_out)
            .get_profile(api_key::API_KEY_PROVIDER, None)
            .unwrap()
            .is_none(),
        "no api-key record may be written into the signed-out store under the old user's namespace"
    );
}
