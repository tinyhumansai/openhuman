use super::*;
use tinysecurity_bus::{PathGrant, ResourceLimits2, SubjectScope};

fn artifact(backend: SandboxBackend) -> SandboxArtifact2 {
    SandboxArtifact2 {
        host_run: false,
        no_workspace: false,
        minimum_isolation: IsolationLevel2::Kernel,
        plan_id: "plan".into(),
        approval_reservation_id: None,
        fingerprint: "fp".into(),
        scope_id: "scope".into(),
        subject: SubjectScope {
            user: "u".into(),
            workspace: "w".into(),
            agent: "a".into(),
        },
        source_id: "source".into(),
        provenance: "prov".into(),
        call_id: "call".into(),
        generation: 1,
        expires_at_ms: 1,
        executor_id: "exec".into(),
        capability_id: "cap".into(),
        backend,
        grants: vec![
            PathGrant {
                path: "/work".into(),
                write: true,
            },
            PathGrant {
                path: "/toolchain".into(),
                write: false,
            },
        ],
        network: PlannedNetwork::None,
        proxy_id: None,
        limits: ResourceLimits2::default(),
        constraints: vec![SandboxConstraint2::Filesystem, SandboxConstraint2::Network],
    }
}

#[test]
fn a_jail_plan_maps_grants_network_and_isolation_floor() {
    let plan = BoxPlan::from_artifact(&artifact(SandboxBackend::Jail), None).unwrap();
    assert_eq!(plan.executor, PlannedExecutor::NativeJail);
    assert_eq!(plan.jail.read_write, ["/work"]);
    assert_eq!(plan.jail.read_only, ["/toolchain"]);
    assert!(!plan.jail.allow_net);
    assert!(plan.jail.allow_subprocess);
    assert_eq!(plan.jail.enforcement, JailEnforcement::Enforced);
    assert!(plan.jail.require_process_isolation);
    assert_eq!(plan.network, NetworkPolicy::Denied);
    assert_eq!(plan.jail.resources, None);
}

#[test]
fn an_unbounded_plan_does_not_synthesize_native_resource_limits() {
    let plan = BoxPlan::from_artifact(&artifact(SandboxBackend::Jail), None).unwrap();
    assert!(
        plan.jail.resources.is_none(),
        "an absent bound must stay absent"
    );
}

#[test]
fn unsupported_planned_bounds_are_refused_instead_of_being_ignored() {
    let mut planned = artifact(SandboxBackend::Docker);
    planned.limits = ResourceLimits2 {
        cpu_time_ms: None,
        cpu_millicores: Some(500),
        memory_bytes: Some(64 << 20),
        pids_max: None,
        disk_bytes: None,
    };
    planned.constraints.push(SandboxConstraint2::Subprocesses);
    assert_eq!(
        BoxPlan::from_artifact(&planned, None),
        Err(PlanMappingError::CpuLimitUnenforceable)
    );
}

#[test]
fn native_memory_limits_are_refused_instead_of_being_dropped() {
    let mut planned = artifact(SandboxBackend::Jail);
    planned.limits.memory_bytes = Some(64 << 20);
    assert_eq!(
        BoxPlan::from_artifact(&planned, None),
        Err(PlanMappingError::NativeResourceLimitsUnenforceable)
    );
}

#[test]
fn a_disk_limit_is_refused_instead_of_silently_dropped() {
    let mut planned = artifact(SandboxBackend::Jail);
    planned.limits.disk_bytes = Some(1 << 30);
    assert_eq!(
        BoxPlan::from_artifact(&planned, None),
        Err(PlanMappingError::DiskLimitUnenforceable)
    );
}

#[test]
fn a_docker_pid_limit_is_refused_instead_of_silently_dropped() {
    let mut planned = artifact(SandboxBackend::Docker);
    planned.limits.pids_max = Some(64);
    assert_eq!(
        BoxPlan::from_artifact(&planned, None),
        Err(PlanMappingError::PidsLimitUnenforceable)
    );
}

#[test]
fn a_host_run_with_resource_limits_is_refused_instead_of_ignoring_them() {
    for limits in [
        ResourceLimits2 {
            cpu_millicores: Some(500),
            ..ResourceLimits2::default()
        },
        ResourceLimits2 {
            memory_bytes: Some(64 << 20),
            ..ResourceLimits2::default()
        },
        ResourceLimits2 {
            pids_max: Some(64),
            ..ResourceLimits2::default()
        },
    ] {
        let mut planned = artifact(SandboxBackend::Noop);
        planned.host_run = true;
        planned.minimum_isolation = IsolationLevel2::None;
        planned.grants.clear();
        planned.constraints.clear();
        planned.limits = limits;
        assert_eq!(
            BoxPlan::from_artifact(&planned, None),
            Err(PlanMappingError::HostResourceLimitUnenforceable)
        );
    }
}

#[test]
fn host_networking_opens_the_box_only_when_planned() {
    let mut planned = artifact(SandboxBackend::Jail);
    planned.network = PlannedNetwork::Host;
    let plan = BoxPlan::from_artifact(&planned, None).unwrap();
    assert_eq!(plan.network, NetworkPolicy::Open);
    assert!(plan.jail.allow_net);
}

#[test]
fn a_process_level_plan_carries_the_credential_store_floor() {
    let home = std::path::Path::new("/home/someone");
    let mut planned = artifact(SandboxBackend::Jail);
    planned.minimum_isolation = IsolationLevel2::Process;
    let plan = BoxPlan::from_artifact(&planned, Some(home)).unwrap();
    assert!(!plan.jail.require_process_isolation);
    assert_eq!(
        plan.jail.enforcement,
        JailEnforcement::Declared,
        "a process-level plan carries best-effort constraints (Seatbelt)"
    );
    assert!(plan
        .jail
        .deny_read
        .contains(&"/home/someone/.ssh".to_owned()));

    // A kernel-isolated jail is allow-list based and names nothing.
    planned.minimum_isolation = IsolationLevel2::Kernel;
    let plan = BoxPlan::from_artifact(&planned, Some(home)).unwrap();
    assert!(plan.jail.deny_read.is_empty());
}

#[test]
fn plans_tinybox_cannot_enforce_exactly_are_refused() {
    assert_eq!(
        BoxPlan::from_artifact(&artifact(SandboxBackend::Noop), None),
        Err(PlanMappingError::NoopBackend)
    );
    for backend in [SandboxBackend::Namespace, SandboxBackend::MicroVm] {
        assert_eq!(
            BoxPlan::from_artifact(&artifact(backend), None),
            Err(PlanMappingError::ExecutorNotWired)
        );
    }

    let mut proxied = artifact(SandboxBackend::Jail);
    proxied.network = PlannedNetwork::EgressProxy;
    assert_eq!(
        BoxPlan::from_artifact(&proxied, None),
        Err(PlanMappingError::EgressProxyUnavailable)
    );

    let mut cpu_time = artifact(SandboxBackend::Jail);
    cpu_time.limits.cpu_time_ms = Some(1000);
    assert_eq!(
        BoxPlan::from_artifact(&cpu_time, None),
        Err(PlanMappingError::CpuTimeUnenforceable)
    );

    let mut zero = artifact(SandboxBackend::Jail);
    zero.limits.memory_bytes = Some(0);
    assert_eq!(
        BoxPlan::from_artifact(&zero, None),
        Err(PlanMappingError::ZeroLimit)
    );

    let mut empty = artifact(SandboxBackend::Jail);
    empty.grants[0].path.clear();
    assert_eq!(
        BoxPlan::from_artifact(&empty, None),
        Err(PlanMappingError::InvalidGrant)
    );
}

fn host_run_artifact() -> SandboxArtifact2 {
    SandboxArtifact2 {
        minimum_isolation: IsolationLevel2::None,
        executor_id: "host".into(),
        capability_id: "host".into(),
        backend: SandboxBackend::Noop,
        grants: vec![],
        network: PlannedNetwork::Host,
        constraints: vec![],
        host_run: true,
        ..artifact(SandboxBackend::Noop)
    }
}

#[test]
fn the_modules_host_run_plan_maps_to_a_spawn_on_the_host() {
    let plan = BoxPlan::from_artifact(&host_run_artifact(), None).expect("maps");
    assert_eq!(plan.executor, PlannedExecutor::Host);
    assert!(plan.jail.read_only.is_empty() && plan.jail.read_write.is_empty());
    assert!(!plan.jail.require_process_isolation);
}

#[test]
fn a_noop_plan_that_is_not_a_host_run_is_still_refused() {
    let mut artifact = host_run_artifact();
    artifact.host_run = false;
    assert_eq!(
        BoxPlan::from_artifact(&artifact, None).unwrap_err(),
        PlanMappingError::NoopBackend
    );
}

#[test]
fn a_host_run_plan_that_claims_a_jail_is_malformed() {
    for tweak in [
        (|a: &mut SandboxArtifact2| a.backend = SandboxBackend::Jail) as fn(&mut SandboxArtifact2),
        |a| a.minimum_isolation = IsolationLevel2::Kernel,
        |a| {
            a.grants = vec![PathGrant {
                path: "/work".into(),
                write: true,
            }]
        },
        |a| a.constraints = vec![SandboxConstraint2::Filesystem],
        |a| a.no_workspace = true,
    ] {
        let mut artifact = host_run_artifact();
        tweak(&mut artifact);
        assert_eq!(
            BoxPlan::from_artifact(&artifact, None).unwrap_err(),
            PlanMappingError::MalformedHostRun
        );
    }
}

#[test]
fn a_plan_with_no_workspace_grant_does_not_mount_one() {
    let mut artifact = artifact(SandboxBackend::Docker);
    assert!(BoxPlan::from_artifact(&artifact, None).unwrap().workspace);
    artifact.no_workspace = true;
    assert!(!BoxPlan::from_artifact(&artifact, None).unwrap().workspace);
}
