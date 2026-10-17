use super::*;
use crate::config::test_env::EnvVarGuard;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use tempfile::TempDir;

fn config_in(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

#[test]
fn api_key_env_seeds_only_an_empty_store() {
    let _lock = crate::config::TEST_ENV_LOCK.blocking_lock();
    let tmp = TempDir::new().unwrap();
    let config = config_in(&tmp);
    let _guard = EnvVarGuard::unset(BACKEND_API_KEY_ENV);

    unsafe { std::env::set_var(BACKEND_API_KEY_ENV, "  ") };
    seed_api_key_from_env(&config);
    assert!(
        !api_key::has_api_key(&config),
        "blank env must not install a key"
    );

    unsafe { std::env::set_var(BACKEND_API_KEY_ENV, "sk-boot") };
    seed_api_key_from_env(&config);
    assert_eq!(
        api_key::get_api_key(&config).unwrap().as_deref(),
        Some("sk-boot")
    );

    unsafe { std::env::set_var(BACKEND_API_KEY_ENV, "sk-rotated") };
    seed_api_key_from_env(&config);
    assert_eq!(
        api_key::get_api_key(&config).unwrap().as_deref(),
        Some("sk-boot"),
        "a stored key is never overwritten from the environment"
    );
}

#[tokio::test]
async fn session_env_is_ignored_without_a_subject_and_when_unset() {
    let _lock = crate::config::TEST_ENV_LOCK.lock().await;
    let tmp = TempDir::new().unwrap();
    let config = config_in(&tmp);
    let _guard = EnvVarGuard::unset(BACKEND_SESSION_TOKEN_ENV);

    unsafe { std::env::remove_var(BACKEND_SESSION_TOKEN_ENV) };
    seed_session_from_env(&DispatchCtx::process_default(), &config).await;
    assert!(get_session_token(&config).unwrap().is_none());

    // An opaque token has no subject claim and no host to supply one, so the
    // install is refused (logged) and the store stays empty.
    unsafe { std::env::set_var(BACKEND_SESSION_TOKEN_ENV, "opaque-token") };
    seed_session_from_env(&DispatchCtx::process_default(), &config).await;
    assert!(get_session_token(&config).unwrap().is_none());
}
