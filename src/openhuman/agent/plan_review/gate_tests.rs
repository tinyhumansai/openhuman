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
