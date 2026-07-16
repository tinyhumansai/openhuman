//! Emergency-stop RPC operations: engage / resume / read the switch, plus the
//! best-effort side effects (stop the a11y session, cascade-deny pending
//! approvals) and event publication.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::rpc::RpcOutcome;

use super::state::EmergencyStop;
use super::types::HaltState;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Engage the kill switch: set the flag, then best-effort stop the a11y
/// session and cascade-deny pending approvals, then publish `AutomationHalted`.
/// Idempotent. Side-effect failures are logged but never fail the RPC — the
/// primary invariant (flag set → actions blocked) does not depend on them.
pub async fn emergency_stop(reason: Option<String>, source: &str) -> RpcOutcome<HaltState> {
    tracing::warn!(source, reason = ?reason, "[rpc:emergency_stop] entry — engaging kill switch");
    let stop = EmergencyStop::init_global();
    stop.engage(reason.clone(), source, now_ms());

    // Best-effort: stop the accessibility session so any in-flight click/type loop halts.
    let a11y = crate::openhuman::screen_intelligence::global_engine()
        .disable(Some("emergency_stop".to_string()))
        .await;
    tracing::info!(
        active = a11y.active,
        "[emergency] accessibility session stopped"
    );

    // Best-effort: cascade-deny every pending approval so parked tool calls fail
    // closed. `list_pending`/`decide` do synchronous SQLite I/O, so run them on a
    // blocking thread rather than stalling a tokio worker.
    let denied = tokio::task::spawn_blocking(cascade_deny_pending)
        .await
        .unwrap_or_else(|err| {
            tracing::warn!(error = %err, "[emergency] cascade-deny task join failed");
            0
        });
    tracing::info!(denied, "[emergency] cascade-denied pending approvals");

    publish_global(DomainEvent::AutomationHalted {
        reason,
        source: source.to_string(),
    });

    let snap = stop.snapshot();
    RpcOutcome::single_log(
        snap,
        format!("[emergency] halted (source={source}, denied={denied})"),
    )
}

/// Deny all pending approvals. Returns how many were denied. Best-effort:
/// a per-row error is logged and skipped.
fn cascade_deny_pending() -> usize {
    use crate::openhuman::approval::{ApprovalDecision, ApprovalGate};
    let Some(gate) = ApprovalGate::try_global() else {
        return 0;
    };
    let rows = match gate.list_pending() {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "[emergency] list_pending failed during cascade-deny");
            return 0;
        }
    };
    let mut denied = 0;
    for row in rows {
        match gate.decide(&row.request_id, ApprovalDecision::Deny) {
            Ok(_) => denied += 1,
            Err(err) => {
                tracing::warn!(request_id = %row.request_id, error = %err, "[emergency] deny failed")
            }
        }
    }
    denied
}

/// Clear the kill switch and publish `AutomationResumed`. Idempotent.
pub async fn emergency_resume(source: &str) -> RpcOutcome<HaltState> {
    tracing::info!(
        source,
        "[rpc:emergency_resume] entry — clearing kill switch"
    );
    let stop = EmergencyStop::init_global();
    stop.clear();
    publish_global(DomainEvent::AutomationResumed {
        source: source.to_string(),
    });
    RpcOutcome::single_log(
        stop.snapshot(),
        format!("[emergency] resumed (source={source})"),
    )
}

/// Read the current switch state.
pub async fn emergency_status() -> RpcOutcome<HaltState> {
    let snap = EmergencyStop::try_global()
        .map(|s| s.snapshot())
        .unwrap_or_default();
    tracing::debug!(engaged = snap.engaged, "[rpc:emergency_status] exit");
    RpcOutcome::new(snap, vec![])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::emergency_stop::state::EMERGENCY_TEST_GUARD;

    #[tokio::test]
    async fn stop_sets_flag_and_status_reports_engaged() {
        let _g = EMERGENCY_TEST_GUARD
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Panic-safe cleanup: clear the process-global switch on drop — even if
        // an assertion panics between engage and the end of the test — so an
        // engaged state can't leak into a later test sharing the binary (#4600
        // review). Supersedes the manual `emergency_resume` reset below.
        let _reset = crate::openhuman::emergency_stop::state::ClearEmergencyOnDrop;
        let out = emergency_stop(Some("user".into()), "user").await;
        assert!(out.value.engaged);
        let status = emergency_status().await;
        assert!(status.value.engaged);
        assert_eq!(status.value.source.as_deref(), Some("user"));
        // reset for other tests sharing the process-global switch
        let _ = emergency_resume("user").await;
    }

    #[tokio::test]
    async fn resume_clears_flag() {
        let _g = EMERGENCY_TEST_GUARD
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Panic-safe cleanup (see the note in the first test) — clears the
        // process-global switch on drop so a mid-test panic can't leak state.
        let _reset = crate::openhuman::emergency_stop::state::ClearEmergencyOnDrop;
        let _ = emergency_stop(None, "user").await;
        let out = emergency_resume("user").await;
        assert!(!out.value.engaged);
        assert!(!emergency_status().await.value.engaged);
    }

    #[tokio::test]
    async fn stop_is_idempotent() {
        let _g = EMERGENCY_TEST_GUARD
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Panic-safe cleanup (see the note in the first test) — clears the
        // process-global switch on drop so a mid-test panic can't leak state.
        let _reset = crate::openhuman::emergency_stop::state::ClearEmergencyOnDrop;
        let _ = emergency_stop(Some("a".into()), "user").await;
        let out = emergency_stop(Some("b".into()), "system").await;
        assert!(out.value.engaged);
        assert_eq!(out.value.reason.as_deref(), Some("b"));
        let _ = emergency_resume("user").await;
    }
}
