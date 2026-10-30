use super::*;

fn policy(
    mode: SandboxMode,
    sandbox: &SandboxConfig,
    runtime: &RuntimeConfig,
    remote: bool,
) -> SandboxPolicy {
    route_for_mode(
        mode,
        Path::new("/tmp/action"),
        Path::new("/tmp/state"),
        sandbox,
        runtime,
        remote,
    )
}

fn docker_runtime() -> RuntimeConfig {
    RuntimeConfig {
        kind: "docker".into(),
        ..RuntimeConfig::default()
    }
}

#[test]
fn a_default_configuration_names_no_executor() {
    let policy = policy(
        SandboxMode::None,
        &SandboxConfig::default(),
        &RuntimeConfig::default(),
        false,
    );
    assert_eq!(policy.backend, SandboxBackendKind::None);
    assert!(policy.allow_network);
    assert!(policy.mount_workspace);
    assert!(policy.read_only_mounts.is_empty() && policy.read_write_mounts.is_empty());
}

#[test]
fn a_configured_backend_becomes_the_requested_executor() {
    let sandbox = |backend| SandboxConfig {
        backend,
        ..SandboxConfig::default()
    };
    let runtime = RuntimeConfig::default();
    assert_eq!(
        policy(
            SandboxMode::None,
            &sandbox(SandboxBackend::Landlock),
            &runtime,
            false
        )
        .backend,
        SandboxBackendKind::Local
    );
    assert_eq!(
        policy(
            SandboxMode::None,
            &sandbox(SandboxBackend::Docker),
            &runtime,
            false
        )
        .backend,
        SandboxBackendKind::Docker
    );
    for backend in [SandboxBackend::Auto, SandboxBackend::None] {
        assert_eq!(
            policy(SandboxMode::None, &sandbox(backend), &runtime, false).backend,
            SandboxBackendKind::None
        );
    }
}

#[test]
fn a_remote_session_and_the_docker_runtime_ask_for_a_container() {
    let remote = policy(
        SandboxMode::Sandboxed,
        &SandboxConfig::default(),
        &RuntimeConfig::default(),
        true,
    );
    assert_eq!(remote.backend, SandboxBackendKind::Docker);
    assert!(!remote.allow_network);
    let runtime = policy(
        SandboxMode::Sandboxed,
        &SandboxConfig::default(),
        &docker_runtime(),
        false,
    );
    assert_eq!(runtime.backend, SandboxBackendKind::Docker);
    assert!(runtime.allow_network);
}

#[test]
fn an_explicit_jail_overrides_the_container_defaults() {
    let sandbox = SandboxConfig {
        backend: SandboxBackend::Landlock,
        ..SandboxConfig::default()
    };
    assert_eq!(
        policy(SandboxMode::Sandboxed, &sandbox, &docker_runtime(), true).backend,
        SandboxBackendKind::Local
    );
}

#[test]
fn the_container_settings_travel_whichever_executor_is_chosen() {
    let policy = policy(
        SandboxMode::None,
        &SandboxConfig::default(),
        &RuntimeConfig::default(),
        false,
    );
    let overrides = policy.docker_overrides.expect("container settings");
    let docker = RuntimeConfig::default().docker;
    assert_eq!(overrides.image, Some(docker.image));
    assert_eq!(overrides.network, Some(docker.network));
    assert_eq!(overrides.read_only_rootfs, Some(docker.read_only_rootfs));
}

#[test]
fn a_workspace_less_container_is_asked_for_only_by_the_docker_runtime() {
    let mut runtime = docker_runtime();
    runtime.docker.mount_workspace = false;
    let policy_for = |runtime: &RuntimeConfig| {
        policy(SandboxMode::None, &SandboxConfig::default(), runtime, false)
    };
    assert!(!policy_for(&runtime).mount_workspace);
    runtime.kind = "native".into();
    assert!(
        policy_for(&runtime).mount_workspace,
        "the setting means nothing outside the docker runtime"
    );
}

#[test]
fn an_explicit_backend_is_routed_as_a_module_preference() {
    let mut config = Config::default();
    config.sandbox.backend = SandboxBackend::Docker;
    assert_eq!(
        route_for_mode(
            SandboxMode::None,
            &config.action_dir,
            &config.workspace_dir,
            &config.sandbox,
            &config.runtime,
            false
        )
        .backend,
        SandboxBackendKind::Docker
    );
}

#[test]
fn sandboxed_mode_prefers_the_native_jail_on_a_native_host() {
    assert_eq!(
        policy(
            SandboxMode::Sandboxed,
            &SandboxConfig::default(),
            &RuntimeConfig::default(),
            false
        )
        .backend,
        SandboxBackendKind::Local
    );
}

#[test]
fn the_configured_docker_runtime_carries_its_workspace_setting() {
    let mut config = Config::default();
    config.runtime = docker_runtime();
    config.runtime.docker.mount_workspace = false;
    let filled = route_for_mode(
        SandboxMode::None,
        &config.action_dir,
        &config.workspace_dir,
        &config.sandbox,
        &config.runtime,
        false,
    );
    assert_eq!(filled.backend, SandboxBackendKind::Docker);
    assert!(!filled.mount_workspace);
}

#[test]
fn an_openshell_selection_routes_to_openshell_without_docker_network_or_host_mounts() {
    let sandbox = SandboxConfig {
        backend: SandboxBackend::OpenShell,
        ..SandboxConfig::default()
    };
    for (mode, runtime, remote) in [
        (SandboxMode::Sandboxed, docker_runtime(), false),
        (SandboxMode::Sandboxed, RuntimeConfig::default(), true),
        (SandboxMode::None, RuntimeConfig::default(), false),
    ] {
        let routed = policy(mode, &sandbox, &runtime, remote);
        assert_eq!(routed.backend, SandboxBackendKind::OpenShell);
        assert!(routed.docker_overrides.is_none());
        assert!(!routed.allow_network);
        assert!(!routed.mount_workspace);
        assert!(routed.read_only_mounts.is_empty() && routed.read_write_mounts.is_empty());
    }
}

#[test]
fn an_explicit_backend_selection_requires_the_sandbox() {
    for backend in [
        SandboxBackend::Landlock,
        SandboxBackend::Firejail,
        SandboxBackend::Bubblewrap,
        SandboxBackend::Docker,
        SandboxBackend::OpenShell,
    ] {
        assert!(backend_requires_sandbox(&backend), "{backend:?}");
    }
    assert!(!backend_requires_sandbox(&SandboxBackend::Auto));
    assert!(!backend_requires_sandbox(&SandboxBackend::None));
}
