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
fn docker_runtime_reports_capabilities() {
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

    assert_eq!(runtime.shell_flavor(), ShellFlavor::Posix);
    assert!(!runtime.hide_window());
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
