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

    // Best-effort: cascade-deny every pending approval so parked tool calls fail closed.
    let denied = cascade_deny_pending();
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
        let _ = emergency_stop(Some("a".into()), "user").await;
        let out = emergency_stop(Some("b".into()), "system").await;
        assert!(out.value.engaged);
        assert_eq!(out.value.reason.as_deref(), Some("b"));
        let _ = emergency_resume("user").await;
    }

    /// `cascade_deny_pending` loop body executes when an `ApprovalGate` is
    /// installed and pending rows exist.
    ///
    /// The previous ops tests run without a global `ApprovalGate`, so
    /// `cascade_deny_pending()` returns 0 early and its loop body (list_pending
    /// → decide(Deny) per row) is never exercised. This test installs a real
    /// gate backed by a temporary workspace and inserts a pending approval row
    /// directly via the store, then calls `emergency_stop` and asserts the row
    /// was denied by the cascade.
    ///
    /// **Isolation note**: `ApprovalGate::init_global` is `OnceLock`-guarded;
    /// once set it persists for the process lifetime. After this test the gate
    /// lives on with its TempDir workspace deleted — subsequent `cascade_deny`
    /// calls hit an empty/recreated DB (SQLite `create_dir_all` + open creates a
    /// fresh file) and return 0, which is safe because those callers never assert
    /// on the deny count.
    #[tokio::test]
    async fn cascade_deny_pending_loop_denies_all_pending_when_gate_installed() {
        // Serialize against every test that touches the process-global
        // EmergencyStop (mirrors the pattern in the sibling tests above).
        let _g = EMERGENCY_TEST_GUARD
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        // Install the process-global ApprovalGate with a test-local workspace.
        // No other unit test calls ApprovalGate::init_global, so this is the
        // first install. The gate persists for the process lifetime; see the
        // isolation note above.
        let _dir = tempfile::TempDir::new().unwrap();
        let workspace_dir = _dir.path().to_path_buf();
        let session_id = format!("session-{}", uuid::Uuid::new_v4());
        let gate = crate::openhuman::approval::ApprovalGate::init_global(
            crate::openhuman::config::Config {
                workspace_dir: workspace_dir.clone(),
                ..crate::openhuman::config::Config::default()
            },
            &session_id,
        );

        // Insert a pending approval row directly via the store (bypassing the
        // intercept/park async flow, which would block until a decision arrives
        // or the TTL elapses). The row has a future expiry so it is not lazily
        // expired by list_pending before cascade_deny runs.
        let pending = crate::openhuman::approval::PendingApproval::new(
            "req-cascade-test",
            "cascade_test_tool",
            "cascade deny smoke test",
            serde_json::json!({}),
            Some(chrono::Utc::now() + chrono::Duration::minutes(10)),
        );
        crate::openhuman::approval::store::insert_pending(
            &crate::openhuman::config::Config {
                workspace_dir: workspace_dir.clone(),
                ..crate::openhuman::config::Config::default()
            },
            &pending,
            &session_id,
        )
        .unwrap();

        // Sanity: row is visible before the stop.
        assert_eq!(
            gate.list_pending().unwrap().len(),
            1,
            "test setup: pending row must be visible before emergency_stop"
        );

        // Engage the kill switch — cascade_deny_pending runs inside and
        // should decide(Deny) every pending row.
        let out = emergency_stop(Some("cascade test".into()), "user").await;
        assert!(
            out.value.engaged,
            "emergency_stop must engage the halt flag"
        );

        // The loop body exercised: the pending row is now decided (denied).
        assert!(
            gate.list_pending().unwrap().is_empty(),
            "cascade_deny_pending must deny all pending rows when an ApprovalGate is installed"
        );

        // Clear the global switch for subsequent tests.
        let _ = emergency_resume("user").await;
    }
}
