use super::*;
use crate::sandbox::types::DockerOverrides;
use std::path::PathBuf;

fn test_policy() -> SandboxPolicy {
    SandboxPolicy {
        backend: SandboxBackendKind::Docker,
        workspace_root: PathBuf::from("/tmp/test-workspace"),
        state_dir: PathBuf::from("/tmp/state"),
        read_only_mounts: vec![],
        read_write_mounts: vec![],
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

#[cfg(feature = "modules")]
#[test]
fn daemon_status_and_orphan_cleanup_use_bounded_tinybox_requests() {
    use crate::modules::tinybox::contract::DEFAULT_DOCKER_CONTROL_TIMEOUT_MS;

    assert_eq!(
        status_request(),
        crate::modules::tinybox::contract::DockerStatusRequest {
            timeout_ms: DEFAULT_DOCKER_CONTROL_TIMEOUT_MS,
        }
    );
    assert_eq!(
        cleanup_request(),
        crate::modules::tinybox::contract::CleanupDockerRequest {
            label: "openhuman.sandbox=true".to_owned(),
            timeout_ms: DEFAULT_DOCKER_CONTROL_TIMEOUT_MS,
        }
    );
}

#[test]
fn docker_backend_handle_maps_module_status_truthfully() {
    let available = docker_handle_for_status(true);
    assert_eq!(available.kind, SandboxBackendKind::Docker);
    assert_eq!(available.status, SandboxStatus::Ready);

    let unavailable = docker_handle_for_status(false);
    assert_eq!(unavailable.kind, SandboxBackendKind::Docker);
    assert_eq!(unavailable.status, SandboxStatus::Error);
}
