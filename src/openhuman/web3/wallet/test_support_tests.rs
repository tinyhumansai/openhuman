use super::*;

#[tokio::test]
async fn workspace_env_guard_restores_workspace_env_when_dropped() {
    let env_lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
    std::env::set_var("OPENHUMAN_WORKSPACE", "/tmp/openhuman-existing-workspace");

    let temp = TempDir::new().expect("temp dir");
    let prev = std::env::var_os("OPENHUMAN_WORKSPACE");
    std::env::set_var("OPENHUMAN_WORKSPACE", temp.path());
    let workspace_guard = WorkspaceEnvGuard {
        prev,
        _env_lock: env_lock,
    };
    assert_eq!(
        std::env::var_os("OPENHUMAN_WORKSPACE"),
        Some(temp.path().as_os_str().to_os_string())
    );

    drop(workspace_guard);
    let _cleanup_lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        std::env::var_os("OPENHUMAN_WORKSPACE"),
        Some(std::ffi::OsString::from(
            "/tmp/openhuman-existing-workspace"
        ))
    );

    match previous {
        Some(value) => std::env::set_var("OPENHUMAN_WORKSPACE", value),
        None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
    }
}
