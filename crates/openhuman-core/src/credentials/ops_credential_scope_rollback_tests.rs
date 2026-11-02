use crate::config::test_env::EnvVarGuard;
use crate::credentials::{AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};
use tempfile::TempDir;

#[tokio::test]
async fn failed_credential_rebind_removes_profile_and_active_user_scope() {
    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    let tmp = TempDir::new().unwrap();
    let _home = EnvVarGuard::set("HOME", tmp.path());
    let root = crate::config::default_root_openhuman_dir().unwrap();
    crate::config::write_active_user_id(&root, "new-user").unwrap();
    let user_dir = crate::config::user_openhuman_dir(&root, "new-user");
    std::fs::create_dir_all(user_dir.join("workspace")).unwrap();
    let config = crate::config::Config {
        config_path: user_dir.join("config.toml"),
        workspace_dir: user_dir.join("workspace"),
        action_dir: user_dir.join("workspace"),
        ..crate::config::Config::default()
    };
    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        APP_SESSION_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
        "session-token",
        std::collections::HashMap::from([("user_id".to_string(), "new-user".to_string())]),
        true,
    )
    .await
    .unwrap();

    let error = super::rollback_failed_credential_rebind(&auth, "workspace rebind failed").await;

    assert!(error.contains("workspace rebind failed"));
    assert!(crate::config::read_active_user_id(&root).is_none());
    assert!(auth
        .get_profile(APP_SESSION_PROVIDER, Some(DEFAULT_AUTH_PROFILE_NAME))
        .await
        .unwrap()
        .is_none());
}
