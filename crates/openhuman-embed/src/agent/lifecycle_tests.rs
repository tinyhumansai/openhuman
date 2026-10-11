use super::*;
use std::sync::Arc;

fn removed(outcome: Result<(), CoreError>) -> bool {
    matches!(outcome, Err(CoreError::AgentRemoved { ref agent_id, .. }) if agent_id == "a")
}

#[tokio::test]
async fn removal_refuses_turns_while_an_accepted_facade_decision_is_still_running() {
    let lifecycle = Arc::new(Lifecycle::new());
    let approvals = lifecycle.approval_state();
    let deciding = approvals.clone();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let decision = std::thread::spawn(move || {
        deciding.with_live(|| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            7
        })
    });
    entered_rx.recv().unwrap();
    let removing = lifecycle.clone();
    let removal = std::thread::spawn(move || removing.mark_removed("agent_removed"));
    // Closing is observable while the facade decision still owns its mutex;
    // the watch notification must wait for that accepted decision to finish.
    let closing = tokio::time::timeout(Duration::from_secs(2), async {
        while !lifecycle.approval_scope().is_closed() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let ran = AtomicBool::new(false);
    let outcome = lifecycle
        .admit("a", "test", async {
            ran.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await;
    let notified = *lifecycle.removed().borrow();
    release_tx.send(()).unwrap();
    assert_eq!(decision.join().unwrap(), Some(7));
    assert!(removal.join().unwrap());
    assert!(
        closing.is_ok(),
        "closing waited for the facade decision mutex"
    );
    assert!(
        !notified,
        "accepted decision was not allowed to finish before notification"
    );
    assert!(removed(outcome));
    assert!(!ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_live_agent_runs_its_turns() {
    let lifecycle = Lifecycle::new();
    let outcome = lifecycle.admit("a", "test", async { Ok(7) }).await;
    assert_eq!(outcome.unwrap(), 7);
    assert!(lifecycle.wait_idle(Duration::from_millis(10)).await);
}

#[tokio::test]
async fn a_removed_agent_refuses_new_turns() {
    let lifecycle = Lifecycle::new();
    lifecycle.mark_removed("agent_removed");
    let ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ran);
    let outcome = lifecycle
        .admit("a", "test", async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await;
    assert!(removed(outcome));
    assert!(!ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn removal_ends_a_turn_in_flight_and_the_agent_goes_idle() {
    let lifecycle = Arc::new(Lifecycle::new());
    let running = Arc::clone(&lifecycle);
    let turn = tokio::spawn(async move {
        running
            .admit("a", "test", std::future::pending::<Result<(), CoreError>>())
            .await
    });
    while lifecycle.in_flight.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert!(!lifecycle.wait_idle(Duration::from_millis(10)).await);

    lifecycle.mark_removed("agent_removed");

    assert!(removed(turn.await.expect("turn task")));
    assert!(lifecycle.wait_idle(Duration::from_secs(1)).await);
}

#[test]
fn teardown_is_claimed_once() {
    let lifecycle = Lifecycle::new();
    assert!(lifecycle.begin_teardown());
    assert!(!lifecycle.begin_teardown());
}

#[tokio::test]
async fn a_claimed_removal_refuses_turns_before_approvals_are_settled() {
    let lifecycle = Arc::new(Lifecycle::new());
    let removing = lifecycle.clone();
    let (entered, callback_entered) = std::sync::mpsc::channel();
    let (release, callback_release) = std::sync::mpsc::channel();
    let removal = std::thread::spawn(move || {
        removing.mark_removed_with("agent_removed", || {
            entered.send(()).unwrap();
            callback_release.recv().unwrap();
        })
    });
    callback_entered.recv().unwrap();
    let ran = AtomicBool::new(false);
    let outcome = lifecycle
        .admit("a", "test", async {
            ran.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await;
    release.send(()).unwrap();
    assert!(removal.join().unwrap());
    assert!(removed(outcome));
    assert!(
        !ran.load(Ordering::SeqCst),
        "removal admitted a new turn while settling approvals"
    );
}

#[test]
fn a_claimed_removal_refuses_approval_decisions_before_denial_finishes() {
    let lifecycle = Arc::new(Lifecycle::new());
    let shared_scope = lifecycle.approval_scope();
    let approvals = crate::Approvals::new("a", lifecycle.removed(), lifecycle.approval_state());
    let removing = lifecycle.clone();
    let (entered, callback_entered) = std::sync::mpsc::channel();
    let (release, callback_release) = std::sync::mpsc::channel();
    let removal = std::thread::spawn(move || {
        removing.mark_removed_with("agent_removed", || {
            entered.send(()).unwrap();
            callback_release.recv().unwrap();
        })
    });
    callback_entered.recv().unwrap();
    let core_closed = shared_scope.is_closed();
    let outcome = approvals.decide("pending-request", crate::ApprovalDecision::ApproveOnce);
    release.send(()).unwrap();
    assert!(removal.join().unwrap());
    assert!(
        core_closed,
        "the shared gate remained open after removal was claimed"
    );
    assert!(
        matches!(outcome, Err(crate::ApprovalsError::NotFound(_))),
        "{outcome:?}"
    );
}

#[test]
fn accepted_approval_decisions_hold_the_removal_claim_barrier() {
    let state = Arc::new(ApprovalState::default());
    let deciding = state.clone();
    let (entered, decision_entered) = std::sync::mpsc::channel();
    let (release, decision_release) = std::sync::mpsc::channel();
    let decision = std::thread::spawn(move || {
        deciding.with_live(|| {
            entered.send(()).unwrap();
            decision_release.recv().unwrap();
            7
        })
    });
    decision_entered.recv().unwrap();
    let held = matches!(
        state.decisions.try_lock(),
        Err(std::sync::TryLockError::WouldBlock)
    );
    release.send(()).unwrap();
    assert_eq!(decision.join().unwrap(), Some(7));
    assert!(
        held,
        "removal could claim while a decision was still being applied"
    );
    assert!(state.claim_removal("agent_removed"));
    let mut ran = false;
    assert_eq!(state.with_live(|| ran = true), None);
    assert!(!ran);
}
