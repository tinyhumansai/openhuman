use super::*;

// `sentry::test::with_captured_events` runs the body inside a Hub backed
// by the `TestTransport`, so `scope.set_user` is observable on subsequent
// events without needing a real DSN.
#[test]
fn bind_attaches_user_id_to_captured_events() {
    let events = sentry::test::with_captured_events(|| {
        bind("507f1f77bcf86cd799439011");
        sentry::capture_message("after bind", sentry::Level::Info);
    });
    assert_eq!(events.len(), 1);
    let user = events[0].user.as_ref().expect("event.user populated");
    assert_eq!(user.id.as_deref(), Some("507f1f77bcf86cd799439011"));
}

#[test]
fn clear_drops_previous_user_from_subsequent_events() {
    let events = sentry::test::with_captured_events(|| {
        bind("507f1f77bcf86cd799439011");
        clear();
        sentry::capture_message("after clear", sentry::Level::Info);
    });
    assert_eq!(events.len(), 1);
    assert!(
        events[0].user.is_none(),
        "scope user must be cleared by clear(): {:?}",
        events[0].user
    );
}

#[test]
fn bind_empty_id_is_treated_as_clear() {
    let events = sentry::test::with_captured_events(|| {
        bind("507f1f77bcf86cd799439011");
        bind("   ");
        sentry::capture_message("after empty bind", sentry::Level::Info);
    });
    assert_eq!(events.len(), 1);
    assert!(
        events[0].user.is_none(),
        "empty/whitespace id must clear scope user, got {:?}",
        events[0].user
    );
}

#[test]
fn second_bind_overwrites_first_user() {
    let events = sentry::test::with_captured_events(|| {
        bind("user-a");
        bind("user-b");
        sentry::capture_message("after rebind", sentry::Level::Info);
    });
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].user.as_ref().and_then(|u| u.id.as_deref()),
        Some("user-b")
    );
}
