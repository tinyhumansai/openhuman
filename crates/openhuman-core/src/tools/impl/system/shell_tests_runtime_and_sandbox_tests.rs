use super::*;
use crate::tools::ShellTool;

#[tokio::test]
async fn shell_blocks_rate_limited() {
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        max_actions_per_hour: 0,
        workspace_dir: test_workdir(),
        action_dir: test_workdir(),
        ..SecurityPolicy::default()
    });
    let tool = ShellTool::new(security, test_runtime(), test_audit());
    let result = tool.execute(json!({"command": "echo test"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Rate limit"));
}

#[tokio::test]
async fn shell_uses_host_node_and_python_from_path() {
    let _storage = native_shell_storage();
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

// -- the protected-path floor at execution -----------------------------------

#[cfg(feature = "security-module")]
fn floor_shell(autonomy: AutonomyLevel) -> ShellTool {
    let (mut policy, _) = crate::security::policy::floor_policy(autonomy);
    policy.workspace_dir = test_workdir();
    policy.action_dir = test_workdir();
    ShellTool::new(Arc::new(policy), test_runtime(), test_audit())
}

/// A sandboxed run's command is admitted at execution even though its text
/// names a protected path; the jail is the boundary.
#[cfg(feature = "security-module")]
#[tokio::test]
async fn a_jailed_command_is_admitted_without_the_protected_path_floor() {
    let tool = floor_shell(AutonomyLevel::Full);
    for command in ["cd .. && ls", "sed 's/../X/' notes.txt", "cat /etc/os-release"] {
        let answer = tool
            .check_for_execution(command, true)
            .await
            .unwrap_or_else(|refusal| panic!("{command}: {}", refusal.output()));
        assert!(answer.floor_waived, "{command}");
    }
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn a_command_outside_the_jail_keeps_the_protected_path_floor() {
    let tool = floor_shell(AutonomyLevel::Full);
    let refusal = tool
        .check_for_execution("cd .. && ls", false)
        .await
        .expect_err("the floor refuses a host command");
    assert!(refusal.output().contains("protected path"), "{}", refusal.output());
    // And the whole tool, called with no run (no sandbox mode), refuses it too.
    let result = tool.execute(json!({"command": "cd .. && ls"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("protected path"), "{}", result.output());
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn a_jailed_command_keeps_the_read_only_refusal_at_execution() {
    let tool = floor_shell(AutonomyLevel::ReadOnly);
    let refusal = tool
        .check_for_execution("rm -rf ../build", true)
        .await
        .expect_err("read-only mode refuses a destructive command");
    assert!(refusal.output().contains("read-only"), "{}", refusal.output());
}
