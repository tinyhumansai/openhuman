use super::*;
use std::sync::Arc;

#[test]
fn core_markers_count_without_a_registered_predicate() {
    for msg in [
        "Session expired. Please log in again.",
        "SESSION_EXPIRED: backend rejected session token",
        "no backend session token; run auth_store_session first",
        "session JWT required",
    ] {
        assert!(is_session_expired_error(msg), "{msg}");
    }
    // Host wire shapes are the host's to claim.
    assert!(!is_session_expired_error(
        "GET /teams failed (401 Unauthorized): {}"
    ));
    assert!(!is_session_expired_error(
        "backend returned 401 Unauthorized"
    ));
}

#[test]
fn a_registered_predicate_extends_the_core_markers() {
    register_session_expiry_predicate("test", Arc::new(|msg: &str| msg.starts_with("HOST-401 ")));
    assert!(is_session_expired_error("HOST-401 on /x"));
    assert!(is_session_expired_error("Session expired"));
    assert!(!is_session_expired_error("HOST-500 on /x"));
    assert!(unregister_session_expiry_predicate("test"));
    assert!(!is_session_expired_error("HOST-401 on /x"));
}

#[test]
fn a_registered_classifier_demotes_host_sentinels() {
    let sentinel = "HOST_BILLING: top up to continue";
    assert_eq!(super::super::expected_error_kind(sentinel), None);
    register_expected_error_classifier(
        "test",
        Arc::new(|msg: &str| {
            msg.starts_with("HOST_BILLING:")
                .then_some(ExpectedErrorKind::BudgetExhausted)
        }),
    );
    assert_eq!(
        super::super::expected_error_kind(sentinel),
        Some(ExpectedErrorKind::BudgetExhausted)
    );
    assert!(unregister_expected_error_classifier("test"));
    assert_eq!(super::super::expected_error_kind(sentinel), None);
}

#[cfg(feature = "crash-reporting")]
#[test]
fn a_registered_crash_noise_filter_is_named_in_the_verdict() {
    fn is_marked(event: &sentry::protocol::Event<'static>) -> bool {
        event.message.as_deref() == Some("host noise")
    }
    let noise = sentry::protocol::Event {
        message: Some("host noise".into()),
        ..Default::default()
    };
    assert_eq!(registered_crash_noise(&noise), None);
    register_crash_noise_filter(CrashNoiseFilter {
        name: "test-host-noise",
        matches: is_marked,
    });
    assert_eq!(registered_crash_noise(&noise), Some("test-host-noise"));
    assert_eq!(
        registered_crash_noise(&sentry::protocol::Event::default()),
        None
    );
    assert!(unregister_crash_noise_filter("test-host-noise"));
}
