use super::*;

#[tokio::test]
async fn shell_blocks_rate_limited() {
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        max_actions_per_hour: 0,
        workspace_dir: std::env::temp_dir(),
        action_dir: std::env::temp_dir(),
        ..SecurityPolicy::default()
    });
    let tool = ShellTool::new(security, test_runtime(), test_audit());
    let result = tool.execute(json!({"command": "echo test"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Rate limit"));
}

#[tokio::test]
async fn shell_uses_host_node_and_python_from_path() {
    for (binary, command, expected) in [
        ("node", "node -e 'console.log(1)'", "1"),
        ("python3", "python3 -c 'print(1)'", "1"),
    ] {
        if std::process::Command::new(binary)
            .arg("--version")
            .output()
            .is_err()
        {
            continue;
        }

        let tool = ShellTool::new(
            test_security(AutonomyLevel::Full),
            test_runtime(),
            test_audit(),
        );
        let result = tool.execute(json!({"command": command})).await.unwrap();
        assert!(!result.is_error, "{binary} failed: {}", result.output());
        assert!(
            result.output().contains(expected),
            "{binary}: {}",
            result.output()
        );
    }
}
