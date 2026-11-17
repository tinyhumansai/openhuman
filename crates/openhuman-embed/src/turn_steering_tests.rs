use super::*;

fn queued_texts(steering: &TurnSteering) -> Vec<String> {
    steering
        .queue()
        .drain_now(QueueLane::Steer)
        .into_iter()
        .map(|queued| queued.text)
        .collect()
}

#[test]
fn steers_queue_in_order_on_the_steer_lane() {
    let steering = TurnSteering::default();
    steering.steer("first").unwrap();
    steering.clone().steer("second").unwrap();
    assert_eq!(queued_texts(&steering), ["first", "second"]);
}

#[test]
fn closing_a_scope_returns_what_is_still_queued_and_refuses_later_steers() {
    let steering = TurnSteering::default();
    let scope = SteeringScope::new(Some(steering.clone()));
    steering.steer("too late to apply").unwrap();
    assert_eq!(scope.close(), ["too late to apply"]);

    let refused = steering.steer("after the end").unwrap_err();
    assert_eq!(refused.text(), "after the end");
    assert_eq!(refused.into_text(), "after the end");
    // A successful turn reported its leftovers already.
    assert!(steering.take_unapplied().is_empty());
}

#[test]
fn a_turn_that_ends_without_closing_keeps_its_steers_for_the_host() {
    let steering = TurnSteering::default();
    let scope = SteeringScope::new(Some(steering.clone()));
    steering.steer("never applied").unwrap();
    // Nothing to take while the turn is still running.
    assert!(steering.take_unapplied().is_empty());
    drop(scope);

    assert!(steering.steer("refused").is_err());
    assert_eq!(steering.take_unapplied(), ["never applied"]);
    assert!(steering.take_unapplied().is_empty());
}

#[test]
fn a_steer_put_back_after_the_turn_ended_is_still_taken() {
    // A cancelled turn can still be unwinding when it is marked finished; a
    // steer it puts back afterwards is not lost.
    let steering = TurnSteering::default();
    drop(SteeringScope::new(Some(steering.clone())));
    steering.queue().push_now(
        QueueLane::Steer,
        steer_message("late", "requeued late", "", ""),
    );
    assert_eq!(steering.take_unapplied(), ["requeued late"]);
}

#[test]
fn a_scope_without_a_handle_has_no_queue_and_nothing_to_return() {
    let scope = SteeringScope::new(None);
    assert!(scope.queue().is_none());
    assert!(scope.close().is_empty());
}

#[test]
fn debug_reports_state_without_message_text() {
    let steering = TurnSteering::default();
    steering.steer("private words").unwrap();
    let rendered = format!("{steering:?}");
    assert!(rendered.contains("finished: false"), "{rendered}");
    assert!(!rendered.contains("private words"), "{rendered}");
}
