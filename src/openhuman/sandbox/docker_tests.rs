use super::*;
use crate::openhuman::sandbox::types::DockerOverrides;
use std::path::PathBuf;

fn test_policy() -> SandboxPolicy {
    SandboxPolicy {
        backend: SandboxBackendKind::Docker,
        workspace_root: PathBuf::from("/tmp/test-workspace"),
        read_only_mounts: vec![],
        allow_network: false,
        env_passthrough: vec!["PATH".into(), "HOME".into()],
        docker_overrides: None,
    }
}

#[test]
fn validate_docker_policy_accepts_safe_config() {
    let policy = test_policy();
    assert!(validate_docker_policy(&policy).is_ok());
}

#[test]
fn validate_docker_policy_rejects_host_network() {
    let mut policy = test_policy();
    policy.docker_overrides = Some(DockerOverrides {
        network: Some("host".into()),
        ..DockerOverrides::default()
    });
    let issues = validate_docker_policy(&policy).unwrap_err();
    assert!(issues.iter().any(|i| i.contains("host network")));
}

#[test]
fn validate_docker_policy_rejects_dangerous_mounts() {
    let mut policy = test_policy();
    policy.read_only_mounts = vec![PathBuf::from("/var/run/docker.sock")];
    let issues = validate_docker_policy(&policy).unwrap_err();
    assert!(issues.iter().any(|i| i.contains("docker.sock")));
}

#[test]
fn validate_docker_policy_rejects_root_workspace() {
    let mut policy = test_policy();
    policy.workspace_root = PathBuf::from("/");
    let issues = validate_docker_policy(&policy).unwrap_err();
    assert!(issues.iter().any(|i| i.contains("dangerous path")));
}

#[test]
fn validate_docker_policy_multiple_issues() {
    let mut policy = test_policy();
    policy.workspace_root = PathBuf::from("/etc");
    policy.read_only_mounts = vec![PathBuf::from("/proc")];
    policy.docker_overrides = Some(DockerOverrides {
        network: Some("host".into()),
        ..DockerOverrides::default()
    });
    let issues = validate_docker_policy(&policy).unwrap_err();
    assert!(issues.len() >= 3);
}

#[tokio::test]
async fn docker_backend_handle_reports_status() {
    let handle = docker_backend_handle().await;
    assert_eq!(handle.kind, SandboxBackendKind::Docker);
    // Status depends on whether Docker is installed in the test env.
    assert!(handle.status == SandboxStatus::Ready || handle.status == SandboxStatus::Error);
}
