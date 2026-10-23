//! The executors the host declares to the module, taken from what TinyBox reports.
//!
//! The module trusts these declarations to be verified facts, never a backend name:
//! a constraint is `Enforced` only when TinyBox says it is, so a best-effort jail
//! (macOS Seatbelt) can never satisfy a hard requirement and an unavailable one is
//! declared unavailable. The capability id changes with every reported fact, so a
//! plan made against one state is refused once the state changes.

use tinysecurity_bus::{
    ConstraintEnforcement2, ConstraintSupport2, IsolationLevel2, SandboxBackend,
    SandboxConstraint2, SandboxExecutor2,
};

use crate::modules::tinybox::contract::JailStatus;

/// The jail executor's identity on the bus.
pub(crate) const JAIL_EXECUTOR: &str = "tinybox-jail";
/// The Docker executor's identity on the bus.
pub(crate) const DOCKER_EXECUTOR: &str = "tinybox-docker";

fn enforcement(reported: &str) -> ConstraintEnforcement2 {
    match reported.trim() {
        "enforced" => ConstraintEnforcement2::Enforced,
        "best effort" | "best_effort" => ConstraintEnforcement2::BestEffort,
        _ => ConstraintEnforcement2::Unsupported,
    }
}

fn isolation(reported: &str) -> IsolationLevel2 {
    match reported.trim() {
        "process" => IsolationLevel2::Process,
        "kernel" => IsolationLevel2::Kernel,
        "hardware" => IsolationLevel2::MicroVm,
        _ => IsolationLevel2::None,
    }
}

fn support(
    constraint: SandboxConstraint2,
    enforcement: ConstraintEnforcement2,
) -> ConstraintSupport2 {
    ConstraintSupport2 {
        constraint,
        enforcement,
    }
}

/// The native jail as TinyBox reports it. The jail enforces the filesystem, the
/// network and subprocess rules it declares; it has no resource bounds.
pub(crate) fn jail(status: &JailStatus) -> SandboxExecutor2 {
    SandboxExecutor2 {
        executor_id: JAIL_EXECUTOR.into(),
        capability_id: format!(
            "{}|{}|fs:{}|net:{}|sub:{}|untrusted:{}",
            status.backend,
            status.isolation,
            status.filesystem,
            status.network,
            status.subprocess,
            status.suitable_for_untrusted_code
        ),
        backend: SandboxBackend::Jail,
        available: status.available,
        isolation: isolation(&status.isolation),
        constraints: vec![
            support(
                SandboxConstraint2::Filesystem,
                enforcement(&status.filesystem),
            ),
            support(SandboxConstraint2::Network, enforcement(&status.network)),
            support(
                SandboxConstraint2::Subprocesses,
                enforcement(&status.subprocess),
            ),
        ],
    }
}

/// A one-shot container. The daemon being reachable is all TinyBox reports; the
/// container itself mounts only the workspace and the declared read-only paths,
/// joins the configured network, and carries the memory and CPU bounds it is run with.
pub(crate) fn docker(available: bool) -> SandboxExecutor2 {
    let enforced = ConstraintEnforcement2::Enforced;
    SandboxExecutor2 {
        executor_id: DOCKER_EXECUTOR.into(),
        capability_id: "tinybox-docker|run|fs:mounts|net:network|mem|cpu".into(),
        backend: SandboxBackend::Docker,
        available,
        isolation: IsolationLevel2::Kernel,
        constraints: vec![
            support(SandboxConstraint2::Filesystem, enforced),
            support(SandboxConstraint2::Network, enforced),
            support(SandboxConstraint2::Memory, enforced),
            support(SandboxConstraint2::CpuMillicores, enforced),
        ],
    }
}

/// The native jail when TinyBox could not report on it: declared unavailable, so the
/// module never plans onto it.
pub(crate) fn jail_unavailable() -> SandboxExecutor2 {
    SandboxExecutor2 {
        executor_id: JAIL_EXECUTOR.into(),
        capability_id: "tinybox-jail|unavailable".into(),
        backend: SandboxBackend::Jail,
        available: false,
        isolation: IsolationLevel2::None,
        constraints: Vec::new(),
    }
}

#[cfg(test)]
#[path = "executors_tests.rs"]
mod tests;
