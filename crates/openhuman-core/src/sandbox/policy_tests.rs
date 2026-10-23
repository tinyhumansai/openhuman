use super::*;

fn policy(sandbox: &SandboxConfig, runtime: &RuntimeConfig, remote: bool) -> SandboxPolicy {
    spawn_policy(
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
    let policy = policy(&SandboxConfig::default(), &RuntimeConfig::default(), false);
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
        policy(&sandbox(SandboxBackend::Landlock), &runtime, false).backend,
        SandboxBackendKind::Local
    );
    assert_eq!(
        policy(&sandbox(SandboxBackend::Docker), &runtime, false).backend,
        SandboxBackendKind::Docker
    );
    for backend in [SandboxBackend::Auto, SandboxBackend::None] {
        assert_eq!(
            policy(&sandbox(backend), &runtime, false).backend,
            SandboxBackendKind::None
        );
    }
}

#[test]
fn a_remote_session_and_the_docker_runtime_ask_for_a_container() {
    let remote = policy(&SandboxConfig::default(), &RuntimeConfig::default(), true);
    assert_eq!(remote.backend, SandboxBackendKind::Docker);
    assert!(!remote.allow_network);
    let runtime = policy(&SandboxConfig::default(), &docker_runtime(), false);
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
        policy(&sandbox, &docker_runtime(), true).backend,
        SandboxBackendKind::Local
    );
}

#[test]
fn the_container_settings_travel_whichever_executor_is_chosen() {
    let policy = policy(&SandboxConfig::default(), &RuntimeConfig::default(), false);
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
    let policy_for = |runtime: &RuntimeConfig| policy(&SandboxConfig::default(), runtime, false);
    assert!(!policy_for(&runtime).mount_workspace);
    runtime.kind = "native".into();
    assert!(
        policy_for(&runtime).mount_workspace,
        "the setting means nothing outside the docker runtime"
    );
}

#[test]
fn the_loaded_configuration_fills_in_a_backend_the_caller_did_not_name() {
    let mut config = Config::default();
    config.security.sandbox.backend = SandboxBackend::Docker;
    let unnamed = policy(&SandboxConfig::default(), &RuntimeConfig::default(), false);
    assert_eq!(
        with_configured_backend(&unnamed, &config).backend,
        SandboxBackendKind::Docker
    );
}

#[test]
fn a_backend_the_caller_named_is_never_replaced_by_the_configuration() {
    let mut config = Config::default();
    config.security.sandbox.backend = SandboxBackend::Landlock;
    let mut named = policy(&SandboxConfig::default(), &RuntimeConfig::default(), false);
    named.backend = SandboxBackendKind::Docker;
    assert_eq!(
        with_configured_backend(&named, &config).backend,
        SandboxBackendKind::Docker
    );
}

#[test]
fn the_configured_docker_runtime_carries_its_workspace_setting() {
    let mut config = Config::default();
    config.runtime = docker_runtime();
    config.runtime.docker.mount_workspace = false;
    let unnamed = policy(&SandboxConfig::default(), &RuntimeConfig::default(), false);
    let filled = with_configured_backend(&unnamed, &config);
    assert_eq!(filled.backend, SandboxBackendKind::Docker);
    assert!(!filled.mount_workspace);
}
