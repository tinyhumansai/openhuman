//! Host-owned keys for TinyAgents' parent-turn busy gate.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use tinyagents_tasks::{ParentBusyGate, ParentBusyLease, TenantThreadKey};

use super::background_completions;
use crate::core::runtime::tenant::{self, Tenant};

/// The calling task's profile, as the tenant its busy keys carry.
pub(super) fn caller(ctx: &crate::core::runtime::DispatchCtx) -> Tenant {
    tenant::isolated_unless_tenant(ctx.tenant(), "background_delivery").profile_only()
}

/// One upstream gate per profile. The gate owns leases and session aliases;
/// OpenHuman only supplies the tenant boundary and the session-to-thread map.
fn gates() -> &'static Mutex<HashMap<String, ParentBusyGate>> {
    static GATES: OnceLock<Mutex<HashMap<String, ParentBusyGate>>> = OnceLock::new();
    GATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn gate_for(tenant_key: &str) -> ParentBusyGate {
    gates()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(tenant_key.to_owned())
        .or_default()
        .clone()
}

fn tenant_key(ctx: &crate::core::runtime::DispatchCtx) -> String {
    tenant::profile_key_in(&caller(ctx), "")
}

/// A user turn in flight on a session for the calling profile, busy until
/// dropped. The upstream lease is the sole busy-state authority.
pub(crate) struct TurnBusy {
    _lease: ParentBusyLease,
}

impl TurnBusy {
    pub(crate) fn start(ctx: &crate::core::runtime::DispatchCtx, session_id: &str) -> Self {
        let tenant_key = tenant_key(ctx);
        let gate = gate_for(&tenant_key);
        let session = TenantThreadKey::new(&tenant_key, session_id);
        if let Some(thread_id) = background_completions::thread_for_session(ctx, session_id) {
            let thread = TenantThreadKey::new(&tenant_key, thread_id);
            let _ = gate.alias_session_to_thread(session.clone(), thread);
        }
        let lease = gate
            .enter(session)
            .expect("background_delivery parent busy gate poisoned");
        Self { _lease: lease }
    }

    /// A turn running on a channel thread whose session id has no thread map.
    pub(crate) fn start_on_thread(
        ctx: &crate::core::runtime::DispatchCtx,
        thread_id: &str,
    ) -> Self {
        let tenant_key = tenant_key(ctx);
        let lease = gate_for(&tenant_key)
            .enter(TenantThreadKey::new(tenant_key, thread_id))
            .expect("background_delivery parent busy gate poisoned");
        Self { _lease: lease }
    }
}

/// Is any in-flight turn of the calling profile running on `thread_id`?
pub(crate) fn is_busy(ctx: &crate::core::runtime::DispatchCtx, thread_id: &str) -> bool {
    let tenant_key = tenant_key(ctx);
    gate_for(&tenant_key).is_busy_scoped(&TenantThreadKey::new(tenant_key, thread_id))
}

/// Forget every in-flight turn of the calling profile on `thread_id`.
pub(crate) fn clear_busy_for_thread(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
) -> usize {
    let tenant_key = tenant_key(ctx);
    gate_for(&tenant_key).clear(&TenantThreadKey::new(tenant_key, thread_id))
}
