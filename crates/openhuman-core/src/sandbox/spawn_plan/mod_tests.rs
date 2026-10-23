use super::*;
use crate::sandbox::types::DockerOverrides;
use std::path::PathBuf;

fn policy(backend: SandboxBackendKind) -> SandboxPolicy {
    SandboxPolicy {
        backend,
        workspace_root: PathBuf::from("/work/action"),
        state_dir: PathBuf::from("/state"),
        read_only_mounts: vec![PathBuf::from("/opt")],
        read_write_mounts: vec![PathBuf::from("/home/u/.cargo/registry")],
        allow_network: false,
        env_passthrough: vec![],
        docker_overrides: None,
        mount_workspace: true,
    }
}

fn with_overrides(cpu: Option<f64>, memory_mb: Option<u64>) -> SandboxPolicy {
    SandboxPolicy {
        docker_overrides: Some(DockerOverrides {
            cpu_limit: cpu,
            memory_limit_mb: memory_mb,
            ..DockerOverrides::default()
        }),
        ..policy(SandboxBackendKind::Docker)
    }
}

#[test]
fn bounds_come_from_the_docker_overrides_and_never_loosen() {
    assert_eq!(
        limits(&policy(SandboxBackendKind::Local)),
        ResourceLimits2::default()
    );
    let bounded = limits(&with_overrides(Some(1.5), Some(512)));
    assert_eq!(bounded.cpu_millicores, Some(1500));
    assert_eq!(bounded.memory_bytes, Some(512 * 1024 * 1024));
    assert_eq!(bounded.cpu_time_ms, None);
    // A fraction rounds up to the next millicore rather than down to a looser bound.
    assert_eq!(
        limits(&with_overrides(Some(0.0004), None)).cpu_millicores,
        Some(1)
    );
    // Absent, zero and non-finite bounds are not bounds.
    for cpu in [
        None,
        Some(0.0),
        Some(-1.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ] {
        assert_eq!(
            limits(&with_overrides(cpu, None)).cpu_millicores,
            None,
            "{cpu:?}"
        );
    }
    assert_eq!(limits(&with_overrides(None, Some(0))).memory_bytes, None);
    assert_eq!(
        limits(&with_overrides(None, Some(u64::MAX))).memory_bytes,
        None
    );
}

#[test]
fn a_spawn_asks_for_its_workspace_and_every_mount_the_policy_names() {
    for backend in [
        SandboxBackendKind::None,
        SandboxBackendKind::Local,
        SandboxBackendKind::Docker,
    ] {
        let asked = grants(&policy(backend)).expect("grants");
        let described: Vec<_> = asked.iter().map(|g| (g.path.as_str(), g.write)).collect();
        assert_eq!(
            described,
            [
                ("/home/u/.cargo/registry", true),
                ("/opt", false),
                ("/work/action", true),
            ],
            "{backend:?}"
        );
    }
}

#[test]
fn a_workspace_less_spawn_asks_for_no_writable_root() {
    let mut container = policy(SandboxBackendKind::Docker);
    container.mount_workspace = false;
    container.read_write_mounts.clear();
    let asked = grants(&container).expect("grants");
    assert!(asked.iter().all(|grant| !grant.write), "{asked:?}");
    let options = options(&container, false);
    assert!(options.no_workspace);
}

#[test]
fn a_named_executor_is_required_and_an_unnamed_one_leaves_the_choice_to_the_module() {
    let named = |backend| options(&policy(backend), false);
    let none = named(SandboxBackendKind::None);
    assert_eq!(none.preferred_backend, None);
    assert!(!none.require_preferred_backend);
    let local = named(SandboxBackendKind::Local);
    assert_eq!(local.preferred_backend, Some(SandboxBackend::Jail));
    assert!(local.require_preferred_backend);
    let docker = named(SandboxBackendKind::Docker);
    assert_eq!(docker.preferred_backend, Some(SandboxBackend::Docker));
    assert!(docker.require_preferred_backend);
    assert!(!docker.no_workspace && !docker.host_run);
}

#[test]
fn a_host_run_request_sets_only_the_host_run_option_for_the_module_to_judge() {
    assert!(options(&policy(SandboxBackendKind::None), true).host_run);
}

#[test]
fn container_bounds_are_requested_only_when_a_container_is_required() {
    // A bound only a container enforces would make the native jail ineligible for a
    // spawn that merely allows either.
    let mut unnamed = with_overrides(Some(2.0), Some(256));
    unnamed.backend = SandboxBackendKind::None;
    assert_eq!(limits(&unnamed), ResourceLimits2::default());
    let mut local = unnamed.clone();
    local.backend = SandboxBackendKind::Local;
    assert_eq!(limits(&local), ResourceLimits2::default());
}

#[test]
fn refusals_name_the_denial_and_never_the_operation() {
    let refused = PlanError::Refused(SandboxDenial2::Approval);
    assert!(refused.to_string().contains("Approval"));
    let unavailable = PlanError::Unavailable("no module".into());
    assert!(unavailable.to_string().contains("no module"));
    assert_ne!(refused, unavailable);
}
