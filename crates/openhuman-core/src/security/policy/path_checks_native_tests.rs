use super::*;

/// An ordinary readable file must not bypass the missing native policy module.
#[tokio::test]
async fn native_path_validation_never_falls_back_when_module_is_unavailable() {
    // The production client's faults intentionally latch for the process. Run
    // this missing-module case in a child so it cannot poison other bus tests.
    const CHILD: &str = "OPENHUMAN_NATIVE_PATH_UNAVAILABLE_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "security::policy::path_checks::native_tests::native_path_validation_never_falls_back_when_module_is_unavailable", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let action = dir.path().join("project");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&action).unwrap();
    std::fs::write(action.join("readme.txt"), "public file").unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "[modules]\nenabled = false\n",
    )
    .unwrap();
    let policy = SecurityPolicy {
        enabled: false,
        workspace_dir: workspace,
        action_dir: action,
        ..SecurityPolicy::default()
    };
    let error = policy.validate_path("readme.txt").await.unwrap_err();
    assert!(error.contains(POLICY_BLOCKED_MARKER), "{error}");
    assert!(error.contains("TinySecurity"), "{error}");
    let error = policy
        .validate_parent_path("new-file.txt")
        .await
        .unwrap_err();
    assert!(error.contains(POLICY_BLOCKED_MARKER), "{error}");
    assert!(error.contains("TinySecurity"), "{error}");
}
