use super::*;
#[cfg(feature = "modules")]
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
        mount_workspace: true,
    }
}

#[cfg(feature = "modules")]
#[test]
fn policy_request_carries_network_workspace_and_read_only_mounts() {
    let mut policy = test_policy();
    policy.workspace_root = PathBuf::from("/");
    policy.read_only_mounts = vec![PathBuf::from("/etc"), PathBuf::from("/usr/lib")];
    policy.docker_overrides = Some(DockerOverrides {
        network: Some("host".into()),
        ..DockerOverrides::default()
    });
    let request = policy_request(&policy);
    assert_eq!(request.network.as_deref(), Some("host"));
    assert_eq!(request.workspace.as_deref(), Some("/"));
    assert_eq!(request.read_only_mounts, ["/etc", "/usr/lib"]);
}

#[cfg(feature = "modules")]
#[test]
fn policy_request_without_overrides_leaves_the_isolated_default() {
    let request = policy_request(&test_policy());
    assert_eq!(request.network, None);
    assert_eq!(request.workspace.as_deref(), Some("/tmp/test-workspace"));
    assert!(request.read_only_mounts.is_empty());
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn validation_never_reports_safe_when_tinybox_is_unavailable() {
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;
    let verdict = validate_docker_policy(&config, &test_policy()).await;
    assert!(
        verdict.is_err(),
        "no module means no verdict, not a clean one"
    );
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

/// Container-side validation (host network, dangerous mounts) is enforced by
/// the TinyBox module since contract 1.17; the host adapter must forward the
/// policy unchanged so the module can refuse it.
#[cfg(feature = "modules")]
#[test]
fn host_network_and_dangerous_mounts_reach_the_module_unfiltered() {
    let mut policy = test_policy();
    policy.workspace_root = PathBuf::from("/etc");
    policy.read_only_mounts = vec![PathBuf::from("/var/run/docker.sock")];
    policy.docker_overrides = Some(DockerOverrides {
        network: Some("host".into()),
        ..DockerOverrides::default()
    });
    let request = SandboxExecRequest {
        command: "true".into(),
        working_dir: PathBuf::from("/workspace"),
        env: Default::default(),
        timeout: std::time::Duration::from_secs(1),
    };
    let run = build_docker_request(&policy, &request).expect("adapter forwards policy");
    assert_eq!(run.workspace, "/etc");
    assert_eq!(run.network.as_deref(), Some("host"));
    assert_eq!(
        run.read_only_mounts,
        vec!["/var/run/docker.sock".to_owned()]
    );
}
