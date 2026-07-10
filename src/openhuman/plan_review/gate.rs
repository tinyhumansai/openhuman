//! `PlanReviewGate` — parks a live interactive turn on a plan the user must
//! review before execution.
//!
//! Flow (mirrors [`crate::openhuman::approval::ApprovalGate`], in-memory):
//! 1. The orchestrator calls the `request_plan_review` tool after laying out a
//!    thread-scoped plan. The tool calls [`PlanReviewGate::request_review`].
//! 2. The gate registers a `oneshot::Sender` keyed by `request_id`, publishes
//!    [`DomainEvent::PlanReviewRequested`] (bridged to the `plan_review_request`
//!    socket event), and parks the turn on the receiver.
//! 3. The UI's `PlanReviewCard` calls `plan_review_decide` (RPC) →
//!    [`PlanReviewGate::decide`] → sends the resolution on the oneshot.
//! 4. The parked turn wakes with [`PlanReviewResolution`] and the tool returns
//!    a result that tells the agent to proceed / stop / revise.
//!
//! On TTL or a dropped sender the gate resolves to `Reject` — fail-closed, so a
//! plan never executes without an explicit approval.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::core::event_bus::{publish_global, DomainEvent};

use super::types::PlanReviewResolution;

/// How long the gate parks a turn before timing out → `Reject`. Matches the
/// default approval TTL (10 min) — long enough for a human to read the plan.
const DEFAULT_PLAN_REVIEW_TTL: Duration = Duration::from_secs(60 * 10);

/// In-memory registry of parked plan reviews. Process-global singleton (see
/// [`global`]); no persistence — a parked interactive turn cannot resume
/// across a restart, so an orphaned review has nothing to recover.
pub struct PlanReviewGate {
    ttl: Duration,
    waiters: Mutex<HashMap<String, oneshot::Sender<PlanReviewResolution>>>,
    /// Newest parked `request_id` per thread, so a typed reply or a UI action
    /// that only knows the thread can resolve the latest review.
    thread_to_request: Mutex<HashMap<String, String>>,
}

impl PlanReviewGate {
    fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            waiters: Mutex::new(HashMap::new()),
            thread_to_request: Mutex::new(HashMap::new()),
        }
    }

    /// Park the current turn on a plan review and block until the user decides
    /// or the TTL elapses. `summary` is a one-line description; `steps` are the
    /// ordered plan items shown in the review card. `thread_id` / `client_id`
    /// route the surface to the originating chat (absent → no routable surface,
    /// so the park TTL-rejects).
    pub async fn request_review(
        &self,
        thread_id: Option<String>,
        client_id: Option<String>,
        summary: String,
        steps: Vec<String>,
    ) -> PlanReviewResolution {
        let request_id = format!("plan-{}", Uuid::new_v4());
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().insert(request_id.clone(), tx);
        if let Some(tid) = thread_id.clone() {
            self.thread_to_request
                .lock()
                .insert(tid, request_id.clone());
        }

        // RAII cleanup: remove the waiter + thread mapping on ANY exit path,
        // including when the parked future is cancelled/dropped before
        // `rx.await` completes (turn cancel, supervisor shutdown). Without this,
        // a cancelled review would leak a `waiters` / `thread_to_request` entry
        // that could be re-decided against a dead turn.
        let _guard = ParkGuard {
            gate: self,
            request_id: request_id.clone(),
            thread_id: thread_id.clone(),
        };

        tracing::info!(
            request_id = %request_id,
            thread_id = ?thread_id,
            steps = steps.len(),
            "[plan_review::gate] parking turn for plan review"
        );

        publish_global(DomainEvent::PlanReviewRequested {
            request_id: request_id.clone(),
            thread_id: thread_id.clone(),
            client_id,
            summary,
            steps,
        });

        let resolution = match tokio::time::timeout(self.ttl, rx).await {
            Ok(Ok(resolution)) => resolution,
            // Sender dropped (decided elsewhere / shutdown) or TTL elapsed →
            // fail closed: never execute a plan the user didn't approve.
            Ok(Err(_)) | Err(_) => {
                tracing::warn!(
                    request_id = %request_id,
                    "[plan_review::gate] review unresolved (timeout/dropped) → reject"
                );
                PlanReviewResolution::Reject
            }
        };

        // `_guard` drops here on the normal path too (cleanup is idempotent with
        // `decide`, which already removed the waiter).
        publish_global(DomainEvent::PlanReviewDecided {
            request_id: request_id.clone(),
            decision: resolution.as_str().to_string(),
        });
        tracing::info!(
            request_id = %request_id,
            decision = resolution.as_str(),
            "[plan_review::gate] review resolved"
        );
        resolution
    }

    /// Resolve a parked review by `request_id`. Returns `true` when a waiter was
    /// woken; `false` when the id is unknown (already decided / expired).
    pub fn decide(&self, request_id: &str, resolution: PlanReviewResolution) -> bool {
        let sender = self.waiters.lock().remove(request_id);
        match sender {
            Some(tx) => tx.send(resolution).is_ok(),
            None => {
                tracing::debug!(
                    request_id = %request_id,
                    "[plan_review::gate] decide for unknown/expired request"
                );
                false
            }
        }
    }

    /// Resolve the newest parked review on `thread_id` (typed-reply / thread-
    /// scoped path). Returns `true` when a waiter was woken.
    pub fn decide_by_thread(&self, thread_id: &str, resolution: PlanReviewResolution) -> bool {
        let request_id = self.thread_to_request.lock().get(thread_id).cloned();
        match request_id {
            Some(id) => self.decide(&id, resolution),
            None => false,
        }
    }
}

/// Removes a parked review's registry entries on drop — covers both the normal
/// return and cancellation of the parked future.
struct ParkGuard<'a> {
    gate: &'a PlanReviewGate,
    request_id: String,
    thread_id: Option<String>,
}

impl Drop for ParkGuard<'_> {
    fn drop(&mut self) {
        self.gate.waiters.lock().remove(&self.request_id);
        if let Some(tid) = &self.thread_id {
            let mut map = self.gate.thread_to_request.lock();
            if map.get(tid) == Some(&self.request_id) {
                map.remove(tid);
            }
        }
    }
}

/// Process-global plan-review gate.
pub fn global() -> &'static PlanReviewGate {
    static GATE: OnceLock<PlanReviewGate> = OnceLock::new();
    GATE.get_or_init(|| PlanReviewGate::new(DEFAULT_PLAN_REVIEW_TTL))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn approve_resolves_parked_turn() {
        let gate = PlanReviewGate::new(Duration::from_secs(5));
        let gate = std::sync::Arc::new(gate);
        let g2 = gate.clone();
        let parked = tokio::spawn(async move {
            g2.request_review(
                Some("t1".into()),
                Some("c1".into()),
                "Ship it".into(),
                vec!["step one".into()],
            )
            .await
        });
        // Let the park register the waiter.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(gate.decide_by_thread("t1", PlanReviewResolution::Approve));
        assert_eq!(parked.await.unwrap(), PlanReviewResolution::Approve);
    }

    #[tokio::test]
    async fn revise_carries_feedback_back() {
        let gate = std::sync::Arc::new(PlanReviewGate::new(Duration::from_secs(5)));
        let g2 = gate.clone();
        let parked = tokio::spawn(async move {
            g2.request_review(Some("t2".into()), None, "Plan".into(), vec![])
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(gate.decide_by_thread(
            "t2",
            PlanReviewResolution::Revise {
                feedback: "add a test step".into(),
            },
        ));
        assert_eq!(
            parked.await.unwrap(),
            PlanReviewResolution::Revise {
                feedback: "add a test step".into(),
            }
        );
    }

    #[tokio::test]
    async fn timeout_fails_closed_to_reject() {
        let gate = PlanReviewGate::new(Duration::from_millis(40));
        let resolution = gate
            .request_review(Some("t3".into()), None, "Plan".into(), vec![])
            .await;
        assert_eq!(resolution, PlanReviewResolution::Reject);
        // The waiter is cleaned up after timeout.
        assert!(!gate.decide_by_thread("t3", PlanReviewResolution::Approve));
    }

    #[tokio::test]
    async fn decide_unknown_request_is_false() {
        let gate = PlanReviewGate::new(Duration::from_secs(5));
        assert!(!gate.decide("nope", PlanReviewResolution::Approve));
    }

    #[tokio::test]
    async fn cancelled_park_cleans_up_waiter() {
        // A parked review whose future is dropped (turn cancel) before it
        // resolves must not leak its waiter / thread mapping.
        let gate = std::sync::Arc::new(PlanReviewGate::new(Duration::from_secs(30)));
        let g2 = gate.clone();
        let handle = tokio::spawn(async move {
            g2.request_review(Some("t-drop".into()), None, "Plan".into(), vec![])
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        handle.abort();
        let _ = handle.await;
        // The drop guard removed the entry, so there is nothing left to decide.
        assert!(!gate.decide_by_thread("t-drop", PlanReviewResolution::Approve));
    }
}
