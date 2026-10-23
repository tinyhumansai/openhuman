use super::*;

fn status(isolation: &str, fs: &str, net: &str, sub: &str, available: bool) -> JailStatus {
    JailStatus {
        backend: "landlock".into(),
        available,
        isolation: isolation.into(),
        suitable_for_untrusted_code: isolation == "kernel",
        filesystem: fs.into(),
        network: net.into(),
        subprocess: sub.into(),
    }
}

fn enforcement_of(
    executor: &SandboxExecutor2,
    constraint: SandboxConstraint2,
) -> Option<ConstraintEnforcement2> {
    executor
        .constraints
        .iter()
        .find(|c| c.constraint == constraint)
        .map(|c| c.enforcement)
}

#[test]
fn a_kernel_jail_declares_exactly_what_tinybox_reports() {
    let executor = jail(&status("kernel", "enforced", "enforced", "enforced", true));
    assert_eq!(executor.executor_id, JAIL_EXECUTOR);
    assert_eq!(executor.backend, SandboxBackend::Jail);
    assert!(executor.available);
    assert_eq!(executor.isolation, IsolationLevel2::Kernel);
    for constraint in [
        SandboxConstraint2::Filesystem,
        SandboxConstraint2::Network,
        SandboxConstraint2::Subprocesses,
    ] {
        assert_eq!(
            enforcement_of(&executor, constraint),
            Some(ConstraintEnforcement2::Enforced)
        );
    }
    // A jail has no resource bounds to claim.
    for constraint in [
        SandboxConstraint2::Memory,
        SandboxConstraint2::CpuMillicores,
        SandboxConstraint2::Processes,
        SandboxConstraint2::Disk,
    ] {
        assert_eq!(enforcement_of(&executor, constraint), None);
    }
}

#[test]
fn a_best_effort_jail_never_claims_enforcement() {
    let seatbelt = jail(&status(
        "process",
        "best effort",
        "best effort",
        "best effort",
        true,
    ));
    assert_eq!(seatbelt.isolation, IsolationLevel2::Process);
    for constraint in [
        SandboxConstraint2::Filesystem,
        SandboxConstraint2::Network,
        SandboxConstraint2::Subprocesses,
    ] {
        assert_eq!(
            enforcement_of(&seatbelt, constraint),
            Some(ConstraintEnforcement2::BestEffort)
        );
    }
}

#[test]
fn unknown_words_are_never_read_as_stronger_than_unsupported() {
    let odd = jail(&status("quantum", "mostly", "", "ENFORCED", false));
    assert!(!odd.available);
    assert_eq!(odd.isolation, IsolationLevel2::None);
    for constraint in [
        SandboxConstraint2::Filesystem,
        SandboxConstraint2::Network,
        SandboxConstraint2::Subprocesses,
    ] {
        assert_eq!(
            enforcement_of(&odd, constraint),
            Some(ConstraintEnforcement2::Unsupported)
        );
    }
    assert_eq!(
        jail(&status(
            "hardware", "enforced", "enforced", "enforced", true
        ))
        .isolation,
        IsolationLevel2::MicroVm
    );
}

#[test]
fn the_capability_id_changes_with_every_reported_fact() {
    let base = jail(&status("kernel", "enforced", "enforced", "enforced", true));
    let weaker = [
        status("process", "enforced", "enforced", "enforced", true),
        status("kernel", "best effort", "enforced", "enforced", true),
        status("kernel", "enforced", "unsupported", "enforced", true),
        status("kernel", "enforced", "enforced", "best effort", true),
    ];
    for changed in weaker {
        assert_ne!(jail(&changed).capability_id, base.capability_id);
    }
    let mut other_backend = status("kernel", "enforced", "enforced", "enforced", true);
    other_backend.backend = "appcontainer".into();
    assert_ne!(jail(&other_backend).capability_id, base.capability_id);
}

#[test]
fn docker_claims_only_what_a_run_can_apply() {
    let executor = docker(true);
    assert_eq!(executor.executor_id, DOCKER_EXECUTOR);
    assert_eq!(executor.backend, SandboxBackend::Docker);
    assert_eq!(executor.isolation, IsolationLevel2::Kernel);
    assert!(executor.available);
    assert!(!docker(false).available);
    for constraint in [
        SandboxConstraint2::Filesystem,
        SandboxConstraint2::Network,
        SandboxConstraint2::Memory,
        SandboxConstraint2::CpuMillicores,
    ] {
        assert_eq!(
            enforcement_of(&executor, constraint),
            Some(ConstraintEnforcement2::Enforced)
        );
    }
    for constraint in [
        SandboxConstraint2::Subprocesses,
        SandboxConstraint2::Processes,
        SandboxConstraint2::Disk,
        SandboxConstraint2::CpuTime,
    ] {
        assert_eq!(enforcement_of(&executor, constraint), None);
    }
}

#[test]
fn a_jail_that_could_not_be_verified_is_declared_unavailable() {
    let executor = jail_unavailable();
    assert_eq!(executor.executor_id, JAIL_EXECUTOR);
    assert!(!executor.available);
    assert_eq!(executor.isolation, IsolationLevel2::None);
    assert!(executor.constraints.is_empty());
}
