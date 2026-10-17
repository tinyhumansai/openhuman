//! Which agent a just-completed cron job belonged to.
//!
//! `CronJobCompleted` names only the job, and a one-shot job
//! (`delete_after_run`) is already gone from the store by the time a
//! subscriber sees the event, so its owner cannot be looked up there. The
//! scheduler publishes the event inside the job's own task, which carries the
//! agent's context, and notes the owner here first; the notification bridge
//! takes it back out (`desktop::notifications`).
//!
//! In-process only — the bus delivers locally — and bounded: if no subscriber
//! drains it (the bridge is not registered), it is cleared rather than grown.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// Notes kept before the map is cleared outright.
const CAPACITY: usize = 1024;

/// Job id → the agent it ran as.
static OWNERS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

/// Notes that `job_id` just completed as the current context's agent, when
/// it ran as one. Call just before publishing `CronJobCompleted`.
pub fn note(ctx: &crate::core::runtime::DispatchCtx, job_id: &str) {
    let Some(agent) = ctx.tenant().ok().and_then(|tenant| tenant.agent) else {
        return;
    };
    let mut owners = OWNERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if owners.len() >= CAPACITY {
        owners.clear();
    }
    owners.insert(job_id.to_string(), agent);
}

/// The agent `job_id` completed as, if one was noted; removes the note.
pub fn take(job_id: &str) -> Option<String> {
    OWNERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(job_id)
}

#[cfg(test)]
#[path = "completion_owner_tests.rs"]
mod tests;
