use super::ApprovalScope;
use std::sync::{mpsc, Arc};

#[test]
fn closing_is_observable_before_an_accepted_registration_finishes() {
    let scope = Arc::new(ApprovalScope::default());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let registering = scope.clone();
    let registration = std::thread::spawn(move || {
        registering.with_open(|| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
    });
    entered_rx.recv().unwrap();
    let removing = scope.clone();
    let removal = std::thread::spawn(move || removing.close("agent_removed"));
    let (observed_tx, observed_rx) = mpsc::channel();
    let observing = scope.clone();
    let observation = std::thread::spawn(move || {
        while !observing.is_closed() {
            std::thread::yield_now();
        }
        observed_tx.send(()).unwrap();
    });
    // The registration stays suspended throughout this observation. A blocking
    // is_closed cannot report removal until the registration is released.
    let visible = observed_rx.recv_timeout(std::time::Duration::from_secs(2));
    release_tx.send(()).unwrap();
    assert_eq!(registration.join().unwrap(), Ok(()));
    removal.join().unwrap();
    observation.join().unwrap();
    assert!(
        visible.is_ok(),
        "removal stayed invisible behind registration"
    );
}

#[test]
fn removal_refuses_late_registration_without_creating_a_pending_row() {
    let scope = ApprovalScope::default();
    assert!(!scope.is_closed());
    scope.close("agent_removed");
    assert!(scope.is_closed());
    let mut created = false;
    let result = scope.with_open(|| created = true);
    assert_eq!(result, Err("agent_removed".to_owned()));
    assert!(!created);
}

#[test]
fn accepted_registration_holds_the_removal_barrier_until_it_finishes() {
    let scope = Arc::new(ApprovalScope::default());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let registering = scope.clone();
    let registration = std::thread::spawn(move || {
        registering.with_open(|| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
    });
    entered_rx.recv().unwrap();
    // Observe ownership directly while the accepted registration is suspended;
    // no scheduler delay or elapsed-time assertion can hide an unlocked barrier.
    let held = matches!(
        scope.closed.try_lock(),
        Err(std::sync::TryLockError::WouldBlock)
    );
    release_tx.send(()).unwrap();
    assert_eq!(registration.join().unwrap(), Ok(()));
    assert!(
        held,
        "registration released the removal barrier before finishing"
    );
    scope.close("agent_removed");
    assert_eq!(scope.with_open(|| ()), Err("agent_removed".to_owned()));
}

#[test]
fn a_reused_agent_id_has_a_fresh_scope_and_preserves_the_old_removal_reason() {
    let old = ApprovalScope::default();
    old.close("agent_removed");
    old.close("agent_dropped");
    assert_eq!(old.with_open(|| ()), Err("agent_removed".to_owned()));
    assert_eq!(ApprovalScope::default().with_open(|| 42), Ok(42));
}
