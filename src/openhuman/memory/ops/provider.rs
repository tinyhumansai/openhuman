//! `memory.provider_status` — what is bound in the memory subsystem slot
//! (`docs/specs/plan-memory.md` §5, `docs/specs/kernel.md` §6 item 6).
//!
//! This is the **memory adapter for status**: it resolves the context's
//! [`MemoryBinding`], converts it into the kernel's generic
//! [`BoundDriver`](crate::core::subsystem::BoundDriver) vocabulary, probes the
//! driver's live health, and projects the result onto the wire shape
//! [`SubsystemStatus`]. `subsystems.status` renders the same value for the
//! `memory` slot — deliberately, so the frontend can read driver capabilities
//! from the memory namespace without knowing the kernel namespace exists.
//!
//! Nothing here is a mutation and nothing here binds: a status call must never
//! be the thing that constructs a driver. It does resolve the binding, which
//! *is* lazily constructing on first use — that is
//! [`CoreContext::memory_binding`]'s existing cached behaviour and identical
//! to what any other memory RPC would trigger.

use crate::core::runtime::context::CoreContext;
use crate::core::subsystem::{DriverHealth, SubsystemStatus};
use crate::openhuman::memory::binding::{to_driver_health, MemoryBinding};
use crate::rpc::RpcOutcome;

/// The status of the memory slot for the current dispatch context.
///
/// Infallible by design: an unresolvable binding (no workspace bound, e.g. a
/// pre-login core) is *reported*, not raised. A status surface that errors
/// exactly when something is wrong is the opposite of useful.
///
/// A standalone CLI subcommand (`openhuman subsystems status`,
/// `openhuman memory status`) never builds a [`CoreContext`] — the generic
/// namespace dispatcher runs without one — so this resolves the configured
/// workspace's binding directly via [`standalone_status`] instead of reporting
/// an unresolved row. In an RPC host a context is always ambient, so that path
/// is unaffected.
pub async fn memory_subsystem_status() -> SubsystemStatus {
    match CoreContext::current().map(|ctx| ctx.memory_binding()) {
        Some(Ok(binding)) => status_from_binding(&binding).await,
        Some(Err(err)) => unresolved_status(err),
        None => standalone_status().await,
    }
}

/// Resolve status from the on-disk config when no [`CoreContext`] is ambient
/// (a bare CLI invocation). Reads the configured workspace's binding the same
/// way `cli_capability::bound_memory_driver_for` does, so `openhuman subsystems
/// status` shows the same resolved row as the table and as an RPC with a live
/// context.
///
/// Never errors: on a config load or bind failure this reports an unresolved
/// row (with a reason) rather than refusing to render — mirroring the
/// capability gate's default-OPEN posture, where a status command that refuses
/// to run because it cannot read config is worse than one that shows the
/// unresolved row.
async fn standalone_status() -> SubsystemStatus {
    let config = match crate::openhuman::config::Config::load_or_init().await {
        Ok(config) => config,
        Err(err) => {
            log::debug!(
                "[memory:provider] standalone status: config unresolved ({err}); reporting unresolved"
            );
            return unresolved_status(format!("no core context; config load failed: {err}"));
        }
    };

    match crate::openhuman::memory::binding::for_workspace(
        &config.workspace_dir,
        &config.subsystems.memory,
    ) {
        Ok(binding) => status_from_binding(&binding).await,
        Err(err) => {
            log::debug!(
                "[memory:provider] standalone status: binding unresolved ({err}); reporting unresolved"
            );
            unresolved_status(format!("no core context; binding failed: {err}"))
        }
    }
}

/// Project one resolved binding. Separate from [`memory_subsystem_status`] so
/// the bound case is testable without standing up a [`CoreContext`].
pub async fn status_from_binding(binding: &MemoryBinding) -> SubsystemStatus {
    let bound = binding.to_bound_driver();
    let health = to_driver_health(binding.unguarded_provider().health().await);
    let last_error = binding
        .fallback()
        .map(|fallback| format!("{}: {}", fallback.configured_driver, fallback.reason));

    log::debug!(
        "[memory:provider] status driver='{}' class={} health={} capabilities=[{}] fallback={}",
        bound.id,
        bound.class,
        health.as_str(),
        bound.capabilities.iter().collect::<Vec<_>>().join(","),
        bound.is_fallback()
    );

    SubsystemStatus::from_bound_with_health(&bound, health).with_last_error(last_error)
}

/// The status reported when no driver could be resolved at all. Distinct from
/// a *fallback*: nothing was bound, so there is no driver id to name and no
/// capability set to advertise.
fn unresolved_status(reason: String) -> SubsystemStatus {
    log::debug!("[memory:provider] status unresolved: {reason}");
    SubsystemStatus {
        slot: crate::core::subsystem::SubsystemSlot::Memory
            .as_str()
            .to_string(),
        driver: String::new(),
        class: crate::core::subsystem::DriverClass::Null
            .as_str()
            .to_string(),
        health: DriverHealth::down(reason.clone()).as_str().to_string(),
        health_reason: Some(reason.clone()),
        contract_version: crate::core::subsystem::format_contract_version(
            crate::openhuman::memory::api::CONTRACT_VERSION,
        ),
        capabilities: Vec::new(),
        fell_back_from: None,
        last_error: Some(reason),
    }
}

/// RPC handler body for `memory.provider_status`.
pub async fn memory_provider_status() -> RpcOutcome<SubsystemStatus> {
    RpcOutcome::new(memory_subsystem_status().await, vec![])
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
