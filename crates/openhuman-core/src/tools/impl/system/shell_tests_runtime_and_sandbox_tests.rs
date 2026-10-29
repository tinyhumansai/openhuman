use super::*;
use crate::tools::ShellTool;

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

fn docker_runtime_config() -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.runtime.kind = "docker".into();
    config.runtime.docker.image = "example/sandbox:pinned".into();
    config
}

#[tokio::test]
async fn sandboxed_shell_follows_the_configured_docker_runtime() {
    let tool = ShellTool::with_config(
        test_security(AutonomyLevel::Full),
        test_runtime(),
        test_audit(),
        Arc::new(docker_runtime_config()),
    );

    let policy = tool
        .sandboxed_policy(&DispatchCtx::unbound(), &std::env::temp_dir())
        .await
        .unwrap();

    assert_eq!(policy.backend, crate::sandbox::SandboxBackendKind::Docker);
    assert_eq!(
        policy
            .docker_overrides
            .and_then(|docker| docker.image)
            .as_deref(),
        Some("example/sandbox:pinned")
    );
}

#[tokio::test]
async fn sandboxed_shell_reads_the_runtime_of_the_bound_context() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    let core = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(
            docker_runtime_config(),
            DomainSet::full(),
            Default::default(),
        )
        .profile("sandboxed-shell-runtime"),
    );
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Full),
        test_runtime(),
        test_audit(),
    );

    let policy = tool
        .sandboxed_policy(&DispatchCtx::new(core), &std::env::temp_dir())
        .await
        .unwrap();

    assert_eq!(policy.backend, crate::sandbox::SandboxBackendKind::Docker);
}
