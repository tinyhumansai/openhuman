//! In-memory status holder for the harness-init run.
//!
//! Mirrors the `STATUS_TABLE` pattern used by the local-inference installers:
//! a process-lifetime singleton guarded by a mutex. `update_step` is the single
//! mutation point and also publishes the matching `DomainEvent` so the event
//! bus stays in lockstep with the snapshot the RPC returns.

use std::sync::{Mutex, OnceLock};

use chrono::Utc;

use super::registry;
use super::types::{HarnessInitSnapshot, OverallState, StepState, StepStatus};
use crate::core::event_bus::{publish_global, DomainEvent};

static STATE: OnceLock<Mutex<HarnessInitSnapshot>> = OnceLock::new();

fn state() -> &'static Mutex<HarnessInitSnapshot> {
    STATE.get_or_init(|| Mutex::new(seed_snapshot()))
}

/// Build the initial snapshot: every registered step `Pending`, overall `Idle`.
fn seed_snapshot() -> HarnessInitSnapshot {
    let steps = registry::all_steps()
        .into_iter()
        .map(|s| StepStatus {
            id: s.id.to_string(),
            label: s.label.to_string(),
            required: s.required,
            state: StepState::Pending,
            message: None,
            percent: None,
            updated_at: None,
        })
        .collect();
    HarnessInitSnapshot {
        overall: OverallState::Idle,
        steps,
        started_at: None,
        finished_at: None,
    }
}

/// Clone the current snapshot for the RPC layer.
pub fn snapshot() -> HarnessInitSnapshot {
    state().lock().unwrap().clone()
}

/// Set the overall lifecycle state, stamping start/finish timestamps once.
pub fn set_overall(overall: OverallState) {
    let mut guard = state().lock().unwrap();
    let now = Utc::now().to_rfc3339();
    match overall {
        OverallState::Running if guard.started_at.is_none() => {
            guard.started_at = Some(now.clone());
        }
        OverallState::Done | OverallState::Failed => {
            guard.finished_at = Some(now.clone());
        }
        _ => {}
    }
    guard.overall = overall;
}

/// Update one step's state and publish a `HarnessInitProgress` event.
pub fn update_step(id: &str, step_state: StepState, message: Option<String>, percent: Option<u8>) {
    {
        let mut guard = state().lock().unwrap();
        if let Some(step) = guard.steps.iter_mut().find(|s| s.id == id) {
            step.state = step_state;
            step.message = message.clone();
            step.percent = percent;
            step.updated_at = Some(Utc::now().to_rfc3339());
        } else {
            log::warn!("[harness_init] update_step for unknown id={id}");
            return;
        }
    }
    publish_global(DomainEvent::HarnessInitProgress {
        step_id: id.to_string(),
        state: serde_json::to_value(step_state)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default(),
        message,
        percent,
    });
}

/// Publish the terminal event once the run finishes.
pub fn publish_completed(overall: OverallState, failed_required: bool) {
    let overall_str = serde_json::to_value(overall)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    publish_global(DomainEvent::HarnessInitCompleted {
        overall: overall_str,
        failed_required,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_snapshot_starts_idle_with_pending_steps() {
        let snap = seed_snapshot();
        assert_eq!(snap.overall, OverallState::Idle);
        assert!(!snap.steps.is_empty());
        assert!(snap.steps.iter().all(|s| s.state == StepState::Pending));
        assert!(snap.started_at.is_none());
        assert!(snap.finished_at.is_none());
    }
}
