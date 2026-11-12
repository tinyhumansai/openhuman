//! Events their offers took for the caller's direct path: held while it
//! runs, dropped once it handled them, delivered when it did not.

use super::*;

#[tokio::test]
async fn an_event_handled_directly_is_not_delivered_again() {
    // Its offer was ambiguous and ran on the caller's direct path, yet the
    // module persisted it: the queued copy must not be triaged a second time.
    let event = trigger_event("deliver-direct-1");
    let claims = Arc::new(Claims::default());
    claims
        .take_direct(&event.dedupe_key())
        .unwrap()
        .finish(true);
    let triage = ScriptedTriage::new([decision(TriageAction::React)]);
    let deliverer = deliverer(&triage).with_claims(claims);
    let response = deliverer
        .deliver(turn(vec![event.clone()], Lane::Background, 1))
        .await;
    assert_eq!(response.status, DeliverStatus::Accepted);
    assert_eq!(triage.runs(), 0);
    assert!(triage.applied().is_empty());
    // A redelivery of the turn is not triaged either.
    let again = deliverer
        .deliver(turn(vec![event], Lane::Background, 2))
        .await;
    assert_eq!(again.status, DeliverStatus::Accepted);
    assert_eq!(triage.runs(), 0);
}

#[tokio::test]
async fn a_digest_leaves_out_only_the_events_handled_directly() {
    let direct = trigger_event("deliver-direct-2");
    let queued = trigger_event("deliver-direct-3");
    let claims = Arc::new(Claims::default());
    claims
        .take_direct(&direct.dedupe_key())
        .unwrap()
        .finish(true);
    let triage = ScriptedTriage::new([decision(TriageAction::React)]);
    let response = deliverer(&triage)
        .with_claims(claims)
        .deliver(turn(vec![direct, queued], Lane::Background, 1))
        .await;
    assert_eq!(response.status, DeliverStatus::Accepted);
    assert_eq!(triage.runs(), 1);
    assert_eq!(
        triage.until_applied(1, Duration::from_secs(5)).await.len(),
        1
    );
}

#[tokio::test]
async fn a_turn_whose_event_the_direct_path_is_running_comes_back_later() {
    // Neither dropped (the direct path may still fail) nor triaged again.
    let event = trigger_event("deliver-direct-4");
    let claims = Arc::new(Claims::default());
    let running = claims.take_direct(&event.dedupe_key()).unwrap();
    let triage = ScriptedTriage::new([decision(TriageAction::React)]);
    let deliverer = deliverer(&triage).with_claims(Arc::clone(&claims));
    let response = deliverer
        .deliver(turn(vec![event.clone()], Lane::Background, 1))
        .await;
    assert_eq!(
        response.status,
        DeliverStatus::Retry {
            after_ms: Some(DIRECT_WAIT_MS)
        }
    );
    assert_eq!(triage.runs(), 0);
    // The direct path failed: the queued copy is triaged on its redelivery.
    running.finish(false);
    let again = deliverer
        .deliver(turn(vec![event], Lane::Background, 2))
        .await;
    assert_eq!(again.status, DeliverStatus::Accepted);
    assert_eq!(triage.runs(), 1);
}
