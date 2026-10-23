use super::*;

#[tokio::test]
async fn shell_uses_host_node_and_python_from_path() {
    for (binary, command, expected) in [
        ("node", "node -e 'console.log(1)'", "1"),
        ("python3", "python3 -c 'print(1)'", "1"),
    ] {
        let available = std::process::Command::new(binary)
            .arg("--version")
            .output()
            .is_ok_and(|version| version.status.success());
        if !available {
            continue;
        }

        let tool = ShellTool::new(test_security(AutonomyLevel::Full), test_runtime());
        let result = tool.execute(json!({"command": command})).await.unwrap();
        assert!(!result.is_error, "{binary} failed: {}", result.output());
        assert!(
            result.output().contains(expected),
            "{binary}: {}",
            result.output()
        );
    }
}
