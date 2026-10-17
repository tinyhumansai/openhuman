//! One turn at a time per hosted profile.
//!
//! A profile's chat turns, relayed-message turns and scheduled cron runs all
//! work the same conversation state, memory and workspace, and the host cannot
//! order them from outside because the cron sweep runs inside OpenHuman. Each
//! of them therefore takes the profile's gate for the whole turn
//! ([`acquire`]): turns of one profile queue behind each other, turns of
//! different profiles never contend.
//!
//! Work with no profile (the desktop, single-user embedded agents) is not
//! gated.
//!
//! The gate is a process-wide table keyed by profile id; an entry is a bare
//! mutex that lives for the life of the process.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Held for the duration of one turn; dropping it lets the next one start.
pub type TurnGuard = OwnedMutexGuard<()>;

fn gates() -> &'static Mutex<HashMap<String, Arc<AsyncMutex<()>>>> {
    static GATES: OnceLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();
    GATES.get_or_init(Default::default)
}

/// Wait for `profile`'s turn gate.
pub async fn acquire_profile(profile: &str) -> TurnGuard {
    let gate = {
        let mut gates = gates().lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(gates.entry(profile.to_owned()).or_default())
    };
    gate.lock_owned().await
}

/// Wait for the gate of the profile the calling task runs for, or return
/// `None` at once when it runs for no profile.
pub async fn acquire(ctx: &crate::core::runtime::DispatchCtx) -> Option<TurnGuard> {
    let profile = ctx.tenant().ok()?.profile?;
    Some(acquire_profile(&profile).await)
}

#[cfg(test)]
#[path = "turn_gate_tests.rs"]
mod tests;
