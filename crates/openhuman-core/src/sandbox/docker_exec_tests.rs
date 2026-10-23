//! Docker sandbox request mapping. TinyBox owns daemon status and cleanup.
#![cfg(all(unix, feature = "modules"))]

use super::*;
#[cfg(feature = "modules")]
use crate::sandbox::types::DockerOverrides;
#[cfg(feature = "modules")]
use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[cfg(feature = "modules")]
use std::time::Duration;

#[cfg(feature = "modules")]
fn policy(workspace: &Path) -> SandboxPolicy {
    SandboxPolicy {
        backend: SandboxBackendKind::Docker,
        workspace_root: workspace.to_path_buf(),
        state_dir: PathBuf::from("/tmp/state"),
        read_only_mounts: vec![],
        read_write_mounts: vec![],
        allow_network: false,
        env_passthrough: vec![],
        docker_overrides: None,
        mount_workspace: true,
    }
}

#[cfg(feature = "modules")]
fn request(command: &str, timeout: Duration) -> SandboxExecRequest {
    SandboxExecRequest {
        command: command.into(),
        working_dir: PathBuf::from("/workspace"),
        env: Default::default(),
        timeout,
    }
}

#[cfg(feature = "modules")]
#[test]
fn module_request_preserves_policy_and_only_forwards_allowed_environment() {
    use crate::modules::tinybox::contract::DockerRunRequest;

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let readonly = temp.path().join("readonly");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&readonly).unwrap();

    let mut sandbox_policy = policy(&workspace);
    sandbox_policy.read_only_mounts.push(readonly.clone());
    sandbox_policy.env_passthrough = vec!["OH_TINYBOX_ALLOWED_ENV".to_owned()];
    sandbox_policy.docker_overrides = Some(DockerOverrides {
        image: Some("debian:12".to_owned()),
        network: Some("bridge".to_owned()),
        memory_limit_mb: Some(2048),
        cpu_limit: Some(2.5),
        read_only_rootfs: Some(false),
        extra_caps_drop: vec!["NET_RAW".to_owned()],
    });

    let _environment = crate::config::TEST_ENV_LOCK.blocking_lock();
    std::env::set_var("OH_TINYBOX_ALLOWED_ENV", "host value");
    std::env::set_var("OH_TINYBOX_UNLISTED_ENV", "must not be forwarded");
    let mut exec_request = request("make test", Duration::from_millis(1250));
    exec_request.env.insert(
        OsString::from("OH_TINYBOX_ALLOWED_ENV"),
        OsString::from("request override"),
    );
    exec_request.env.insert(
        OsString::from("REQUEST_ENV"),
        OsString::from("request value"),
    );

    let actual = build_docker_request(&sandbox_policy, &exec_request).unwrap();
    std::env::remove_var("OH_TINYBOX_ALLOWED_ENV");
    std::env::remove_var("OH_TINYBOX_UNLISTED_ENV");
    assert_eq!(
        actual,
        DockerRunRequest {
            workspace: workspace.to_string_lossy().into_owned(),
            image: Some("debian:12".to_owned()),
            command: "make test".to_owned(),
            login_shell: false,
            network: Some("bridge".to_owned()),
            memory_limit_mb: Some(2048),
            cpus: Some(2.5),
            read_only_rootfs: Some(false),
            read_only_mounts: vec![readonly.to_string_lossy().into_owned()],
            extra_cap_drops: vec!["NET_RAW".to_owned()],
            env: [
                (
                    "OH_TINYBOX_ALLOWED_ENV".to_owned(),
                    "request override".to_owned()
                ),
                ("REQUEST_ENV".to_owned(), "request value".to_owned()),
            ]
            .into(),
            label: Some("openhuman.sandbox=true".to_owned()),
            timeout_ms: 1250,
        }
    );
}

#[cfg(feature = "modules")]
#[test]
fn module_request_rounds_small_timeouts_up_and_rejects_unbounded_timeouts() {
    use crate::modules::tinybox::contract::MAX_DOCKER_RUN_TIMEOUT_MS;

    let temp = tempfile::tempdir().unwrap();
    let mut exec_request = request("true", Duration::from_nanos(1));
    let short = build_docker_request(&policy(temp.path()), &exec_request).unwrap();
    assert_eq!(short.timeout_ms, 1);

    exec_request.timeout = Duration::from_millis(MAX_DOCKER_RUN_TIMEOUT_MS + 1);
    let error = build_docker_request(&policy(temp.path()), &exec_request)
        .expect_err("unbounded request must not bypass the contract limit");
    assert!(error
        .to_string()
        .contains("timeout exceeds the supported limit"));
}

#[cfg(all(feature = "modules", unix))]
#[test]
fn module_request_rejects_non_utf8_host_paths_without_lossy_remapping() {
    use std::os::unix::ffi::OsStringExt;

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(OsString::from_vec(vec![b'w', 0xff]));
    let error = build_docker_request(&policy(&path), &request("true", Duration::from_secs(1)))
        .expect_err("the module contract cannot represent an arbitrary OS path");
    assert!(error.to_string().contains("workspace path is not UTF-8"));
}
