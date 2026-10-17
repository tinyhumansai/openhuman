use super::*;
use crate::config::{DockerRuntimeConfig, RuntimeConfig};

#[test]
fn native_runtime_reports_capabilities_and_shell_flavor() {
    let runtime = NativeRuntime::new();
    assert_eq!(runtime.name(), "native");
    assert!(runtime.has_shell_access());
    assert!(runtime.has_filesystem_access());
    assert!(runtime.supports_long_running());
    assert_eq!(runtime.memory_budget(), 0);
    assert!(runtime.storage_path().ends_with("openhuman/runtime"));
    assert!(!runtime.hide_window());
    assert_eq!(runtime.shell_flavor(), host_shell_flavor());
}

#[test]
#[cfg(feature = "modules")]
fn docker_runtime_builds_expected_flags() {
    let runtime = DockerRuntime::new(DockerRuntimeConfig {
        image: "alpine:3.20".into(),
        network: "host".into(),
        mount_workspace: true,
        read_only_rootfs: true,
        memory_limit_mb: Some(512),
        cpu_limit: Some(1.5),
        ..DockerRuntimeConfig::default()
    });
    assert_eq!(runtime.name(), "docker");
    assert!(runtime.has_shell_access());
    assert!(runtime.has_filesystem_access());
    assert!(!runtime.supports_long_running());
    assert_eq!(runtime.memory_budget(), 512);
    assert!(runtime.storage_path().ends_with("openhuman/runtime/docker"));

    let tempdir = tempfile::tempdir().unwrap();
    let (request, mount_workspace) = runtime
        .build_docker_request("pwd", tempdir.path())
        .expect("Docker runtime should use the TinyBox request path");
    assert!(mount_workspace);
    assert_eq!(
        request.workspace,
        tempdir.path().canonicalize().unwrap().to_string_lossy()
    );
    assert_eq!(request.image.as_deref(), Some("alpine:3.20"));
    assert_eq!(request.network.as_deref(), Some("host"));
    assert_eq!(request.memory_limit_mb, Some(512));
    assert_eq!(request.cpus, Some(1.5));
    assert_eq!(request.read_only_rootfs, Some(true));
    assert!(request.read_only_mounts.is_empty());
    assert!(request.extra_cap_drops.is_empty());
    assert_eq!(request.command, "pwd");
    assert!(request.login_shell, "legacy DockerRuntime used sh -lc");
    assert_eq!(request.label.as_deref(), Some("openhuman.runtime=true"));
    assert_eq!(runtime.shell_flavor(), ShellFlavor::Posix);
    assert!(!runtime.hide_window());
}

#[cfg(feature = "modules")]
#[test]
fn docker_runtime_without_workspace_uses_unmounted_module_request() {
    let runtime = DockerRuntime::new(DockerRuntimeConfig {
        mount_workspace: false,
        ..DockerRuntimeConfig::default()
    });
    let tempdir = tempfile::tempdir().unwrap();
    let (request, mount_workspace) = runtime
        .build_docker_request("printf ok", tempdir.path())
        .unwrap();
    assert!(!mount_workspace);
    assert!(request.workspace.is_empty());
    assert_eq!(request.command, "printf ok");
    assert!(request.login_shell);
}

#[test]
fn create_runtime_supports_native_and_docker_and_rejects_unknown() {
    let native = create_runtime(&RuntimeConfig::default(), false).unwrap();
    assert_eq!(native.name(), "native");

    let docker = create_runtime(
        &RuntimeConfig {
            kind: "docker".into(),
            docker: DockerRuntimeConfig::default(),
            ..RuntimeConfig::default()
        },
        false,
    )
    .unwrap();
    assert_eq!(docker.name(), "docker");

    let err = create_runtime(
        &RuntimeConfig {
            kind: "vm".into(),
            ..RuntimeConfig::default()
        },
        false,
    )
    .err()
    .unwrap();
    assert!(err.to_string().contains("Unsupported runtime kind: vm"));
}

/// `[shell] hide_window` plumbs through `create_runtime` into the native
/// adapter, which hands it to TinyBox on the native shell request.
#[test]
fn hide_window_reaches_the_native_runtime_only() {
    assert!(create_runtime(&RuntimeConfig::default(), true)
        .unwrap()
        .hide_window());
    assert!(!create_runtime(&RuntimeConfig::default(), false)
        .unwrap()
        .hide_window());
    assert!(NativeRuntime::with_hide_window(true).hide_window());
    assert!(!NativeRuntime::new().hide_window());
    let docker = create_runtime(
        &RuntimeConfig {
            kind: "docker".into(),
            ..RuntimeConfig::default()
        },
        true,
    )
    .unwrap();
    assert!(
        !docker.hide_window(),
        "docker runs in a container, not a console"
    );
}

/// The shell flavor told to the model is the one TinyBox's `SelectShell`
/// returns for this machine, and is the contract's own type.
#[test]
fn host_shell_flavor_matches_the_platform_the_module_selects() {
    let expected = if cfg!(windows) {
        tinybox_bus::ShellFlavor::Cmd
    } else {
        tinybox_bus::ShellFlavor::Posix
    };
    assert_eq!(host_shell_flavor(), expected);
}
